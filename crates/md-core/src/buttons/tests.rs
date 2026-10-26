use proptest::prelude::*;

use super::*;
use crate::profile::MAGIC_MOUSE_2_BT;
use crate::report::TouchState;

const ZONES: ButtonZones = MAGIC_MOUSE_2_BT.buttons;

fn touch(id: u8, x: i32, size: u8) -> Touch {
    Touch {
        id,
        x,
        y: 0,
        touch_major: 40,
        touch_minor: 30,
        size,
        orientation: 0,
        state: TouchState::Drag,
    }
}

fn with_state(mut touch: Touch, state: TouchState) -> Touch {
    touch.state = state;
    touch
}

fn set(buttons: &[Button]) -> ButtonSet {
    let mut set = ButtonSet::EMPTY;
    for button in buttons {
        set.set(*button, true);
    }
    set
}

fn press(clicks: u8, touches: &[Touch], middle_click: bool) -> ButtonSet {
    let mut buttons = Buttons::new();
    buttons.update(&ZONES, clicks, touches, middle_click);
    buttons.pressed()
}

fn kernel_emit_buttons(last_state: u8, clicks: u8, touches: &[Touch], emulate_3button: bool) -> u8 {
    let mut state = clicks & 3;
    let ntouches = touches.iter().filter(|t| t.state.bits() != 0).count();
    let touch_of = |id: u8| *touches.iter().rev().find(|t| t.id == id).unwrap();
    let firm_touch = || {
        let mut touch: Option<u8> = None;
        for record in &touches[..ntouches] {
            let idx = record.id;
            if touch_of(idx).size < 8 {
            } else if touch.is_some() {
                touch = None;
                break;
            } else {
                touch = Some(idx);
            }
        }
        touch
    };
    if emulate_3button {
        if state == 0 {
        } else if last_state != 0 {
            state = last_state;
        } else if let Some(id) = firm_touch() {
            let x = touch_of(id).x;
            state = if x < -350 {
                1
            } else if x > 350 {
                2
            } else {
                4
            };
        }
    }
    state
}

#[test]
fn single_firm_touch_picks_the_zone() {
    let cases = [
        (-1100, set(&[Button::Left])),
        (-351, set(&[Button::Left])),
        (-350, set(&[Button::Middle])),
        (0, set(&[Button::Middle])),
        (350, set(&[Button::Middle])),
        (351, set(&[Button::Right])),
        (1258, set(&[Button::Right])),
    ];
    for (x, expected) in cases {
        for clicks in [0x01, 0x02, 0x03] {
            assert_eq!(
                press(clicks, &[touch(1, x, 8)], true),
                expected,
                "x {x} clicks {clicks}"
            );
        }
    }
}

#[test]
fn middle_zone_is_left_when_middle_click_is_off() {
    for x in [-350, 0, 350] {
        assert_eq!(press(0x02, &[touch(1, x, 20)], false), set(&[Button::Left]));
    }
    assert_eq!(
        press(0x01, &[touch(1, 900, 20)], false),
        set(&[Button::Right])
    );
    assert_eq!(
        press(0x02, &[touch(1, -900, 20)], false),
        set(&[Button::Left])
    );
}

#[test]
fn no_touch_on_the_surface_uses_the_mouse_guess() {
    assert_eq!(press(0x01, &[], true), set(&[Button::Left]));
    assert_eq!(press(0x02, &[], true), set(&[Button::Right]));
    assert_eq!(press(0x03, &[], true), set(&[Button::Left, Button::Right]));
    assert_eq!(press(0x03, &[], false), set(&[Button::Left, Button::Right]));
}

#[test]
fn only_the_low_two_click_bits_count() {
    assert_eq!(press(0x00, &[touch(1, 0, 20)], true), ButtonSet::EMPTY);
    assert_eq!(press(0xfc, &[touch(1, 0, 20)], true), ButtonSet::EMPTY);
    assert_eq!(press(0x81, &[], true), set(&[Button::Left]));
}

