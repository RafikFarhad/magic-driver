use std::fmt::Write as _;

use super::*;
use crate::config::Config;
use crate::profile::MAGIC_MOUSE_2_BT;
use crate::synth::{SCROLL_PROBE_FINGER_SPACING_MM, TOUCHPAD_SLOTS, VIRTUAL_TOUCHPAD};
use crate::testkit::{ReportBuilder, TouchRecord, touch_state};
use crate::tracker::Phase;

const FRAME: u64 = 11_111;

fn engine(mode: Mode) -> Engine {
    Engine::new(&MAGIC_MOUSE_2_BT, Config::default(), mode)
}

fn feed(engine: &mut Engine, reports: &[ReportBuilder]) -> Vec<(u64, Output)> {
    let mut timeline = Vec::new();
    let mut out = Vec::new();
    for (index, builder) in (0_u64..).zip(reports) {
        let t = index * FRAME;
        let bytes = builder.build();
        engine.handle(
            Input::Report {
                t: Micros::new(t),
                bytes: &bytes,
            },
            &mut out,
        );
        timeline.extend(out.drain(..).map(|output| (t, output)));
    }
    timeline
}

fn outputs(engine: &mut Engine, reports: &[ReportBuilder]) -> Vec<Output> {
    feed(engine, reports)
        .into_iter()
        .map(|(_, output)| output)
        .collect()
}

fn reset(engine: &mut Engine) -> Vec<Output> {
    let mut out = Vec::new();
    engine.handle(Input::Reset, &mut out);
    out
}

fn motion(dx: i32, dy: i32) -> Output {
    Output::PointerMotion { dx, dy }
}

fn button(button: Button, pressed: bool) -> Output {
    Output::Button { button, pressed }
}

fn finger(id: u8, x: i32, y: i32) -> TouchRecord {
    TouchRecord::new(id, x, y).size(12)
}

fn frames(outputs: &[Output]) -> Vec<Frame> {
    outputs
        .iter()
        .filter_map(|output| match output {
            Output::TouchFrame(frame) => Some(*frame),
            _ => None,
        })
        .collect()
}

fn render(timeline: &[(u64, Output)]) -> String {
    let mut text = String::new();
    for (t, output) in timeline {
        writeln!(text, "{t:>7} {output}").unwrap();
    }
    text
}

#[test]
fn movement_passes_through_unchanged() {
    let deltas: [(i16, i16); 7] = [
        (1, 0),
        (0, -1),
        (-37, 52),
        (300, -450),
        (i16::MIN, i16::MAX),
        (i16::MAX, i16::MIN),
        (2, 2),
    ];
    let reports: Vec<ReportBuilder> = deltas
        .iter()
        .map(|(dx, dy)| {
            ReportBuilder::new()
                .motion(*dx, *dy)
                .touch(finger(1, 0, 1500))
        })
        .collect();
    let expected: Vec<Output> = deltas
        .iter()
        .map(|(dx, dy)| motion(i32::from(*dx), i32::from(*dy)))
        .collect();
    for mode in [Mode::PointerOnly, Mode::RawForward, Mode::ScrollProbe] {
        let mut engine = engine(mode);
        let pointer: Vec<Output> = outputs(&mut engine, &reports)
            .into_iter()
            .filter(|output| matches!(output, Output::PointerMotion { .. }))
            .collect();
        assert_eq!(pointer, expected, "{mode:?}");
    }
}

#[test]
fn zero_motion_emits_nothing() {
    let mut engine = engine(Mode::PointerOnly);
    assert!(outputs(&mut engine, &[ReportBuilder::new(), ReportBuilder::new()]).is_empty());
}

#[test]
fn left_click_with_finger_on_the_left() {
    let mut engine = engine(Mode::PointerOnly);
    let touch = finger(1, -700, 0);
    let got = outputs(
        &mut engine,
        &[
            ReportBuilder::new().touch(touch),
            ReportBuilder::new().left(true).touch(touch),
            ReportBuilder::new().left(true).touch(touch),
            ReportBuilder::new().touch(touch),
        ],
    );
    assert_eq!(
        got,
        vec![button(Button::Left, true), button(Button::Left, false)]
    );
}

#[test]
fn right_click_with_finger_on_the_right_overrides_the_mouse_guess() {
    let mut engine = engine(Mode::PointerOnly);
    let touch = finger(1, 700, 0);
    let got = outputs(
        &mut engine,
        &[
            ReportBuilder::new().left(true).touch(touch),
            ReportBuilder::new().touch(touch),
        ],
    );
    assert_eq!(
        got,
        vec![button(Button::Right, true), button(Button::Right, false)]
    );
}

