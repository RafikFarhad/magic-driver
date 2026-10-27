use super::*;
use crate::profile::MAGIC_MOUSE_2_BT;
use crate::report;
use crate::testkit::scenarios::finger;
use crate::testkit::{ReportBuilder, TouchRecord, touch_state};

const FRAME: Micros = Micros::new(11_111);
const SETTINGS: ScrollSettings = ScrollSettings {
    gain: 1.0,
    horizontal: true,
};

struct Harness {
    tracker: Tracker,
    synth: ScrollSynth,
    t: Micros,
}

impl Harness {
    fn new() -> Harness {
        Harness {
            tracker: Tracker::new(MAGIC_MOUSE_2_BT.surface),
            synth: ScrollSynth::new(MAGIC_MOUSE_2_BT.surface),
            t: Micros::ZERO,
        }
    }

    fn touch(&mut self, touch: TouchRecord) {
        let bytes = ReportBuilder::new().touch(touch).build();
        let report = report::parse(&MAGIC_MOUSE_2_BT, &bytes).unwrap();
        self.tracker.update(self.t, &report.touches);
    }

    fn step(&mut self, touch: TouchRecord, settings: ScrollSettings) -> Option<Frame> {
        self.touch(touch);
        let track = self.tracker.by_touch_id(touch.id).unwrap().id;
        let frame = self
            .synth
            .update(self.t, Intent::Scroll { track }, &self.tracker, settings);
        self.t += FRAME;
        frame
    }
}

fn centre_pair() -> [TouchpadPoint; 2] {
    let half = VIRTUAL_TOUCHPAD.mm_to_units(SCROLL_FINGER_SPACING_MM / 2.0);
    let centre = VIRTUAL_TOUCHPAD.centre();
    [
        TouchpadPoint {
            x: centre.x - half,
            y: centre.y,
        },
        TouchpadPoint {
            x: centre.x + half,
            y: centre.y,
        },
    ]
}

fn points(frame: &Frame) -> Vec<TouchpadPoint> {
    frame
        .contacts()
        .map(|(_, contact)| contact.point())
        .collect()
}

#[test]
fn scroll_starts_with_two_fingers_20_mm_apart_at_the_centre() {
    let mut harness = Harness::new();
    let frame = harness.step(finger(0, 300), SETTINGS).unwrap();
    assert_eq!(points(&frame), centre_pair().to_vec());
    assert_eq!(
        frame.slots[1].unwrap().x - frame.slots[0].unwrap().x,
        VIRTUAL_TOUCHPAD.mm_to_units(20.0)
    );
}

#[test]
fn fingers_move_by_the_finger_delta_times_the_gain() {
    for gain in [1.0, 2.0] {
        let mut harness = Harness::new();
        let settings = ScrollSettings { gain, ..SETTINGS };
        harness.step(finger(0, 300), settings);
        let forward_units = -71;
        let sideways_units = 26;
        let frame = harness
            .step(finger(sideways_units, 300 + forward_units), settings)
            .unwrap();
        let surface = MAGIC_MOUSE_2_BT.surface;
        let expected_x = VIRTUAL_TOUCHPAD.mm_to_units(surface.x.units_to_mm(sideways_units) * gain);
        let expected_y = VIRTUAL_TOUCHPAD.mm_to_units(surface.y.units_to_mm(forward_units) * gain);
        let [left, right] = centre_pair();
        assert_eq!(
            points(&frame),
            vec![
                TouchpadPoint {
                    x: left.x + expected_x,
                    y: left.y + expected_y
                },
                TouchpadPoint {
                    x: right.x + expected_x,
                    y: right.y + expected_y
                },
            ],
            "gain {gain}"
        );
    }
}

#[test]
fn horizontal_off_zeroes_the_horizontal_component() {
    let mut harness = Harness::new();
    let settings = ScrollSettings {
        horizontal: false,
        ..SETTINGS
    };
    harness.step(finger(0, 300), settings);
    let frame = harness.step(finger(100, 250), settings).unwrap();
    let [left, right] = centre_pair();
    let frame_points = points(&frame);
    assert_eq!(frame_points[0].x, left.x);
    assert_eq!(frame_points[1].x, right.x);
    assert!(frame_points[0].y < left.y);
    assert!(harness.step(finger(300, 250), settings).is_none());
}

#[test]
fn unchanged_positions_emit_no_frame() {
    let mut harness = Harness::new();
    harness.step(finger(0, 300), SETTINGS);
    assert!(harness.step(finger(0, 300), SETTINGS).is_none());
}

#[test]
fn freeze_keeps_the_fingers_and_none_lifts_them() {
    let mut harness = Harness::new();
    harness.step(finger(0, 300), SETTINGS);
    harness.step(finger(0, 250), SETTINGS);
    let frozen = *harness.synth.frame();
    harness.touch(finger(0, 100));
    assert_eq!(
        harness
            .synth
            .update(harness.t, Intent::Freeze, &harness.tracker, SETTINGS),
        None
    );
    assert_eq!(*harness.synth.frame(), frozen);
    assert_eq!(
        harness
            .synth
            .update(harness.t, Intent::None, &harness.tracker, SETTINGS),
        Some(Frame::EMPTY)
    );
    assert_eq!(
        harness
            .synth
            .update(harness.t, Intent::None, &harness.tracker, SETTINGS),
        None
    );
}

