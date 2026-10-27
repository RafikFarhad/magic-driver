use crate::Micros;
use crate::classify::{Classifier, Thresholds};
use crate::config::Config;
use crate::context::Context;
use crate::profile::Profile;
use crate::recognise::{Intent, Recogniser, RecogniserSettings};
use crate::synth::{
    Frame, SCRIPT_FRAME_INTERVAL, ScrollSettings, ScrollSynth, SwipeSynth, SwipeUpScript,
    TrackingIds,
};
use crate::tracker::Tracker;

#[derive(Debug, Clone, PartialEq)]
pub struct ScrollPath {
    classifier: Classifier,
    recogniser: Recogniser,
    scroll: ScrollSynth,
    swipe: SwipeSynth,
    script: Option<SwipeUpScript>,
    tracking_ids: TrackingIds,
    intent: Intent,
}

impl ScrollPath {
    pub fn new(profile: &Profile) -> ScrollPath {
        ScrollPath {
            classifier: Classifier::new(),
            recogniser: Recogniser::new(),
            scroll: ScrollSynth::new(profile.surface),
            swipe: SwipeSynth::new(profile.surface),
            script: None,
            tracking_ids: TrackingIds::default(),
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
            &RecogniserSettings::from_config(config, profile),
        );
        self.synthesise(t, tracker, context, config)
    }

    fn synthesise(
        &mut self,
        t: Micros,
        tracker: &Tracker,
        context: &Context,
        config: &Config,
    ) -> Option<Frame> {
        if let Some(script) = self.script.as_mut() {
            if context.is_button_down() && !script.is_cancelled_or_lifting() {
                let hold = Micros::from_millis(u64::from(config.advanced.scroll.cancel_hold_ms));
                script.cancel(t + hold);
            }
            let frame = script.update(t, &mut self.tracking_ids);
            if script.is_done() {
                self.script = None;
            }
            return frame;
        }
        let fingers = usize::from(config.advanced.gestures.kwin_swipe_fingers);
        match self.intent {
            Intent::Overview => {
                let lifted = self.lift_strokes();
                let start = if lifted.is_some() {
                    t + SCRIPT_FRAME_INTERVAL
                } else {
                    t
                };
                let mut script = SwipeUpScript::new(start, fingers);
                let frame = lifted.or_else(|| script.update(t, &mut self.tracking_ids));
                self.script = Some(script);
                frame
            }
            Intent::Swipe { .. } => {
                if let Some(lift) = self.scroll.lift_all() {
                    return Some(lift);
                }
                self.swipe
                    .update(t, self.intent, tracker, &mut self.tracking_ids, fingers)
            }
            Intent::Freeze if self.swipe.is_active() => {
                self.swipe
                    .update(t, self.intent, tracker, &mut self.tracking_ids, fingers)
            }
            Intent::None => self.lift_strokes(),
            Intent::Freeze | Intent::Hold | Intent::Scroll { .. } => {
                if let Some(lift) = self.swipe.lift_all() {
                    return Some(lift);
                }
                self.scroll.update(
                    t,
                    self.intent,
                    tracker,
                    &mut self.tracking_ids,
                    &ScrollSettings::from_config(config),
                )
            }
        }
    }

    fn lift_strokes(&mut self) -> Option<Frame> {
        let scroll = self.scroll.lift_all();
        let swipe = self.swipe.lift_all();
        scroll.or(swipe)
    }

    pub fn next_deadline(&self, config: &Config) -> Option<Micros> {
        let thresholds = Thresholds::from_config(config);
        let classifier = if self.intent == Intent::Hold {
            self.classifier.next_deadline(&thresholds)
        } else {
            None
        };
        [
            classifier,
            self.recogniser.next_deadline(),
            self.script.and_then(|script| script.next_deadline()),
        ]
        .into_iter()
        .flatten()
        .min()
    }

    pub fn lift_all(&mut self) -> Option<Frame> {
        self.classifier.reset();
        self.recogniser.reset();
        self.intent = Intent::None;
        let script = self.script.take().and_then(|mut script| script.lift_all());
        let strokes = self.lift_strokes();
        script.or(strokes)
    }
}
