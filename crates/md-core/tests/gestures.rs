use std::fmt::Write as _;

use md_core::config::Config;
use md_core::profile::MAGIC_MOUSE_2_BT;
use md_core::recognise::RecogniserState;
use md_core::synth::{
    Frame, SCRIPT_DISTANCE_MM, SCRIPT_FRAME_INTERVAL, SWIPE_FINGER_SPACING_MM, VIRTUAL_TOUCHPAD,
};
use md_core::testkit::scenarios::{self, Scenario, drive, finger, palm, second_finger};
use md_core::testkit::{ReportBuilder, touch_state};
use md_core::{Engine, Input, Micros, Mode, Output};

fn config(source: &str) -> Config {
    Config::parse(source, &MAGIC_MOUSE_2_BT).unwrap().config
}

fn run_with(config: Config, scenario: &Scenario) -> Vec<(Micros, Output)> {
    let mut engine = Engine::new(&MAGIC_MOUSE_2_BT, config, Mode::Normal);
    drive(&mut engine, &scenario.timed())
}

fn run(scenario: &Scenario) -> Vec<(Micros, Output)> {
    run_with(Config::default(), scenario)
}

fn frames(timeline: &[(Micros, Output)]) -> Vec<(Micros, Frame)> {
    timeline
        .iter()
        .filter_map(|(t, output)| match output {
            Output::TouchFrame(frame) => Some((*t, *frame)),
            _ => None,
        })
        .collect()
}

fn render(timeline: &[(Micros, Output)]) -> String {
    let mut text = String::new();
    for (t, output) in timeline {
        writeln!(text, "{:>8} {output}", t.as_u64()).unwrap();
    }
    text
}

fn states(config: Config, scenario: &Scenario) -> Vec<RecogniserState> {
    let mut engine = Engine::new(&MAGIC_MOUSE_2_BT, config, Mode::Normal);
    let mut out = Vec::new();
    let mut seen = Vec::new();
    for (t, bytes) in scenario.timed() {
        engine.handle(Input::Report { t, bytes: &bytes }, &mut out);
        out.clear();
        let state = engine.recogniser().unwrap().state();
        if seen.last() != Some(&state) {
            seen.push(state);
        }
    }
    seen
}

fn slot0_travel(frames: &[(Micros, Frame)]) -> (i32, i32) {
    let moving: Vec<_> = frames
        .iter()
        .filter_map(|(_, frame)| frame.slots[0])
        .collect();
    let (first, last) = (moving[0], moving[moving.len() - 1]);
    (last.x - first.x, last.y - first.y)
}

#[test]
fn slow_scroll_is_unchanged_at_gain_min() {
    let scenario = scenarios::slow_scroll();
    let medium = frames(&run(&scenario));
    let off = frames(&run_with(
        config("[scroll]\nacceleration = \"off\"\n"),
        &scenario,
    ));
    assert_eq!(medium, off);
    let high = frames(&run_with(
        config("[scroll]\nacceleration = \"high\"\n"),
        &scenario,
    ));
    assert_eq!(high, off);
}

#[test]
fn flick_is_amplified_by_the_acceleration_presets() {
    let scenario = scenarios::flick();
    let travel = |source: &str| -slot0_travel(&frames(&run_with(config(source), &scenario))).1;
    let off = travel("[scroll]\nacceleration = \"off\"\n");
    let low = travel("[scroll]\nacceleration = \"low\"\n");
    let medium = travel("");
    let high = travel("[scroll]\nacceleration = \"high\"\n");
    assert!(off > 0);
    assert!(low > off, "low {low} off {off}");
    assert!(medium > low, "medium {medium} low {low}");
    assert!(high >= medium, "high {high} medium {medium}");
    assert!(
        f64::from(medium) > 1.5 * f64::from(off),
        "medium {medium} off {off}"
    );
}

#[test]
fn axis_lock_keeps_a_drifting_vertical_stroke_vertical() {
    let scenario = scenarios::diagonal_drift();
    let locked = frames(&run(&scenario));
    assert!(locked.len() > 40);
    let (dx, dy) = slot0_travel(&locked);
    assert_eq!(dx, 0);
    assert!(dy < -VIRTUAL_TOUCHPAD.mm_to_units(20.0));
    let free = frames(&run_with(
        config("[scroll]\naxis_lock = false\n"),
        &scenario,
    ));
    let (dx, dy) = slot0_travel(&free);
    assert!(dx > VIRTUAL_TOUCHPAD.mm_to_units(5.0), "{dx}");
    assert!(dy < -VIRTUAL_TOUCHPAD.mm_to_units(20.0));
}