#[test]
fn soft_touches_are_not_firm() {
    assert_eq!(press(0x01, &[touch(1, 900, 7)], true), set(&[Button::Left]));
    assert_eq!(
        press(0x02, &[touch(1, -900, 0)], true),
        set(&[Button::Right])
    );
}

#[test]
fn several_touches() {
    let two_firm = [touch(1, -900, 20), touch(2, 900, 20)];
    assert_eq!(press(0x02, &two_firm, true), set(&[Button::Right]));
    assert_eq!(press(0x01, &two_firm, true), set(&[Button::Left]));
    let one_firm_one_soft = [touch(1, -900, 4), touch(2, 900, 20)];
    assert_eq!(press(0x01, &one_firm_one_soft, true), set(&[Button::Right]));
    let palm_and_finger = [touch(1, 0, 40), touch(2, 900, 3), touch(3, -900, 2)];
    assert_eq!(press(0x02, &palm_and_finger, true), set(&[Button::Middle]));
    let three_firm = [touch(1, 0, 40), touch(2, 900, 30), touch(3, -900, 20)];
    assert_eq!(press(0x01, &three_firm, true), set(&[Button::Left]));
}

#[test]
fn hover_touches_count_as_down_like_the_kernel() {
    let hover = with_state(touch(1, 900, 20), TouchState::Hover);
    assert_eq!(press(0x01, &[hover], true), set(&[Button::Right]));
}

#[test]
fn kernel_considers_the_first_records_up_to_the_down_count() {
    let gone_left = with_state(touch(1, -900, 20), TouchState::None);
    let down_right = touch(2, 900, 20);
    assert_eq!(
        press(0x02, &[gone_left, down_right], true),
        set(&[Button::Left])
    );
    assert_eq!(
        press(0x01, &[down_right, gone_left], true),
        set(&[Button::Right])
    );
    assert_eq!(press(0x01, &[gone_left], true), set(&[Button::Left]));
    let gone_right = with_state(touch(1, 900, 20), TouchState::None);
    assert_eq!(press(0x01, &[gone_right], true), set(&[Button::Left]));
}

#[test]
fn duplicate_ids_use_the_last_record_like_the_kernel() {
    let first = touch(4, -900, 20);
    let second = touch(4, 900, 20);
    assert_eq!(press(0x01, &[first, second], true), set(&[Button::Left]));
    let soft_then_firm = [touch(4, -900, 1), touch(4, 900, 20)];
    assert_eq!(press(0x01, &soft_then_firm, true), set(&[Button::Left]));
    let single = [
        touch(4, -900, 1),
        touch(4, 900, 20),
        with_state(touch(5, 0, 1), TouchState::None),
    ];
    assert_eq!(press(0x02, &single, true), set(&[Button::Right]));
    let firm_then_lifted_soft = [
        touch(4, 900, 20),
        with_state(touch(4, -900, 1), TouchState::None),
    ];
    assert_eq!(
        press(0x01, &firm_then_lifted_soft, true),
        set(&[Button::Left])
    );
}

#[test]
fn chosen_button_is_latched_until_release() {
    let mut buttons = Buttons::new();
    let edges = buttons.update(&ZONES, 0x01, &[touch(1, -900, 20)], true);
    assert_eq!(
        edges.as_slice(),
        &[ButtonEdge {
            button: Button::Left,
            pressed: true
        }]
    );
    for (clicks, x) in [(0x01, 900), (0x02, 900), (0x03, 0), (0x02, -900)] {
        let edges = buttons.update(&ZONES, clicks, &[touch(1, x, 20)], true);
        assert!(edges.is_empty());
        assert_eq!(buttons.pressed(), set(&[Button::Left]));
    }
    let edges = buttons.update(&ZONES, 0x00, &[touch(1, 900, 20)], true);
    assert_eq!(
        edges.as_slice(),
        &[ButtonEdge {
            button: Button::Left,
            pressed: false
        }]
    );
    let edges = buttons.update(&ZONES, 0x01, &[touch(1, 900, 20)], true);
    assert_eq!(
        edges.as_slice(),
        &[ButtonEdge {
            button: Button::Right,
            pressed: true
        }]
    );
}

