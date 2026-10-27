use super::*;
use crate::config::Config;
use crate::engine::{Engine, Input, Mode};
use crate::profile::MAGIC_MOUSE_2_BT;
use crate::testkit::scenarios::{FINGER_ID, finger, palm};
use crate::testkit::{ReportBuilder, TouchRecord, touch_state};

const FRAME: u64 = 11_111;

struct Harness {
    engine: Engine,
    t: Micros,
}

impl Harness {
    fn new() -> Harness {
        Harness::with_config(Config::default())
    }

    fn with_config(config: Config) -> Harness {
        Harness {
            engine: Engine::new(&MAGIC_MOUSE_2_BT, config, Mode::Normal),
            t: Micros::ZERO,
        }
    }

    fn feed(&mut self, report: &ReportBuilder) -> (RecogniserState, Intent) {
        let bytes = report.build();
        let mut out = Vec::new();
        self.engine.handle(
            Input::Report {
                t: self.t,
                bytes: &bytes,
            },
            &mut out,
        );
        self.t += Micros::new(FRAME);
        self.observe()
    }

    fn fire(&mut self) -> (RecogniserState, Intent) {
        let deadline = self.engine.next_deadline().expect("deadline");
        let mut out = Vec::new();
        self.engine.handle(Input::Timer { t: deadline }, &mut out);
        self.t = self.t.max(deadline + Micros::new(1));
        self.observe()
    }

    fn observe(&self) -> (RecogniserState, Intent) {
        let path = self.engine.scroll_path().expect("normal mode");
        (path.recogniser().state(), path.intent())
    }

    fn recogniser(&self) -> &Recogniser {
        self.engine.recogniser().expect("normal mode")
    }

    fn track(&self, touch_id: u8) -> TrackId {
        self.engine
            .tracker()
            .by_touch_id(touch_id)
            .expect("track")
            .id
    }
}

fn report(touches: &[TouchRecord]) -> ReportBuilder {
    touches
        .iter()
        .fold(ReportBuilder::new(), |builder, touch| builder.touch(*touch))
}

fn moved(touch: TouchRecord, dy: i32) -> TouchRecord {
    TouchRecord {
        y: touch.y + dy,
        ..touch
    }
}

fn scroll(harness: &mut Harness, touch: TouchRecord, frames: usize) -> TouchRecord {
    harness.feed(&report(&[palm(), touch.state(touch_state::START)]));
    let mut current = touch;
    for _ in 0..frames {
        current = moved(current, -40);
        harness.feed(&report(&[palm(), current]));
    }
    current
}

fn lift(harness: &mut Harness, touch: TouchRecord) -> (RecogniserState, Intent) {
    harness.feed(&report(&[palm(), touch.state(touch_state::LIFT_50)]))
}

#[test]
fn idle_without_deliberate_tracks() {
    let mut harness = Harness::new();
    assert_eq!(
        harness.feed(&report(&[palm()])),
        (RecogniserState::Idle, Intent::None)
    );
    let touch = finger(0, 300);
    assert_eq!(
        harness.feed(&report(&[palm(), touch.state(touch_state::START)])),
        (RecogniserState::Idle, Intent::None)
    );
}

#[test]
fn one_deliberate_track_starts_a_scroll() {
    let mut harness = Harness::new();
    let _ = scroll(&mut harness, finger(0, 600), 4);
    let track = harness.track(FINGER_ID);
    assert_eq!(
        harness.observe(),
        (RecogniserState::Scroll, Intent::Scroll { track })
    );
}

#[test]
fn two_deliberate_tracks_moving_vertically_stay_idle() {
    let mut harness = Harness::new();
    let a = finger(-300, 600);
    let b = TouchRecord::new(4, 300, 600);
    harness.feed(&report(&[
        a.state(touch_state::START),
        b.state(touch_state::START),
    ]));
    let (mut a, mut b) = (a, b);
    for _ in 0..4 {
        a = moved(a, -40);
        b = moved(b, -40);
        harness.feed(&report(&[a, b]));
    }
    assert_eq!(harness.observe(), (RecogniserState::Idle, Intent::None));
}

#[test]
fn a_second_deliberate_track_does_not_change_the_followed_one() {
    let mut harness = Harness::new();
    let current = scroll(&mut harness, finger(0, 600), 4);
    let track = harness.track(FINGER_ID);
    let mut other = TouchRecord::new(4, 600, 600);
    harness.feed(&report(&[current, other.state(touch_state::START)]));
    for _ in 0..4 {
        other = moved(other, -40);
        harness.feed(&report(&[current, other]));
    }
    assert_eq!(
        harness.observe(),
        (RecogniserState::Scroll, Intent::Scroll { track })
    );
}