#[test]
fn hold_places_stationary_fingers_and_a_scroll_continues_them() {
    let mut harness = Harness::new();
    harness.touch(finger(0, 300).state(touch_state::START));
    let hold = harness
        .synth
        .update(harness.t, Intent::Hold, &harness.tracker, SETTINGS)
        .unwrap();
    assert_eq!(points(&hold), centre_pair().to_vec());
    harness.t += FRAME;
    harness.touch(finger(0, 200));
    assert_eq!(
        harness
            .synth
            .update(harness.t, Intent::Hold, &harness.tracker, SETTINGS),
        None
    );
    harness.t += FRAME;
    harness.step(finger(0, 150), SETTINGS);
    let moving = harness.step(finger(0, 100), SETTINGS).unwrap();
    let ids = |frame: &Frame| -> Vec<u16> {
        frame
            .contacts()
            .map(|(_, contact)| contact.tracking_id)
            .collect()
    };
    assert_eq!(ids(&moving), ids(&hold));
    assert!(points(&moving)[0].y < centre_pair()[0].y);
}

#[test]
fn every_stroke_gets_new_tracking_ids() {
    let mut harness = Harness::new();
    let first = harness.step(finger(0, 300), SETTINGS).unwrap();
    harness.synth.lift_all();
    let second = harness.step(finger(0, 300), SETTINGS).unwrap();
    assert_eq!(first.slots[0].unwrap().tracking_id, 0);
    assert_eq!(first.slots[1].unwrap().tracking_id, 1);
    assert_eq!(second.slots[0].unwrap().tracking_id, 2);
    assert_eq!(second.slots[1].unwrap().tracking_id, 3);
}

#[test]
fn a_finger_jump_is_spread_over_several_frames() {
    let mut harness = Harness::new();
    harness.step(finger(0, -1_500), SETTINGS);
    let mut frames = Vec::new();
    for _ in 0..40 {
        if let Some(frame) = harness.step(finger(0, 2_000), SETTINGS) {
            frames.push(frame);
        }
    }
    let target = VIRTUAL_TOUCHPAD.centre().y
        + VIRTUAL_TOUCHPAD.mm_to_units(MAGIC_MOUSE_2_BT.surface.y.units_to_mm(3_500));
    assert!(frames.len() > 5);
    assert_eq!(frames.last().unwrap().slots[0].unwrap().y, target);
    let mut previous = centre_pair()[0].y;
    for frame in &frames {
        let y = frame.slots[0].unwrap().y;
        let step_mm = f64::from(y - previous) / 40.0;
        assert!(step_mm <= JUMP_MAX_STEP_MM * 11_111.0 / 12_000.0 + 1e-9);
        previous = y;
    }
}

#[test]
fn step_limiter_ramps_up_and_caps_the_normalised_step() {
    let mut limiter = StepLimiter::new(Micros::ZERO);
    let t = JUMP_REFERENCE_INTERVAL;
    assert!((limiter.max_step_mm(t) - JUMP_MAX_STEP_INCREASE_MM).abs() < 1e-9);
    limiter.record(t, 5.0);
    let t2 = t + JUMP_REFERENCE_INTERVAL;
    assert!((limiter.max_step_mm(t2) - JUMP_MAX_STEP_MM).abs() < 1e-9);
    limiter.record(t2, 10.0);
    assert!((limiter.max_step_mm(t2 + JUMP_REFERENCE_INTERVAL) - JUMP_MAX_STEP_MM).abs() < 1e-9);
}

#[test]
fn step_limiter_scales_with_the_frame_interval() {
    let limiter = StepLimiter::new(Micros::ZERO);
    assert!(limiter.max_step_mm(Micros::ZERO).abs() < f64::EPSILON);
    let half = limiter.max_step_mm(Micros::from_millis(6));
    assert!((half - JUMP_MAX_STEP_INCREASE_MM / 2.0).abs() < 1e-9);
    let long = limiter.max_step_mm(Micros::from_millis(500));
    assert!((long - JUMP_MAX_STEP_INCREASE_MM * 2.0).abs() < 1e-9);
}

#[test]
fn step_limiter_keeps_the_last_step_after_an_unchecked_gap() {
    let mut limiter = StepLimiter::new(Micros::ZERO);
    limiter.record(Micros::from_millis(12), 4.0);
    limiter.record(Micros::from_millis(112), 9.0);
    let next = Micros::from_millis(124);
    assert!((limiter.max_step_mm(next) - 9.0).abs() < 1e-9);
    limiter.record(next, 0.0);
    assert!((limiter.max_step_mm(next) - 9.0).abs() < 1e-9);
}