#[test]
fn middle_click_and_the_middle_click_switch() {
    let mut engine = engine(Mode::PointerOnly);
    let touch = finger(1, 0, 0);
    let click = [
        ReportBuilder::new().left(true).touch(touch),
        ReportBuilder::new().touch(touch),
    ];
    assert_eq!(
        outputs(&mut engine, &click),
        vec![button(Button::Middle, true), button(Button::Middle, false)]
    );
    let mut config = Config::default();
    config.pointer.middle_click = false;
    let mut out = Vec::new();
    engine.handle(Input::ConfigChanged(config.clone()), &mut out);
    assert!(out.is_empty());
    assert_eq!(engine.config(), &config);
    assert_eq!(
        outputs(&mut engine, &click),
        vec![button(Button::Left, true), button(Button::Left, false)]
    );
}

#[test]
fn config_change_during_a_press_keeps_the_latched_button() {
    let mut engine = engine(Mode::PointerOnly);
    let touch = finger(1, 0, 0);
    let mut got = outputs(&mut engine, &[ReportBuilder::new().left(true).touch(touch)]);
    let mut config = Config::default();
    config.pointer.middle_click = false;
    engine.handle(Input::ConfigChanged(config), &mut got);
    got.extend(outputs(&mut engine, &[ReportBuilder::new().touch(touch)]));
    assert_eq!(
        got,
        vec![button(Button::Middle, true), button(Button::Middle, false)]
    );
}

#[test]
fn finger_moving_during_a_press_cannot_change_the_button() {
    let mut engine = engine(Mode::PointerOnly);
    let got = outputs(
        &mut engine,
        &[
            ReportBuilder::new().left(true).touch(finger(1, -700, 0)),
            ReportBuilder::new().right(true).touch(finger(1, 0, 0)),
            ReportBuilder::new().right(true).touch(finger(1, 700, 0)),
            ReportBuilder::new().touch(finger(1, 700, 0)),
        ],
    );
    assert_eq!(
        got,
        vec![button(Button::Left, true), button(Button::Left, false)]
    );
}

#[test]
fn click_without_touches_uses_the_mouse_guess() {
    let mut engine = engine(Mode::PointerOnly);
    let got = outputs(
        &mut engine,
        &[
            ReportBuilder::new().right(true),
            ReportBuilder::new(),
            ReportBuilder::new().left(true).right(true),
            ReportBuilder::new(),
        ],
    );
    assert_eq!(
        got,
        vec![
            button(Button::Right, true),
            button(Button::Right, false),
            button(Button::Left, true),
            button(Button::Right, true),
            button(Button::Left, false),
            button(Button::Right, false),
        ]
    );
}

#[test]
fn motion_is_emitted_before_the_button_in_the_same_report() {
    let mut engine = engine(Mode::PointerOnly);
    let got = outputs(
        &mut engine,
        &[ReportBuilder::new()
            .motion(4, -1)
            .left(true)
            .touch(finger(1, -700, 0))],
    );
    assert_eq!(got, vec![motion(4, -1), button(Button::Left, true)]);
}

#[test]
fn reset_releases_pressed_buttons() {
    let mut engine = engine(Mode::PointerOnly);
    outputs(&mut engine, &[ReportBuilder::new().left(true).right(true)]);
    assert_eq!(
        reset(&mut engine),
        vec![button(Button::Left, false), button(Button::Right, false)]
    );
    assert!(reset(&mut engine).is_empty());
    assert!(!engine.context().is_button_down());
    assert!(outputs(&mut engine, &[ReportBuilder::new()]).is_empty());
}

#[test]
fn reset_drops_tracks_and_context() {
    let mut engine = engine(Mode::PointerOnly);
    outputs(
        &mut engine,
        &[ReportBuilder::new().motion(50, 0).touch(finger(1, 0, 0))],
    );
    assert_eq!(engine.tracker().tracks().count(), 1);
    reset(&mut engine);
    assert_eq!(engine.tracker().tracks().count(), 0);
    assert!(engine.context().motion_energy(Micros::ZERO).abs() < f64::EPSILON);
}

