use md_core::config::Config;
use md_core::profile::MAGIC_MOUSE_2_BT;
use md_core::synth::{
    Frame, SCROLL_FINGER_SPACING_MM, SWIPE_FINGER_SPACING_MM, TOUCHPAD_SLOTS, VIRTUAL_TOUCHPAD,
};
use md_core::testkit::scenarios::{self, Scenario, drive, finger, palm};
use md_core::testkit::{ReportBuilder, TouchRecord, touch_state};
use md_core::{Engine, Input, Micros, Mode, Output};
use proptest::prelude::*;

const LIBINPUT_JUMP_MM: f64 = 20.0;
const LIBINPUT_JUMP_INCREASE_MM: f64 = 7.0;
const LIBINPUT_REFERENCE_US: f64 = 12_000.0;

#[derive(Debug, Clone)]
enum Step {
    Report {
        gap_us: u64,
        clicks: u8,
        dx: i16,
        touches: Vec<TouchRecord>,
    },
    Reset,
}

fn touch_strategy() -> impl Strategy<Value = TouchRecord> {
    (
        0_u8..5,
        -1_100_i32..=1_258,
        -1_589_i32..=2_047,
        prop_oneof![4 => 20_u8..60, 1 => 120_u8..255],
        prop_oneof![
            2 => Just(touch_state::START),
            10 => Just(touch_state::DRAG),
            1 => Just(touch_state::LIFT_50),
            1 => Just(touch_state::HOVER),
            1 => Just(touch_state::GONE),
        ],
    )
        .prop_map(|(id, x, y, major, state)| {
            TouchRecord::new(id, x, y)
                .touch_major(major)
                .touch_minor(major / 2)
                .state(state)
        })
}

fn gap_strategy() -> impl Strategy<Value = u64> {
    prop_oneof![
        10 => 9_000_u64..13_000,
        2 => 0_u64..3_000,
        2 => 13_000_u64..40_000,
        1 => 40_000_u64..400_000,
    ]
}

fn step_strategy() -> impl Strategy<Value = Step> {
    prop_oneof![
        30 => (
            gap_strategy(),
            prop_oneof![8 => Just(0_u8), 1 => 1_u8..4],
            prop_oneof![8 => Just(0_i16), 1 => -40_i16..40],
            proptest::collection::vec(touch_strategy(), 0..4),
        )
            .prop_map(|(gap_us, clicks, dx, touches)| Step::Report { gap_us, clicks, dx, touches }),
        1 => Just(Step::Reset),
    ]
}

#[derive(Debug, Clone, Copy)]
struct Movement {
    tracking_id: u16,
    last_t: Micros,
    x: i32,
    y: i32,
    last_delta_mm: f64,
}

const SWIPE_FINGERS: usize = 4;

struct Checker {
    open: [Option<Movement>; TOUCHPAD_SLOTS],
    last: Frame,
    buttons_down: usize,
    scroll_frames: usize,
    swipe_frames: usize,
}

impl Checker {
    fn new() -> Checker {
        Checker {
            open: [None; TOUCHPAD_SLOTS],
            last: Frame::EMPTY,
            buttons_down: 0,
            scroll_frames: 0,
            swipe_frames: 0,
        }
    }

    fn apply(&mut self, t: Micros, output: &Output) -> Result<(), TestCaseError> {
        match output {
            Output::Button { pressed, .. } => {
                if *pressed {
                    self.buttons_down += 1;
                } else {
                    self.buttons_down = self.buttons_down.saturating_sub(1);
                }
                Ok(())
            }
            Output::TouchFrame(frame) => self.frame(t, frame),
            Output::PointerMotion { .. } | Output::Trace(_) => Ok(()),
        }
    }

