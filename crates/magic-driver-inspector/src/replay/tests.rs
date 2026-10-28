use std::io::Cursor;

use md_core::profile::MAGIC_MOUSE_2_BT;
use md_core::recording::RecordingWriter;
use md_core::testkit::scenarios;
use md_trace::{DecisionReason, GestureState, OutputEvent};

use super::*;
use crate::view::fold;

fn header(profile_id: &str) -> RecordingHeader {
    RecordingHeader {
        profile_id: profile_id.to_owned(),
        magic_driver_version: "0.1.0".to_owned(),
        kernel_version: "synthetic".to_owned(),
        start_unix_s: 0,
    }
}

fn recording_text(profile_id: &str, reports: &[(Micros, Vec<u8>)]) -> Vec<u8> {
    let mut writer = RecordingWriter::new(Vec::new(), &header(profile_id)).unwrap();
    for (t, bytes) in reports {
        writer.write_report(*t, bytes).unwrap();
    }
    writer.flush().unwrap();
    writer.into_inner()
}

fn defaults() -> Config {
    Config::for_profile(&MAGIC_MOUSE_2_BT)
}

fn output_events(timeline: &Timeline) -> Vec<(u64, OutputEvent)> {
    timeline
        .entries()
        .filter_map(|entry| match &entry.record {
            TraceRecord::Output(output) => Some((output.t, output.output.clone())),
            _ => None,
        })
        .collect()
}

#[test]
fn a_recording_is_read_with_its_profile_and_reports() {
    let reports = scenarios::flick().timed();
    let recording =
        read_recording(Cursor::new(recording_text("magic-mouse-2-bt", &reports))).unwrap();
    assert_eq!(recording.header, header("magic-mouse-2-bt"));
    assert_eq!(recording.profile.id, MAGIC_MOUSE_2_BT.id);
    assert_eq!(recording.reports, reports);
}

#[test]
fn unknown_profiles_and_broken_recordings_are_errors() {
    let unknown = read_recording(Cursor::new(recording_text("trackpad-9", &[])));
    assert!(matches!(unknown, Err(ReplayError::UnknownProfile(id)) if id == "trackpad-9"));
    let mut broken = recording_text("magic-mouse-2-bt", &[]);
    broken.extend_from_slice(b"12 zz\n");
    assert!(matches!(
        read_recording(Cursor::new(broken)),
        Err(ReplayError::Recording(_))
    ));
    assert!(matches!(
        read_recording(Cursor::new(Vec::new())),
        Err(ReplayError::Recording(_))
    ));
}

#[test]
fn load_recording_needs_a_regular_file() {
    let dir = tempfile::tempdir().unwrap();
    assert!(matches!(
        load_recording(&dir.path().join("missing.rec")),
        Err(ReplayError::Open(_))
    ));
    assert!(matches!(
        load_recording(dir.path()),
        Err(ReplayError::NotAFile)
    ));
    let path = dir.path().join("flick.rec");
    std::fs::write(
        &path,
        recording_text("magic-mouse-2-bt", &scenarios::flick().timed()),
    )
    .unwrap();
    assert_eq!(
        load_recording(&path).unwrap().reports.len(),
        scenarios::flick().len()
    );
}

#[test]
fn the_offline_engine_produces_the_same_outputs_as_the_engine_driver() {
    for (name, scenario) in scenarios::all() {
        let reports = scenario.timed();
        let timeline = run_engine(&MAGIC_MOUSE_2_BT, &defaults(), &reports);
        let mut engine = Engine::new(&MAGIC_MOUSE_2_BT, defaults(), Mode::Normal);
        let expected: Vec<(u64, OutputEvent)> = scenarios::drive(&mut engine, &reports)
            .into_iter()
            .filter_map(|(t, output)| {
                md_core::trace::output_record(t, &output).map(|record| match record {
                    TraceRecord::Output(output) => (output.t, output.output),
                    other => panic!("{other:?}"),
                })
            })
            .collect();
        assert_eq!(output_events(&timeline), expected, "{name}");
        assert_eq!(timeline.report_count(), reports.len(), "{name}");
        assert!(
            matches!(
                timeline.entries().next().map(|entry| &entry.record),
                Some(TraceRecord::Report(_)) | None
            ),
            "{name}"
        );
    }
}