#[test]
fn axis_lock_keeps_a_drifting_horizontal_stroke_horizontal() {
    let scenario = scenarios::horizontal_drift();
    let (dx, dy) = slot0_travel(&frames(&run(&scenario)));
    assert_eq!(dy, 0);
    assert!(dx > VIRTUAL_TOUCHPAD.mm_to_units(30.0));
    let (dx, dy) = slot0_travel(&frames(&run_with(
        config("[scroll]\naxis_lock = false\n"),
        &scenario,
    )));
    assert!(dy > VIRTUAL_TOUCHPAD.mm_to_units(3.0), "{dy}");
    assert!(dx > VIRTUAL_TOUCHPAD.mm_to_units(30.0));
}

fn assert_swipe(frames: &[(Micros, Frame)], fingers: usize, direction: i32) {
    assert!(frames.len() > 5);
    let (_, first) = frames[0];
    assert_eq!(first.contact_count(), fingers);
    let row: Vec<_> = first.contacts().map(|(_, contact)| contact).collect();
    for pair in row.windows(2) {
        assert_eq!(
            pair[1].x - pair[0].x,
            VIRTUAL_TOUCHPAD.mm_to_units(SWIPE_FINGER_SPACING_MM)
        );
        assert_eq!(pair[1].y, pair[0].y);
    }
    let mut previous = row[0].x;
    for (_, frame) in &frames[..frames.len() - 1] {
        assert_eq!(frame.contact_count(), fingers);
        for (index, (_, contact)) in frame.contacts().enumerate() {
            assert_eq!(contact.y, row[index].y);
            assert_eq!(contact.tracking_id, row[index].tracking_id);
        }
        let x = frame.slots[0].unwrap().x;
        assert!((x - previous) * direction >= 0);
        previous = x;
    }
    let travel = (previous - row[0].x) * direction;
    assert!(travel > VIRTUAL_TOUCHPAD.mm_to_units(20.0), "{travel}");
    assert!(frames[frames.len() - 1].1.is_empty());
}

#[test]
fn two_finger_swipe_emits_kwin_swipe_fingers_moving_sideways() {
    let swipe = frames(&run(&scenarios::two_finger_swipe()));
    assert_swipe(&swipe, 4, 1);
    let left = frames(&run(&scenarios::two_finger_swipe_left()));
    assert_swipe(&left, 4, -1);
    assert_eq!(
        states(Config::default(), &scenarios::two_finger_swipe()),
        [
            RecogniserState::Idle,
            RecogniserState::Swipe,
            RecogniserState::Suppressed,
            RecogniserState::Idle
        ]
    );
}

#[test]
fn swipe_finger_count_follows_the_config() {
    for fingers in [3, 5] {
        let config = config(&format!(
            "[advanced.gestures]\nkwin_swipe_fingers = {fingers}\n"
        ));
        let swipe = frames(&run_with(config, &scenarios::two_finger_swipe()));
        assert_swipe(&swipe, fingers, 1);
    }
}

#[test]
fn swipe_disabled_means_two_fingers_produce_nothing() {
    let config = config("[gestures]\nswipe_desktops = false\n");
    for scenario in [
        scenarios::two_finger_swipe(),
        scenarios::two_finger_swipe_left(),
    ] {
        assert!(frames(&run_with(config.clone(), &scenario)).is_empty());
        assert!(
            !states(config.clone(), &scenario).contains(&RecogniserState::Swipe),
            "swipe state while disabled"
        );
    }
}

#[test]
fn swipe_distance_follows_the_config() {
    let config = config("[advanced.gestures]\nswipe_distance_mm = 40.0\n");
    assert!(frames(&run_with(config, &scenarios::two_finger_swipe())).is_empty());
}