#[test]
fn lift_ends_the_scroll_and_opens_the_fling_window() {
    let mut harness = Harness::new();
    let current = scroll(&mut harness, finger(0, 600), 4);
    let lift_t = harness.t;
    assert_eq!(
        lift(&mut harness, current),
        (RecogniserState::Idle, Intent::None)
    );
    assert_eq!(
        harness.recogniser().fling(),
        Some(Fling {
            start: lift_t,
            until: lift_t + FLING_WINDOW
        })
    );
    assert_eq!(harness.engine.next_deadline(), None);
}

#[test]
fn press_during_scroll_cancels_then_suppresses_until_lift_and_release() {
    let mut harness = Harness::new();
    let current = scroll(&mut harness, finger(0, 600), 4);
    let press_t = harness.t;
    assert_eq!(
        harness.feed(&report(&[palm(), current]).left(true)),
        (RecogniserState::Cancelling, Intent::Freeze)
    );
    assert_eq!(
        harness.engine.next_deadline(),
        Some(press_t + Micros::from_millis(60))
    );
    assert_eq!(
        harness.feed(&report(&[palm(), moved(current, -40)]).left(true)),
        (RecogniserState::Cancelling, Intent::Freeze)
    );
    assert_eq!(harness.fire(), (RecogniserState::Suppressed, Intent::None));
    assert_eq!(harness.engine.next_deadline(), None);
    assert_eq!(
        harness.feed(&report(&[palm(), current])),
        (RecogniserState::Suppressed, Intent::None)
    );
    assert_eq!(
        harness.feed(&report(&[palm(), current]).left(true)),
        (RecogniserState::Suppressed, Intent::None)
    );
    assert_eq!(
        harness.feed(&report(&[palm(), current.state(touch_state::LIFT_50)]).left(true)),
        (RecogniserState::Suppressed, Intent::None)
    );
    assert_eq!(
        harness.feed(&report(&[palm()]).left(true)),
        (RecogniserState::Suppressed, Intent::None)
    );
    assert_eq!(
        harness.feed(&report(&[palm()])),
        (RecogniserState::Idle, Intent::None)
    );
}

#[test]
fn suppressed_ignores_touches_rejected_before_the_cancel() {
    let mut harness = Harness::new();
    let current = scroll(&mut harness, finger(0, 600), 4);
    harness.feed(&report(&[palm(), current]).left(true));
    harness.fire();
    harness.feed(&report(&[palm(), current]));
    assert_eq!(
        harness.feed(&report(&[palm()])),
        (RecogniserState::Idle, Intent::None)
    );
}

#[test]
fn mouse_motion_during_scroll_cancels() {
    let mut harness = Harness::new();
    let current = scroll(&mut harness, finger(0, 600), 4);
    assert_eq!(
        harness.feed(&report(&[palm(), current]).motion(40, 0)),
        (RecogniserState::Cancelling, Intent::Freeze)
    );
    assert_eq!(harness.fire(), (RecogniserState::Suppressed, Intent::None));
    lift(&mut harness, current);
    assert_eq!(
        harness.feed(&report(&[palm()])),
        (RecogniserState::Idle, Intent::None)
    );
}

#[test]
fn lift_in_the_same_report_as_a_press_still_freezes() {
    let mut harness = Harness::new();
    let current = scroll(&mut harness, finger(0, 600), 4);
    assert_eq!(
        harness.feed(&report(&[palm(), current.state(touch_state::LIFT_50)]).left(true)),
        (RecogniserState::Cancelling, Intent::Freeze)
    );
}

#[test]
fn zero_cancel_hold_lifts_immediately() {
    let mut config = Config::default();
    config.advanced.scroll.cancel_hold_ms = 0;
    let mut harness = Harness::with_config(config);
    let current = scroll(&mut harness, finger(0, 600), 4);
    assert_eq!(
        harness.feed(&report(&[palm(), current]).left(true)),
        (RecogniserState::Suppressed, Intent::None)
    );
    assert_eq!(harness.engine.next_deadline(), None);
}

#[test]
fn scroll_disabled_never_scrolls() {
    let mut config = Config::default();
    config.scroll.enabled = false;
    let mut harness = Harness::with_config(config);
    let _ = scroll(&mut harness, finger(0, 600), 8);
    assert_eq!(harness.observe(), (RecogniserState::Idle, Intent::None));
}

#[test]
fn disabling_scroll_during_a_stroke_cancels_it() {
    let mut harness = Harness::new();
    let current = scroll(&mut harness, finger(0, 600), 4);
    let mut config = Config::default();
    config.scroll.enabled = false;
    let mut out = Vec::new();
    harness
        .engine
        .handle(Input::ConfigChanged(config), &mut out);
    assert_eq!(
        harness.feed(&report(&[palm(), current])),
        (RecogniserState::Cancelling, Intent::Freeze)
    );
}