#[test]
fn pointer_only_mode_tracks_touches_without_emitting_them() {
    let mut engine = engine(Mode::PointerOnly);
    let got = outputs(
        &mut engine,
        &[
            ReportBuilder::new().touch(finger(1, 0, 0).state(touch_state::START)),
            ReportBuilder::new().touch(finger(1, 30, 40)),
        ],
    );
    assert!(got.is_empty());
    let track = engine.tracker().by_touch_id(1).expect("track");
    assert_eq!(track.phase, Phase::Active);
    assert!(engine.context().motion_energy(Micros::ZERO).abs() < f64::EPSILON);
}

#[test]
fn other_report_ids_and_malformed_reports_are_counted_and_emit_nothing() {
    let mut engine = engine(Mode::RawForward);
    let mut out = Vec::new();
    let mut valid = ReportBuilder::new().motion(1, 1).left(true).build();
    engine.handle(
        Input::Report {
            t: Micros::ZERO,
            bytes: &valid,
        },
        &mut out,
    );
    out.clear();
    valid[0] = 0x29;
    let inputs: [&[u8]; 4] = [&valid, &[], &[0x12, 0x01, 0x05], &[0x12; 15]];
    for bytes in inputs {
        engine.handle(
            Input::Report {
                t: Micros::ZERO,
                bytes,
            },
            &mut out,
        );
    }
    assert!(out.is_empty());
    assert_eq!(
        engine.stats(),
        EngineStats {
            reports: 5,
            ignored: 1,
            malformed: 3
        }
    );
    assert!(engine.context().is_button_down());
}

#[test]
fn short_reports_carry_pointer_and_buttons_but_leave_tracks_alone() {
    let mut engine = engine(Mode::RawForward);
    let full = ReportBuilder::new().touch(finger(1, -700, 0)).build();
    let short = [0x12, 0x01, 0x05, 0x00, 0xfb, 0xff, 0x00, 0x00];
    let mut out = Vec::new();
    engine.handle(
        Input::Report {
            t: Micros::ZERO,
            bytes: &full,
        },
        &mut out,
    );
    assert_eq!(frames(&out).len(), 1);
    out.clear();
    engine.handle(
        Input::Report {
            t: Micros::new(FRAME),
            bytes: &short,
        },
        &mut out,
    );
    assert_eq!(out, vec![motion(5, -5), button(Button::Left, true)]);
    assert_eq!(engine.tracker().contacts().count(), 1);
}

#[test]
fn timers_and_deadlines_are_idle() {
    let mut engine = engine(Mode::ScrollProbe);
    assert_eq!(engine.next_deadline(), None);
    let mut out = Vec::new();
    engine.handle(Input::Timer { t: Micros::MAX }, &mut out);
    assert!(out.is_empty());
}

#[test]
fn tracing_flag_is_stored() {
    let mut engine = engine(Mode::PointerOnly);
    assert!(!engine.is_tracing());
    engine.set_tracing(true);
    assert!(engine.is_tracing());
    engine.set_tracing(false);
    assert!(!engine.is_tracing());
}

#[test]
fn handle_appends_to_the_output_vector() {
    let mut engine = engine(Mode::PointerOnly);
    let mut out = vec![motion(9, 9)];
    let bytes = ReportBuilder::new().motion(1, 2).build();
    engine.handle(
        Input::Report {
            t: Micros::ZERO,
            bytes: &bytes,
        },
        &mut out,
    );
    assert_eq!(out, vec![motion(9, 9), motion(1, 2)]);
}

#[test]
fn accessors_report_construction_values() {
    let engine = engine(Mode::ScrollProbe);
    assert_eq!(engine.mode(), Mode::ScrollProbe);
    assert_eq!(engine.profile().id, "magic-mouse-2-bt");
    assert_eq!(engine.stats(), EngineStats::default());
}

#[test]
fn output_display_is_one_line_per_event() {
    assert_eq!(motion(3, -2).to_string(), "pointer dx=3 dy=-2");
    assert_eq!(
        button(Button::Right, true).to_string(),
        "button right press"
    );
    assert_eq!(
        button(Button::Middle, false).to_string(),
        "button middle release"
    );
    assert_eq!(
        Output::TouchFrame(Frame::EMPTY).to_string(),
        "frame 0:- 1:- 2:- 3:- 4:-"
    );
}

