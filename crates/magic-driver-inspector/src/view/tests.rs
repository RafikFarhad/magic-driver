use md_core::config::Config;
use md_core::profile::MAGIC_MOUSE_2_BT;
use md_core::testkit::scenarios::{self, FINGER_ID, PALM_ID};
use md_trace::{DroppedRecord, OutputRecord};

use super::*;
use crate::replay::run_engine;

fn engine_timeline(scenario: &scenarios::Scenario) -> Timeline {
    run_engine(
        &MAGIC_MOUSE_2_BT,
        &Config::for_profile(&MAGIC_MOUSE_2_BT),
        &scenario.timed(),
    )
}

fn first_time(timeline: &Timeline, wanted: impl Fn(&TraceRecord) -> bool) -> u64 {
    timeline
        .entries()
        .find(|entry| wanted(&entry.record))
        .map(|entry| entry.t)
        .expect("record present")
}

#[test]
fn a_scroll_shows_the_promoted_finger_the_rejected_palm_and_two_synthetic_fingers() {
    let timeline = engine_timeline(&scenarios::flick());
    let scrolling = first_time(
        &timeline,
        |record| matches!(record, TraceRecord::Recogniser(change) if change.to == GestureState::Scroll),
    );
    let state = fold(&timeline, scrolling);
    assert_eq!(state.recogniser, GestureState::Scroll);
    let finger = state.decision_for(FINGER_ID).expect("finger decision");
    assert_eq!(finger.class, TrackClass::Deliberate);
    assert_eq!(finger.reason, DecisionReason::Promoted);
    let palm = state.decision_for(PALM_ID).expect("palm decision");
    assert_eq!(palm.class, TrackClass::Rejected);
    assert_eq!(palm.reason, DecisionReason::Palm);

    let mapper = SurfaceMapper::new(&MAGIC_MOUSE_2_BT);
    let dots = mapper.dots(&state);
    assert_eq!(dots.len(), 2);
    let palm_dot = dots.iter().find(|dot| dot.id == PALM_ID).unwrap();
    let thresholds = Thresholds::from_config(&Config::for_profile(&MAGIC_MOUSE_2_BT));
    assert!(thresholds.is_palm_sized(palm_dot));
    assert!(palm_dot.y > mapper.rear_zone_start(thresholds.rear_zone));
    assert_eq!(palm_dot.class, Some(TrackClass::Rejected));
    let finger_dot = dots.iter().find(|dot| dot.id == FINGER_ID).unwrap();
    assert!(!thresholds.is_palm_sized(finger_dot));
    assert_eq!(finger_dot.kind, TouchStateKind::Drag);

    let end = timeline.end_time().unwrap();
    let synthetic_at = |t: u64| fold(&timeline, t).synthetic.len();
    let during = timeline
        .entries()
        .filter(|entry| entry.t >= scrolling)
        .find(|entry| matches!(&entry.record, TraceRecord::Output(OutputRecord { output: OutputEvent::TouchFrame { contacts }, .. }) if !contacts.is_empty()))
        .map(|entry| entry.t)
        .unwrap();
    assert_eq!(synthetic_at(during), 2);
    assert_eq!(synthetic_at(end), 0);
    assert_eq!(fold(&timeline, end).recogniser, GestureState::Idle);
}

#[test]
fn the_cursor_matches_a_full_fold_at_every_position_forward_and_backward() {
    let mut reports = Vec::new();
    for scenario in std::iter::repeat_with(|| {
        [
            scenarios::flick(),
            scenarios::click_while_scrolling(),
            scenarios::two_finger_swipe(),
            scenarios::two_finger_double_tap(),
        ]
    })
    .take(20)
    .flatten()
    {
        let offset = reports
            .last()
            .map_or(0, |(t, _): &(md_core::Micros, Vec<u8>)| {
                t.as_u64() + 500_000
            });
        reports.extend(
            scenario
                .timed()
                .into_iter()
                .map(|(t, bytes)| (md_core::Micros::new(t.as_u64() + offset), bytes)),
        );
    }
    let timeline = run_engine(
        &MAGIC_MOUSE_2_BT,
        &Config::for_profile(&MAGIC_MOUSE_2_BT),
        &reports,
    );
    assert!(
        timeline.len() as u64 > 2 * CHECKPOINT_INTERVAL,
        "{}",
        timeline.len()
    );
    let start = timeline.start_time().unwrap();
    let end = timeline.end_time().unwrap();
    let step = (end - start) / 97;
    let positions: Vec<u64> = (0..=97).map(|index| start + index * step).collect();
    let mut cursor = ViewCursor::new();
    for t in positions.iter().chain(positions.iter().rev()) {
        assert_eq!(cursor.seek(&timeline, *t), &fold(&timeline, *t), "at {t}");
    }
    cursor.seek(&timeline, end);
    assert_eq!(cursor.seq(), timeline.end_seq());
    cursor.reset();
    assert_eq!(cursor.seq(), 0);
    assert_eq!(cursor.state(), &ViewState::default());
}