#[test]
fn timers_after_the_last_report_are_played_out() {
    let full = scenarios::two_finger_double_tap().timed();
    let report_times: Vec<u64> = full.iter().map(|(t, _)| t.as_u64()).collect();
    let first_timer_frame = output_events(&run_engine(&MAGIC_MOUSE_2_BT, &defaults(), &full))
        .into_iter()
        .find(|(t, event)| {
            matches!(event, OutputEvent::TouchFrame { .. }) && !report_times.contains(t)
        })
        .map(|(t, _)| t)
        .expect("the overview script runs on timers");
    let reports: Vec<(Micros, Vec<u8>)> = full
        .into_iter()
        .filter(|(t, _)| t.as_u64() < first_timer_frame)
        .collect();
    let last_report = reports.last().unwrap().0.as_u64();
    let timeline = run_engine(&MAGIC_MOUSE_2_BT, &defaults(), &reports);
    let frames_after_last_report = output_events(&timeline)
        .into_iter()
        .filter(|(t, event)| *t > last_report && matches!(event, OutputEvent::TouchFrame { .. }))
        .count();
    assert!(frames_after_last_report > 1, "{frames_after_last_report}");
    assert_eq!(
        fold(&timeline, timeline.end_time().unwrap())
            .synthetic
            .len(),
        0
    );
    assert_eq!(trace_records(&timeline).len(), timeline.len());
}

#[test]
fn palm_rejection_presets_change_the_decisions_on_rerun() {
    let reports = scenarios::slow_scroll().timed();
    let profile = &MAGIC_MOUSE_2_BT;
    let low = with_palm_rejection(&defaults(), PalmRejection::Low, profile);
    let high = with_palm_rejection(&defaults(), PalmRejection::High, profile);
    assert_eq!(low.touch.palm_rejection, PalmRejection::Low);
    assert!(
        (low.advanced.touch.palm_size_mm - PalmRejection::Low.preset(profile).palm_size_mm).abs()
            < f64::EPSILON
    );
    assert_ne!(low.advanced.touch, high.advanced.touch);
    assert_eq!(
        with_palm_rejection(&defaults(), PalmRejection::Medium, profile),
        defaults()
    );
    let reasons = |config: &Config| -> Vec<DecisionReason> {
        run_engine(profile, config, &reports)
            .entries()
            .filter_map(|entry| match &entry.record {
                TraceRecord::Decision(decision) => Some(decision.reason),
                _ => None,
            })
            .collect()
    };
    assert!(reasons(&low).contains(&DecisionReason::Promoted));
    assert!(reasons(&high).contains(&DecisionReason::Palm));
}

#[test]
fn acceleration_presets_change_the_scroll_travel() {
    let reports = scenarios::flick().timed();
    let profile = &MAGIC_MOUSE_2_BT;
    let off = with_acceleration(&defaults(), ScrollAcceleration::Off);
    let high = with_acceleration(&defaults(), ScrollAcceleration::High);
    assert_eq!(off.scroll.acceleration, ScrollAcceleration::Off);
    assert!((off.advanced.scroll.gain_max - off.advanced.scroll.gain_min).abs() < f64::EPSILON);
    assert_eq!(
        with_acceleration(&defaults(), ScrollAcceleration::Medium),
        defaults()
    );
    let travel = |config: &Config| -> i32 {
        let frames: Vec<i32> = output_events(&run_engine(profile, config, &reports))
            .into_iter()
            .filter_map(|(_, event)| match event {
                OutputEvent::TouchFrame { contacts } => contacts.first().map(|contact| contact.y),
                _ => None,
            })
            .collect();
        let min = frames.iter().min().copied().unwrap_or(0);
        let max = frames.iter().max().copied().unwrap_or(0);
        max - min
    };
    assert!(travel(&high) > travel(&off));
}

