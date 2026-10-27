use serde::Serialize;

use crate::Micros;
use crate::profile::SurfaceGeometry;
use crate::report::{Touch, TouchState};
use crate::surface::SurfacePoint;

pub const MAX_TRACKS: usize = 16;
pub const VELOCITY_WINDOW: Micros = Micros::from_millis(50);

const VELOCITY_SAMPLES: usize = 8;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
pub struct TrackId(u64);

impl TrackId {
    pub const fn as_u64(self) -> u64 {
        self.0
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Phase {
    Hover,
    Starting,
    Active,
    Lifting,
    Gone,
}

impl Phase {
    pub const fn is_contact(self) -> bool {
        matches!(self, Phase::Starting | Phase::Active)
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct Velocity {
    pub x_mm_s: f64,
    pub y_mm_s: f64,
}

impl Velocity {
    pub const ZERO: Velocity = Velocity {
        x_mm_s: 0.0,
        y_mm_s: 0.0,
    };

    pub fn speed_mm_s(&self) -> f64 {
        self.x_mm_s.hypot(self.y_mm_s)
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct TouchSize {
    pub touch_major: u8,
    pub touch_minor: u8,
    pub size: u8,
}

impl TouchSize {
    const fn of(touch: &Touch) -> TouchSize {
        TouchSize {
            touch_major: touch.touch_major,
            touch_minor: touch.touch_minor,
            size: touch.size,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
struct Sample {
    t: Micros,
    pos: SurfacePoint,
}

#[derive(Debug, Clone, Copy, PartialEq)]
struct History {
    samples: [Sample; VELOCITY_SAMPLES],
    len: usize,
    next: usize,
}

impl History {
    const EMPTY: History = History {
        samples: [Sample {
            t: Micros::ZERO,
            pos: SurfacePoint::new(0, 0),
        }; VELOCITY_SAMPLES],
        len: 0,
        next: 0,
    };

    fn push(&mut self, sample: Sample) {
        self.samples[self.next] = sample;
        self.next = (self.next + 1) % VELOCITY_SAMPLES;
        self.len = (self.len + 1).min(VELOCITY_SAMPLES);
    }

    fn velocity(&self, geometry: &SurfaceGeometry) -> Velocity {
        let Some(newest) = self.newest() else {
            return Velocity::ZERO;
        };
        let window_start = newest.t.saturating_sub(VELOCITY_WINDOW);
        let oldest = (0..self.len)
            .map(|age| {
                self.samples[(self.next + VELOCITY_SAMPLES - self.len + age) % VELOCITY_SAMPLES]
            })
            .find(|sample| sample.t >= window_start && sample.t < newest.t);
        let Some(oldest) = oldest else {
            return Velocity::ZERO;
        };
        let seconds = (newest.t - oldest.t).as_secs_f64();
        let from = geometry.to_mm(oldest.pos);
        let to = geometry.to_mm(newest.pos);
        Velocity {
            x_mm_s: (to.x - from.x) / seconds,
            y_mm_s: (to.y - from.y) / seconds,
        }
    }

    fn newest(&self) -> Option<Sample> {
        (self.len > 0).then(|| self.samples[(self.next + VELOCITY_SAMPLES - 1) % VELOCITY_SAMPLES])
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Track {
    pub id: TrackId,
    pub touch_id: u8,
    pub start_t: Micros,
    pub start_pos: SurfacePoint,
    pub pos: SurfacePoint,
    pub velocity: Velocity,
    pub size: TouchSize,
    pub phase: Phase,
    pub last_t: Micros,
    contacted: bool,
    history: History,
}

impl Track {
    pub const fn delta(&self) -> SurfacePoint {
        SurfacePoint::new(self.pos.x - self.start_pos.x, self.pos.y - self.start_pos.y)
    }

    pub const fn has_contacted(&self) -> bool {
        self.contacted
    }

    fn new(id: TrackId, t: Micros, touch: &Touch) -> Track {
        Track {
            id,
            touch_id: touch.id,
            start_t: t,
            start_pos: touch.position(),
            pos: touch.position(),
            velocity: Velocity::ZERO,
            size: TouchSize::of(touch),
            phase: Phase::Hover,
            last_t: t,
            contacted: false,
            history: History::EMPTY,
        }
    }

    fn begin_contact(&mut self, t: Micros) {
        self.contacted = true;
        self.start_t = t;
        self.start_pos = self.pos;
        self.history = History::EMPTY;
        self.velocity = Velocity::ZERO;
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Tracker {
    geometry: SurfaceGeometry,
    slots: [Option<Track>; MAX_TRACKS],
    next_id: u64,
}

impl Tracker {
    pub fn new(geometry: SurfaceGeometry) -> Tracker {
        Tracker {
            geometry,
            slots: [None; MAX_TRACKS],
            next_id: 0,
        }
    }

    pub fn update(&mut self, t: Micros, touches: &[Touch]) {
        for slot in &mut self.slots {
            if slot.is_some_and(|track| track.phase == Phase::Gone) {
                *slot = None;
            }
        }
        let mut seen = [false; MAX_TRACKS];
        for touch in touches {
            let index = usize::from(touch.id) % MAX_TRACKS;
            seen[index] = true;
            self.apply(index, t, touch);
        }
        for (slot, seen) in self.slots.iter_mut().zip(seen) {
            if let Some(track) = slot
                && !seen
            {
                track.phase = Phase::Gone;
                track.velocity = Velocity::ZERO;
                track.last_t = t;
            }
        }
    }

    pub fn reset(&mut self) {
        self.slots = [None; MAX_TRACKS];
    }

    pub fn tracks(&self) -> impl Iterator<Item = &Track> {
        self.slots.iter().flatten()
    }

    pub fn contacts(&self) -> impl Iterator<Item = &Track> {
        self.tracks().filter(|track| track.phase.is_contact())
    }

    pub fn get(&self, id: TrackId) -> Option<&Track> {
        self.tracks().find(|track| track.id == id)
    }

    pub fn by_touch_id(&self, touch_id: u8) -> Option<&Track> {
        self.slots.get(usize::from(touch_id))?.as_ref()
    }

    fn allocate_id(&mut self) -> TrackId {
        let id = TrackId(self.next_id);
        self.next_id += 1;
        id
    }

    fn apply(&mut self, index: usize, t: Micros, touch: &Touch) {
        let current = self.slots[index].map(|track| track.phase);
        let Some(phase) = phase_for(touch.state, current) else {
            if let Some(track) = &mut self.slots[index] {
                track.phase = Phase::Gone;
                track.pos = touch.position();
                track.velocity = Velocity::ZERO;
                track.last_t = t;
            }
            return;
        };
        let restarts = self.slots[index].is_some_and(|track| {
            phase.is_contact() && !track.phase.is_contact() && track.contacted
        });
        if self.slots[index].is_none() || restarts {
            let id = self.allocate_id();
            self.slots[index] = Some(Track::new(id, t, touch));
        }
        let geometry = self.geometry;
        let Some(track) = &mut self.slots[index] else {
            return;
        };
        let was_contact = track.phase.is_contact();
        track.pos = touch.position();
        track.size = TouchSize::of(touch);
        track.last_t = t;
        track.phase = phase;
        if phase.is_contact() && !was_contact {
            track.begin_contact(t);
        }
        if !track.contacted {
            track.start_t = t;
            track.start_pos = track.pos;
            return;
        }
        track.history.push(Sample { t, pos: track.pos });
        track.velocity = track.history.velocity(&geometry);
    }
}

fn phase_for(state: TouchState, current: Option<Phase>) -> Option<Phase> {
    match state {
        TouchState::None => None,
        TouchState::Hover => Some(Phase::Hover),
        TouchState::Start => Some(Phase::Starting),
        TouchState::Drag => Some(Phase::Active),
        TouchState::Lift(_) => Some(Phase::Lifting),
        TouchState::Unknown(_) => Some(match current {
            Some(phase) if phase.is_contact() => Phase::Active,
            Some(Phase::Lifting) => Phase::Lifting,
            _ => Phase::Hover,
        }),
    }
}

#[cfg(test)]
mod tests;