fn stroke(id: u8) -> Vec<ReportBuilder> {
    let palm = TouchRecord::new(1, 0, 1600)
        .touch_major(120)
        .touch_minor(90);
    let finger = |state: u8, y: i32| TouchRecord::new(id, -200, y).state(state);
    vec![
        ReportBuilder::new().touch(palm),
        ReportBuilder::new()
            .touch(palm)
            .touch(TouchRecord::new(id, -200, 400).state(touch_state::HOVER)),
        ReportBuilder::new()
            .touch(palm)
            .touch(finger(touch_state::START, 400)),
        ReportBuilder::new()
            .touch(palm)
            .touch(finger(touch_state::DRAG, 300)),
        ReportBuilder::new()
            .motion(2, 0)
            .touch(palm)
            .touch(finger(touch_state::DRAG, 150)),
        ReportBuilder::new()
            .touch(palm)
            .touch(finger(touch_state::LIFT_50, 140)),
        ReportBuilder::new()
            .touch(palm)
            .touch(finger(touch_state::GONE, 140)),
        ReportBuilder::new().touch(palm.state(touch_state::LIFT_70)),
        ReportBuilder::new(),
    ]
}

fn assert_slots_pair(frames: &[Frame]) {
    let mut open: [Option<u16>; TOUCHPAD_SLOTS] = [None; TOUCHPAD_SLOTS];
    for frame in frames {
        for (slot, contact) in frame.slots.iter().enumerate() {
            match (open[slot], contact) {
                (Some(id), Some(contact)) => assert_eq!(
                    id, contact.tracking_id,
                    "slot {slot} changed id without closing"
                ),
                (None, Some(contact)) => open[slot] = Some(contact.tracking_id),
                (Some(_), None) => open[slot] = None,
                (None, None) => {}
            }
        }
    }
    assert_eq!(open, [None; TOUCHPAD_SLOTS], "slots left open");
}

#[test]
fn raw_forward_maps_each_contact_to_its_own_slot() {
    let mut engine = engine(Mode::RawForward);
    let timeline = feed(&mut engine, &stroke(2));
    let frames = frames(
        &timeline
            .iter()
            .map(|(_, output)| *output)
            .collect::<Vec<_>>(),
    );
    assert_slots_pair(&frames);
    let max_contacts = frames.iter().map(Frame::contact_count).max().unwrap();
    assert_eq!(max_contacts, 2);
    insta::assert_snapshot!("raw_forward_stroke_with_palm", render(&timeline));
}

#[test]
fn raw_forward_positions_are_scaled_into_the_virtual_touchpad() {
    let mut engine = engine(Mode::RawForward);
    let got = outputs(
        &mut engine,
        &[
            ReportBuilder::new().touch(finger(1, 0, 0)),
            ReportBuilder::new().touch(finger(1, 26, 71)),
        ],
    );
    let frames = frames(&got);
    let first = frames[0].slots[0].unwrap();
    let second = frames[1].slots[0].unwrap();
    assert_eq!(first.tracking_id, second.tracking_id);
    let units = VIRTUAL_TOUCHPAD.units_per_mm;
    assert!((second.x - first.x - units).abs() <= 1);
    assert!((second.y - first.y - units).abs() <= 1);
}

#[test]
fn raw_forward_frames_are_only_sent_on_change() {
    let mut engine = engine(Mode::RawForward);
    let still = ReportBuilder::new().touch(finger(1, 0, 0));
    let got = outputs(&mut engine, &[still.clone(), still.clone(), still]);
    assert_eq!(frames(&got).len(), 1);
}

#[test]
fn raw_forward_does_not_reuse_a_slot_in_the_frame_that_closes_it() {
    let mut engine = engine(Mode::RawForward);
    let got = outputs(
        &mut engine,
        &[
            ReportBuilder::new().touch(finger(1, 0, 0)),
            ReportBuilder::new().touch(finger(2, 100, 0)),
            ReportBuilder::new().touch(finger(2, 120, 0)),
        ],
    );
    let frames = frames(&got);
    assert_eq!(frames[0].slots[0].map(|c| c.tracking_id), Some(0));
    assert!(frames[1].slots[0].is_none());
    assert_eq!(frames[1].slots[1].map(|c| c.tracking_id), Some(1));
    assert_slots_pair(&[frames[0], frames[1], frames[2], Frame::EMPTY]);
}

#[test]
fn raw_forward_uses_at_most_the_touchpad_slots() {
    let mut engine = engine(Mode::RawForward);
    let crowd = (0..8_u8).fold(ReportBuilder::new(), |builder, id| {
        builder.touch(finger(id, i32::from(id) * 100 - 400, 0))
    });
    let got = outputs(&mut engine, &[crowd]);
    assert_eq!(frames(&got)[0].contact_count(), TOUCHPAD_SLOTS);
}

