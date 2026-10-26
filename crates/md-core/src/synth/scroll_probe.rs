use crate::profile::SurfaceGeometry;
use crate::surface::{SurfacePoint, SurfacePointMm};
use crate::synth::frame::{
    Contact, Frame, TouchpadGeometry, TouchpadPoint, TrackingIds, VIRTUAL_TOUCHPAD,
};
use crate::tracker::{TrackId, Tracker};

pub const SCROLL_PROBE_FINGER_SPACING_MM: f64 = 20.0;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Followed {
    track: TrackId,
    anchor: SurfacePoint,
    tracking_ids: [u16; 2],
}

#[derive(Debug, Clone, PartialEq)]
pub struct ScrollProbe {
    surface: SurfaceGeometry,
    touchpad: TouchpadGeometry,
    followed: Option<Followed>,
    frame: Frame,
    tracking_ids: TrackingIds,
}

impl ScrollProbe {
    pub fn new(surface: SurfaceGeometry) -> ScrollProbe {
        ScrollProbe {
            surface,
            touchpad: VIRTUAL_TOUCHPAD,
            followed: None,
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
        let newest = tracker.contacts().max_by_key(|track| track.id);
        if let Some(followed) = self.followed {
            match tracker
                .get(followed.track)
                .filter(|track| track.phase.is_contact())
            {
                Some(track) if newest.is_none_or(|newest| newest.id == followed.track) => {
                    self.place(&mut next, followed, track.pos);
                }
                _ => self.followed = None,
            }
        } else if previous.is_empty()
            && let Some(track) = newest
        {
            let followed = Followed {
                track: track.id,
                anchor: track.pos,
                tracking_ids: [self.tracking_ids.allocate(), self.tracking_ids.allocate()],
            };
            self.followed = Some(followed);
            self.place(&mut next, followed, track.pos);
        }
        self.frame = next;
        (next != previous).then_some(next)
    }

    pub fn lift_all(&mut self) -> Option<Frame> {
        self.followed = None;
        let was_empty = self.frame.is_empty();
        self.frame = Frame::EMPTY;
        (!was_empty).then_some(Frame::EMPTY)
    }

    fn place(&self, frame: &mut Frame, followed: Followed, pos: SurfacePoint) {
        let from = self.surface.to_mm(followed.anchor);
        let to = self.surface.to_mm(pos);
        let delta = SurfacePointMm::new(to.x - from.x, to.y - from.y);
        let centre = self.touchpad.centre();
        let half = self
            .touchpad
            .mm_to_units(SCROLL_PROBE_FINGER_SPACING_MM / 2.0);
        let starts = [
            TouchpadPoint {
                x: centre.x - half,
                y: centre.y,
            },
            TouchpadPoint {
                x: centre.x + half,
                y: centre.y,
            },
        ];
        for (slot, (start, tracking_id)) in
            starts.into_iter().zip(followed.tracking_ids).enumerate()
        {
            frame.slots[slot] = Some(Contact::new(
                tracking_id,
                self.touchpad.offset(start, delta),
            ));
        }
    }
}
