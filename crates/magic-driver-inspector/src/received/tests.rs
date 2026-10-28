use smithay_client_toolkit::seat::pointer::AxisScroll;

use super::*;

fn axis(
    time: u32,
    source: Option<wl_pointer::AxisSource>,
    vertical: AxisScroll,
    horizontal: AxisScroll,
) -> PointerEventKind {
    PointerEventKind::Axis {
        time,
        horizontal,
        vertical,
        source,
    }
}

fn scroll(value: f64) -> AxisScroll {
    AxisScroll {
        absolute: value,
        ..AxisScroll::default()
    }
}

fn stop() -> AxisScroll {
    AxisScroll {
        stop: true,
        ..AxisScroll::default()
    }
}

fn frame(item: Option<Received>) -> ScrollFrame {
    match item {
        Some(Received::Scroll(frame)) => frame,
        other => panic!("expected a scroll frame, got {other:?}"),
    }
}

#[test]
fn a_finger_scroll_frame_keeps_the_source_values_and_time() {
    let frame = frame(pointer_received(&axis(
        1_234,
        Some(wl_pointer::AxisSource::Finger),
        scroll(-3.5),
        scroll(0.75),
    )));
    assert_eq!(frame.time, Some(1_234));
    assert_eq!(frame.source, Some(AxisSource::Finger));
    assert_eq!(frame.vertical.value, Some(-3.5));
    assert_eq!(frame.horizontal.value, Some(0.75));
    assert_eq!(frame.vertical.discrete, None);
    assert!(!frame.has_stop());
    assert_eq!(frame.axis(ScrollAxis::Vertical), &frame.vertical);
    assert_eq!(frame.axis(ScrollAxis::Horizontal), &frame.horizontal);
}

#[test]
fn a_stop_frame_has_a_stop_and_no_value() {
    let frame = frame(pointer_received(&axis(
        2_000,
        Some(wl_pointer::AxisSource::Finger),
        stop(),
        AxisScroll::default(),
    )));
    assert!(frame.has_stop());
    assert!(frame.vertical.stop);
    assert_eq!(frame.vertical.value, None);
    assert!(frame.horizontal.is_empty());
    assert_eq!(describe(&Received::Scroll(frame)), "scroll finger v: stop");
}

#[test]
fn wheel_frames_keep_discrete_steps_value120_and_the_direction() {
    let vertical = AxisScroll {
        absolute: 15.0,
        discrete: 1,
        value120: 120,
        relative_direction: Some(wl_pointer::AxisRelativeDirection::Inverted),
        stop: false,
    };
    let frame = frame(pointer_received(&axis(
        0,
        Some(wl_pointer::AxisSource::Wheel),
        vertical,
        AxisScroll::default(),
    )));
    assert_eq!(frame.time, None);
    assert_eq!(frame.source, Some(AxisSource::Wheel));
    assert_eq!(frame.vertical.discrete, Some(1));
    assert_eq!(frame.vertical.value120, Some(120));
    assert_eq!(frame.vertical.inverted, Some(true));
    assert_eq!(
        describe(&Received::Scroll(frame)),
        "scroll wheel v: 15.00 discrete 1 value120 120 inverted"
    );
    let identical = AxisScroll {
        relative_direction: Some(wl_pointer::AxisRelativeDirection::Identical),
        ..scroll(2.0)
    };
    let frame = self::frame(pointer_received(&axis(
        5,
        None,
        AxisScroll::default(),
        identical,
    )));
    assert_eq!(frame.source, None);
    assert_eq!(frame.horizontal.inverted, Some(false));
    assert_eq!(
        describe(&Received::Scroll(frame)),
        "scroll no source h: 2.00"
    );
}

#[test]
fn every_axis_source_is_named() {
    assert_eq!(
        axis_source(wl_pointer::AxisSource::Wheel),
        AxisSource::Wheel
    );
    assert_eq!(
        axis_source(wl_pointer::AxisSource::Finger),
        AxisSource::Finger
    );
    assert_eq!(
        axis_source(wl_pointer::AxisSource::Continuous),
        AxisSource::Continuous
    );
    assert_eq!(
        axis_source(wl_pointer::AxisSource::WheelTilt),
        AxisSource::WheelTilt
    );
    assert_eq!(AxisSource::WheelTilt.label(), "wheel tilt");
    assert_eq!(AxisSource::Unknown(9).label(), "source 9");
}

#[test]
fn focus_changes_are_kept_and_other_pointer_events_ignored() {
    assert_eq!(
        pointer_received(&PointerEventKind::Enter { serial: 1 }),
        Some(Received::Focus(true))
    );
    assert_eq!(
        pointer_received(&PointerEventKind::Leave { serial: 2 }),
        Some(Received::Focus(false))
    );
    assert_eq!(
        pointer_received(&PointerEventKind::Motion { time: 3 }),
        None
    );
    assert_eq!(
        pointer_received(&PointerEventKind::Press {
            time: 4,
            button: 0x110,
            serial: 5
        }),
        None
    );
    assert_eq!(
        pointer_received(&axis(6, None, AxisScroll::default(), AxisScroll::default())),
        Some(Received::Scroll(ScrollFrame {
            time: Some(6),
            ..ScrollFrame::default()
        }))
    );
    assert_eq!(
        pointer_received(&axis(0, None, AxisScroll::default(), AxisScroll::default())),
        None
    );
    assert_eq!(
        describe(&Received::Focus(true)),
        "pointer entered the window"
    );
    assert_eq!(describe(&Received::Focus(false)), "pointer left the window");
}

