use serde::Serialize;

use crate::BoundedVec;
use crate::Micros;
use crate::classify::{Class, Classifier};
use crate::context::Context;
use crate::tracker::{MAX_TRACKS, TrackId, Tracker};

pub const FLING_WINDOW: Micros = Micros::from_millis(3_000);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RecogniserState {
    Idle,
    Scroll,
    Cancelling,
    Suppressed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Intent {
    None,
    Hold,
    Scroll { track: TrackId },
    Freeze,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RecogniserSettings {
    pub scroll_enabled: bool,
    pub cancel_hold: Micros,
}

type Watched = BoundedVec<TrackId, MAX_TRACKS>;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum State {
    Idle,
    Scroll { track: TrackId },
    Cancelling { until: Micros, watched: Watched },
    Suppressed { watched: Watched },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Fling {
    pub start: Micros,
    pub until: Micros,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Recogniser {
    state: State,
    fling: Option<Fling>,
    holding: bool,
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
        }
    }

    pub const fn state(&self) -> RecogniserState {
        match self.state {
            State::Idle => RecogniserState::Idle,
            State::Scroll { .. } => RecogniserState::Scroll,
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
            State::Idle | State::Scroll { .. } | State::Suppressed { .. } => None,
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
        settings: RecogniserSettings,
    ) -> Intent {
        match self.state {
            State::Scroll { track } => {
                self.scroll(t, track, tracker, classifier, context, settings)
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
                    self.idle(t, classifier, context, settings)
                }
            }
            State::Idle => self.idle(t, classifier, context, settings),
        }
    }

    fn scroll(
        &mut self,
        t: Micros,
        track: TrackId,
        tracker: &Tracker,
        classifier: &Classifier,
        context: &Context,
        settings: RecogniserSettings,
    ) -> Intent {
        let cancelled = !settings.scroll_enabled
            || context.is_button_down()
            || classifier.class_of(track) == Some(Class::Rejected);
        let lifted = tracker
            .get(track)
            .is_none_or(|track| !track.phase.is_contact());
        if cancelled {
            self.state = State::Cancelling {
                until: t + settings.cancel_hold,
                watched: watched(track, classifier),
            };
            if settings.cancel_hold == Micros::ZERO {
                self.state = State::Suppressed {
                    watched: watched(track, classifier),
                };
                return Intent::None;
            }
            Intent::Freeze
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

    fn idle(
        &mut self,
        t: Micros,
        classifier: &Classifier,
        context: &Context,
        settings: RecogniserSettings,
    ) -> Intent {
        if !settings.scroll_enabled || context.is_button_down() {
            return self.end_hold();
        }
        let mut deliberate = classifier.tracks_in(Class::Deliberate);
        if let (Some(track), None) = (deliberate.next(), deliberate.next()) {
            self.holding = false;
            self.fling = None;
            self.state = State::Scroll { track };
            return Intent::Scroll { track };
        }
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

fn watched(track: TrackId, classifier: &Classifier) -> Watched {
    let mut watched = Watched::new();
    let _ = watched.push(track);
    for entry in classifier.entries() {
        let rejected_now = classifier
            .decisions()
            .iter()
            .any(|decision| decision.track == entry.track && decision.class == Class::Rejected);
        if entry.track != track && (entry.class != Class::Rejected || rejected_now) {
            let _ = watched.push(entry.track);
        }
    }
    watched
}
#[cfg(test)]
mod tests;
