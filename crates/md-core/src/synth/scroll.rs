use crate::Micros;
use crate::config::Config;
use crate::primitives::Axis;
use crate::profile::SurfaceGeometry;
use crate::recognise::Intent;
use crate::surface::{SurfacePoint, SurfacePointMm};
use crate::synth::chase::{Chase, place};
use crate::synth::frame::{
    Contact, Frame, TouchpadGeometry, TouchpadPoint, TrackingIds, VIRTUAL_TOUCHPAD,
};
use crate::synth::gain::GainCurve;
use crate::tracker::{Track, TrackId, Tracker};

pub const SCROLL_FINGER_SPACING_MM: f64 = 20.0;
pub const JUMP_REFERENCE_INTERVAL: Micros = Micros::from_millis(12);
pub const JUMP_MAX_STEP_MM: f64 = 10.0;
pub const JUMP_MAX_STEP_INCREASE_MM: f64 = 5.0;
pub const AXIS_LOCK_DECIDE_MM: f64 = 1.0;
pub const AXIS_LOCK_DOMINANCE: f64 = 1.5;
pub const AXIS_LOCK_FORCE_MM: f64 = 4.0;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ScrollSettings {
    pub gain: GainCurve,
    pub horizontal: bool,
    pub axis_lock: bool,
}

impl ScrollSettings {
    pub fn from_config(config: &Config) -> ScrollSettings {
        ScrollSettings {
            gain: GainCurve::from_config(config),
            horizontal: config.scroll.horizontal,
            axis_lock: config.scroll.axis_lock,
        }
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct AxisLock {
    axis: Option<Axis>,
}

impl AxisLock {
    pub fn update(&mut self, travel: SurfacePointMm) -> Option<Axis> {
        if self.axis.is_some() {
            return self.axis;
        }
        let (x, y) = (travel.x.abs(), travel.y.abs());
        let distance = x.hypot(y);
        if distance < AXIS_LOCK_DECIDE_MM {
            return None;
        }
        self.axis = if y >= x * AXIS_LOCK_DOMINANCE {
            Some(Axis::Vertical)
        } else if x >= y * AXIS_LOCK_DOMINANCE {
            Some(Axis::Horizontal)
        } else if distance >= AXIS_LOCK_FORCE_MM {
            Some(if y >= x {
                Axis::Vertical
            } else {
                Axis::Horizontal
            })
        } else {
            None
        };
        self.axis
    }
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
struct Following {
    track: TrackId,
    last: SurfacePoint,
    origin: TouchpadPoint,
    travel: SurfacePointMm,
    undecided: SurfacePointMm,
    lock: AxisLock,
}

#[derive(Debug, Clone, Copy, PartialEq)]
struct Stroke {
    tracking_ids: [u16; 2],
    following: Option<Following>,
    chase: Chase,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ScrollSynth {
    surface: SurfaceGeometry,
    touchpad: TouchpadGeometry,
    stroke: Option<Stroke>,
    frame: Frame,
}

impl ScrollSynth {
    pub fn new(surface: SurfaceGeometry) -> ScrollSynth {
        ScrollSynth {
            surface,
            touchpad: VIRTUAL_TOUCHPAD,
            stroke: None,
            frame: Frame::EMPTY,
        }
    }

    pub const fn frame(&self) -> &Frame {
        &self.frame
    }

    pub fn is_active(&self) -> bool {
        self.frame.contact_count() > 0
    }

    pub fn update(
        &mut self,
        t: Micros,
        intent: Intent,
        tracker: &Tracker,
        tracking_ids: &mut TrackingIds,
        settings: &ScrollSettings,
    ) -> Option<Frame> {
        match intent {
            Intent::Freeze => {}
            Intent::Hold => {
                self.begin(t, tracking_ids);
            }
            Intent::Scroll { track } => {
                self.begin(t, tracking_ids);
                if let Some(track) = tracker.get(track).filter(|track| track.phase.is_contact()) {
                    self.follow(t, track, settings);
                }
            }
            Intent::None | Intent::Swipe { .. } | Intent::Overview => return self.lift_all(),
        }
        let previous = self.frame;
        self.frame = self.place();
        (self.frame != previous).then_some(self.frame)
    }

    pub fn lift_all(&mut self) -> Option<Frame> {
        self.stroke = None;
        let was_empty = self.frame.is_empty();
        self.frame = Frame::EMPTY;
        if was_empty { None } else { Some(Frame::EMPTY) }
    }

    fn begin(&mut self, t: Micros, tracking_ids: &mut TrackingIds) {
        if self.stroke.is_none() {
            self.stroke = Some(Stroke {
                tracking_ids: [tracking_ids.allocate(), tracking_ids.allocate()],
                following: None,
                chase: Chase::new(t),
            });
        }
    }

    fn follow(&mut self, t: Micros, track: &Track, settings: &ScrollSettings) {
        let Some(mut stroke) = self.stroke else {
            return;
        };
        let mut following = match stroke.following {
            Some(following) if following.track == track.id => following,
            _ => Following {
                track: track.id,
                last: track.pos,
                origin: stroke.chase.offset(),
                travel: SurfacePointMm::default(),
                undecided: SurfacePointMm::default(),
                lock: AxisLock::default(),
            },
        };
        let from = self.surface.to_mm(following.last);
        let to = self.surface.to_mm(track.pos);
        let gain = settings.gain.gain(track.velocity.speed_mm_s());
        let mut dx = if settings.horizontal {
            (to.x - from.x) * gain
        } else {
            0.0
        };
        let mut dy = (to.y - from.y) * gain;
        if settings.axis_lock {
            let held = following.undecided;
            match following.lock.update(self.surface.to_mm(track.delta())) {
                Some(Axis::Vertical) => {
                    dx = 0.0;
                    dy += held.y;
                }
                Some(Axis::Horizontal) => {
                    dx += held.x;
                    dy = 0.0;
                }
                None => {
                    following.undecided = SurfacePointMm::new(held.x + dx, held.y + dy);
                    dx = 0.0;
                    dy = 0.0;
                }
            }
            if following.lock != AxisLock::default() {
                following.undecided = SurfacePointMm::default();
            }
        }
        following.last = track.pos;
        following.travel = SurfacePointMm::new(following.travel.x + dx, following.travel.y + dy);
        let target = TouchpadPoint {
            x: following.origin.x + self.touchpad.mm_to_units(following.travel.x),
            y: following.origin.y + self.touchpad.mm_to_units(following.travel.y),
        };
        stroke.chase.step(t, target, &self.starts(), &self.touchpad);
        stroke.following = Some(following);
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
                place(&self.touchpad, start, stroke.chase.offset()),
            ));
        }
        frame
    }
}

#[cfg(test)]
mod tests;
