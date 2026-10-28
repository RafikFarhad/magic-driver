use md_core::config::Config;
use md_core::profile::MAGIC_MOUSE_2_BT;
use md_core::testkit::scenarios;

use super::*;
use crate::received::{
    AxisChange, AxisSource, GestureEvent, GestureKind, GesturePhase, Received, ScrollFrame,
};
use crate::replay::run_engine;
use crate::view::fold;

fn received_log() -> ReceivedLog {
    let mut log = ReceivedLog::new();
    log.record(Received::Focus(true), 1_000_000);
    for index in 0..20_u32 {
        log.record(
            Received::Scroll(ScrollFrame {
                time: Some(1_000 + index * 10),
                source: Some(AxisSource::Finger),
                vertical: AxisChange {
                    value: Some(f64::from(index) - 10.0),
                    stop: index == 19,
                    ..AxisChange::default()
                },
                horizontal: AxisChange {
                    discrete: (index == 3).then_some(1),
                    value120: (index == 3).then_some(120),
                    ..AxisChange::default()
                },
            }),
            1_000_000 + u64::from(index) * 10_000,
        );
    }
    log.record(
        Received::Gesture(GestureEvent {
            time: 1_300,
            kind: GestureKind::Hold,
            phase: GesturePhase::Begin { fingers: 2 },
        }),
        1_300_000,
    );
    log
}

#[test]
fn every_panel_draws_headless_at_every_position_of_a_replay() {
    let profile = &MAGIC_MOUSE_2_BT;
    let config = Config::for_profile(profile);
    let timeline = run_engine(
        profile,
        &config,
        &scenarios::click_while_scrolling().timed(),
    );
    let mapper = SurfaceMapper::new(profile);
    let thresholds = Thresholds::from_config(&config);
    let touchpad = md_core::trace::profile_info(profile).touchpad;
    let log = received_log();
    let mut alignment = ClockAlignment::new();
    alignment.observe(0, 1_000_000);
    let ctx = egui::Context::default();
    let start = timeline.start_time().unwrap();
    let end = timeline.end_time().unwrap();
    for position in (start..=end).step_by(usize::try_from((end - start) / 20).unwrap().max(1)) {
        let state = fold(&timeline, position);
        for following in [true, false] {
            let view = ReceivedView {
                status: "listening",
                info: Some(ReceiverInfo {
                    seat_version: Some(9),
                    pointer_version: Some(9),
                    gestures_version: Some(3),
                    hold: true,
                }),
                log: &log,
                now_local_us: 1_400_000,
                trace_alignment: Some(alignment),
                position,
                following,
            };
            let mut output = ctx.run_ui(egui::RawInput::default(), |ui| {
                raw_panel(ui, mapper, &thresholds, &state, &timeline, position);
                decisions_panel(ui, &state, &timeline, position, touchpad);
                received_panel(ui, &view);
            });
            assert!(!output.shapes.is_empty());
            output.textures_delta.clear();
        }
    }
}

#[test]
fn times_are_printed_in_seconds_with_milliseconds() {
    assert_eq!(format_time(0), "0.000 s");
    assert_eq!(format_time(1_234_567), "1.234 s");
    assert_eq!(format_time(61_000_999), "61.000 s");
}