#[test]
fn one_finger_horizontal_scroll_never_becomes_a_swipe() {
    for scenario in [
        scenarios::horizontal_scroll(),
        scenarios::horizontal_drift(),
    ] {
        let frames = frames(&run(&scenario));
        assert!(!frames.is_empty());
        assert!(
            frames
                .iter()
                .all(|(_, frame)| frame.is_empty() || frame.contact_count() == 2)
        );
        assert!(!states(Config::default(), &scenario).contains(&RecogniserState::Swipe));
    }
}

#[test]
fn two_fingers_moving_vertically_do_nothing() {
    let scenario = scenarios::two_finger_vertical();
    assert!(frames(&run(&scenario)).is_empty());
    assert_eq!(
        states(Config::default(), &scenario),
        [RecogniserState::Idle]
    );
}

#[test]
fn lifting_one_swipe_finger_ends_the_swipe_and_the_other_never_scrolls() {
    let mut scenario = Scenario::new();
    let (mut ax, mut bx) = (-500, 100);
    scenario.push(
        ReportBuilder::new()
            .touch(palm())
            .touch(finger(ax, 200).state(touch_state::START))
            .touch(second_finger(bx, 200).state(touch_state::START)),
    );
    for _ in 0..12 {
        ax += 40;
        bx += 40;
        scenario.push(
            ReportBuilder::new()
                .touch(palm())
                .touch(finger(ax, 200))
                .touch(second_finger(bx, 200)),
        );
    }
    scenario.push(
        ReportBuilder::new()
            .touch(palm())
            .touch(finger(ax, 200))
            .touch(second_finger(bx, 200).state(touch_state::LIFT_50)),
    );
    let mut y = 200;
    for _ in 0..30 {
        y -= 30;
        scenario.push(ReportBuilder::new().touch(palm()).touch(finger(ax, y)));
    }
    scenario.push(ReportBuilder::new().touch(palm()));
    let reports = scenario.timed();
    let lift_t = reports[13].0;
    let frames = frames(&run(&scenario));
    let lift = frames
        .iter()
        .position(|(_, frame)| frame.is_empty())
        .unwrap();
    assert_eq!(frames[lift].0, lift_t);
    assert_eq!(frames.len(), lift + 1);
    assert!(
        frames[..lift]
            .iter()
            .all(|(_, frame)| frame.contact_count() == 4)
    );
}

#[test]
fn click_during_a_swipe_freezes_for_the_cancel_hold_then_lifts() {
    let mut scenario = Scenario::new();
    let (mut ax, mut bx) = (-500, 100);
    scenario.push(
        ReportBuilder::new()
            .touch(finger(ax, 200).state(touch_state::START))
            .touch(second_finger(bx, 200).state(touch_state::START)),
    );
    for frame in 0..20 {
        ax += 40;
        bx += 40;
        scenario.push(
            ReportBuilder::new()
                .left((12..16).contains(&frame))
                .touch(finger(ax, 200))
                .touch(second_finger(bx, 200)),
        );
    }
    scenario.push(ReportBuilder::new());
    let timeline = run(&scenario);
    let press_t = timeline
        .iter()
        .find(|(_, output)| matches!(output, Output::Button { pressed: true, .. }))
        .unwrap()
        .0;
    let frames = frames(&timeline);
    let after: Vec<_> = frames.iter().filter(|(t, _)| *t >= press_t).collect();
    assert_eq!(after.len(), 1);
    assert!(after[0].1.is_empty());
    assert_eq!(after[0].0, press_t + Micros::from_millis(60));
    assert_eq!(frames[frames.len() - 2].1.contact_count(), 4);
}

#[test]
fn two_finger_swipe_frames() {
    insta::assert_snapshot!(
        "two_finger_swipe",
        render(&run(&scenarios::two_finger_swipe()))
    );
}

#[test]
fn double_tap_plays_the_scripted_swipe_up_from_the_timer() {
    let scenario = scenarios::two_finger_double_tap();
    let reports = scenario.timed();
    let frames = frames(&run(&scenario));
    let (start, first) = frames[0];
    assert!(reports.iter().any(|(t, _)| *t == start));
    assert_eq!(first.contact_count(), 4);
    for pair in frames.windows(2) {
        assert_eq!(pair[1].0 - pair[0].0, SCRIPT_FRAME_INTERVAL);
    }
    assert!(
        frames[1..]
            .iter()
            .any(|(t, _)| reports.iter().all(|(report_t, _)| report_t != t))
    );
    let top = frames[frames.len() - 2].1;
    for ((_, start), (_, end)) in first.contacts().zip(top.contacts()) {
        assert_eq!(start.x, end.x);
        assert_eq!(start.tracking_id, end.tracking_id);
        assert_eq!(
            start.y - end.y,
            VIRTUAL_TOUCHPAD.mm_to_units(SCRIPT_DISTANCE_MM)
        );
    }
    assert!(frames[frames.len() - 1].1.is_empty());
}

