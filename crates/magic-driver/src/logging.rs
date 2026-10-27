use std::env;
use std::io;

use md_core::config::LogLevel;
use tracing_subscriber::filter::LevelFilter;
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::reload;
use tracing_subscriber::util::SubscriberInitExt;
use tracing_subscriber::{Registry, fmt};

pub const JOURNAL_STREAM_VARIABLE: &str = "JOURNAL_STREAM";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LogTarget {
    Journal,
    Stderr,
}

pub fn target_for(journal_stream: Option<&str>) -> LogTarget {
    match journal_stream {
        Some(value) if !value.is_empty() => LogTarget::Journal,
        _ => LogTarget::Stderr,
    }
}

pub fn level_filter(level: LogLevel, debug: bool) -> LevelFilter {
    if debug {
        return LevelFilter::DEBUG;
    }
    match level {
        LogLevel::Warn => LevelFilter::WARN,
        LogLevel::Info => LevelFilter::INFO,
        LogLevel::Debug => LevelFilter::DEBUG,
    }
}

pub struct Logging {
    handle: reload::Handle<LevelFilter, Registry>,
    debug: bool,
    target: LogTarget,
}

impl std::fmt::Debug for Logging {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Logging")
            .field("debug", &self.debug)
            .field("target", &self.target)
            .finish_non_exhaustive()
    }
}

impl Logging {
    pub fn init(level: LogLevel, debug: bool) -> Logging {
        let (filter, handle) = reload::Layer::new(level_filter(level, debug));
        let mut target = target_for(env::var(JOURNAL_STREAM_VARIABLE).ok().as_deref());
        let journald = if target == LogTarget::Journal {
            tracing_journald::layer()
                .ok()
                .map(|layer| layer.with_syslog_identifier("magic-driver".to_owned()))
        } else {
            None
        };
        if journald.is_none() {
            target = LogTarget::Stderr;
        }
        let stderr = (target == LogTarget::Stderr)
            .then(|| fmt::layer().with_writer(io::stderr).with_target(false));
        let _ = tracing_subscriber::registry()
            .with(filter)
            .with(journald)
            .with(stderr)
            .try_init();
        Logging {
            handle,
            debug,
            target,
        }
    }

    pub fn set_level(&self, level: LogLevel) {
        let _ = self.handle.reload(level_filter(level, self.debug));
    }
}

#[cfg(test)]
pub mod capture {
    use std::fmt::Debug;
    use std::sync::{Arc, Mutex};

    use tracing::field::{Field, Visit};
    use tracing::{Event, Level, Subscriber};
    use tracing_subscriber::filter::LevelFilter;
    use tracing_subscriber::layer::{Context, Layer, SubscriberExt};

    #[derive(Debug, Clone, PartialEq, Eq)]
    pub struct Record {
        pub level: Level,
        pub message: String,
    }

    #[derive(Debug, Clone, Default)]
    pub struct CaptureLayer {
        records: Arc<Mutex<Vec<Record>>>,
    }

    struct MessageVisitor<'a>(&'a mut String);

    impl Visit for MessageVisitor<'_> {
        fn record_debug(&mut self, field: &Field, value: &dyn Debug) {
            if field.name() == "message" {
                *self.0 = format!("{value:?}");
            }
        }
    }

    impl<S: Subscriber> Layer<S> for CaptureLayer {
        fn on_event(&self, event: &Event<'_>, _context: Context<'_, S>) {
            let mut message = String::new();
            event.record(&mut MessageVisitor(&mut message));
            self.records.lock().expect("capture lock").push(Record {
                level: *event.metadata().level(),
                message,
            });
        }
    }

    pub fn capture_at(level: LevelFilter, body: impl FnOnce()) -> Vec<Record> {
        let layer = CaptureLayer::default();
        let records = layer.records.clone();
        let subscriber = tracing_subscriber::registry().with(level).with(layer);
        tracing::subscriber::with_default(subscriber, body);
        records.lock().expect("capture lock").clone()
    }

    pub fn capture(body: impl FnOnce()) -> Vec<Record> {
        capture_at(LevelFilter::TRACE, body)
    }
}

#[cfg(test)]
mod tests {
    use tracing::Level;

    use super::capture::capture_at;
    use super::*;

    #[test]
    fn the_journal_is_used_when_systemd_connected_stderr_to_it() {
        assert_eq!(target_for(Some("8:12345")), LogTarget::Journal);
        assert_eq!(target_for(Some("")), LogTarget::Stderr);
        assert_eq!(target_for(None), LogTarget::Stderr);
    }

    #[test]
    fn the_config_level_maps_to_a_filter_and_debug_overrides_it() {
        assert_eq!(level_filter(LogLevel::Warn, false), LevelFilter::WARN);
        assert_eq!(level_filter(LogLevel::Info, false), LevelFilter::INFO);
        assert_eq!(level_filter(LogLevel::Debug, false), LevelFilter::DEBUG);
        assert_eq!(level_filter(LogLevel::Warn, true), LevelFilter::DEBUG);
    }

    #[test]
    fn capture_respects_the_level() {
        let records = capture_at(LevelFilter::INFO, || {
            tracing::debug!("hidden");
            tracing::info!("shown {}", 1);
            tracing::warn!("warned");
        });
        assert_eq!(records.len(), 2);
        assert_eq!(records[0].level, Level::INFO);
        assert_eq!(records[0].message, "shown 1");
    }
}