    fn frame(&mut self, t: Micros, frame: &Frame) -> Result<(), TestCaseError> {
        prop_assert_ne!(*frame, self.last, "repeated frame");
        let count = frame.contact_count();
        prop_assert!(
            count == 0 || count == 2 || count == SWIPE_FINGERS,
            "frame with {} contacts",
            count
        );
        let previous = self.last.contact_count();
        prop_assert!(
            count == 0 || previous == 0 || count == previous,
            "scroll and swipe output overlap: {} contacts after {}",
            count,
            previous
        );
        let spacing = if count == 2 {
            self.scroll_frames += 1;
            SCROLL_FINGER_SPACING_MM
        } else {
            self.swipe_frames += usize::from(count == SWIPE_FINGERS);
            SWIPE_FINGER_SPACING_MM
        };
        for slot in 1..count {
            let (a, b) = (frame.slots[slot - 1].unwrap(), frame.slots[slot].unwrap());
            prop_assert_eq!(b.x - a.x, VIRTUAL_TOUCHPAD.mm_to_units(spacing));
            prop_assert_eq!(a.y, b.y);
        }
        if self.buttons_down > 0 {
            for (slot, contact) in frame.contacts() {
                prop_assert_eq!(
                    self.last.slots[slot],
                    Some(contact),
                    "touch output moved or started while a button is down"
                );
            }
        }
        for (slot, contact) in frame.slots.iter().enumerate() {
            let Some(contact) = contact else {
                self.open[slot] = None;
                continue;
            };
            prop_assert!(VIRTUAL_TOUCHPAD.contains(contact.point()));
            let movement = match self.open[slot] {
                Some(previous) => {
                    prop_assert_eq!(previous.tracking_id, contact.tracking_id, "slot reused");
                    advance(previous, t, contact.x, contact.y)?
                }
                None => Movement {
                    tracking_id: contact.tracking_id,
                    last_t: t,
                    x: contact.x,
                    y: contact.y,
                    last_delta_mm: 0.0,
                },
            };
            self.open[slot] = Some(movement);
        }
        self.last = *frame;
        Ok(())
    }

    fn all_closed(&self) -> bool {
        self.open.iter().all(Option::is_none)
    }
}

#[allow(clippy::cast_precision_loss)]
fn advance(previous: Movement, t: Micros, x: i32, y: i32) -> Result<Movement, TestCaseError> {
    if previous.x == x && previous.y == y {
        return Ok(previous);
    }
    let mut next = Movement {
        last_t: t,
        x,
        y,
        ..previous
    };
    let dt = (t - previous.last_t).as_u64() as f64;
    if dt > 0.0 && dt <= 2.0 * LIBINPUT_REFERENCE_US {
        let mm = f64::from(x - previous.x).hypot(f64::from(y - previous.y))
            / f64::from(VIRTUAL_TOUCHPAD.units_per_mm);
        let normalised = mm * LIBINPUT_REFERENCE_US / dt;
        let increase = normalised - previous.last_delta_mm;
        prop_assert!(
            normalised <= LIBINPUT_JUMP_MM / 2.0 + 1e-6,
            "step {normalised} mm per 12 ms"
        );
        prop_assert!(
            increase <= LIBINPUT_JUMP_INCREASE_MM - 2.0 + 1e-6,
            "step grew by {increase} mm per 12 ms"
        );
        next.last_delta_mm = normalised;
    }
    Ok(next)
}

fn run(steps: &[Step], finish: Option<&Step>) -> Result<(), TestCaseError> {
    let mut engine = Engine::new(&MAGIC_MOUSE_2_BT, Config::default(), Mode::Normal);
    let mut checker = Checker::new();
    let mut out = Vec::new();
    let mut t = Micros::ZERO;
    for step in steps.iter().chain(finish) {
        match step {
            Step::Report {
                gap_us,
                clicks,
                dx,
                touches,
            } => {
                let next = t + Micros::new(*gap_us);
                while let Some(deadline) = engine.next_deadline().filter(|d| *d < next) {
                    prop_assert!(deadline >= t, "deadline in the past");
                    engine.handle(Input::Timer { t: deadline }, &mut out);
                    for output in out.drain(..) {
                        checker.apply(deadline, &output)?;
                    }
                    t = deadline;
                }
                t = next;
                let bytes = touches
                    .iter()
                    .fold(
                        ReportBuilder::new().clicks(*clicks).motion(*dx, 0),
                        |builder, touch| builder.touch(*touch),
                    )
                    .build();
                engine.handle(Input::Report { t, bytes: &bytes }, &mut out);
            }
            Step::Reset => {
                engine.handle(Input::Reset, &mut out);
            }
        }
        for output in out.drain(..) {
            checker.apply(t, &output)?;
        }
        if matches!(step, Step::Reset) {
            prop_assert!(checker.all_closed(), "slot open after reset");
            prop_assert_eq!(engine.next_deadline(), None);
        }
    }
    let mut guard = 0;
    while let Some(deadline) = engine.next_deadline() {
        guard += 1;
        prop_assert!(guard < 16, "deadlines never settle");
        engine.handle(Input::Timer { t: deadline }, &mut out);
        for output in out.drain(..) {
            checker.apply(deadline, &output)?;
        }
    }
    if finish.is_some() {
        prop_assert!(checker.all_closed(), "slot open at the end");
    }
    Ok(())
}

