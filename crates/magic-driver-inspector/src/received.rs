use std::collections::VecDeque;
use std::fmt::Write as _;

use smithay_client_toolkit::seat::pointer::{AxisScroll, PointerEventKind};
use wayland_client::protocol::wl_pointer;
use wayland_protocols::wp::pointer_gestures::zv1::client::{
    zwp_pointer_gesture_hold_v1, zwp_pointer_gesture_pinch_v1, zwp_pointer_gesture_swipe_v1,
};

use crate::clock::{ClockAlignment, WrappingMillis};

pub mod wayland;

pub const RECEIVED_LOG_CAPACITY: usize = 4_096;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScrollAxis {
    Vertical,
    Horizontal,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AxisSource {
    Wheel,
    Finger,
    Continuous,
    WheelTilt,
    Unknown(u32),
}

impl AxisSource {
    pub fn label(self) -> String {
        match self {
            AxisSource::Wheel => "wheel".to_owned(),
            AxisSource::Finger => "finger".to_owned(),
            AxisSource::Continuous => "continuous".to_owned(),
            AxisSource::WheelTilt => "wheel tilt".to_owned(),
            AxisSource::Unknown(value) => format!("source {value}"),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GestureKind {
    Swipe,
    Pinch,
    Hold,
}

impl GestureKind {
    pub const fn label(self) -> &'static str {
        match self {
            GestureKind::Swipe => "swipe",
            GestureKind::Pinch => "pinch",
            GestureKind::Hold => "hold",
        }
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct AxisChange {
    pub value: Option<f64>,
    pub discrete: Option<i32>,
    pub value120: Option<i32>,
    pub inverted: Option<bool>,
    pub stop: bool,
}

impl AxisChange {
    pub fn is_empty(&self) -> bool {
        *self == AxisChange::default()
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct ScrollFrame {
    pub time: Option<u32>,
    pub source: Option<AxisSource>,
    pub vertical: AxisChange,
    pub horizontal: AxisChange,
}

impl ScrollFrame {
    pub const fn axis(&self, axis: ScrollAxis) -> &AxisChange {
        match axis {
            ScrollAxis::Vertical => &self.vertical,
            ScrollAxis::Horizontal => &self.horizontal,
        }
    }

    pub fn has_stop(&self) -> bool {
        self.vertical.stop || self.horizontal.stop
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum GesturePhase {
    Begin {
        fingers: u32,
    },
    Update {
        dx: f64,
        dy: f64,
        scale: Option<f64>,
        rotation: Option<f64>,
    },
    End {
        cancelled: bool,
    },
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct GestureEvent {
    pub time: u32,
    pub kind: GestureKind,
    pub phase: GesturePhase,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Received {
    Focus(bool),
    Scroll(ScrollFrame),
    Gesture(GestureEvent),
}

impl Received {
    pub fn time(&self) -> Option<u32> {
        match self {
            Received::Focus(_) => None,
            Received::Scroll(frame) => frame.time,
            Received::Gesture(gesture) => Some(gesture.time),
        }
    }
}

pub fn pointer_received(kind: &PointerEventKind) -> Option<Received> {
    match kind {
        PointerEventKind::Enter { .. } => Some(Received::Focus(true)),
        PointerEventKind::Leave { .. } => Some(Received::Focus(false)),
        PointerEventKind::Axis {
            time,
            horizontal,
            vertical,
            source,
        } => {
            let frame = ScrollFrame {
                time: (*time != 0).then_some(*time),
                source: source.map(axis_source),
                vertical: axis_change(vertical),
                horizontal: axis_change(horizontal),
            };
            (frame != ScrollFrame::default()).then_some(Received::Scroll(frame))
        }
        PointerEventKind::Motion { .. }
        | PointerEventKind::Press { .. }
        | PointerEventKind::Release { .. } => None,
    }
}

fn axis_change(scroll: &AxisScroll) -> AxisChange {
    AxisChange {
        value: (scroll.absolute != 0.0).then_some(scroll.absolute),
        discrete: (scroll.discrete != 0).then_some(scroll.discrete),
        value120: (scroll.value120 != 0).then_some(scroll.value120),
        inverted: scroll
            .relative_direction
            .map(|direction| direction == wl_pointer::AxisRelativeDirection::Inverted),
        stop: scroll.stop,
    }
}

pub fn axis_source(source: wl_pointer::AxisSource) -> AxisSource {
    match source {
        wl_pointer::AxisSource::Wheel => AxisSource::Wheel,
        wl_pointer::AxisSource::Finger => AxisSource::Finger,
        wl_pointer::AxisSource::Continuous => AxisSource::Continuous,
        wl_pointer::AxisSource::WheelTilt => AxisSource::WheelTilt,
        other => AxisSource::Unknown(other.into()),
    }
}

pub const fn gesture_begin(kind: GestureKind, time: u32, fingers: u32) -> GestureEvent {
    GestureEvent {
        time,
        kind,
        phase: GesturePhase::Begin { fingers },
    }
}

pub const fn gesture_end(kind: GestureKind, time: u32, cancelled: i32) -> GestureEvent {
    GestureEvent {
        time,
        kind,
        phase: GesturePhase::End {
            cancelled: cancelled != 0,
        },
    }
}

pub fn swipe_event(event: &zwp_pointer_gesture_swipe_v1::Event) -> Option<GestureEvent> {
    let kind = GestureKind::Swipe;
    match event {
        zwp_pointer_gesture_swipe_v1::Event::Begin { time, fingers, .. } => {
            Some(gesture_begin(kind, *time, *fingers))
        }
        zwp_pointer_gesture_swipe_v1::Event::Update { time, dx, dy } => Some(GestureEvent {
            time: *time,
            kind,
            phase: GesturePhase::Update {
                dx: *dx,
                dy: *dy,
                scale: None,
                rotation: None,
            },
        }),
        zwp_pointer_gesture_swipe_v1::Event::End {
            time, cancelled, ..
        } => Some(gesture_end(kind, *time, *cancelled)),
        _ => None,
    }
}

pub fn pinch_event(event: &zwp_pointer_gesture_pinch_v1::Event) -> Option<GestureEvent> {
    let kind = GestureKind::Pinch;
    match event {
        zwp_pointer_gesture_pinch_v1::Event::Begin { time, fingers, .. } => {
            Some(gesture_begin(kind, *time, *fingers))
        }
        zwp_pointer_gesture_pinch_v1::Event::Update {
            time,
            dx,
            dy,
            scale,
            rotation,
        } => Some(GestureEvent {
            time: *time,
            kind,
            phase: GesturePhase::Update {
                dx: *dx,
                dy: *dy,
                scale: Some(*scale),
                rotation: Some(*rotation),
            },
        }),
        zwp_pointer_gesture_pinch_v1::Event::End {
            time, cancelled, ..
        } => Some(gesture_end(kind, *time, *cancelled)),
        _ => None,
    }
}

pub fn hold_event(event: &zwp_pointer_gesture_hold_v1::Event) -> Option<GestureEvent> {
    let kind = GestureKind::Hold;
    match event {
        zwp_pointer_gesture_hold_v1::Event::Begin { time, fingers, .. } => {
            Some(gesture_begin(kind, *time, *fingers))
        }
        zwp_pointer_gesture_hold_v1::Event::End {
            time, cancelled, ..
        } => Some(gesture_end(kind, *time, *cancelled)),
        _ => None,
    }
}

pub fn describe(item: &Received) -> String {
    match item {
        Received::Focus(true) => "pointer entered the window".to_owned(),
        Received::Focus(false) => "pointer left the window".to_owned(),
        Received::Scroll(frame) => describe_scroll(frame),
        Received::Gesture(gesture) => describe_gesture(gesture),
    }
}

fn describe_gesture(gesture: &GestureEvent) -> String {
    let kind = gesture.kind.label();
    match gesture.phase {
        GesturePhase::Begin { fingers } => format!("{kind} begin, {fingers} fingers"),
        GesturePhase::Update {
            dx,
            dy,
            scale,
            rotation,
        } => {
            let mut text = format!("{kind} update dx {dx:.1} dy {dy:.1}");
            if let Some(scale) = scale {
                let _ = write!(text, " scale {scale:.3}");
            }
            if let Some(rotation) = rotation {
                let _ = write!(text, " rotation {rotation:.1}");
            }
            text
        }
        GesturePhase::End { cancelled: true } => format!("{kind} end (cancelled)"),
        GesturePhase::End { cancelled: false } => format!("{kind} end"),
    }
}

fn describe_scroll(frame: &ScrollFrame) -> String {
    let source = frame
        .source
        .map_or_else(|| "no source".to_owned(), AxisSource::label);
    let mut text = format!("scroll {source}");
    for (name, change) in [("v", &frame.vertical), ("h", &frame.horizontal)] {
        if !change.is_empty() {
            let _ = write!(text, " {name}: {}", describe_axis(change));
        }
    }
    text
}

fn describe_axis(change: &AxisChange) -> String {
    let mut parts = Vec::new();
    if let Some(value) = change.value {
        parts.push(format!("{value:.2}"));
    }
    if let Some(discrete) = change.discrete {
        parts.push(format!("discrete {discrete}"));
    }
    if let Some(value120) = change.value120 {
        parts.push(format!("value120 {value120}"));
    }
    if change.inverted == Some(true) {
        parts.push("inverted".to_owned());
    }
    if change.stop {
        parts.push("stop".to_owned());
    }
    parts.join(" ")
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ReceivedEntry {
    pub local_us: u64,
    pub wayland_ms: Option<u64>,
    pub item: Received,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ReceivedLog {
    entries: VecDeque<ReceivedEntry>,
    capacity: usize,
    millis: WrappingMillis,
    alignment: ClockAlignment,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ScrollPoint {
    pub local_us: u64,
    pub value: f64,
    pub source: Option<AxisSource>,
    pub discrete: Option<i32>,
    pub value120: Option<i32>,
    pub stop: bool,
}

impl ReceivedLog {
    pub fn new() -> ReceivedLog {
        ReceivedLog::with_capacity(RECEIVED_LOG_CAPACITY)
    }

    pub fn with_capacity(capacity: usize) -> ReceivedLog {
        ReceivedLog {
            entries: VecDeque::new(),
            capacity: capacity.max(1),
            millis: WrappingMillis::new(),
            alignment: ClockAlignment::new(),
        }
    }

    pub fn record(&mut self, item: Received, dispatched_local_us: u64) {
        let wayland_ms = item.time().map(|time| self.millis.extend(time));
        let local_us = match wayland_ms {
            Some(ms) => {
                let remote = ms.saturating_mul(1_000);
                self.alignment.observe(remote, dispatched_local_us);
                self.alignment
                    .to_local(remote)
                    .unwrap_or(dispatched_local_us)
            }
            None => dispatched_local_us,
        };
        if self.entries.len() == self.capacity {
            self.entries.pop_front();
        }
        self.entries.push_back(ReceivedEntry {
            local_us,
            wayland_ms,
            item,
        });
    }

    pub fn clear(&mut self) {
        self.entries.clear();
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    pub fn entries(&self) -> impl DoubleEndedIterator<Item = &ReceivedEntry> {
        self.entries.iter()
    }

    pub fn scroll_series(&self, axis: ScrollAxis, from_us: u64, to_us: u64) -> Vec<ScrollPoint> {
        self.entries
            .iter()
            .filter(|entry| (from_us..=to_us).contains(&entry.local_us))
            .filter_map(|entry| match &entry.item {
                Received::Scroll(frame) => {
                    let change = frame.axis(axis);
                    (!change.is_empty()).then_some(ScrollPoint {
                        local_us: entry.local_us,
                        value: change.value.unwrap_or(0.0),
                        source: frame.source,
                        discrete: change.discrete,
                        value120: change.value120,
                        stop: change.stop,
                    })
                }
                Received::Focus(_) | Received::Gesture(_) => None,
            })
            .collect()
    }
}

impl Default for ReceivedLog {
    fn default() -> Self {
        ReceivedLog::new()
    }
}

#[cfg(test)]
mod tests;
