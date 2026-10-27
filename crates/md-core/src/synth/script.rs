use crate::Micros;
use crate::synth::chase::Chase;
use crate::synth::frame::{
    Frame, TOUCHPAD_SLOTS, TouchpadGeometry, TouchpadPoint, TrackingIds, VIRTUAL_TOUCHPAD,
};
use crate::synth::swipe::{allocate_row, finger_row, row_frame};

pub const SCRIPT_FRAME_INTERVAL: Micros = Micros::from_millis(10);
pub const SCRIPT_STEP_MM: f64 = 4.0;
pub const SCRIPT_DISTANCE_MM: f64 = 40.0;
pub const SCRIPT_MAX_STEPS: u32 = 64;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Phase {
    Place,
    Move,
    Lift,
    Done,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SwipeUpScript {
    touchpad: TouchpadGeometry,
    fingers: usize,
    tracking_ids: [u16; TOUCHPAD_SLOTS],
    phase: Phase,
    due: Micros,
    steps: u32,
    chase: Chase,
    frame: Frame,
}

impl SwipeUpScript {
    pub fn new(start: Micros, fingers: usize) -> SwipeUpScript {
        SwipeUpScript {
            touchpad: VIRTUAL_TOUCHPAD,
            fingers: fingers.clamp(1, TOUCHPAD_SLOTS),
            tracking_ids: [0; TOUCHPAD_SLOTS],
            phase: Phase::Place,
            due: start,
            steps: 0,
            chase: Chase::new(start),
            frame: Frame::EMPTY,
        }
    }

    pub fn next_deadline(&self) -> Option<Micros> {
        (self.phase != Phase::Done).then_some(self.due)
    }

    pub fn is_done(&self) -> bool {
        self.phase == Phase::Done
    }

    pub fn update(&mut self, t: Micros, tracking_ids: &mut TrackingIds) -> Option<Frame> {
        if self.phase == Phase::Done || t < self.due {
            return None;
        }
        self.due = t + SCRIPT_FRAME_INTERVAL;
        match self.phase {
            Phase::Place => {
                let (ids, count) = allocate_row(tracking_ids, self.fingers);
                self.tracking_ids = ids;
                self.fingers = count;
                self.chase = Chase::new(t);
                self.phase = Phase::Move;
            }
            Phase::Move => {
                self.steps += 1;
                let distance = self.touchpad.mm_to_units(SCRIPT_DISTANCE_MM);
                let wanted = self
                    .touchpad
                    .mm_to_units(f64::from(self.steps) * SCRIPT_STEP_MM)
                    .min(distance);
                let row = finger_row(&self.touchpad, self.fingers);
                self.chase.step(
                    t,
                    TouchpadPoint { x: 0, y: -wanted },
                    &row[..self.fingers],
                    &self.touchpad,
                );
                if -self.chase.offset().y >= distance || self.steps >= SCRIPT_MAX_STEPS {
                    self.phase = Phase::Lift;
                }
            }
            Phase::Lift => {
                self.phase = Phase::Done;
                return self.lift_all();
            }
            Phase::Done => return None,
        }
        let previous = self.frame;
        self.frame = row_frame(
            &self.touchpad,
            &self.tracking_ids[..self.fingers],
            self.chase.offset(),
        );
        (self.frame != previous).then_some(self.frame)
    }

    pub fn cancel(&mut self, lift_at: Micros) {
        match self.phase {
            Phase::Place => self.phase = Phase::Done,
            Phase::Move => {
                self.phase = Phase::Lift;
                self.due = lift_at;
            }
            Phase::Lift | Phase::Done => {}
        }
    }

    pub fn is_cancelled_or_lifting(&self) -> bool {
        matches!(self.phase, Phase::Lift | Phase::Done)
    }

    pub fn lift_all(&mut self) -> Option<Frame> {
        self.phase = Phase::Done;
        let was_empty = self.frame.is_empty();
        self.frame = Frame::EMPTY;
        (!was_empty).then_some(Frame::EMPTY)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::synth::VIRTUAL_TOUCHPAD;

    fn play(fingers: usize) -> Vec<(Micros, Frame)> {
        let mut ids = TrackingIds::default();
        let mut script = SwipeUpScript::new(Micros::from_millis(5), fingers);
        let mut frames = Vec::new();
        assert_eq!(script.update(Micros::ZERO, &mut ids), None);
        while let Some(deadline) = script.next_deadline() {
            if let Some(frame) = script.update(deadline, &mut ids) {
                frames.push((deadline, frame));
            }
            assert!(frames.len() < 100);
        }
        frames
    }

    #[test]
    fn script_places_moves_up_and_lifts_every_finger() {
        for fingers in 3..=5 {
            let frames = play(fingers);
            let (first_t, first) = frames[0];
            assert_eq!(first_t, Micros::from_millis(5));
            assert_eq!(first.contact_count(), fingers);
            let (_, last) = frames[frames.len() - 1];
            assert!(last.is_empty());
            let top = frames[frames.len() - 2].1;
            let travelled = first.slots[0].unwrap().y - top.slots[0].unwrap().y;
            assert_eq!(travelled, VIRTUAL_TOUCHPAD.mm_to_units(SCRIPT_DISTANCE_MM));
            for (_, frame) in &frames[..frames.len() - 1] {
                assert_eq!(frame.contact_count(), fingers);
                let x: Vec<i32> = frame.contacts().map(|(_, c)| c.x).collect();
                let first_x: Vec<i32> = first.contacts().map(|(_, c)| c.x).collect();
                assert_eq!(x, first_x);
            }
        }
    }

    #[test]
    fn script_frames_come_every_interval() {
        let frames = play(4);
        for pair in frames.windows(2) {
            assert_eq!(pair[1].0 - pair[0].0, SCRIPT_FRAME_INTERVAL);
        }
        assert!(frames.len() >= 10 && frames.len() <= 14, "{}", frames.len());
    }

    #[test]
    fn cancel_freezes_until_the_lift_time() {
        let mut ids = TrackingIds::default();
        let mut script = SwipeUpScript::new(Micros::ZERO, 4);
        let placed = script.update(Micros::ZERO, &mut ids).unwrap();
        let moved = script.update(SCRIPT_FRAME_INTERVAL, &mut ids).unwrap();
        assert_ne!(placed, moved);
        script.cancel(Micros::from_millis(70));
        assert_eq!(script.next_deadline(), Some(Micros::from_millis(70)));
        assert_eq!(script.update(Micros::from_millis(20), &mut ids), None);
        assert_eq!(script.update(Micros::from_millis(69), &mut ids), None);
        assert_eq!(
            script.update(Micros::from_millis(70), &mut ids),
            Some(Frame::EMPTY)
        );
        assert!(script.is_done());
    }

    #[test]
    fn cancel_before_the_first_frame_emits_nothing() {
        let mut ids = TrackingIds::default();
        let mut script = SwipeUpScript::new(Micros::from_millis(10), 4);
        script.cancel(Micros::from_millis(70));
        assert!(script.is_done());
        assert_eq!(script.update(Micros::from_millis(10), &mut ids), None);
    }

    #[test]
    fn lift_all_ends_the_script() {
        let mut ids = TrackingIds::default();
        let mut script = SwipeUpScript::new(Micros::ZERO, 4);
        assert!(script.update(Micros::ZERO, &mut ids).is_some());
        assert_eq!(script.lift_all(), Some(Frame::EMPTY));
        assert!(script.is_done());
        assert_eq!(script.next_deadline(), None);
        assert_eq!(script.lift_all(), None);
    }
}
