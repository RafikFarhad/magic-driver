use md_trace::{
    ButtonName, ContactInfo, DecisionReason, DecisionRecord, GestureState, OutputEvent,
    OutputRecord, ProfileInfo, RecogniserRecord, ReportRecord, SurfaceInfo, TouchInfo,
    TouchpadInfo, TraceRecord, TrackClass,
};

use crate::Micros;
use crate::classify::{Class, Decision, Reason};
use crate::engine::Output;
use crate::primitives::Button;
use crate::profile::Profile;
use crate::recognise::RecogniserState;
use crate::report::{ParseError, Report};
use crate::synth::{Frame, VIRTUAL_TOUCHPAD};
use crate::tracker::Tracker;

pub fn report_record(t: Micros, parsed: &Result<Report, ParseError>) -> TraceRecord {
    let record = match parsed {
        Ok(report) => ReportRecord {
            t: t.as_u64(),
            clicks: report.header.clicks,
            dx: report.header.dx,
            dy: report.header.dy,
            touches: report
                .touches
                .iter()
                .map(|touch| TouchInfo {
                    id: touch.id,
                    x: touch.x,
                    y: touch.y,
                    touch_major: touch.touch_major,
                    touch_minor: touch.touch_minor,
                    size: touch.size,
                    orientation: touch.orientation,
                    state: touch.state.bits(),
                })
                .collect(),
            error: None,
            processing_ns: None,
        },
        Err(error) => ReportRecord {
            t: t.as_u64(),
            clicks: 0,
            dx: 0,
            dy: 0,
            touches: Vec::new(),
            error: Some(error.to_string()),
            processing_ns: None,
        },
    };
    TraceRecord::Report(record)
}

pub fn output_record(t: Micros, output: &Output) -> Option<TraceRecord> {
    let event = match output {
        Output::PointerMotion { dx, dy } => OutputEvent::PointerMotion { dx: *dx, dy: *dy },
        Output::Button { button, pressed } => OutputEvent::Button {
            button: button_name(*button),
            pressed: *pressed,
        },
        Output::TouchFrame(frame) => OutputEvent::TouchFrame {
            contacts: contacts(frame),
        },
        Output::Trace(_) => return None,
    };
    Some(TraceRecord::Output(OutputRecord {
        t: t.as_u64(),
        output: event,
    }))
}

pub fn decision_record(t: Micros, decision: &Decision, tracker: &Tracker) -> TraceRecord {
    TraceRecord::Decision(DecisionRecord {
        t: t.as_u64(),
        track: decision.track.as_u64(),
        touch_id: tracker.get(decision.track).map(|track| track.touch_id),
        previous: track_class(decision.previous),
        class: track_class(decision.class),
        reason: decision_reason(decision.reason),
    })
}

pub fn recogniser_record(t: Micros, from: RecogniserState, to: RecogniserState) -> TraceRecord {
    TraceRecord::Recogniser(RecogniserRecord {
        t: t.as_u64(),
        from: gesture_state(from),
        to: gesture_state(to),
    })
}

pub fn profile_info(profile: &Profile) -> ProfileInfo {
    let surface = profile.surface;
    ProfileInfo {
        id: profile.id.to_owned(),
        name: profile.name.to_owned(),
        surface: SurfaceInfo {
            x_min: surface.x.min,
            x_max: surface.x.max,
            x_size_mm: surface.x.size_mm,
            y_min: surface.y.min,
            y_max: surface.y.max,
            y_size_mm: surface.y.size_mm,
        },
        touchpad: TouchpadInfo {
            max_x: VIRTUAL_TOUCHPAD.max_x,
            max_y: VIRTUAL_TOUCHPAD.max_y,
            units_per_mm: VIRTUAL_TOUCHPAD.units_per_mm,
        },
    }
}

fn contacts(frame: &Frame) -> Vec<ContactInfo> {
    frame
        .contacts()
        .map(|(slot, contact)| ContactInfo {
            slot: u8::try_from(slot).unwrap_or(u8::MAX),
            tracking_id: contact.tracking_id,
            x: contact.x,
            y: contact.y,
        })
        .collect()
}

pub const fn button_name(button: Button) -> ButtonName {
    match button {
        Button::Left => ButtonName::Left,
        Button::Right => ButtonName::Right,
        Button::Middle => ButtonName::Middle,
    }
}

pub const fn track_class(class: Class) -> TrackClass {
    match class {
        Class::Candidate => TrackClass::Candidate,
        Class::Deliberate => TrackClass::Deliberate,
        Class::Rejected => TrackClass::Rejected,
    }
}

pub const fn decision_reason(reason: Reason) -> DecisionReason {
    match reason {
        Reason::Palm => DecisionReason::Palm,
        Reason::RearZone => DecisionReason::RearZone,
        Reason::MouseMoving => DecisionReason::MouseMoving,
        Reason::ClickGuard => DecisionReason::ClickGuard,
        Reason::Resting => DecisionReason::Resting,
        Reason::Promoted => DecisionReason::Promoted,
    }
}

pub const fn gesture_state(state: RecogniserState) -> GestureState {
    match state {
        RecogniserState::Idle => GestureState::Idle,
        RecogniserState::Scroll => GestureState::Scroll,
        RecogniserState::Swipe => GestureState::Swipe,
        RecogniserState::TapPending => GestureState::TapPending,
        RecogniserState::Cancelling => GestureState::Cancelling,
        RecogniserState::Suppressed => GestureState::Suppressed,
    }
}