#[test]
fn swipe_pinch_and_hold_events_are_decoded() {
    assert_eq!(
        swipe_event(&zwp_pointer_gesture_swipe_v1::Event::Update {
            time: 10,
            dx: 4.0,
            dy: -0.5
        }),
        Some(GestureEvent {
            time: 10,
            kind: GestureKind::Swipe,
            phase: GesturePhase::Update {
                dx: 4.0,
                dy: -0.5,
                scale: None,
                rotation: None
            }
        })
    );
    assert_eq!(
        swipe_event(&zwp_pointer_gesture_swipe_v1::Event::End {
            serial: 1,
            time: 20,
            cancelled: 0
        }),
        Some(gesture_end(GestureKind::Swipe, 20, 0))
    );
    let pinch = pinch_event(&zwp_pointer_gesture_pinch_v1::Event::Update {
        time: 30,
        dx: 0.0,
        dy: 1.0,
        scale: 1.25,
        rotation: 3.0,
    })
    .unwrap();
    assert_eq!(pinch.kind, GestureKind::Pinch);
    assert_eq!(
        describe(&Received::Gesture(pinch)),
        "pinch update dx 0.0 dy 1.0 scale 1.250 rotation 3.0"
    );
    let hold = hold_event(&zwp_pointer_gesture_hold_v1::Event::End {
        serial: 2,
        time: 40,
        cancelled: 1,
    })
    .unwrap();
    assert_eq!(
        hold,
        GestureEvent {
            time: 40,
            kind: GestureKind::Hold,
            phase: GesturePhase::End { cancelled: true }
        }
    );
    assert_eq!(describe(&Received::Gesture(hold)), "hold end (cancelled)");
    let begin = gesture_begin(GestureKind::Hold, 50, 2);
    assert_eq!(begin.phase, GesturePhase::Begin { fingers: 2 });
    assert_eq!(describe(&Received::Gesture(begin)), "hold begin, 2 fingers");
    assert_eq!(
        describe(&Received::Gesture(gesture_end(GestureKind::Swipe, 1, 0))),
        "swipe end"
    );
    assert_eq!(Received::Gesture(begin).time(), Some(50));
    assert_eq!(Received::Focus(true).time(), None);
}

#[test]
fn the_log_aligns_wayland_times_to_the_local_clock() {
    let mut log = ReceivedLog::new();
    assert!(log.is_empty());
    let at = |time: u32, value: f64| {
        Received::Scroll(ScrollFrame {
            time: Some(time),
            source: Some(AxisSource::Finger),
            vertical: AxisChange {
                value: Some(value),
                ..AxisChange::default()
            },
            horizontal: AxisChange::default(),
        })
    };
    log.record(at(1_000, 1.0), 5_000_000);
    log.record(at(1_010, 2.0), 5_030_000);
    log.record(Received::Focus(false), 5_040_000);
    let entries: Vec<&ReceivedEntry> = log.entries().collect();
    assert_eq!(entries[0].local_us, 5_000_000);
    assert_eq!(entries[1].local_us, 5_010_000);
    assert_eq!(entries[1].wayland_ms, Some(1_010));
    assert_eq!(entries[2].local_us, 5_040_000);
    assert_eq!(entries[2].wayland_ms, None);
    assert_eq!(log.len(), 3);
    let series = log.scroll_series(ScrollAxis::Vertical, 5_005_000, 5_050_000);
    assert_eq!(series.len(), 1);
    assert!((series[0].value - 2.0).abs() < f64::EPSILON);
    assert_eq!(series[0].source, Some(AxisSource::Finger));
    assert!(
        log.scroll_series(ScrollAxis::Horizontal, 0, u64::MAX)
            .is_empty()
    );
    log.clear();
    assert!(log.is_empty());
}

#[test]
fn the_log_survives_the_wayland_clock_wrapping() {
    let mut log = ReceivedLog::new();
    let item = |time: u32| {
        Received::Gesture(GestureEvent {
            time,
            kind: GestureKind::Hold,
            phase: GesturePhase::Begin { fingers: 2 },
        })
    };
    log.record(item(u32::MAX - 5), 1_000_000);
    log.record(item(4), 1_010_000);
    let entries: Vec<&ReceivedEntry> = log.entries().collect();
    assert_eq!(entries[1].wayland_ms, Some(u64::from(u32::MAX) + 5));
    assert_eq!(entries[1].local_us, 1_010_000);
}

#[test]
fn the_log_is_bounded() {
    let mut log = ReceivedLog::with_capacity(3);
    for index in 0..10_u64 {
        log.record(Received::Focus(index % 2 == 0), index);
    }
    assert_eq!(log.len(), 3);
    assert_eq!(log.entries().next().map(|entry| entry.local_us), Some(7));
    assert_eq!(ReceivedLog::default().len(), 0);
}