#[test]
fn middle_click_setting_change_does_not_alter_a_held_button() {
    let mut buttons = Buttons::new();
    buttons.update(&ZONES, 0x01, &[touch(1, 0, 20)], true);
    assert!(
        buttons
            .update(&ZONES, 0x01, &[touch(1, 0, 20)], false)
            .is_empty()
    );
    assert_eq!(buttons.pressed(), set(&[Button::Middle]));
}

#[test]
fn release_all_pairs_every_press() {
    let mut buttons = Buttons::new();
    assert!(buttons.release_all().is_empty());
    buttons.update(&ZONES, 0x03, &[], true);
    let edges = buttons.release_all();
    assert_eq!(
        edges.as_slice(),
        &[
            ButtonEdge {
                button: Button::Left,
                pressed: false
            },
            ButtonEdge {
                button: Button::Right,
                pressed: false
            }
        ]
    );
    assert!(buttons.pressed().is_empty());
    assert!(buttons.release_all().is_empty());
    assert!(buttons.update(&ZONES, 0x00, &[], true).is_empty());
}

#[test]
fn button_set_iterates_in_fixed_order() {
    let all = set(&[Button::Middle, Button::Right, Button::Left]);
    assert_eq!(
        all.iter().collect::<Vec<_>>(),
        vec![Button::Left, Button::Right, Button::Middle]
    );
    assert_eq!(
        ButtonSet::from_click_bits(0xff),
        set(&[Button::Left, Button::Right])
    );
}

fn touch_strategy() -> impl Strategy<Value = Touch> {
    (
        0_u8..4,
        -1100_i32..=1258,
        0_u8..16,
        prop_oneof![
            Just(TouchState::None),
            Just(TouchState::Hover),
            Just(TouchState::Start),
            Just(TouchState::Drag),
            Just(TouchState::Lift(0x50)),
        ],
    )
        .prop_map(|(id, x, size, state)| with_state(touch(id, x, size), state))
}

#[derive(Debug, Clone)]
enum Step {
    Report(u8, Vec<Touch>),
    Reset,
}

fn step_strategy() -> impl Strategy<Value = Step> {
    prop_oneof![
        8 => (0_u8..4, proptest::collection::vec(touch_strategy(), 0..5))
            .prop_map(|(clicks, touches)| Step::Report(clicks, touches)),
        1 => Just(Step::Reset),
    ]
}

fn kernel_bits(set: ButtonSet) -> u8 {
    set.iter().map(bit).fold(0, |bits, b| bits | b)
}

proptest! {
    #[test]
    fn matches_the_kernel_emit_buttons(steps in proptest::collection::vec(step_strategy(), 1..40)) {
        let mut buttons = Buttons::new();
        let mut kernel_state = 0_u8;
        for step in &steps {
            match step {
                Step::Report(clicks, touches) => {
                    buttons.update(&ZONES, *clicks, touches, true);
                    kernel_state = kernel_emit_buttons(kernel_state, *clicks, touches, true);
                }
                Step::Reset => {
                    buttons.release_all();
                    kernel_state = 0;
                }
            }
            prop_assert_eq!(kernel_bits(buttons.pressed()), kernel_state);
        }
    }

    #[test]
    fn press_and_release_always_pair(
        steps in proptest::collection::vec(step_strategy(), 1..40),
        middle_click in any::<bool>(),
    ) {
        let mut buttons = Buttons::new();
        let mut held = ButtonSet::EMPTY;
        let check = |edges: ButtonEdges, held: &mut ButtonSet| -> Result<(), TestCaseError> {
            for edge in &edges {
                prop_assert_ne!(held.contains(edge.button), edge.pressed);
                held.set(edge.button, edge.pressed);
            }
            Ok(())
        };
        for step in &steps {
            let edges = match step {
                Step::Report(clicks, touches) => buttons.update(&ZONES, *clicks, touches, middle_click),
                Step::Reset => buttons.release_all(),
            };
            check(edges, &mut held)?;
            prop_assert_eq!(held, buttons.pressed());
        }
        check(buttons.release_all(), &mut held)?;
        prop_assert!(held.is_empty());
    }
}