fn no_touches() -> Step {
    Step::Report {
        gap_us: 11_111,
        clicks: 0,
        dx: 0,
        touches: Vec::new(),
    }
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(512))]

    #[test]
    fn every_opened_slot_closes_after_reset(steps in proptest::collection::vec(step_strategy(), 0..80)) {
        run(&steps, Some(&Step::Reset))?;
    }

    #[test]
    fn every_opened_slot_closes_when_all_touches_end(steps in proptest::collection::vec(step_strategy(), 0..80)) {
        run(&steps, Some(&no_touches()))?;
    }

    #[test]
    fn no_scroll_output_between_a_press_and_its_release_and_no_libinput_jumps(
        steps in proptest::collection::vec(step_strategy(), 0..120)
    ) {
        run(&steps, None)?;
    }

    #[test]
    fn synthetic_fingers_stay_on_the_surface_during_any_single_stroke(
        path in proptest::collection::vec((-1_100_i32..=1_258, -1_589_i32..=774), 2..200)
    ) {
        let mut scenario = Scenario::new();
        let (x, y) = path[0];
        scenario.push(ReportBuilder::new().touch(palm()).touch(finger(x, y).state(touch_state::START)));
        for (x, y) in &path[1..] {
            scenario.push(ReportBuilder::new().touch(palm()).touch(finger(*x, *y)));
        }
        let mut engine = Engine::new(&MAGIC_MOUSE_2_BT, Config::default(), Mode::Normal);
        let timeline = drive(&mut engine, &scenario.timed());
        let mut checker = Checker::new();
        for (t, output) in &timeline {
            checker.apply(*t, output)?;
        }
    }
}

#[test]
fn longest_stroke_corner_to_corner_stays_on_the_surface() {
    let surface = MAGIC_MOUSE_2_BT.surface;
    let mut scenario = Scenario::new();
    let corners = [
        (surface.x.min, surface.y.min),
        (surface.x.max, 774),
        (surface.x.min, 774),
        (surface.x.max, surface.y.min),
    ];
    scenario.push(
        ReportBuilder::new().touch(finger(corners[0].0, corners[0].1).state(touch_state::START)),
    );
    for pair in corners.windows(2) {
        let ((x0, y0), (x1, y1)) = (pair[0], pair[1]);
        for step in 1..=60 {
            let x = x0 + (x1 - x0) * step / 60;
            let y = y0 + (y1 - y0) * step / 60;
            scenario.push(ReportBuilder::new().touch(finger(x, y)));
        }
    }
    scenario.push(ReportBuilder::new());
    let mut engine = Engine::new(&MAGIC_MOUSE_2_BT, Config::default(), Mode::Normal);
    let timeline = drive(&mut engine, &scenario.timed());
    let mut checker = Checker::new();
    let mut frames = 0;
    for (t, output) in &timeline {
        checker.apply(*t, output).unwrap();
        if matches!(output, Output::TouchFrame(_)) {
            frames += 1;
        }
    }
    assert!(frames > 100);
    assert!(checker.all_closed());
}

#[derive(Debug, Clone)]
enum Gesture {
    OneFinger {
        start: (i32, i32),
        step: (i32, i32),
        steps: usize,
    },
    TwoFingers {
        start: (i32, i32),
        step: (i32, i32),
        steps: usize,
    },
    Tap {
        frames: usize,
        click: bool,
    },
    Click {
        frames: usize,
    },
    Gap {
        frames: usize,
    },
}