#[test]
fn the_cursor_restarts_from_the_window_after_eviction() {
    let mut timeline = engine_timeline(&scenarios::click_while_scrolling());
    let start = timeline.start_time().unwrap();
    let end = timeline.end_time().unwrap();
    let mut cursor = ViewCursor::new();
    let seen = cursor.seek(&timeline, end).clone();
    timeline.evict_before(end / 2);
    assert_eq!(cursor.seek(&timeline, end), &seen);
    let window_start = timeline.start_time().unwrap();
    assert!(window_start > start);
    let mut expected = ViewState::default();
    for entry in timeline
        .entries()
        .take_while(|entry| entry.t <= window_start)
    {
        expected.apply(entry);
    }
    assert_eq!(cursor.seek(&timeline, window_start), &expected);
    let mut expected = ViewState::default();
    for entry in timeline.entries() {
        expected.apply(entry);
    }
    assert_eq!(cursor.seek(&timeline, end), &expected);
    assert_eq!(cursor.seq(), timeline.end_seq());
}

#[test]
fn buttons_and_pointer_events_follow_the_output_records() {
    let timeline = engine_timeline(&scenarios::click_right());
    let pressed = first_time(&timeline, |record| {
        matches!(
            record,
            TraceRecord::Output(OutputRecord {
                output: OutputEvent::Button { pressed: true, .. },
                ..
            })
        )
    });
    let state = fold(&timeline, pressed);
    assert!(state.buttons.right);
    assert!(!state.buttons.left);
    assert!(matches!(
        state.pointer_events.back().map(|event| &event.event),
        Some(OutputEvent::Button {
            button: ButtonName::Right,
            pressed: true
        })
    ));
    let state = fold(&timeline, timeline.end_time().unwrap());
    assert_eq!(state.buttons, ButtonsDown::default());
    assert!(state.reports > 0);
}

#[test]
fn recent_event_lists_are_capped() {
    let records = (0..100_u64).map(|t| {
        TraceRecord::Output(OutputRecord {
            t,
            output: OutputEvent::PointerMotion {
                dx: i32::try_from(t).unwrap(),
                dy: 0,
            },
        })
    });
    let timeline = Timeline::from_records(records);
    let state = fold(&timeline, 1_000);
    assert_eq!(state.pointer_events.len(), RECENT_EVENTS);
    assert_eq!(state.pointer_events.front().map(|event| event.t), Some(60));
}

#[test]
fn dropped_and_session_records_are_shown() {
    let timeline = Timeline::from_records([
        TraceRecord::Dropped(DroppedRecord { count: 4 }),
        TraceRecord::Session(SessionRecord {
            t: 10,
            state: "degraded".to_owned(),
            detail: "no hidraw reports".to_owned(),
        }),
        TraceRecord::Dropped(DroppedRecord { count: 6 }),
    ]);
    let state = fold(&timeline, 10);
    assert_eq!(state.dropped, 10);
    assert_eq!(
        state.session.as_ref().map(|session| session.state.as_str()),
        Some("degraded")
    );
}

#[test]
fn decisions_are_forgotten_when_the_touch_id_lifts_or_restarts() {
    let touch = |id: u8, state: u8| TouchInfo {
        id,
        x: 0,
        y: 0,
        touch_major: 10,
        touch_minor: 10,
        size: 5,
        orientation: 0,
        state,
    };
    let report = |t: u64, touches: Vec<TouchInfo>| {
        TraceRecord::Report(ReportRecord {
            t,
            clicks: 0,
            dx: 0,
            dy: 0,
            touches,
            error: None,
            processing_ns: None,
        })
    };
    let decision = |t: u64, touch_id: u8| {
        TraceRecord::Decision(DecisionRecord {
            t,
            track: u64::from(touch_id),
            touch_id: Some(touch_id),
            previous: TrackClass::Candidate,
            class: TrackClass::Rejected,
            reason: DecisionReason::Resting,
        })
    };
    let timeline = Timeline::from_records([
        report(0, vec![touch(1, 0x30), touch(2, 0x30)]),
        decision(0, 1),
        decision(0, 2),
        report(10, vec![touch(1, 0x40), touch(2, 0x40)]),
        report(20, vec![touch(1, 0x40)]),
        report(30, vec![touch(1, 0x30)]),
    ]);
    assert_eq!(fold(&timeline, 10).touch_decisions.len(), 2);
    let state = fold(&timeline, 20);
    assert!(state.decision_for(1).is_some());
    assert!(state.decision_for(2).is_none());
    assert!(fold(&timeline, 30).decision_for(1).is_none());
}