#[test]
fn the_config_is_the_explicit_file_else_the_user_file_else_defaults() {
    let dir = tempfile::tempdir().unwrap();
    let profile = &MAGIC_MOUSE_2_BT;
    let file = dir.path().join("config.toml");
    std::fs::write(
        &file,
        "version = 1\n[touch]\npalm_rejection = \"high\"\n[scroll]\nspeed = 3\n",
    )
    .unwrap();
    let explicit = choose_config(Some(&file), None, profile).unwrap();
    assert_eq!(explicit.origin, ConfigOrigin::File(file.clone()));
    assert_eq!(explicit.config.touch.palm_rejection, PalmRejection::High);
    assert_eq!(explicit.warnings.len(), 1, "{:?}", explicit.warnings);

    let user = choose_config(None, Some(file.clone()), profile).unwrap();
    assert_eq!(user.origin, ConfigOrigin::File(file));

    let missing = dir.path().join("missing.toml");
    let defaults_for_missing = choose_config(None, Some(missing.clone()), profile).unwrap();
    assert_eq!(
        defaults_for_missing.origin,
        ConfigOrigin::MissingDefaultFile(missing.clone())
    );
    assert_eq!(defaults_for_missing.config, defaults());
    let no_home = choose_config(None, None, profile).unwrap();
    assert_eq!(no_home.origin, ConfigOrigin::Defaults);
    assert!(choose_config(Some(&missing), None, profile).is_err());

    let invalid = dir.path().join("invalid.toml");
    std::fs::write(
        &invalid,
        "version = 1\n[touch]\npalm_rejection = \"extreme\"\n",
    )
    .unwrap();
    let error = choose_config(Some(&invalid), None, profile).unwrap_err();
    assert!(error.contains("invalid.toml"), "{error}");
    assert!(error.contains("line 3"), "{error}");
}

#[test]
fn the_default_config_path_follows_xdg() {
    assert_eq!(
        default_config_path(Some("/xdg".into()), Some("/home/u".into())),
        Some(PathBuf::from("/xdg/magic-driver/config.toml"))
    );
    assert_eq!(
        default_config_path(Some("relative".into()), Some("/home/u".into())),
        Some(PathBuf::from("/home/u/.config/magic-driver/config.toml"))
    );
    assert_eq!(
        default_config_path(Some(String::new().into()), Some("/home/u".into())),
        Some(PathBuf::from("/home/u/.config/magic-driver/config.toml"))
    );
    assert_eq!(default_config_path(None, None), None);
}

#[test]
fn an_offline_source_opens_a_recording_and_reruns_with_a_new_config() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("swipe.rec");
    let reports = scenarios::two_finger_swipe().timed();
    std::fs::write(&path, recording_text("magic-mouse-2-bt", &reports)).unwrap();
    let mut source = OfflineSource::open(&path, None, None).unwrap();
    assert_eq!(source.path, path);
    assert_eq!(source.config, defaults());
    assert_eq!(source.timeline.report_count(), reports.len());
    let swipes = |timeline: &Timeline| {
        timeline
            .entries()
            .filter(|entry| matches!(&entry.record, TraceRecord::Recogniser(change) if change.to == GestureState::Swipe))
            .count()
    };
    assert_eq!(swipes(&source.timeline), 1);
    let mut disabled = defaults();
    disabled.gestures.swipe_desktops = false;
    source.rerun(disabled.clone());
    assert_eq!(source.config, disabled);
    assert_eq!(swipes(&source.timeline), 0);
    assert_eq!(source.chosen.config, defaults());
    assert!(OfflineSource::open(&dir.path().join("none.rec"), None, None).is_err());
}
