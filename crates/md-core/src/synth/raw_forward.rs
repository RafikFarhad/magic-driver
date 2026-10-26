use crate::profile::SurfaceGeometry;
use crate::synth::frame::{
    Contact, Frame, TOUCHPAD_SLOTS, TouchpadGeometry, TrackingIds, VIRTUAL_TOUCHPAD,
};
use crate::tracker::{TrackId, Tracker};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Assignment {
    track: TrackId,
    tracking_id: u16,
}

#[derive(Debug, Clone, PartialEq)]
pub struct RawForward {
    surface: SurfaceGeometry,
    touchpad: TouchpadGeometry,
    assigned: [Option<Assignment>; TOUCHPAD_SLOTS],
    frame: Frame,
    tracking_ids: TrackingIds,
}

impl RawForward {
    pub fn new(surface: SurfaceGeometry) -> RawForward {
        RawForward {
            surface,
            touchpad: VIRTUAL_TOUCHPAD,
            assigned: [None; TOUCHPAD_SLOTS],
            frame: Frame::EMPTY,
            tracking_ids: TrackingIds::default(),
        }
    }

    pub const fn frame(&self) -> &Frame {
        &self.frame
    }

    pub fn update(&mut self, tracker: &Tracker) -> Option<Frame> {
        let previous = self.frame;
        let mut next = Frame::EMPTY;
        for (slot, assignment) in self.assigned.iter_mut().enumerate() {
            let Some(current) = *assignment else {
                continue;
            };
            match tracker
                .get(current.track)
                .filter(|track| track.phase.is_contact())
            {
                Some(track) => {
                    let point = self.touchpad.map_surface_point(&self.surface, track.pos);
                    next.slots[slot] = Some(Contact::new(current.tracking_id, point));
                }
                None => *assignment = None,
            }
        }
        for track in tracker.contacts() {
            if self
                .assigned
                .iter()
                .flatten()
                .any(|assignment| assignment.track == track.id)
            {
                continue;
            }
            let free = (0..TOUCHPAD_SLOTS)
                .find(|slot| self.assigned[*slot].is_none() && previous.slots[*slot].is_none());
            let Some(slot) = free else {
                break;
            };
            let tracking_id = self.tracking_ids.allocate();
            self.assigned[slot] = Some(Assignment {
                track: track.id,
                tracking_id,
            });
            let point = self.touchpad.map_surface_point(&self.surface, track.pos);
            next.slots[slot] = Some(Contact::new(tracking_id, point));
        }
        self.frame = next;
        (next != previous).then_some(next)
    }

    pub fn lift_all(&mut self) -> Option<Frame> {
        self.assigned = [None; TOUCHPAD_SLOTS];
        let was_empty = self.frame.is_empty();
        self.frame = Frame::EMPTY;
        (!was_empty).then_some(Frame::EMPTY)
    }
}