#[test]
fn the_surface_mapper_puts_the_front_left_corner_at_the_origin() {
    let mapper = SurfaceMapper::new(&MAGIC_MOUSE_2_BT);
    let surface = MAGIC_MOUSE_2_BT.surface;
    assert_eq!(mapper.normalise(surface.x.min, surface.y.min), (0.0, 0.0));
    assert_eq!(mapper.normalise(surface.x.max, surface.y.max), (1.0, 1.0));
    assert_eq!(mapper.normalise(-5_000, 9_000), (0.0, 1.0));
    let (x, _) = mapper.normalise(79, 0);
    assert!((x - 0.5).abs() < 0.001);
    assert_eq!(mapper.size_mm(), (90.56, 51.52));
    assert!((mapper.rear_zone_start(0.35) - 0.65).abs() < 1e-9);
    assert!((mapper.rear_zone_start(2.0)).abs() < 1e-9);
    let units_per_mm = MAGIC_MOUSE_2_BT.surface.x.units_per_mm();
    assert!((mapper.contact_mm(100) - 400.0 / units_per_mm).abs() < 1e-9);
    assert_eq!(mapper.profile().id, MAGIC_MOUSE_2_BT.id);
}

#[test]
fn touch_states_have_labels_for_every_report_state() {
    let kinds: Vec<TouchStateKind> = [0x00, 0x20, 0x30, 0x40, 0x50, 0x60, 0x70, 0x80, 0x43]
        .into_iter()
        .map(TouchStateKind::from_bits)
        .collect();
    assert_eq!(
        kinds,
        [
            TouchStateKind::None,
            TouchStateKind::Hover,
            TouchStateKind::Start,
            TouchStateKind::Drag,
            TouchStateKind::Lift,
            TouchStateKind::Lift,
            TouchStateKind::Lift,
            TouchStateKind::Unknown,
            TouchStateKind::Drag,
        ]
    );
    assert_eq!(TouchStateKind::Drag.label(), "touching");
}

#[test]
fn synthetic_fingers_are_shown_in_mm_from_the_touchpad_centre() {
    let pad = TouchpadInfo {
        max_x: 4_000,
        max_y: 4_000,
        units_per_mm: 10,
    };
    let fingers = synthetic_fingers(
        pad,
        &[
            ContactInfo {
                slot: 0,
                tracking_id: 7,
                x: 1_900,
                y: 2_000,
            },
            ContactInfo {
                slot: 1,
                tracking_id: 8,
                x: 2_100,
                y: 1_950,
            },
        ],
    );
    assert_eq!(fingers.len(), 2);
    assert!((fingers[0].x_mm + 10.0).abs() < 1e-9);
    assert!(fingers[0].y_mm.abs() < 1e-9);
    assert!((fingers[1].x_mm - 10.0).abs() < 1e-9);
    assert!((fingers[1].y_mm + 5.0).abs() < 1e-9);
    assert_eq!((fingers[1].slot, fingers[1].tracking_id), (1, 8));
}

#[test]
fn thresholds_from_the_hello_config_match_the_engine_config() {
    let mut config = Config::for_profile(&MAGIC_MOUSE_2_BT);
    config.advanced.touch.rear_zone = 0.5;
    config.advanced.touch.palm_size_mm = 14.0;
    let value = serde_json::to_value(&config).unwrap();
    let fallback = Thresholds::from_config(&Config::for_profile(&MAGIC_MOUSE_2_BT));
    let thresholds = Thresholds::from_config_value(&value, fallback);
    assert_eq!(thresholds, Thresholds::from_config(&config));
    assert!((thresholds.rear_zone - 0.5).abs() < f64::EPSILON);
    assert_eq!(
        Thresholds::from_config_value(&serde_json::Value::Null, fallback),
        fallback
    );
}
