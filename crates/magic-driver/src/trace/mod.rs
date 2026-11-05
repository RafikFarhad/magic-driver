pub mod client;
pub mod server;

use md_core::config::Config;
use md_core::profile::Profile;
use md_trace::{Hello, PROTOCOL_VERSION, SessionRecord, TraceRecord};

use crate::session::SessionEvent;

pub fn hello(profile: &Profile, config: &Config, session: &str) -> TraceRecord {
    TraceRecord::Hello(Hello {
        protocol_version: PROTOCOL_VERSION,
        daemon_version: env!("CARGO_PKG_VERSION").to_owned(),
        profile: md_core::trace::profile_info(profile),
        config: serde_json::to_value(config).unwrap_or(serde_json::Value::Null),
        session: session.to_owned(),
    })
}

pub fn session_record(t: md_core::Micros, event: &SessionEvent<'_>) -> Option<TraceRecord> {
    let (state, detail) = match event {
        SessionEvent::Acquired {
            nodes,
            elapsed,
            attempts,
        } => (
            "active",
            format!(
                "{} on {}, taken over after {} ms and {attempts} attempts",
                nodes.profile.name,
                nodes.hidraw.display(),
                elapsed.as_millis()
            ),
        ),
        SessionEvent::AcquireFailed {
            nodes, step, error, ..
        } => (
            "acquiring",
            format!("cannot {step} for {}: {error}", nodes.hidraw.display()),
        ),
        SessionEvent::Released { nodes, reason } => {
            ("released", format!("{}: {reason}", nodes.hidraw.display()))
        }
        SessionEvent::Degraded { nodes, cause, .. } => {
            ("degraded", format!("{}: {cause}", nodes.hidraw.display()))
        }
        SessionEvent::Retrying { nodes, cause } => (
            "acquiring",
            format!("{}: retrying after {cause}", nodes.hidraw.display()),
        ),
        SessionEvent::RawLost {
            nodes,
            failure,
            retry_in,
            ..
        } => (
            "recovering",
            format!(
                "{}: {failure}, reopening the raw device in {} ms",
                nodes.hidraw.display(),
                retry_in.as_millis()
            ),
        ),
        SessionEvent::RawReopenFailed {
            nodes,
            error,
            retry_in,
            ..
        } => (
            "recovering",
            format!(
                "{}: cannot reopen the raw device: {error}, retrying in {} ms",
                nodes.hidraw.display(),
                retry_in.as_millis()
            ),
        ),
        SessionEvent::RawRecovered { nodes, elapsed, .. } => (
            "active",
            format!(
                "{}: raw reports flow again after {} ms",
                nodes.hidraw.display(),
                elapsed.as_millis()
            ),
        ),
        SessionEvent::Report { .. }
        | SessionEvent::StockEvents { .. }
        | SessionEvent::Trace { .. } => return None,
    };
    Some(TraceRecord::Session(SessionRecord {
        t: t.as_u64(),
        state: state.to_owned(),
        detail,
    }))
}
