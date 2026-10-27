use serde::Serialize;

use crate::BoundedVec;
use crate::Micros;
use crate::classify::{Class, Classified, Classifier};
use crate::config::Config;
use crate::context::Context;
use crate::profile::{Profile, SurfaceGeometry};
use crate::tracker::{MAX_TRACKS, TrackId, Tracker};

pub const FLING_WINDOW: Micros = Micros::from_millis(3_000);
pub const TWO_FINGER_START_WINDOW: Micros = Micros::from_millis(150);
pub const SWIPE_HORIZONTAL_DOMINANCE: f64 = 2.0;
pub const TAP_SPREAD: Micros = Micros::from_millis(100);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RecogniserState {
    Idle,
    Scroll,
    Swipe,
    TapPending,
    Cancelling,
    Suppressed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Intent {
    None,
    Hold,
    Scroll { track: TrackId },
    Swipe { tracks: [TrackId; 2] },
    Freeze,
    Overview,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RecogniserSettings {
    pub scroll_enabled: bool,
    pub cancel_hold: Micros,
    pub swipe_enabled: bool,
    pub swipe_distance_mm: f64,
    pub double_tap_enabled: bool,
    pub double_tap: Micros,
    pub promote_distance_mm: f64,
    pub surface: SurfaceGeometry,
}

impl RecogniserSettings {
    pub fn from_config(config: &Config, profile: &Profile) -> RecogniserSettings {
        RecogniserSettings {
            scroll_enabled: config.scroll.enabled,
            cancel_hold: Micros::from_millis(u64::from(config.advanced.scroll.cancel_hold_ms)),
            swipe_enabled: config.gestures.swipe_desktops,
            swipe_distance_mm: config.advanced.gestures.swipe_distance_mm,
            double_tap_enabled: config.gestures.double_tap_overview,
            double_tap: Micros::from_millis(u64::from(config.advanced.gestures.double_tap_ms)),
            promote_distance_mm: config.advanced.touch.promote_distance_mm,
            surface: profile.surface,
        }
    }
}

type Watched = BoundedVec<TrackId, MAX_TRACKS>;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum State {
    Idle,
    Scroll { track: TrackId },
    Swipe { tracks: [TrackId; 2] },
    TapPending { until: Micros },
    Cancelling { until: Micros, watched: Watched },
    Suppressed { watched: Watched },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Fling {
    pub start: Micros,
    pub until: Micros,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Tap {
    pub start: Micros,
    pub end: Micros,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TapOutcome {
    Tap(Tap),
    Failed,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
struct TapTouch {
    track: TrackId,
    start_t: Micros,
    end_t: Option<Micros>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
struct TapGroup {
    touches: BoundedVec<TapTouch, MAX_TRACKS>,
    spoiled: bool,
}

impl TapGroup {
    fn clear(&mut self) {
        *self = TapGroup::default();
    }

    fn is_empty(&self) -> bool {
        self.touches.is_empty()
    }

    fn update(
        &mut self,
        t: Micros,
        tracker: &Tracker,
        classifier: &Classifier,
        context: &Context,
        settings: &RecogniserSettings,
    ) -> Option<TapOutcome> {
        if context.is_button_down() {
            self.spoiled = true;
        }
        let mut touches = self.touches;
        for touch in touches.iter_mut().filter(|touch| touch.end_t.is_none()) {
            if let Some(track) = tracker.get(touch.track) {
                let moved = distance_mm(&settings.surface, track);
                if moved >= settings.promote_distance_mm {
                    self.spoiled = true;
                }
            }
            match classifier.class_of(touch.track) {
                Some(Class::Candidate) => {}
                Some(Class::Deliberate | Class::Rejected) => self.spoiled = true,
                None => touch.end_t = Some(t),
            }
        }
        self.touches = touches;
        for entry in classifier.entries() {
            match entry.class {
                Class::Deliberate => self.spoiled = true,
                Class::Rejected => {}
                Class::Candidate => {
                    if self.touches.iter().all(|touch| touch.track != entry.track)
                        && self
                            .touches
                            .push(TapTouch {
                                track: entry.track,
                                start_t: entry.start_t(),
                                end_t: None,
                            })
                            .is_err()
                    {
                        self.spoiled = true;
                    }
                }
            }
        }
        if self.touches.is_empty() {
            self.spoiled = false;
            return None;
        }
        if self.touches.iter().any(|touch| touch.end_t.is_none()) {
            return None;
        }
        let outcome = self.evaluate();
        self.clear();
        Some(outcome)
    }

    fn evaluate(&self) -> TapOutcome {
        let [a, b] = match self.touches.as_slice() {
            [a, b] if !self.spoiled => [*a, *b],
            _ => return TapOutcome::Failed,
        };
        let (Some(a_end), Some(b_end)) = (a.end_t, b.end_t) else {
            return TapOutcome::Failed;
        };
        if spread(a.start_t, b.start_t) > TAP_SPREAD || spread(a_end, b_end) > TAP_SPREAD {
            return TapOutcome::Failed;
        }
        TapOutcome::Tap(Tap {
            start: a.start_t.min(b.start_t),
            end: a_end.max(b_end),
        })
    }
}

fn spread(a: Micros, b: Micros) -> Micros {
    a.max(b) - a.min(b)
}

fn distance_mm(surface: &SurfaceGeometry, track: &crate::tracker::Track) -> f64 {
    let delta = surface.to_mm(track.delta());
    delta.x.hypot(delta.y)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Recogniser {
    state: State,
    fling: Option<Fling>,
    holding: bool,
    taps: TapGroup,
}

impl Default for Recogniser {
    fn default() -> Self {
        Recogniser::new()
    }
}

impl Recogniser {
    pub fn new() -> Recogniser {
        Recogniser {
            state: State::Idle,
            fling: None,
            holding: false,
            taps: TapGroup::default(),
        }
    }

    pub const fn state(&self) -> RecogniserState {
        match self.state {
            State::Idle => RecogniserState::Idle,
            State::Scroll { .. } => RecogniserState::Scroll,
            State::Swipe { .. } => RecogniserState::Swipe,
            State::TapPending { .. } => RecogniserState::TapPending,
            State::Cancelling { .. } => RecogniserState::Cancelling,
            State::Suppressed { .. } => RecogniserState::Suppressed,
        }
    }

    pub const fn fling(&self) -> Option<Fling> {
        self.fling
    }

    pub const fn is_holding(&self) -> bool {
        self.holding
    }

    pub fn next_deadline(&self) -> Option<Micros> {
        match self.state {
            State::Cancelling { until, .. } => Some(until),
            State::TapPending { until } if self.taps.is_empty() => Some(until),
            State::Idle
            | State::Scroll { .. }
            | State::Swipe { .. }
            | State::TapPending { .. }
            | State::Suppressed { .. } => None,
        }
    }

    pub fn reset(&mut self) {
        *self = Recogniser::new();
    }

    pub fn update(
        &mut self,
        t: Micros,
        tracker: &Tracker,
        classifier: &Classifier,
        context: &Context,
        settings: &RecogniserSettings,
    ) -> Intent {
        match self.state {
            State::Scroll { track } => {
                self.scroll(t, track, tracker, classifier, context, settings)
            }
            State::Swipe { tracks } => {
                self.swipe(t, tracks, tracker, classifier, context, settings)
            }
            State::Cancelling { until, watched } => {
                if t < until {
                    Intent::Freeze
                } else {
                    self.state = State::Suppressed { watched };
                    Intent::None
                }
            }
            State::Suppressed { watched } => {
                let touching = watched.iter().any(|id| {
                    tracker
                        .get(*id)
                        .is_some_and(|track| track.phase.is_contact())
                });
                if touching || context.is_button_down() {
                    Intent::None
                } else {
                    self.state = State::Idle;
                    self.idle(t, tracker, classifier, context, settings)
                }
            }
            State::Idle | State::TapPending { .. } => {
                self.idle(t, tracker, classifier, context, settings)
            }
        }
    }

    fn cancel(
        &mut self,
        t: Micros,
        tracks: &[TrackId],
        classifier: &Classifier,
        settings: &RecogniserSettings,
    ) -> Intent {
        let watched = watched(tracks, classifier);
        if settings.cancel_hold == Micros::ZERO {
            self.state = State::Suppressed { watched };
            return Intent::None;
        }
        self.state = State::Cancelling {
            until: t + settings.cancel_hold,
            watched,
        };
        Intent::Freeze
    }

    fn scroll(
        &mut self,
        t: Micros,
        track: TrackId,
        tracker: &Tracker,
        classifier: &Classifier,
        context: &Context,
        settings: &RecogniserSettings,
    ) -> Intent {
        let cancelled = !settings.scroll_enabled
            || context.is_button_down()
            || classifier.class_of(track) == Some(Class::Rejected);
        let lifted = tracker
            .get(track)
            .is_none_or(|track| !track.phase.is_contact());
        if cancelled {
            self.cancel(t, &[track], classifier, settings)
        } else if lifted {
            self.state = State::Idle;
            self.fling = Some(Fling {
                start: t,
                until: t + FLING_WINDOW,
            });
            Intent::None
        } else {
            Intent::Scroll { track }
        }
    }

    fn swipe(
        &mut self,
        t: Micros,
        tracks: [TrackId; 2],
        tracker: &Tracker,
        classifier: &Classifier,
        context: &Context,
        settings: &RecogniserSettings,
    ) -> Intent {
        let cancelled = !settings.swipe_enabled
            || context.is_button_down()
            || tracks
                .iter()
                .any(|track| classifier.class_of(*track) == Some(Class::Rejected));
        let lifted = tracks.iter().any(|track| {
            tracker
                .get(*track)
                .is_none_or(|track| !track.phase.is_contact())
        });
        if cancelled {
            self.cancel(t, &tracks, classifier, settings)
        } else if lifted {
            let mut watched = Watched::new();
            for track in tracks {
                let _ = watched.push(track);
            }
            self.state = State::Suppressed { watched };
            Intent::None
        } else {
            Intent::Swipe { tracks }
        }
    }

    fn idle(
        &mut self,
        t: Micros,
        tracker: &Tracker,
        classifier: &Classifier,
        context: &Context,
        settings: &RecogniserSettings,
    ) -> Intent {
        let tap = if settings.double_tap_enabled {
            self.taps.update(t, tracker, classifier, context, settings)
        } else {
            self.taps.clear();
            None
        };
        if context.is_button_down() {
            self.state = State::Idle;
            self.taps.clear();
            return self.end_hold();
        }
        if let Some(intent) = self.start_stroke(tracker, classifier, settings) {
            return intent;
        }
        if let Some(outcome) = tap {
            match (self.state, outcome) {
                (State::TapPending { until }, TapOutcome::Tap(tap)) if tap.start <= until => {
                    self.state = State::Idle;
                    self.holding = false;
                    self.fling = None;
                    return Intent::Overview;
                }
                (_, TapOutcome::Tap(tap)) => {
                    self.state = State::TapPending {
                        until: tap.end + settings.double_tap,
                    };
                }
                (_, TapOutcome::Failed) => self.state = State::Idle,
            }
        }
        if let State::TapPending { until } = self.state
            && t >= until
            && self.taps.is_empty()
        {
            self.state = State::Idle;
        }
        if !settings.scroll_enabled {
            return self.end_hold();
        }
        self.hold(t, classifier)
    }

    fn start_stroke(
        &mut self,
        tracker: &Tracker,
        classifier: &Classifier,
        settings: &RecogniserSettings,
    ) -> Option<Intent> {
        let mut deliberate = BoundedVec::<TrackId, MAX_TRACKS>::new();
        for track in classifier.tracks_in(Class::Deliberate) {
            let _ = deliberate.push(track);
        }
        match *deliberate.as_slice() {
            [track] if settings.scroll_enabled => {
                if settings.swipe_enabled
                    && second_finger_pending(track, tracker, classifier, settings)
                {
                    return None;
                }
                self.holding = false;
                self.fling = None;
                self.taps.clear();
                self.state = State::Scroll { track };
                Some(Intent::Scroll { track })
            }
            [a, b] if settings.swipe_enabled => {
                let tracks = [a.min(b), a.max(b)];
                if !swipe_travelled(tracks, tracker, settings) {
                    return None;
                }
                self.end_hold();
                self.taps.clear();
                self.state = State::Swipe { tracks };
                Some(Intent::Swipe { tracks })
            }
            _ => None,
        }
    }

    fn hold(&mut self, t: Micros, classifier: &Classifier) -> Intent {
        if self
            .fling
            .is_some_and(|fling| !self.holding && t >= fling.until)
        {
            self.fling = None;
        }
        let touched_during_fling = self.fling.is_some_and(|fling| {
            classifier
                .entries()
                .iter()
                .any(|entry| entry.class == Class::Candidate && entry.start_t() >= fling.start)
        });
        if touched_during_fling {
            self.holding = true;
            return Intent::Hold;
        }
        self.end_hold()
    }

    fn end_hold(&mut self) -> Intent {
        if self.holding {
            self.holding = false;
            self.fling = None;
        }
        Intent::None
    }
}

fn second_finger_pending(
    track: TrackId,
    tracker: &Tracker,
    classifier: &Classifier,
    settings: &RecogniserSettings,
) -> bool {
    let Some(start) = classifier
        .entries()
        .iter()
        .find(|entry| entry.track == track)
        .map(Classified::start_t)
    else {
        return false;
    };
    classifier.entries().iter().any(|entry| {
        entry.track != track
            && entry.class == Class::Candidate
            && spread(entry.start_t(), start) <= TWO_FINGER_START_WINDOW
            && tracker.get(entry.track).is_some_and(|other| {
                distance_mm(&settings.surface, other) >= settings.promote_distance_mm / 2.0
            })
    })
}

fn swipe_travelled(tracks: [TrackId; 2], tracker: &Tracker, settings: &RecogniserSettings) -> bool {
    let (Some(a), Some(b)) = (tracker.get(tracks[0]), tracker.get(tracks[1])) else {
        return false;
    };
    let a = settings.surface.to_mm(a.delta());
    let b = settings.surface.to_mm(b.delta());
    if a.x * b.x <= 0.0 {
        return false;
    }
    let x = f64::midpoint(a.x, b.x).abs();
    let y = f64::midpoint(a.y, b.y).abs();
    x >= settings.swipe_distance_mm && x >= SWIPE_HORIZONTAL_DOMINANCE * y
}

fn watched(tracks: &[TrackId], classifier: &Classifier) -> Watched {
    let mut watched = Watched::new();
    for track in tracks {
        let _ = watched.push(*track);
    }
    for entry in classifier.entries() {
        let rejected_now = classifier
            .decisions()
            .iter()
            .any(|decision| decision.track == entry.track && decision.class == Class::Rejected);
        if !tracks.contains(&entry.track) && (entry.class != Class::Rejected || rejected_now) {
            let _ = watched.push(entry.track);
        }
    }
    watched
}

#[cfg(test)]
mod tests;