#[test]
fn raw_forward_hover_and_lift_are_not_forwarded() {
    let mut engine = engine(Mode::RawForward);
    let got = outputs(
        &mut engine,
        &[
            ReportBuilder::new().touch(finger(1, 0, 0).state(touch_state::HOVER)),
            ReportBuilder::new().touch(finger(1, 0, 0).state(touch_state::LIFT_60)),
        ],
    );
    assert!(frames(&got).is_empty());
}

#[test]
fn reset_lifts_every_slot() {
    for mode in [Mode::RawForward, Mode::ScrollProbe] {
        let mut engine = engine(mode);
        let report = ReportBuilder::new()
            .left(true)
            .touch(finger(1, -700, 0))
            .touch(finger(2, 700, 0))
            .touch(finger(3, 0, 1500));
        let mut got = outputs(&mut engine, &[report]);
        let opened = frames(&got)[0].contact_count();
        assert!(opened >= 2, "{mode:?}");
        let lifted = reset(&mut engine);
        assert_eq!(
            lifted,
            vec![
                button(Button::Left, false),
                Output::TouchFrame(Frame::EMPTY)
            ],
            "{mode:?}"
        );
        got.extend(lifted);
        assert_slots_pair(&frames(&got));
        assert!(reset(&mut engine).is_empty());
    }
}

#[test]
fn scroll_probe_turns_one_finger_into_two_side_by_side() {
    let mut engine = engine(Mode::ScrollProbe);
    let timeline = feed(&mut engine, &stroke(2));
    let all: Vec<Output> = timeline.iter().map(|(_, output)| *output).collect();
    let frames = frames(&all);
    assert_slots_pair(&frames);
    for frame in &frames {
        let count = frame.contact_count();
        assert!(count == 0 || count == 2, "{frame}");
        if let (Some(a), Some(b)) = (frame.slots[0], frame.slots[1]) {
            assert_eq!(
                b.x - a.x,
                VIRTUAL_TOUCHPAD.mm_to_units(SCROLL_PROBE_FINGER_SPACING_MM)
            );
            assert_eq!(a.y, b.y);
        }
    }
    insta::assert_snapshot!("scroll_probe_stroke_with_palm", render(&timeline));
}

#[test]
fn scroll_probe_starts_at_the_centre_and_follows_the_finger_delta() {
    let mut engine = engine(Mode::ScrollProbe);
    let got = outputs(
        &mut engine,
        &[
            ReportBuilder::new().touch(finger(4, 500, -900).state(touch_state::START)),
            ReportBuilder::new().touch(finger(4, 500, -900 + 71 * 5)),
        ],
    );
    let frames = frames(&got);
    let centre = VIRTUAL_TOUCHPAD.centre();
    let half = VIRTUAL_TOUCHPAD.mm_to_units(SCROLL_PROBE_FINGER_SPACING_MM / 2.0);
    let start = frames[0];
    assert_eq!(start.slots[0].unwrap().x, centre.x - half);
    assert_eq!(start.slots[1].unwrap().x, centre.x + half);
    assert_eq!(start.slots[0].unwrap().y, centre.y);
    let moved = frames[1].slots[0].unwrap();
    let units = VIRTUAL_TOUCHPAD.units_per_mm;
    assert!((moved.y - centre.y - 5 * units).abs() <= 2, "{moved:?}");
    assert_eq!(moved.x, centre.x - half);
}

#[test]
fn scroll_probe_follows_the_newest_contact_and_lifts_before_switching() {
    let mut engine = engine(Mode::ScrollProbe);
    let got = outputs(
        &mut engine,
        &[
            ReportBuilder::new().touch(finger(1, 0, 0)),
            ReportBuilder::new()
                .touch(finger(1, 0, 0))
                .touch(finger(2, 300, 0)),
            ReportBuilder::new().touch(finger(2, 300, 0)),
            ReportBuilder::new().touch(finger(2, 300, 100)),
            ReportBuilder::new(),
        ],
    );
    let frames = frames(&got);
    let ids: Vec<Option<u16>> = frames
        .iter()
        .map(|f| f.slots[0].map(|c| c.tracking_id))
        .collect();
    assert_eq!(ids, vec![Some(0), None, Some(2), Some(2), None]);
    assert_eq!(frames[2].slots[0].unwrap().y, VIRTUAL_TOUCHPAD.centre().y);
    assert_slots_pair(&frames);
}