fn gesture_strategy() -> impl Strategy<Value = Gesture> {
    prop_oneof![
        3 => ((-900_i32..900, -1_200_i32..600), (-40_i32..40, -40_i32..40), 3_usize..40)
            .prop_map(|(start, step, steps)| Gesture::OneFinger { start, step, steps }),
        3 => ((-1_000_i32..300, -1_200_i32..600), (-60_i32..60, -20_i32..20), 3_usize..30)
            .prop_map(|(start, step, steps)| Gesture::TwoFingers { start, step, steps }),
        2 => (1_usize..10, proptest::bool::weighted(0.2))
            .prop_map(|(frames, click)| Gesture::Tap { frames, click }),
        1 => (1_usize..10).prop_map(|frames| Gesture::Click { frames }),
        3 => (0_usize..40).prop_map(|frames| Gesture::Gap { frames }),
    ]
}

fn steps_on_surface(starts: &[(i32, i32)], step: (i32, i32), steps: usize) -> Vec<(i32, i32)> {
    let surface = MAGIC_MOUSE_2_BT.surface;
    let mut kept = Vec::new();
    let mut offset = (0, 0);
    while kept.len() < steps {
        let next = (offset.0 + step.0, offset.1 + step.1);
        let inside = starts.iter().all(|(x, y)| {
            (surface.x.min..=surface.x.max).contains(&(x + next.0))
                && (surface.y.min..=surface.y.max).contains(&(y + next.1))
        });
        if !inside {
            break;
        }
        offset = next;
        kept.push(step);
    }
    kept
}

fn build(gestures: &[Gesture]) -> Scenario {
    let mut scenario = Scenario::new();
    let background = [palm()];
    for gesture in gestures {
        match *gesture {
            Gesture::OneFinger { start, step, steps } => {
                scenario.stroke(&background, start, &steps_on_surface(&[start], step, steps));
            }
            Gesture::TwoFingers { start, step, steps } => {
                let second = (start.0 + 600, start.1);
                let kept = steps_on_surface(&[start, second], step, steps);
                scenario.two_finger_stroke(&background, [start, second], &kept);
            }
            Gesture::Tap { frames, click } => {
                scenario.two_finger_tap(&background, frames, click);
            }
            Gesture::Click { frames } => {
                let touch = finger(-600, 200);
                scenario.push(
                    ReportBuilder::new()
                        .touch(palm())
                        .touch(touch.state(touch_state::START)),
                );
                scenario.repeat(
                    frames,
                    &ReportBuilder::new().left(true).touch(palm()).touch(touch),
                );
                scenario.push(
                    ReportBuilder::new()
                        .touch(palm())
                        .touch(touch.state(touch_state::LIFT_50)),
                );
                scenario.push(ReportBuilder::new().touch(palm()));
            }
            Gesture::Gap { frames } => {
                scenario.repeat(frames, &ReportBuilder::new().touch(palm()));
            }
        }
    }
    scenario.repeat(40, &ReportBuilder::new().touch(palm()));
    scenario
}

fn check(scenario: &Scenario) -> Result<Checker, TestCaseError> {
    let mut engine = Engine::new(&MAGIC_MOUSE_2_BT, Config::default(), Mode::Normal);
    let timeline = drive(&mut engine, &scenario.timed());
    let mut checker = Checker::new();
    for (t, output) in &timeline {
        checker.apply(*t, output)?;
    }
    prop_assert!(checker.all_closed(), "slot open at the end");
    prop_assert_eq!(engine.next_deadline(), None);
    Ok(checker)
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(256))]

    #[test]
    fn swipe_and_scroll_outputs_never_overlap(
        gestures in proptest::collection::vec(gesture_strategy(), 1..12)
    ) {
        check(&build(&gestures))?;
    }
}

#[test]
fn scroll_swipe_and_overview_in_one_session_never_overlap() {
    let mut scenario = scenarios::slow_scroll();
    scenario.two_finger_stroke(
        &[palm()],
        [(-500, 200), (100, 200)],
        &scenarios::two_finger_swipe_steps(40),
    );
    scenario.repeat(5, &ReportBuilder::new().touch(palm()));
    scenario.stroke(&[palm()], (-900, 0), &[(20, 0); 40]);
    scenario.two_finger_tap(&[palm()], 6, false);
    scenario.repeat(12, &ReportBuilder::new().touch(palm()));
    scenario.two_finger_tap(&[palm()], 6, false);
    scenario.stroke(&[palm()], (-100, 600), &[(0, -24); 30]);
    let checker = check(&scenario).unwrap();
    assert!(checker.scroll_frames > 50, "{}", checker.scroll_frames);
    assert!(checker.swipe_frames > 15, "{}", checker.swipe_frames);
}
