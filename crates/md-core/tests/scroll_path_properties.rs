use md_core::config::Config;
use md_core::profile::MAGIC_MOUSE_2_BT;
use md_core::synth::{Frame, SCROLL_FINGER_SPACING_MM, TOUCHPAD_SLOTS, VIRTUAL_TOUCHPAD};
use md_core::testkit::scenarios::{Scenario, drive, finger, palm};
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

struct Checker {
    open: [Option<Movement>; TOUCHPAD_SLOTS],
    last: Frame,
    buttons_down: usize,
}

impl Checker {
    fn new() -> Checker {
        Checker {
            open: [None; TOUCHPAD_SLOTS],
            last: Frame::EMPTY,
            buttons_down: 0,
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
            Output::PointerMotion { .. } => Ok(()),
        }
    }

    fn frame(&mut self, t: Micros, frame: &Frame) -> Result<(), TestCaseError> {
        prop_assert_ne!(*frame, self.last, "repeated frame");
        let count = frame.contact_count();
        prop_assert!(count == 0 || count == 2, "frame with {} contacts", count);
        if count == 2 {
            let (a, b) = (frame.slots[0].unwrap(), frame.slots[1].unwrap());
            prop_assert_eq!(
                b.x - a.x,
                VIRTUAL_TOUCHPAD.mm_to_units(SCROLL_FINGER_SPACING_MM)
            );
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
