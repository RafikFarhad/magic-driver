use crate::Micros;
use crate::profile::SurfaceGeometry;
use crate::recognise::Intent;
use crate::synth::chase::{Chase, place};
use crate::synth::frame::{
    Contact, Frame, TOUCHPAD_SLOTS, TouchpadGeometry, TouchpadPoint, TrackingIds, VIRTUAL_TOUCHPAD,
};
use crate::tracker::{TrackId, Tracker};

pub const SWIPE_FINGER_SPACING_MM: f64 = 15.0;

pub fn finger_row(touchpad: &TouchpadGeometry, fingers: usize) -> [TouchpadPoint; TOUCHPAD_SLOTS] {
    let centre = touchpad.centre();
    let mut row = [centre; TOUCHPAD_SLOTS];
    let count = fingers.clamp(1, TOUCHPAD_SLOTS);
    let middle = f64::from(u8::try_from(count - 1).unwrap_or(0)) / 2.0;
    for (index, point) in row.iter_mut().enumerate().take(count) {
        let position = f64::from(u8::try_from(index).unwrap_or(0)) - middle;
        point.x = centre.x + touchpad.mm_to_units(position * SWIPE_FINGER_SPACING_MM);
    }
    row
}

pub fn row_frame(
    touchpad: &TouchpadGeometry,
    tracking_ids: &[u16],
    offset: TouchpadPoint,
) -> Frame {
    let row = finger_row(touchpad, tracking_ids.len());
    let mut frame = Frame::EMPTY;
    for (slot, (start, tracking_id)) in row.iter().zip(tracking_ids).enumerate() {
        frame.slots[slot] = Some(Contact::new(*tracking_id, place(touchpad, *start, offset)));
    }
    frame
}

pub fn allocate_row(
    tracking_ids: &mut TrackingIds,
    fingers: usize,
) -> ([u16; TOUCHPAD_SLOTS], usize) {
    let count = fingers.clamp(1, TOUCHPAD_SLOTS);
    let mut ids = [0; TOUCHPAD_SLOTS];
    for id in ids.iter_mut().take(count) {
        *id = tracking_ids.allocate();
    }
    (ids, count)
}

#[derive(Debug, Clone, Copy, PartialEq)]
struct SwipeStroke {
    tracks: [TrackId; 2],
    tracking_ids: [u16; TOUCHPAD_SLOTS],
    fingers: usize,
    anchor_x_mm: f64,
    chase: Chase,
}

#[derive(Debug, Clone, PartialEq)]
pub struct SwipeSynth {
    surface: SurfaceGeometry,
    touchpad: TouchpadGeometry,
    stroke: Option<SwipeStroke>,
    frame: Frame,
}

impl SwipeSynth {
    pub fn new(surface: SurfaceGeometry) -> SwipeSynth {
        SwipeSynth {
            surface,
            touchpad: VIRTUAL_TOUCHPAD,
            stroke: None,
            frame: Frame::EMPTY,
        }
    }

    pub fn is_active(&self) -> bool {
        !self.frame.is_empty()
    }

    pub fn update(
        &mut self,
        t: Micros,
        intent: Intent,
        tracker: &Tracker,
        tracking_ids: &mut TrackingIds,
        fingers: usize,
    ) -> Option<Frame> {
        match intent {
            Intent::Freeze => {}
            Intent::Swipe { tracks } => self.follow(t, tracks, tracker, tracking_ids, fingers),
            Intent::None | Intent::Hold | Intent::Scroll { .. } | Intent::Overview => {
                return self.lift_all();
            }
        }
        let previous = self.frame;
        self.frame = self.place();
        (self.frame != previous).then_some(self.frame)
    }

    pub fn lift_all(&mut self) -> Option<Frame> {
        self.stroke = None;
        let was_empty = self.frame.is_empty();
        self.frame = Frame::EMPTY;
        (!was_empty).then_some(Frame::EMPTY)
    }

    fn centroid_x_mm(&self, tracks: [TrackId; 2], tracker: &Tracker, start: bool) -> Option<f64> {
        let mut sum = 0.0;
        for track in tracks {
            let track = tracker.get(track)?;
            let point = if start { track.start_pos } else { track.pos };
            sum += self.surface.to_mm(point).x;
        }
        Some(sum / 2.0)
    }

    fn follow(
        &mut self,
        t: Micros,
        tracks: [TrackId; 2],
        tracker: &Tracker,
        tracking_ids: &mut TrackingIds,
        fingers: usize,
    ) {
        let mut stroke = match self.stroke {
            Some(stroke) if stroke.tracks == tracks => stroke,
            _ => {
                let Some(anchor_x_mm) = self.centroid_x_mm(tracks, tracker, true) else {
                    return;
                };
                let (ids, count) = allocate_row(tracking_ids, fingers);
                self.stroke = Some(SwipeStroke {
                    tracks,
                    tracking_ids: ids,
                    fingers: count,
                    anchor_x_mm,
                    chase: Chase::new(t),
                });
                return;
            }
        };
        let Some(x_mm) = self.centroid_x_mm(tracks, tracker, false) else {
            return;
        };
        let target = TouchpadPoint {
            x: self.touchpad.mm_to_units(x_mm - stroke.anchor_x_mm),
            y: 0,
        };
        let row = finger_row(&self.touchpad, stroke.fingers);
        stroke
            .chase
            .step(t, target, &row[..stroke.fingers], &self.touchpad);
        self.stroke = Some(stroke);
    }

    fn place(&self) -> Frame {
        match &self.stroke {
            Some(stroke) => row_frame(
                &self.touchpad,
                &stroke.tracking_ids[..stroke.fingers],
                stroke.chase.offset(),
            ),
            None => Frame::EMPTY,
        }
    }
}
