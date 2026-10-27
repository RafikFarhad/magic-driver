use super::*;
use crate::engine::{Engine, Input, Mode};
use crate::profile::MAGIC_MOUSE_2_BT;
use crate::testkit::scenarios::{finger, palm};
use crate::testkit::{ReportBuilder, TouchRecord, touch_state};

const FRAME: u64 = 11_111;

struct Harness {
    engine: Engine,
    frame: u64,
}

impl Harness {
    fn new() -> Harness {
        Harness::with_config(Config::default())
    }

    fn with_config(config: Config) -> Harness {
        Harness {
            engine: Engine::new(&MAGIC_MOUSE_2_BT, config, Mode::Normal),
            frame: 0,
        }
    }

    fn t(&self) -> Micros {
        Micros::new(self.frame * FRAME)
    }

    fn feed(&mut self, report: &ReportBuilder) -> Vec<Decision> {
        let bytes = report.build();
        let mut out = Vec::new();
        self.engine.handle(
            Input::Report {
                t: self.t(),
                bytes: &bytes,
            },
            &mut out,
        );
        self.frame += 1;
        self.classifier().decisions().to_vec()
    }

    fn classifier(&self) -> &Classifier {
        self.engine.classifier().expect("normal mode")
    }

    fn class(&self, touch_id: u8) -> Option<Class> {
        let track = self.engine.tracker().by_touch_id(touch_id)?;
        self.classifier().class_of(track.id)
    }

    fn reason(&self, touch_id: u8) -> Option<Reason> {
        let track = self.engine.tracker().by_touch_id(touch_id)?;
        self.classifier()
            .entries()
            .iter()
            .find(|entry| entry.track == track.id)
            .and_then(|entry| entry.reason)
    }
}

fn start(touch: TouchRecord) -> ReportBuilder {
    ReportBuilder::new().touch(touch.state(touch_state::START))
}

fn at(touch: TouchRecord) -> ReportBuilder {
    ReportBuilder::new().touch(touch)
}

fn moved(touch: TouchRecord, dy: i32) -> TouchRecord {
    TouchRecord {
        y: touch.y + dy,
        ..touch
    }
}

fn promote(harness: &mut Harness, touch: TouchRecord) -> TouchRecord {
    harness.feed(&start(touch));
    let mut current = touch;
    for _ in 0..4 {
        current = moved(current, -40);
        harness.feed(&at(current));
    }
    current
}

#[test]
fn touch_size_converts_raw_major_with_the_profile_scale() {
    let mut tracker = Tracker::new(MAGIC_MOUSE_2_BT.surface);
    let touch = finger(0, 0).touch_major(130).touch_minor(20);
    let report = crate::report::parse(&MAGIC_MOUSE_2_BT, &at(touch).build()).unwrap();
    tracker.update(Micros::ZERO, &report.touches);
    let track = tracker.contacts().next().unwrap();
    let mm = touch_size_mm(&MAGIC_MOUSE_2_BT.surface, 4, track);
    assert!((mm - 520.0 / MAGIC_MOUSE_2_BT.surface.x.units_per_mm()).abs() < 1e-9);
}

#[test]
fn palm_size_rejects_large_contacts_on_either_axis() {
    for touch in [
        finger(0, 0).touch_major(140),
        finger(0, 0).touch_major(20).touch_minor(140),
    ] {
        let mut harness = Harness::new();
        let decisions = harness.feed(&start(touch));
        assert_eq!(harness.class(touch.id), Some(Class::Rejected));
        assert_eq!(harness.reason(touch.id), Some(Reason::Palm));
        assert_eq!(decisions.len(), 1);
        assert_eq!(decisions[0].previous, Class::Candidate);
        assert_eq!(decisions[0].reason, Reason::Palm);
    }
}

#[test]
fn palm_size_threshold_follows_the_config() {
    let touch = finger(0, 0).touch_major(100);
    let mut harness = Harness::new();
    harness.feed(&start(touch));
    assert_eq!(harness.class(touch.id), Some(Class::Candidate));
    let mut config = Config::default();
    config.advanced.touch.palm_size_mm = 10.0;
    let mut harness = Harness::with_config(config);
    harness.feed(&start(touch));
    assert_eq!(harness.reason(touch.id), Some(Reason::Palm));
}

