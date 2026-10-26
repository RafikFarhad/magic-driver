use std::fmt;

use serde::Serialize;

use crate::Micros;
use crate::buttons::{ButtonEdge, Buttons};
use crate::config::Config;
use crate::context::Context;
use crate::primitives::Button;
use crate::profile::Profile;
use crate::report::{self, Report};
use crate::synth::{Frame, RawForward, ScrollProbe};
use crate::tracker::Tracker;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Mode {
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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Output {
    PointerMotion { dx: i32, dy: i32 },
    Button { button: Button, pressed: bool },
    TouchFrame(Frame),
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
}

impl TouchOutput {
    fn new(mode: Mode, profile: &Profile) -> TouchOutput {
        match mode {
            Mode::PointerOnly => TouchOutput::None,
            Mode::RawForward => TouchOutput::RawForward(RawForward::new(profile.surface)),
            Mode::ScrollProbe => TouchOutput::ScrollProbe(ScrollProbe::new(profile.surface)),
        }
    }

    fn update(&mut self, tracker: &Tracker) -> Option<Frame> {
        match self {
            TouchOutput::None => None,
            TouchOutput::RawForward(raw) => raw.update(tracker),
            TouchOutput::ScrollProbe(probe) => probe.update(tracker),
        }
    }

    fn lift_all(&mut self) -> Option<Frame> {
        match self {
            TouchOutput::None => None,
            TouchOutput::RawForward(raw) => raw.lift_all(),
            TouchOutput::ScrollProbe(probe) => probe.lift_all(),
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
            Input::Timer { .. } => {}
            Input::ConfigChanged(config) => self.config = config,
            Input::Reset => self.reset(out),
        }
    }

    pub fn next_deadline(&self) -> Option<Micros> {
        None
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

    fn handle_report(&mut self, t: Micros, bytes: &[u8], out: &mut Vec<Output>) {
        self.stats.reports += 1;
        let report = match report::parse(self.profile, bytes) {
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
            if let Some(frame) = self.touch_output.update(&self.tracker) {
                out.push(Output::TouchFrame(frame));
            }
        }
    }

    fn pointer(&mut self, t: Micros, report: &Report, out: &mut Vec<Output>) {
        let header = report.header;
        if header.dx != 0 || header.dy != 0 {
            out.push(Output::PointerMotion {
                dx: header.dx,
                dy: header.dy,
            });
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
        out.push(Output::Button {
            button: edge.button,
            pressed: edge.pressed,
        });
    }

    fn reset(&mut self, out: &mut Vec<Output>) {
        for edge in &self.buttons.release_all() {
            out.push(Output::Button {
                button: edge.button,
                pressed: edge.pressed,
            });
        }
        if let Some(frame) = self.touch_output.lift_all() {
            out.push(Output::TouchFrame(frame));
        }
        self.tracker.reset();
        self.context.reset();
    }
}

#[cfg(test)]
mod tests;
