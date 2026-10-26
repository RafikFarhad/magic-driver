use md_core::config::Config;
use md_core::profile::MAGIC_MOUSE_2_BT;
use md_core::synth::{Frame, TOUCHPAD_SLOTS, VIRTUAL_TOUCHPAD};
use md_core::testkit::{ReportBuilder, TouchRecord};
use md_core::{Engine, Input, Micros, Mode, Output};
use proptest::prelude::*;

#[derive(Debug, Clone)]
enum Step {
    Report {
        clicks: u8,
        touches: Vec<TouchRecord>,
    },
    Short {
        clicks: u8,
    },
    Garbage(Vec<u8>),
    Reset,
}

fn touch_strategy() -> impl Strategy<Value = TouchRecord> {
    (
        0_u8..8,
        -2048_i32..=2047,
        -2047_i32..=2048,
        0_u8..64,
        (0_u8..16).prop_map(|nibble| nibble << 4),
    )
        .prop_map(|(id, x, y, size, state)| TouchRecord::new(id, x, y).size(size).state(state))
}

fn step_strategy() -> impl Strategy<Value = Step> {
    prop_oneof![
        12 => (0_u8..4, proptest::collection::vec(touch_strategy(), 0..7))
            .prop_map(|(clicks, touches)| Step::Report { clicks, touches }),
        1 => (0_u8..4).prop_map(|clicks| Step::Short { clicks }),
        1 => proptest::collection::vec(any::<u8>(), 0..40).prop_map(Step::Garbage),
        1 => Just(Step::Reset),
    ]
}

fn bytes_for(step: &Step) -> Option<Vec<u8>> {
    match step {
        Step::Report { clicks, touches } => Some(
            touches
                .iter()
                .fold(ReportBuilder::new().clicks(*clicks), |builder, touch| {
                    builder.touch(*touch)
                })
                .build(),
        ),
        Step::Short { clicks } => Some(vec![0x12, *clicks, 1, 0, 0xff, 0xff, 0, 0]),
        Step::Garbage(bytes) => Some(bytes.clone()),
        Step::Reset => None,
    }
}

struct SlotChecker {
    mode: Mode,
    open: [Option<u16>; TOUCHPAD_SLOTS],
    last: Frame,
}

impl SlotChecker {
    fn new(mode: Mode) -> SlotChecker {
        SlotChecker {
            mode,
            open: [None; TOUCHPAD_SLOTS],
            last: Frame::EMPTY,
        }
    }

    fn apply(&mut self, outputs: &[Output]) -> Result<(), TestCaseError> {
        for output in outputs {
            let Output::TouchFrame(frame) = output else {
                continue;
            };
            prop_assert_ne!(*frame, self.last, "repeated frame");
            let mut ids: Vec<u16> = frame.contacts().map(|(_, c)| c.tracking_id).collect();
            let count = ids.len();
            ids.sort_unstable();
            ids.dedup();
            prop_assert_eq!(ids.len(), count, "duplicate tracking id in {}", frame);
            if self.mode == Mode::ScrollProbe {
                prop_assert!(count == 0 || count == 2, "scroll probe frame {}", frame);
            }
            for (slot, contact) in frame.slots.iter().enumerate() {
                if let Some(contact) = contact {
                    prop_assert!(VIRTUAL_TOUCHPAD.contains(contact.point()));
                    if let Some(open) = self.open[slot] {
                        prop_assert_eq!(
                            open,
                            contact.tracking_id,
                            "slot {} reused while open",
                            slot
                        );
                    }
                }
                self.open[slot] = contact.map(|contact| contact.tracking_id);
            }
            self.last = *frame;
        }
        Ok(())
    }

    fn all_closed(&self) -> bool {
        self.open.iter().all(Option::is_none)
    }
}

fn run(mode: Mode, steps: &[Step], finish: &Step) -> Result<(), TestCaseError> {
    let mut engine = Engine::new(&MAGIC_MOUSE_2_BT, Config::default(), mode);
    let mut checker = SlotChecker::new(mode);
    let mut out = Vec::new();
    for (index, step) in (0_u64..).zip(steps.iter().chain(std::iter::once(finish))) {
        let t = Micros::new(index * 11_111);
        match bytes_for(step) {
            Some(bytes) => engine.handle(Input::Report { t, bytes: &bytes }, &mut out),
            None => engine.handle(Input::Reset, &mut out),
        }
        checker.apply(&out)?;
        if matches!(step, Step::Reset) {
            prop_assert!(checker.all_closed(), "slot open after reset");
        }
        let contacts = engine.tracker().contacts().count();
        let open = checker.last.contact_count();
        match mode {
            Mode::RawForward => prop_assert!(open <= contacts),
            _ => prop_assert!(contacts > 0 || open == 0),
        }
        out.clear();
    }
    prop_assert!(checker.all_closed(), "slot open at the end");
    Ok(())
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(512))]

    #[test]
    fn raw_forward_closes_every_slot_after_reset(steps in proptest::collection::vec(step_strategy(), 0..60)) {
        run(Mode::RawForward, &steps, &Step::Reset)?;
    }

    #[test]
    fn raw_forward_closes_every_slot_when_all_touches_end(steps in proptest::collection::vec(step_strategy(), 0..60)) {
        run(Mode::RawForward, &steps, &Step::Report { clicks: 0, touches: Vec::new() })?;
    }

    #[test]
    fn scroll_probe_closes_every_slot_after_reset(steps in proptest::collection::vec(step_strategy(), 0..60)) {
        run(Mode::ScrollProbe, &steps, &Step::Reset)?;
    }

    #[test]
    fn scroll_probe_closes_every_slot_when_all_touches_end(steps in proptest::collection::vec(step_strategy(), 0..60)) {
        run(Mode::ScrollProbe, &steps, &Step::Report { clicks: 0, touches: Vec::new() })?;
    }

    #[test]
    fn pointer_only_never_emits_touch_frames(steps in proptest::collection::vec(step_strategy(), 0..60)) {
        let mut engine = Engine::new(&MAGIC_MOUSE_2_BT, Config::default(), Mode::PointerOnly);
        let mut out = Vec::new();
        for step in &steps {
            match bytes_for(step) {
                Some(bytes) => engine.handle(Input::Report { t: Micros::ZERO, bytes: &bytes }, &mut out),
                None => engine.handle(Input::Reset, &mut out),
            }
        }
        prop_assert!(!out.iter().any(|output| matches!(output, Output::TouchFrame(_))));
    }
}