#[test]
fn rear_zone_rejects_tracks_that_start_there() {
    let boundary = 2047 - 1273;
    let mut harness = Harness::new();
    harness.feed(&start(finger(0, boundary + 5)));
    assert_eq!(harness.reason(2), Some(Reason::RearZone));
    let mut harness = Harness::new();
    harness.feed(&start(finger(0, boundary - 5)));
    assert_eq!(harness.class(2), Some(Class::Candidate));
}

#[test]
fn rear_zone_only_checks_the_start_position() {
    let mut harness = Harness::new();
    let touch = finger(0, 600);
    harness.feed(&start(touch));
    let mut current = touch;
    for _ in 0..4 {
        current = moved(current, 300);
        harness.feed(&at(current));
    }
    assert_eq!(harness.class(2), Some(Class::Deliberate));
}

#[test]
fn mouse_motion_rejects_candidates() {
    let mut harness = Harness::new();
    let touch = finger(0, 300);
    harness.feed(&start(touch));
    harness.feed(&at(touch).motion(30, 0));
    assert_eq!(harness.reason(2), Some(Reason::MouseMoving));
    for _ in 0..30 {
        harness.feed(&at(moved(touch, -200)));
    }
    assert_eq!(harness.reason(2), Some(Reason::MouseMoving));
}

#[test]
fn small_mouse_motion_below_the_threshold_is_tolerated() {
    let mut harness = Harness::new();
    let touch = finger(0, 300);
    harness.feed(&start(touch));
    harness.feed(&at(touch).motion(1, 0));
    assert_eq!(harness.class(2), Some(Class::Candidate));
}

#[test]
fn mouse_motion_cancels_a_deliberate_track() {
    let mut harness = Harness::new();
    let current = promote(&mut harness, finger(0, 600));
    assert_eq!(harness.class(2), Some(Class::Deliberate));
    let decisions = harness.feed(&at(current).motion(40, 10));
    assert_eq!(harness.reason(2), Some(Reason::MouseMoving));
    assert!(decisions[0].cancels_deliberate());
}

#[test]
fn tracks_present_at_a_press_are_rejected_until_lift() {
    let mut harness = Harness::new();
    let resting = finger(-600, 300);
    harness.feed(&start(resting));
    harness.feed(&at(resting).left(true));
    assert_eq!(harness.reason(2), Some(Reason::ClickGuard));
    harness.feed(&at(resting));
    for _ in 0..10 {
        harness.feed(&at(moved(resting, -300)));
    }
    assert_eq!(harness.reason(2), Some(Reason::ClickGuard));
}

#[test]
fn a_press_cancels_a_deliberate_track() {
    let mut harness = Harness::new();
    let current = promote(&mut harness, finger(0, 600));
    let decisions = harness.feed(&at(current).left(true));
    assert_eq!(harness.reason(2), Some(Reason::ClickGuard));
    assert!(decisions[0].cancels_deliberate());
}

#[test]
fn tracks_starting_while_a_button_is_down_are_rejected() {
    let mut harness = Harness::new();
    harness.feed(&ReportBuilder::new().left(true));
    harness.feed(&start(finger(0, 300)).left(true));
    assert_eq!(harness.reason(2), Some(Reason::ClickGuard));
}

#[test]
fn click_guard_covers_tracks_starting_just_after_a_release() {
    let mut harness = Harness::new();
    harness.feed(&ReportBuilder::new().left(true));
    harness.feed(&ReportBuilder::new());
    harness.feed(&start(finger(0, 300)));
    assert_eq!(harness.reason(2), Some(Reason::ClickGuard));

    let mut harness = Harness::new();
    harness.feed(&ReportBuilder::new().left(true));
    harness.feed(&ReportBuilder::new());
    for _ in 0..10 {
        harness.feed(&ReportBuilder::new());
    }
    harness.feed(&start(finger(0, 300)));
    assert_eq!(harness.class(2), Some(Class::Candidate));
}

#[test]
fn promotion_needs_the_minimum_distance() {
    let mut harness = Harness::new();
    let touch = finger(0, 300);
    harness.feed(&start(touch));
    harness.feed(&at(moved(touch, -50)));
    assert_eq!(harness.class(2), Some(Class::Candidate));
    let decisions = harness.feed(&at(moved(touch, -110)));
    assert_eq!(harness.class(2), Some(Class::Deliberate));
    assert_eq!(harness.reason(2), Some(Reason::Promoted));
    assert_eq!(decisions[0].reason, Reason::Promoted);
    assert_eq!(decisions[0].previous, Class::Candidate);
}

