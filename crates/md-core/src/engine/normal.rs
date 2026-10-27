use crate::Micros;
use crate::classify::{Classifier, Thresholds};
use crate::config::Config;
use crate::context::Context;
use crate::profile::Profile;
use crate::recognise::{Intent, Recogniser, RecogniserSettings};
use crate::synth::{Frame, ScrollSettings, ScrollSynth};
use crate::tracker::Tracker;

#[derive(Debug, Clone, PartialEq)]
pub struct ScrollPath {
    classifier: Classifier,
    recogniser: Recogniser,
    synth: ScrollSynth,
    intent: Intent,
}

impl ScrollPath {
    pub fn new(profile: &Profile) -> ScrollPath {
        ScrollPath {
            classifier: Classifier::new(),
            recogniser: Recogniser::new(),
            synth: ScrollSynth::new(profile.surface),
            intent: Intent::None,
        }
    }

    pub const fn classifier(&self) -> &Classifier {
        &self.classifier
    }

    pub const fn recogniser(&self) -> &Recogniser {
        &self.recogniser
    }

    pub const fn intent(&self) -> Intent {
        self.intent
    }

    pub fn update(
        &mut self,
        t: Micros,
        tracker: &Tracker,
        context: &Context,
        profile: &Profile,
        config: &Config,
    ) -> Option<Frame> {
        let thresholds = Thresholds::from_config(config);
        self.classifier
            .update(t, tracker, context, profile, &thresholds);
        self.intent = self.recogniser.update(
            t,
            tracker,
            &self.classifier,
            context,
            RecogniserSettings {
                scroll_enabled: config.scroll.enabled,
                cancel_hold: Micros::from_millis(u64::from(config.advanced.scroll.cancel_hold_ms)),
            },
        );
        self.synth.update(
            t,
            self.intent,
            tracker,
            ScrollSettings {
                gain: config.advanced.scroll.gain_min,
                horizontal: config.scroll.horizontal,
            },
        )
    }

    pub fn next_deadline(&self, config: &Config) -> Option<Micros> {
        let thresholds = Thresholds::from_config(config);
        let classifier = if self.intent == Intent::Hold {
            self.classifier.next_deadline(&thresholds)
        } else {
            None
        };
        match (classifier, self.recogniser.next_deadline()) {
            (Some(a), Some(b)) => Some(a.min(b)),
            (a, b) => a.or(b),
        }
    }

    pub fn lift_all(&mut self) -> Option<Frame> {
        self.classifier.reset();
        self.recogniser.reset();
        self.intent = Intent::None;
        self.synth.lift_all()
    }
}
