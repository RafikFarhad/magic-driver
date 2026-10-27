use std::fmt;

use serde::Serialize;

use crate::Micros;
use crate::buttons::{ButtonEdge, Buttons};
use crate::classify::Classifier;
use crate::config::Config;
use crate::context::Context;
use crate::primitives::Button;
use crate::profile::Profile;
use crate::recognise::Recogniser;
use crate::recognise::RecogniserState;
use crate::report::{self, Report};
use crate::synth::{Frame, RawForward, ScrollProbe};
use crate::tracker::Tracker;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Mode {
    Normal,
    PointerOnly,
    RawForward,
    ScrollProbe,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Input<'a> {
    Report { t: Micros, bytes: &'a [u8] },
    Timer { t: Micros },
    ConfigChanged(Config),
    Reset,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Output {
    PointerMotion {
        dx: i32,
        dy: i32,
    },
    Button {
        button: Button,
        pressed: bool,
    },
    TouchFrame(Frame),
    #[serde(skip)]
    Trace(md_trace::TraceRecord),
}

impl Output {
    pub const fn is_trace(&self) -> bool {
        matches!(self, Output::Trace(_))
    }
}

impl fmt::Display for Output {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Output::PointerMotion { dx, dy } => write!(f, "pointer dx={dx} dy={dy}"),
            Output::Button { button, pressed } => write!(
                f,
                "button {button} {}",
                if *pressed { "press" } else { "release" }
            ),
            Output::TouchFrame(frame) => write!(f, "{frame}"),
            Output::Trace(record) => write!(f, "trace {record}"),
        }
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct EngineStats {
    pub reports: u64,
    pub ignored: u64,
    pub malformed: u64,
}

#[derive(Debug, Clone, PartialEq)]
enum TouchOutput {
    None,
    RawForward(RawForward),
    ScrollProbe(ScrollProbe),
    Normal(Box<ScrollPath>),
}

impl TouchOutput {
    fn new(mode: Mode, profile: &Profile) -> TouchOutput {
        match mode {
            Mode::PointerOnly => TouchOutput::None,
            Mode::RawForward => TouchOutput::RawForward(RawForward::new(profile.surface)),
            Mode::ScrollProbe => TouchOutput::ScrollProbe(ScrollProbe::new(profile.surface)),
            Mode::Normal => TouchOutput::Normal(Box::new(ScrollPath::new(profile))),
        }
    }

    fn update(
        &mut self,
        t: Micros,
        tracker: &Tracker,
        context: &Context,
        profile: &Profile,
        config: &Config,
    ) -> Option<Frame> {
        match self {
            TouchOutput::None => None,
            TouchOutput::RawForward(raw) => raw.update(tracker),
            TouchOutput::ScrollProbe(probe) => probe.update(tracker),
            TouchOutput::Normal(path) => path.update(t, tracker, context, profile, config),
        }
    }

    fn next_deadline(&self, config: &Config) -> Option<Micros> {
        match self {
            TouchOutput::Normal(path) => path.next_deadline(config),
            TouchOutput::None | TouchOutput::RawForward(_) | TouchOutput::ScrollProbe(_) => None,
        }
    }

    fn lift_all(&mut self) -> Option<Frame> {
        match self {
            TouchOutput::None => None,
            TouchOutput::RawForward(raw) => raw.lift_all(),
            TouchOutput::ScrollProbe(probe) => probe.lift_all(),
            TouchOutput::Normal(path) => path.lift_all(),
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Engine {
    profile: &'static Profile,
    config: Config,
    mode: Mode,
    tracker: Tracker,
    context: Context,
    buttons: Buttons,
    touch_output: TouchOutput,
    tracing: bool,
    stats: EngineStats,
}

impl Engine {
    pub fn new(profile: &'static Profile, config: Config, mode: Mode) -> Engine {
        Engine {
            profile,
            config,
            mode,
            tracker: Tracker::new(profile.surface),
            context: Context::new(),
            buttons: Buttons::new(),
            touch_output: TouchOutput::new(mode, profile),
            tracing: false,
            stats: EngineStats::default(),
        }
    }

    pub fn handle(&mut self, input: Input<'_>, out: &mut Vec<Output>) {
        match input {
            Input::Report { t, bytes } => self.handle_report(t, bytes, out),
            Input::Timer { t } => self.handle_timer(t, out),
            Input::ConfigChanged(config) => self.config = config,
            Input::Reset => self.reset(out),
        }
    }

    pub fn next_deadline(&self) -> Option<Micros> {
        self.touch_output.next_deadline(&self.config)
    }

    pub fn set_tracing(&mut self, enabled: bool) {
        self.tracing = enabled;
    }

    pub const fn is_tracing(&self) -> bool {
        self.tracing
    }

    pub const fn profile(&self) -> &'static Profile {
        self.profile
    }

    pub const fn config(&self) -> &Config {
        &self.config
    }

    pub const fn mode(&self) -> Mode {
        self.mode
    }

    pub const fn stats(&self) -> EngineStats {
        self.stats
    }

    pub const fn tracker(&self) -> &Tracker {
        &self.tracker
    }

    pub const fn context(&self) -> &Context {
        &self.context
    }

    pub const fn scroll_path(&self) -> Option<&ScrollPath> {
        match &self.touch_output {
            TouchOutput::Normal(path) => Some(path),
            TouchOutput::None | TouchOutput::RawForward(_) | TouchOutput::ScrollProbe(_) => None,
        }
    }

    pub fn classifier(&self) -> Option<&Classifier> {
        self.scroll_path().map(ScrollPath::classifier)
    }

    pub fn recogniser(&self) -> Option<&Recogniser> {
        self.scroll_path().map(ScrollPath::recogniser)
    }

    fn emit(&self, t: Micros, output: Output, out: &mut Vec<Output>) {
        let record = if self.tracing {
            crate::trace::output_record(t, &output)
        } else {
            None
        };
        out.push(output);
        if let Some(record) = record {
            out.push(Output::Trace(record));
        }
    }

    fn recogniser_state(&self) -> Option<RecogniserState> {
        self.scroll_path().map(|path| path.recogniser().state())
    }

    fn update_touches(&mut self, t: Micros, out: &mut Vec<Output>) {
        let before = if self.tracing {
            self.recogniser_state()
        } else {
            None
        };
        let frame =
            self.touch_output
                .update(t, &self.tracker, &self.context, self.profile, &self.config);
        if self.tracing {
            self.trace_decisions(t, before, out);
        }
        if let Some(frame) = frame {
            self.emit(t, Output::TouchFrame(frame), out);
        }
    }

    fn trace_decisions(&self, t: Micros, before: Option<RecogniserState>, out: &mut Vec<Output>) {
        let Some(path) = self.scroll_path() else {
            return;
        };
        for decision in path.classifier().decisions() {
            out.push(Output::Trace(crate::trace::decision_record(
                t,
                decision,
                &self.tracker,
            )));
        }
        let after = path.recogniser().state();
        if let Some(before) = before.filter(|before| *before != after) {
            out.push(Output::Trace(crate::trace::recogniser_record(
                t, before, after,
            )));
        }
    }

    fn handle_timer(&mut self, t: Micros, out: &mut Vec<Output>) {
        if self.next_deadline().is_none_or(|deadline| deadline > t) {
            return;
        }
        self.update_touches(t, out);
    }

    fn handle_report(&mut self, t: Micros, bytes: &[u8], out: &mut Vec<Output>) {
        self.stats.reports += 1;
        let parsed = report::parse(self.profile, bytes);
        if self.tracing {
            out.push(Output::Trace(crate::trace::report_record(t, &parsed)));
        }
        let report = match parsed {
            Ok(report) => report,
            Err(error) => {
                if error.is_malformed() {
                    self.stats.malformed += 1;
                } else {
                    self.stats.ignored += 1;
                }
                return;
            }
        };
        self.pointer(t, &report, out);
        if report.has_touch_area() {
            self.tracker.update(t, &report.touches);
            self.update_touches(t, out);
        }
    }

    fn pointer(&mut self, t: Micros, report: &Report, out: &mut Vec<Output>) {
        let header = report.header;
        if header.dx != 0 || header.dy != 0 {
            self.emit(
                t,
                Output::PointerMotion {
                    dx: header.dx,
                    dy: header.dy,
                },
                out,
            );
        }
        self.context.on_motion(t, header.dx, header.dy);
        let edges = self.buttons.update(
            &self.profile.buttons,
            header.clicks,
            &report.touches,
            self.config.pointer.middle_click,
        );
        for edge in &edges {
            self.emit_button(t, *edge, out);
        }
    }

    fn emit_button(&mut self, t: Micros, edge: ButtonEdge, out: &mut Vec<Output>) {
        self.context.on_button(t, edge.button, edge.pressed);
        self.emit(
            t,
            Output::Button {
                button: edge.button,
                pressed: edge.pressed,
            },
            out,
        );
    }

    fn reset(&mut self, out: &mut Vec<Output>) {
        let t = Micros::ZERO;
        let before = self.recogniser_state();
        for edge in &self.buttons.release_all() {
            self.emit(
                t,
                Output::Button {
                    button: edge.button,
                    pressed: edge.pressed,
                },
                out,
            );
        }
        if let Some(frame) = self.touch_output.lift_all() {
            self.emit(t, Output::TouchFrame(frame), out);
        }
        if self.tracing
            && let Some(before) = before.filter(|state| *state != RecogniserState::Idle)
        {
            out.push(Output::Trace(crate::trace::recogniser_record(
                t,
                before,
                RecogniserState::Idle,
            )));
        }
        self.tracker.reset();
        self.context.reset();
    }
}

mod normal;

pub use normal::ScrollPath;

#[cfg(test)]
mod tests;