#[test]
fn promotion_distance_follows_the_config() {
    let mut config = Config::default();
    config.advanced.touch.promote_distance_mm = 0.5;
    let mut harness = Harness::with_config(config);
    let touch = finger(0, 300);
    harness.feed(&start(touch));
    harness.feed(&at(moved(touch, -50)));
    assert_eq!(harness.class(2), Some(Class::Deliberate));
}

#[test]
fn promotion_needs_a_report_after_the_start() {
    let mut harness = Harness::new();
    harness.feed(&ReportBuilder::new());
    harness.feed(&at(finger(0, 300)));
    assert_eq!(harness.class(2), Some(Class::Candidate));
}

#[test]
fn slow_drift_does_not_promote_within_the_window() {
    let mut harness = Harness::new();
    let touch = finger(0, 300);
    harness.feed(&start(touch));
    let mut current = touch;
    for _ in 0..30 {
        current = moved(current, -6);
        harness.feed(&at(current));
    }
    assert!((touch.y - current.y) > 106);
    assert_eq!(harness.reason(2), Some(Reason::Resting));
}

#[test]
fn still_candidates_become_resting_after_the_dwell_time() {
    let mut harness = Harness::new();
    let touch = finger(0, 300);
    harness.feed(&start(touch));
    assert_eq!(
        harness
            .classifier()
            .next_deadline(&Thresholds::from_config(&Config::default())),
        Some(Micros::from_millis(300))
    );
    for _ in 0..26 {
        harness.feed(&at(touch));
    }
    assert_eq!(harness.class(2), Some(Class::Candidate));
    harness.feed(&at(touch));
    assert_eq!(harness.class(2), Some(Class::Candidate));
    harness.feed(&at(touch));
    assert_eq!(harness.reason(2), Some(Reason::Resting));
    assert_eq!(
        harness
            .classifier()
            .next_deadline(&Thresholds::from_config(&Config::default())),
        None
    );
}

#[test]
fn rejection_is_sticky_until_lift_and_a_new_touch_starts_fresh() {
    let mut harness = Harness::new();
    let touch = finger(0, 300);
    harness.feed(&start(touch));
    harness.feed(&at(touch).motion(40, 0));
    assert_eq!(harness.reason(2), Some(Reason::MouseMoving));
    harness.feed(&at(touch.state(touch_state::LIFT_50)));
    assert_eq!(harness.class(2), None);
    harness.feed(&ReportBuilder::new());
    for _ in 0..10 {
        harness.feed(&ReportBuilder::new());
    }
    let _ = promote(&mut harness, touch);
    assert_eq!(harness.class(2), Some(Class::Deliberate));
}

#[test]
fn deliberate_tracks_are_not_rejected_by_size_or_dwell() {
    let mut harness = Harness::new();
    let current = promote(&mut harness, finger(0, 600));
    for _ in 0..60 {
        harness.feed(&at(current.touch_major(200)));
    }
    assert_eq!(harness.class(2), Some(Class::Deliberate));
}

#[test]
fn decisions_are_only_reported_on_change() {
    let mut harness = Harness::new();
    let touch = finger(0, 300);
    assert!(harness.feed(&start(touch)).is_empty());
    assert!(harness.feed(&at(touch)).is_empty());
    let mut decisions = Vec::new();
    let mut current = touch;
    for _ in 0..6 {
        current = moved(current, -40);
        decisions.extend(harness.feed(&at(current)));
    }
    assert_eq!(decisions.len(), 1);
}

#[test]
fn hover_tracks_are_not_classified() {
    let mut harness = Harness::new();
    harness.feed(&at(finger(0, 300).state(touch_state::HOVER)));
    assert!(harness.classifier().entries().is_empty());
}

#[test]
fn resting_palm_and_finger_are_classified_independently() {
    let mut harness = Harness::new();
    harness.feed(&start(palm()).touch(finger(0, 300).state(touch_state::START)));
    assert_eq!(harness.reason(1), Some(Reason::Palm));
    assert_eq!(harness.class(2), Some(Class::Candidate));
}

#[test]
fn reset_forgets_every_track() {
    let mut harness = Harness::new();
    harness.feed(&start(finger(0, 300)));
    let mut out = Vec::new();
    harness.engine.handle(Input::Reset, &mut out);
    assert!(harness.classifier().entries().is_empty());
}