#[test]
fn no_scroll_starts_while_a_button_is_down() {
    let mut harness = Harness::new();
    harness.feed(&report(&[palm()]).left(true));
    let touch = finger(0, 600);
    harness.feed(&report(&[palm(), touch.state(touch_state::START)]).left(true));
    let mut current = touch;
    for _ in 0..8 {
        current = moved(current, -40);
        assert_eq!(
            harness.feed(&report(&[palm(), current]).left(true)),
            (RecogniserState::Idle, Intent::None)
        );
    }
}

#[test]
fn touch_during_the_fling_window_holds_until_classified() {
    let mut harness = Harness::new();
    let current = scroll(&mut harness, finger(0, 600), 4);
    lift(&mut harness, current);
    harness.feed(&report(&[palm()]));
    let rest = finger(0, 300);
    assert_eq!(
        harness.feed(&report(&[palm(), rest.state(touch_state::START)])),
        (RecogniserState::Idle, Intent::Hold)
    );
    assert!(harness.recogniser().is_holding());
    assert_eq!(
        harness.feed(&report(&[palm(), rest])),
        (RecogniserState::Idle, Intent::Hold)
    );
    assert_eq!(harness.fire(), (RecogniserState::Idle, Intent::None));
    assert!(!harness.recogniser().is_holding());
    assert_eq!(harness.recogniser().fling(), None);
}

#[test]
fn hold_turns_into_a_scroll_when_the_touch_is_promoted() {
    let mut harness = Harness::new();
    let current = scroll(&mut harness, finger(0, 600), 4);
    lift(&mut harness, current);
    harness.feed(&report(&[palm()]));
    assert_eq!(
        harness.feed(&report(&[palm(), finger(0, 600).state(touch_state::START)])),
        (RecogniserState::Idle, Intent::Hold)
    );
    let mut touch = finger(0, 600);
    for _ in 0..4 {
        touch = moved(touch, -40);
        harness.feed(&report(&[palm(), touch]));
    }
    let track = harness.track(FINGER_ID);
    assert_eq!(
        harness.observe(),
        (RecogniserState::Scroll, Intent::Scroll { track })
    );
}

#[test]
fn a_press_ends_the_hold() {
    let mut harness = Harness::new();
    let current = scroll(&mut harness, finger(0, 600), 4);
    lift(&mut harness, current);
    harness.feed(&report(&[palm()]));
    let rest = finger(0, 300);
    harness.feed(&report(&[palm(), rest.state(touch_state::START)]));
    assert_eq!(
        harness.feed(&report(&[palm(), rest]).left(true)),
        (RecogniserState::Idle, Intent::None)
    );
}

#[test]
fn touches_after_the_fling_window_do_not_hold() {
    let mut harness = Harness::new();
    let current = scroll(&mut harness, finger(0, 600), 4);
    lift(&mut harness, current);
    harness.t += FLING_WINDOW;
    assert_eq!(
        harness.feed(&report(&[palm(), finger(0, 300).state(touch_state::START)])),
        (RecogniserState::Idle, Intent::None)
    );
    assert_eq!(harness.recogniser().fling(), None);
}

#[test]
fn touches_already_present_at_the_lift_do_not_hold() {
    let mut harness = Harness::new();
    let resting = TouchRecord::new(4, 800, 300);
    harness.feed(&report(&[resting.state(touch_state::START)]));
    let touch = finger(0, 600);
    harness.feed(&report(&[resting, touch.state(touch_state::START)]));
    let mut current = touch;
    for _ in 0..4 {
        current = moved(current, -40);
        harness.feed(&report(&[resting, current]));
    }
    assert_eq!(harness.observe().0, RecogniserState::Scroll);
    assert_eq!(
        harness.feed(&report(&[resting, current.state(touch_state::LIFT_50)])),
        (RecogniserState::Idle, Intent::None)
    );
    assert_eq!(
        harness.feed(&report(&[resting])),
        (RecogniserState::Idle, Intent::None)
    );
}

#[test]
fn reset_returns_to_idle() {
    let mut harness = Harness::new();
    let current = scroll(&mut harness, finger(0, 600), 4);
    harness.feed(&report(&[palm(), current]).left(true));
    let mut out = Vec::new();
    harness.engine.handle(Input::Reset, &mut out);
    assert_eq!(harness.recogniser().state(), RecogniserState::Idle);
    assert_eq!(harness.engine.next_deadline(), None);
}