#[test]
fn double_tap_overview_frames() {
    insta::assert_snapshot!(
        "two_finger_double_tap",
        render(&run(&scenarios::two_finger_double_tap()))
    );
}

#[test]
fn single_tap_tap_with_click_and_slow_second_tap_do_nothing() {
    for (name, scenario) in [
        ("single", scenarios::two_finger_single_tap()),
        ("click", scenarios::two_finger_tap_with_click()),
        ("slow", scenarios::two_finger_slow_second_tap()),
    ] {
        let timeline = run(&scenario);
        assert!(frames(&timeline).is_empty(), "{name}");
        assert!(
            timeline
                .iter()
                .all(|(_, output)| matches!(output, Output::Button { .. })),
            "{name}"
        );
    }
}

#[test]
fn a_tap_waits_for_the_second_one_until_the_double_tap_window_ends() {
    let scenario = scenarios::two_finger_single_tap();
    let mut engine = Engine::new(&MAGIC_MOUSE_2_BT, Config::default(), Mode::Normal);
    let mut out = Vec::new();
    let mut pending = None;
    for (t, bytes) in scenario.timed() {
        engine.handle(Input::Report { t, bytes: &bytes }, &mut out);
        let recogniser = engine.recogniser().unwrap();
        if pending.is_none() && recogniser.state() == RecogniserState::TapPending {
            pending = Some(t);
            assert_eq!(engine.next_deadline(), Some(t + Micros::from_millis(300)));
        }
        if pending.is_some_and(|pending| t >= pending + Micros::from_millis(300)) {
            assert_eq!(recogniser.state(), RecogniserState::Idle);
        }
    }
    assert!(pending.is_some());
    assert!(out.is_empty());
}

#[test]
fn double_tap_window_follows_the_config() {
    let scenario = scenarios::two_finger_slow_second_tap();
    let config = config("[advanced.gestures]\ndouble_tap_ms = 600\n");
    let frames = frames(&run_with(config, &scenario));
    assert_eq!(frames[0].1.contact_count(), 4);
}

#[test]
fn double_tap_disabled_does_nothing() {
    let config = config("[gestures]\ndouble_tap_overview = false\n");
    let scenario = scenarios::two_finger_double_tap();
    assert!(frames(&run_with(config.clone(), &scenario)).is_empty());
    assert!(!states(config, &scenario).contains(&RecogniserState::TapPending));
}

#[test]
fn double_tap_during_a_fling_lifts_the_hold_before_the_script() {
    let mut scenario = scenarios::flick();
    scenario.repeat(10, &ReportBuilder::new().touch(palm()));
    scenario.two_finger_tap(&[palm()], 6, false);
    scenario.repeat(12, &ReportBuilder::new().touch(palm()));
    scenario.two_finger_tap(&[palm()], 6, false);
    scenario.repeat(30, &ReportBuilder::new().touch(palm()));
    let frames = frames(&run(&scenario));
    let mut previous = 0;
    let mut scripted = false;
    for (_, frame) in &frames {
        let count = frame.contact_count();
        assert!(count == 0 || previous == 0 || count == previous);
        scripted |= count == 4;
        previous = count;
    }
    assert!(scripted);
    assert!(frames[frames.len() - 1].1.is_empty());
}

#[test]
fn scroll_disabled_keeps_the_gestures() {
    let config = config("[scroll]\nenabled = false\n");
    assert_swipe(
        &frames(&run_with(config.clone(), &scenarios::two_finger_swipe())),
        4,
        1,
    );
    assert_eq!(
        frames(&run_with(
            config.clone(),
            &scenarios::two_finger_double_tap()
        ))[0]
            .1
            .contact_count(),
        4
    );
    assert!(frames(&run_with(config, &scenarios::horizontal_scroll())).is_empty());
}
