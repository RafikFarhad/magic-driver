use std::collections::{BTreeMap, VecDeque};

use md_core::config::Config;
use md_core::profile::{Profile, SurfaceDirection};
use md_trace::{
    ButtonName, ContactInfo, DecisionReason, DecisionRecord, GestureState, OutputEvent,
    ReportRecord, SessionRecord, TouchInfo, TouchpadInfo, TraceRecord, TrackClass,
};

use crate::timeline::{Entry, Timeline};

pub const RECENT_EVENTS: usize = 40;
pub const CHECKPOINT_INTERVAL: u64 = 2048;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TouchDecision {
    pub track: u64,
    pub class: TrackClass,
    pub reason: DecisionReason,
    pub t: u64,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ButtonsDown {
    pub left: bool,
    pub right: bool,
    pub middle: bool,
}

impl ButtonsDown {
    fn set(&mut self, button: ButtonName, pressed: bool) {
        match button {
            ButtonName::Left => self.left = pressed,
            ButtonName::Right => self.right = pressed,
            ButtonName::Middle => self.middle = pressed,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TimedEvent<T> {
    pub t: u64,
    pub event: T,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ViewState {
    pub report: Option<ReportRecord>,
    pub reports: u64,
    pub touch_decisions: BTreeMap<u8, TouchDecision>,
    pub recogniser: GestureState,
    pub synthetic: Vec<ContactInfo>,
    pub buttons: ButtonsDown,
    pub pointer_events: VecDeque<TimedEvent<OutputEvent>>,
    pub decisions: VecDeque<DecisionRecord>,
    pub session: Option<SessionRecord>,
    pub dropped: u64,
}

impl Default for ViewState {
    fn default() -> Self {
        ViewState {
            report: None,
            reports: 0,
            touch_decisions: BTreeMap::new(),
            recogniser: GestureState::Idle,
            synthetic: Vec::new(),
            buttons: ButtonsDown::default(),
            pointer_events: VecDeque::new(),
            decisions: VecDeque::new(),
            session: None,
            dropped: 0,
        }
    }
}

impl ViewState {
    pub fn apply(&mut self, entry: &Entry) {
        match &entry.record {
            TraceRecord::Report(report) => self.apply_report(report),
            TraceRecord::Decision(decision) => {
                if let Some(touch_id) = decision.touch_id {
                    self.touch_decisions.insert(
                        touch_id,
                        TouchDecision {
                            track: decision.track,
                            class: decision.class,
                            reason: decision.reason,
                            t: decision.t,
                        },
                    );
                }
                push_capped(&mut self.decisions, *decision);
            }
            TraceRecord::Recogniser(change) => self.recogniser = change.to,
            TraceRecord::Output(output) => match &output.output {
                OutputEvent::TouchFrame { contacts } => self.synthetic.clone_from(contacts),
                OutputEvent::Button { button, pressed } => {
                    self.buttons.set(*button, *pressed);
                    push_capped(
                        &mut self.pointer_events,
                        TimedEvent {
                            t: output.t,
                            event: output.output.clone(),
                        },
                    );
                }
                OutputEvent::PointerMotion { .. } => push_capped(
                    &mut self.pointer_events,
                    TimedEvent {
                        t: output.t,
                        event: output.output.clone(),
                    },
                ),
            },
            TraceRecord::Session(session) => self.session = Some(session.clone()),
            TraceRecord::Dropped(dropped) => self.dropped += dropped.count,
            TraceRecord::Hello(_) => {}
        }
    }

    fn apply_report(&mut self, report: &ReportRecord) {
        self.reports += 1;
        if report.error.is_none() {
            self.touch_decisions.retain(|touch_id, _| {
                report
                    .touches
                    .iter()
                    .any(|touch| touch.id == *touch_id && !starts_new_track(touch.state))
            });
        }
        self.report = Some(report.clone());
    }

    pub fn decision_for(&self, touch_id: u8) -> Option<&TouchDecision> {
        self.touch_decisions.get(&touch_id)
    }
}

const fn starts_new_track(state: u8) -> bool {
    matches!(state & 0xf0, 0x00 | 0x20 | 0x30)
}

fn push_capped<T>(queue: &mut VecDeque<T>, item: T) {
    if queue.len() == RECENT_EVENTS {
        queue.pop_front();
    }
    queue.push_back(item);
}

#[derive(Debug, Clone, Default)]
pub struct ViewCursor {
    seq: u64,
    state: ViewState,
    checkpoints: Vec<(u64, ViewState)>,
}

impl ViewCursor {
    pub fn new() -> ViewCursor {
        ViewCursor::default()
    }

    pub fn state(&self) -> &ViewState {
        &self.state
    }

    pub const fn seq(&self) -> u64 {
        self.seq
    }

    pub fn reset(&mut self) {
        *self = ViewCursor::default();
    }

    pub fn seek(&mut self, timeline: &Timeline, t: u64) -> &ViewState {
        let target = timeline.seq_at(t);
        self.seek_seq(timeline, target)
    }

    pub fn seek_seq(&mut self, timeline: &Timeline, target: u64) -> &ViewState {
        let first = timeline.first_seq();
        let target = target.clamp(first, timeline.end_seq());
        self.checkpoints.retain(|(seq, _)| *seq >= first);
        if self.seq < first || target < self.seq {
            self.restore(first, target);
        }
        while self.seq < target {
            let Some(entry) = timeline.get(self.seq) else {
                break;
            };
            self.state.apply(entry);
            self.seq += 1;
            if self.seq.is_multiple_of(CHECKPOINT_INTERVAL)
                && self
                    .checkpoints
                    .last()
                    .is_none_or(|(seq, _)| *seq < self.seq)
            {
                self.checkpoints.push((self.seq, self.state.clone()));
            }
        }
        &self.state
    }

    fn restore(&mut self, first: u64, target: u64) {
        if let Some((seq, state)) = self
            .checkpoints
            .iter()
            .rev()
            .find(|(seq, _)| *seq <= target)
        {
            self.seq = *seq;
            self.state = state.clone();
        } else {
            self.seq = first;
            self.state = ViewState::default();
        }
    }
}

pub fn fold(timeline: &Timeline, t: u64) -> ViewState {
    let mut state = ViewState::default();
    for entry in timeline.entries().take_while(|entry| entry.t <= t) {
        state.apply(entry);
    }
    state
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TouchStateKind {
    None,
    Hover,
    Start,
    Drag,
    Lift,
    Unknown,
}

impl TouchStateKind {
    pub const fn from_bits(bits: u8) -> TouchStateKind {
        match bits & 0xf0 {
            0x00 => TouchStateKind::None,
            0x20 => TouchStateKind::Hover,
            0x30 => TouchStateKind::Start,
            0x40 => TouchStateKind::Drag,
            0x50..=0x70 => TouchStateKind::Lift,
            _ => TouchStateKind::Unknown,
        }
    }

    pub const fn label(self) -> &'static str {
        match self {
            TouchStateKind::None => "none",
            TouchStateKind::Hover => "hover",
            TouchStateKind::Start => "start",
            TouchStateKind::Drag => "touching",
            TouchStateKind::Lift => "lift",
            TouchStateKind::Unknown => "unknown",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TouchDot {
    pub id: u8,
    pub x: f64,
    pub y: f64,
    pub major_mm: f64,
    pub minor_mm: f64,
    pub orientation: i8,
    pub state: u8,
    pub kind: TouchStateKind,
    pub class: Option<TrackClass>,
    pub reason: Option<DecisionReason>,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SurfaceMapper {
    profile: &'static Profile,
}

impl SurfaceMapper {
    pub const fn new(profile: &'static Profile) -> SurfaceMapper {
        SurfaceMapper { profile }
    }

    pub const fn profile(&self) -> &'static Profile {
        self.profile
    }

    pub fn size_mm(&self) -> (f64, f64) {
        (
            self.profile.surface.x.size_mm,
            self.profile.surface.y.size_mm,
        )
    }

    pub fn normalise(&self, x: i32, y: i32) -> (f64, f64) {
        let surface = &self.profile.surface;
        let nx = fraction(x, surface.x.min, surface.x.max);
        let x = match surface.x.increases_toward {
            SurfaceDirection::Left => 1.0 - nx,
            _ => nx,
        };
        let ny = fraction(y, surface.y.min, surface.y.max);
        let y = match surface.y.increases_toward {
            SurfaceDirection::Front => 1.0 - ny,
            _ => ny,
        };
        (x, y)
    }

    pub fn contact_mm(&self, units: u8) -> f64 {
        f64::from(i32::from(units) * self.profile.touch_size_scale)
            / self.profile.surface.x.units_per_mm()
    }

    pub fn rear_zone_start(&self, rear_zone: f64) -> f64 {
        1.0 - rear_zone.clamp(0.0, 1.0)
    }

    pub fn dot(&self, touch: &TouchInfo, decision: Option<&TouchDecision>) -> TouchDot {
        let (x, y) = self.normalise(touch.x, touch.y);
        TouchDot {
            id: touch.id,
            x,
            y,
            major_mm: self.contact_mm(touch.touch_major),
            minor_mm: self.contact_mm(touch.touch_minor),
            orientation: touch.orientation,
            state: touch.state,
            kind: TouchStateKind::from_bits(touch.state),
            class: decision.map(|decision| decision.class),
            reason: decision.map(|decision| decision.reason),
        }
    }

    pub fn dots(&self, state: &ViewState) -> Vec<TouchDot> {
        state.report.as_ref().map_or_else(Vec::new, |report| {
            report
                .touches
                .iter()
                .map(|touch| self.dot(touch, state.decision_for(touch.id)))
                .collect()
        })
    }
}

fn fraction(value: i32, min: i32, max: i32) -> f64 {
    let span = f64::from(max - min);
    if span <= 0.0 {
        return 0.5;
    }
    (f64::from(value - min) / span).clamp(0.0, 1.0)
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SyntheticFinger {
    pub slot: u8,
    pub tracking_id: u16,
    pub x_mm: f64,
    pub y_mm: f64,
}

pub fn synthetic_fingers(pad: TouchpadInfo, contacts: &[ContactInfo]) -> Vec<SyntheticFinger> {
    let units = f64::from(pad.units_per_mm.max(1));
    let centre_x = f64::from(pad.max_x) / 2.0;
    let centre_y = f64::from(pad.max_y) / 2.0;
    contacts
        .iter()
        .map(|contact| SyntheticFinger {
            slot: contact.slot,
            tracking_id: contact.tracking_id,
            x_mm: (f64::from(contact.x) - centre_x) / units,
            y_mm: (f64::from(contact.y) - centre_y) / units,
        })
        .collect()
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Thresholds {
    pub rear_zone: f64,
    pub palm_size_mm: f64,
    pub promote_distance_mm: f64,
    pub mouse_motion_threshold: f64,
}

impl Thresholds {
    pub fn from_config(config: &Config) -> Thresholds {
        let touch = &config.advanced.touch;
        Thresholds {
            rear_zone: touch.rear_zone,
            palm_size_mm: touch.palm_size_mm,
            promote_distance_mm: touch.promote_distance_mm,
            mouse_motion_threshold: touch.mouse_motion_threshold,
        }
    }

    pub fn from_config_value(value: &serde_json::Value, fallback: Thresholds) -> Thresholds {
        let read = |key: &str, default: f64| {
            value
                .pointer(&format!("/advanced/touch/{key}"))
                .and_then(serde_json::Value::as_f64)
                .filter(|value| value.is_finite())
                .unwrap_or(default)
        };
        Thresholds {
            rear_zone: read("rear_zone", fallback.rear_zone),
            palm_size_mm: read("palm_size_mm", fallback.palm_size_mm),
            promote_distance_mm: read("promote_distance_mm", fallback.promote_distance_mm),
            mouse_motion_threshold: read("mouse_motion_threshold", fallback.mouse_motion_threshold),
        }
    }

    pub fn is_palm_sized(&self, dot: &TouchDot) -> bool {
        dot.major_mm.max(dot.minor_mm) > self.palm_size_mm
    }
}

#[cfg(test)]
mod tests;
