use serde::Serialize;

use crate::BoundedVec;
use crate::Micros;
use crate::config::Config;
use crate::context::Context;
use crate::profile::{Profile, SurfaceDirection, SurfaceGeometry};
use crate::surface::SurfacePoint;
use crate::tracker::{MAX_TRACKS, Track, TrackId, Tracker};

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Class {
    #[default]
    Candidate,
    Deliberate,
    Rejected,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Reason {
    #[default]
    Palm,
    RearZone,
    MouseMoving,
    ClickGuard,
    Resting,
    Promoted,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize)]
pub struct Decision {
    pub track: TrackId,
    pub previous: Class,
    pub class: Class,
    pub reason: Reason,
}

impl Decision {
    pub const fn cancels_deliberate(&self) -> bool {
        matches!(self.previous, Class::Deliberate) && matches!(self.class, Class::Rejected)
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Classified {
    pub track: TrackId,
    pub class: Class,
    pub reason: Option<Reason>,
    start_t: Micros,
    anchor: SurfacePoint,
    anchor_t: Micros,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Thresholds {
    pub palm_size_mm: f64,
    pub rear_zone: f64,
    pub mouse_motion_threshold: f64,
    pub click_guard: Micros,
    pub promote_distance_mm: f64,
    pub promote_window: Micros,
    pub rest_dwell: Micros,
}

impl Thresholds {
    pub fn from_config(config: &Config) -> Thresholds {
        let touch = &config.advanced.touch;
        Thresholds {
            palm_size_mm: touch.palm_size_mm,
            rear_zone: touch.rear_zone,
            mouse_motion_threshold: touch.mouse_motion_threshold,
            click_guard: Micros::from_millis(u64::from(touch.click_guard_ms)),
            promote_distance_mm: touch.promote_distance_mm,
            promote_window: Micros::from_millis(u64::from(touch.promote_window_ms)),
            rest_dwell: Micros::from_millis(u64::from(touch.rest_dwell_ms)),
        }
    }
}

pub type Decisions = BoundedVec<Decision, MAX_TRACKS>;

#[derive(Debug, Clone, PartialEq)]
pub struct Classifier {
    entries: BoundedVec<Classified, MAX_TRACKS>,
    decisions: Decisions,
}

impl Default for Classifier {
    fn default() -> Self {
        Classifier::new()
    }
}

impl Classifier {
    pub fn new() -> Classifier {
        Classifier {
            entries: BoundedVec::new(),
            decisions: BoundedVec::new(),
        }
    }

    pub fn update(
        &mut self,
        t: Micros,
        tracker: &Tracker,
        context: &Context,
        profile: &Profile,
        thresholds: &Thresholds,
    ) {
        self.decisions.clear();
        let button_down = context.is_button_down();
        let moving = context.motion_energy(t) > thresholds.mouse_motion_threshold;

        let previous = self.entries;
        self.entries.clear();
        for track in tracker.contacts() {
            let entry = previous
                .iter()
                .find(|entry| entry.track == track.id)
                .copied()
                .unwrap_or(Classified {
                    track: track.id,
                    class: Class::Candidate,
                    reason: None,
                    start_t: track.start_t,
                    anchor: track.start_pos,
                    anchor_t: track.start_t,
                });
            let input = RuleInput {
                t,
                track,
                context,
                surface: &profile.surface,
                size_scale: profile.touch_size_scale,
                thresholds,
                button_down,
                moving,
            };
            let next = classify(entry, &input);
            if next.class != entry.class
                && let Some(reason) = next.reason
            {
                let _ = self.decisions.push(Decision {
                    track: track.id,
                    previous: entry.class,
                    class: next.class,
                    reason,
                });
            }
            let _ = self.entries.push(next);
        }
    }

    pub fn reset(&mut self) {
        *self = Classifier::new();
    }

    pub fn decisions(&self) -> &[Decision] {
        &self.decisions
    }

    pub fn entries(&self) -> &[Classified] {
        &self.entries
    }

    pub fn class_of(&self, track: TrackId) -> Option<Class> {
        self.entries
            .iter()
            .find(|entry| entry.track == track)
            .map(|entry| entry.class)
    }

    pub fn tracks_in(&self, class: Class) -> impl Iterator<Item = TrackId> + '_ {
        self.entries
            .iter()
            .filter(move |entry| entry.class == class)
            .map(|entry| entry.track)
    }

    pub fn next_deadline(&self, thresholds: &Thresholds) -> Option<Micros> {
        self.entries
            .iter()
            .filter(|entry| entry.class == Class::Candidate)
            .map(|entry| entry.start_t + thresholds.rest_dwell)
            .min()
    }
}

impl Classified {
    pub const fn start_t(&self) -> Micros {
        self.start_t
    }
}

struct RuleInput<'a> {
    t: Micros,
    track: &'a Track,
    context: &'a Context,
    surface: &'a SurfaceGeometry,
    size_scale: i32,
    thresholds: &'a Thresholds,
    button_down: bool,
    moving: bool,
}

fn classify(entry: Classified, input: &RuleInput<'_>) -> Classified {
    match entry.class {
        Class::Rejected => entry,
        Class::Deliberate => match deliberate_cancel(input) {
            Some(reason) => rejected(entry, reason),
            None => entry,
        },
        Class::Candidate => candidate(entry, input),
    }
}

fn rejected(entry: Classified, reason: Reason) -> Classified {
    Classified {
        class: Class::Rejected,
        reason: Some(reason),
        ..entry
    }
}

fn deliberate_cancel(input: &RuleInput<'_>) -> Option<Reason> {
    if input.button_down {
        Some(Reason::ClickGuard)
    } else if input.moving {
        Some(Reason::MouseMoving)
    } else {
        None
    }
}

fn candidate(entry: Classified, input: &RuleInput<'_>) -> Classified {
    let track = input.track;
    let thresholds = input.thresholds;
    if touch_size_mm(input.surface, input.size_scale, track) > thresholds.palm_size_mm {
        return rejected(entry, Reason::Palm);
    }
    if in_rear_zone(input.surface, thresholds.rear_zone, track.start_pos) {
        return rejected(entry, Reason::RearZone);
    }
    if input.button_down || started_in_click_guard(input) {
        return rejected(entry, Reason::ClickGuard);
    }
    if input.moving {
        return rejected(entry, Reason::MouseMoving);
    }
    let mut entry = entry;
    let travelled = distance_mm(input.surface, entry.anchor, track.pos);
    if travelled >= thresholds.promote_distance_mm && track.last_t > track.start_t {
        return Classified {
            class: Class::Deliberate,
            reason: Some(Reason::Promoted),
            ..entry
        };
    }
    if input.t.saturating_sub(track.start_t) >= thresholds.rest_dwell {
        return rejected(entry, Reason::Resting);
    }
    if input.t.saturating_sub(entry.anchor_t) >= thresholds.promote_window {
        entry.anchor = track.pos;
        entry.anchor_t = input.t;
    }
    entry
}

fn started_in_click_guard(input: &RuleInput<'_>) -> bool {
    input.context.last_release().is_some_and(|release| {
        input.track.start_t >= release
            && input.track.start_t.saturating_sub(release) < input.thresholds.click_guard
    })
}

pub fn touch_size_mm(surface: &SurfaceGeometry, size_scale: i32, track: &Track) -> f64 {
    let raw = track.size.touch_major.max(track.size.touch_minor);
    f64::from(i32::from(raw) * size_scale) / surface.x.units_per_mm()
}

pub fn in_rear_zone(surface: &SurfaceGeometry, rear_zone: f64, point: SurfacePoint) -> bool {
    let axis = surface.y;
    let depth = f64::from(axis.span_units()) * rear_zone;
    match axis.increases_toward {
        SurfaceDirection::Rear => f64::from(point.y) > f64::from(axis.max) - depth,
        SurfaceDirection::Front => f64::from(point.y) < f64::from(axis.min) + depth,
        SurfaceDirection::Left | SurfaceDirection::Right => false,
    }
}

fn distance_mm(surface: &SurfaceGeometry, from: SurfacePoint, to: SurfacePoint) -> f64 {
    let from = surface.to_mm(from);
    let to = surface.to_mm(to);
    (to.x - from.x).hypot(to.y - from.y)
}
#[cfg(test)]
mod tests;
