use crate::Micros;
use crate::profile::SurfaceGeometry;
use crate::recognise::Intent;
use crate::surface::SurfacePoint;
use crate::synth::frame::{
    Contact, Frame, TouchpadGeometry, TouchpadPoint, TrackingIds, VIRTUAL_TOUCHPAD,
};
use crate::tracker::{TrackId, Tracker};

pub const SCROLL_FINGER_SPACING_MM: f64 = 20.0;
pub const JUMP_REFERENCE_INTERVAL: Micros = Micros::from_millis(12);
pub const JUMP_MAX_STEP_MM: f64 = 10.0;
pub const JUMP_MAX_STEP_INCREASE_MM: f64 = 5.0;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ScrollSettings {
    pub gain: f64,
    pub horizontal: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Following {
    track: TrackId,
    anchor: SurfacePoint,
    origin: TouchpadPoint,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct StepLimiter {
    last_move_t: Micros,
    last_step_mm: f64,
}

impl StepLimiter {
    pub const fn new(t: Micros) -> StepLimiter {
        StepLimiter {
            last_move_t: t,
            last_step_mm: 0.0,
        }
    }

    pub fn max_step_mm(&self, t: Micros) -> f64 {
        let dt = t.saturating_sub(self.last_move_t);
        if dt == Micros::ZERO {
            return 0.0;
        }
        let window = dt.min(checked_interval());
        let normalised = JUMP_MAX_STEP_MM.min(self.last_step_mm + JUMP_MAX_STEP_INCREASE_MM);
        normalised * window.as_secs_f64() / JUMP_REFERENCE_INTERVAL.as_secs_f64()
    }

    pub fn record(&mut self, t: Micros, step_mm: f64) {
        if step_mm <= 0.0 {
            return;
        }
        let dt = t.saturating_sub(self.last_move_t);
        if dt > Micros::ZERO && dt <= checked_interval() {
            self.last_step_mm = step_mm * JUMP_REFERENCE_INTERVAL.as_secs_f64() / dt.as_secs_f64();
        }
        self.last_move_t = t;
    }
}

fn checked_interval() -> Micros {
    JUMP_REFERENCE_INTERVAL + JUMP_REFERENCE_INTERVAL
}

#[derive(Debug, Clone, Copy, PartialEq)]
struct Stroke {
    tracking_ids: [u16; 2],
    offset: TouchpadPoint,
    following: Option<Following>,
    limiter: StepLimiter,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ScrollSynth {
    surface: SurfaceGeometry,
    touchpad: TouchpadGeometry,
    stroke: Option<Stroke>,
    frame: Frame,
    tracking_ids: TrackingIds,
}

impl ScrollSynth {
    pub fn new(surface: SurfaceGeometry) -> ScrollSynth {
        ScrollSynth {
            surface,
            touchpad: VIRTUAL_TOUCHPAD,
            stroke: None,
            frame: Frame::EMPTY,
            tracking_ids: TrackingIds::default(),
        }
    }

    pub const fn frame(&self) -> &Frame {
        &self.frame
    }

    pub fn update(
        &mut self,
        t: Micros,
        intent: Intent,
        tracker: &Tracker,
        settings: ScrollSettings,
    ) -> Option<Frame> {
        match intent {
            Intent::None => return self.lift_all(),
            Intent::Freeze => {}
            Intent::Hold => {
                self.begin(t);
            }
            Intent::Scroll { track } => {
                self.begin(t);
                if let Some(track) = tracker.get(track).filter(|track| track.phase.is_contact()) {
                    self.follow(t, track.id, track.pos, settings);
                }
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

    fn begin(&mut self, t: Micros) {
        if self.stroke.is_none() {
            self.stroke = Some(Stroke {
                tracking_ids: [self.tracking_ids.allocate(), self.tracking_ids.allocate()],
                offset: TouchpadPoint::default(),
                following: None,
                limiter: StepLimiter::new(t),
            });
        }
    }

    fn follow(&mut self, t: Micros, track: TrackId, pos: SurfacePoint, settings: ScrollSettings) {
        let Some(mut stroke) = self.stroke else {
            return;
        };
        let following = match stroke.following {
            Some(following) if following.track == track => following,
            _ => Following {
                track,
                anchor: pos,
                origin: stroke.offset,
            },
        };
        stroke.following = Some(following);
        let from = self.surface.to_mm(following.anchor);
        let to = self.surface.to_mm(pos);
        let dx = if settings.horizontal {
            (to.x - from.x) * settings.gain
        } else {
            0.0
        };
        let dy = (to.y - from.y) * settings.gain;
        let target = TouchpadPoint {
            x: following.origin.x + self.touchpad.mm_to_units(dx),
            y: following.origin.y + self.touchpad.mm_to_units(dy),
        };
        let wanted_x = f64::from(target.x - stroke.offset.x);
        let wanted_y = f64::from(target.y - stroke.offset.y);
        let wanted = wanted_x.hypot(wanted_y);
        let max_units = stroke.limiter.max_step_mm(t) * f64::from(self.touchpad.units_per_mm);
        let scale = if wanted > max_units {
            max_units / wanted
        } else {
            1.0
        };
        let step = TouchpadPoint {
            x: truncate(wanted_x * scale),
            y: truncate(wanted_y * scale),
        };
        if step != TouchpadPoint::default() {
            let before = self.place_first(stroke.offset);
            stroke.offset = TouchpadPoint {
                x: stroke.offset.x + step.x,
                y: stroke.offset.y + step.y,
            };
            let after = self.place_first(stroke.offset);
            let moved_mm = f64::from(after.x - before.x).hypot(f64::from(after.y - before.y))
                / f64::from(self.touchpad.units_per_mm);
            stroke.limiter.record(t, moved_mm);
        }
        self.stroke = Some(stroke);
    }

    fn starts(&self) -> [TouchpadPoint; 2] {
        let centre = self.touchpad.centre();
        let half = self.touchpad.mm_to_units(SCROLL_FINGER_SPACING_MM / 2.0);
        [
            TouchpadPoint {
                x: centre.x - half,
                y: centre.y,
            },
            TouchpadPoint {
                x: centre.x + half,
                y: centre.y,
            },
        ]
    }

    fn place_first(&self, offset: TouchpadPoint) -> TouchpadPoint {
        let [start, _] = self.starts();
        self.shifted(start, offset)
    }

    fn shifted(&self, start: TouchpadPoint, offset: TouchpadPoint) -> TouchpadPoint {
        self.touchpad.clamp(TouchpadPoint {
            x: start.x.saturating_add(offset.x),
            y: start.y.saturating_add(offset.y),
        })
    }

    fn place(&self) -> Frame {
        let mut frame = Frame::EMPTY;
        let Some(stroke) = &self.stroke else {
            return frame;
        };
        for (slot, (start, tracking_id)) in self
            .starts()
            .into_iter()
            .zip(stroke.tracking_ids)
            .enumerate()
        {
            frame.slots[slot] = Some(Contact::new(
                tracking_id,
                self.shifted(start, stroke.offset),
            ));
        }
        frame
    }
}

#[allow(clippy::cast_possible_truncation)]
fn truncate(value: f64) -> i32 {
    value.trunc() as i32
}
#[cfg(test)]
mod tests;
