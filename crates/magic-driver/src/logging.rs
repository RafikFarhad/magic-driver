use std::env;
use std::io::{self, IsTerminal};
use std::os::fd::AsRawFd;

use md_core::config::LogLevel;
use tracing::Subscriber;
use tracing_journald::{Priority, PriorityMappings};
use tracing_subscriber::filter::LevelFilter;
use tracing_subscriber::fmt::MakeWriter;
use tracing_subscriber::layer::{Layer, SubscriberExt};
use tracing_subscriber::registry::LookupSpan;
use tracing_subscriber::reload;
use tracing_subscriber::util::SubscriberInitExt;
use tracing_subscriber::{Registry, fmt};

pub const JOURNAL_STREAM_VARIABLE: &str = "JOURNAL_STREAM";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LogTarget {
    Journal,
    Stderr,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StreamId {
    pub device: u64,
    pub inode: u64,
}

pub fn parse_journal_stream(value: &str) -> Option<StreamId> {
    let (device, inode) = value.split_once(':')?;
    Some(StreamId {
        device: device.parse().ok()?,
        inode: inode.parse().ok()?,
    })
}

pub fn target_for(journal_stream: Option<&str>, stderr: Option<StreamId>) -> LogTarget {
    match (journal_stream.and_then(parse_journal_stream), stderr) {
        (Some(journal), Some(stderr)) if journal == stderr => LogTarget::Journal,
        _ => LogTarget::Stderr,
    }
}

fn stderr_stream_id() -> Option<StreamId> {
    let stat = nix::sys::stat::fstat(io::stderr().as_raw_fd()).ok()?;
    Some(StreamId {
        device: stat.st_dev,
        inode: stat.st_ino,
    })
}

pub fn journal_priorities() -> PriorityMappings {
    PriorityMappings {
        error: Priority::Error,
        warn: Priority::Warning,
        info: Priority::Notice,
        debug: Priority::Informational,
        trace: Priority::Debug,
    }
}

fn stderr_layer<S, W>(writer: W, ansi: bool) -> impl Layer<S>
where
    S: Subscriber + for<'span> LookupSpan<'span>,
    W: for<'writer> MakeWriter<'writer> + 'static,
{
    fmt::layer()
        .with_writer(writer)
        .with_target(false)
        .with_ansi(ansi)
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
        let mut target = target_for(
            env::var(JOURNAL_STREAM_VARIABLE).ok().as_deref(),
            stderr_stream_id(),
        );
        let journald = if target == LogTarget::Journal {
            tracing_journald::layer().ok().map(|layer| {
                layer
                    .with_syslog_identifier("magic-driver".to_owned())
                    .with_priority_mappings(journal_priorities())
            })
        } else {
            None
        };
        if journald.is_none() {
            target = LogTarget::Stderr;
        }
        let stderr = (target == LogTarget::Stderr)
            .then(|| stderr_layer(io::stderr, io::stderr().is_terminal()));
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

    const STREAM: StreamId = StreamId {
        device: 8,
        inode: 12_345,
    };

    #[test]
    fn journal_stream_values_parse_as_device_and_inode() {
        assert_eq!(parse_journal_stream("8:12345"), Some(STREAM));
        assert_eq!(
            parse_journal_stream("18446744073709551615:1"),
            Some(StreamId {
                device: u64::MAX,
                inode: 1
            })
        );
        for invalid in [
            "",
            "8",
            "8:",
            ":12345",
            "8:12345:1",
            "a:b",
            "-8:12345",
            " 8:12345",
        ] {
            assert_eq!(parse_journal_stream(invalid), None, "{invalid:?}");
        }
    }

    #[test]
    fn the_journal_is_used_only_when_stderr_is_the_journal_stream() {
        assert_eq!(
            target_for(Some("8:12345"), Some(STREAM)),
            LogTarget::Journal
        );
        let terminal = StreamId {
            device: 24,
            inode: 7,
        };
        assert_eq!(
            target_for(Some("8:12345"), Some(terminal)),
            LogTarget::Stderr
        );
        assert_eq!(target_for(Some("8:12345"), None), LogTarget::Stderr);
        assert_eq!(target_for(Some("garbage"), Some(STREAM)), LogTarget::Stderr);
        assert_eq!(target_for(Some(""), Some(STREAM)), LogTarget::Stderr);
        assert_eq!(target_for(None, Some(STREAM)), LogTarget::Stderr);
    }

    #[test]
    fn only_warnings_and_errors_pass_journalctl_priority_warning() {
        let priorities = journal_priorities();
        let warning = Priority::Warning as u8;
        assert_eq!(priorities.error as u8, b'3');
        assert_eq!(priorities.warn as u8, b'4');
        assert!(priorities.info as u8 > warning);
        assert!(priorities.debug as u8 > warning);
        assert!(priorities.trace as u8 > warning);
    }

    #[derive(Clone, Default)]
    struct Buffer(std::sync::Arc<std::sync::Mutex<Vec<u8>>>);

    impl io::Write for Buffer {
        fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
            self.0.lock().expect("buffer lock").extend_from_slice(bytes);
            Ok(bytes.len())
        }

        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    impl<'writer> MakeWriter<'writer> for Buffer {
        type Writer = Buffer;

        fn make_writer(&'writer self) -> Buffer {
            self.clone()
        }
    }

    #[test]
    fn stderr_output_is_plain_text_with_the_level_when_not_a_terminal() {
        let buffer = Buffer::default();
        let subscriber = tracing_subscriber::registry()
            .with(LevelFilter::INFO)
            .with(stderr_layer(buffer.clone(), false));
        tracing::subscriber::with_default(subscriber, || {
            tracing::error!("lost the raw stream");
            tracing::warn!("recovered");
            tracing::info!("active");
        });
        let text = String::from_utf8(buffer.0.lock().expect("buffer lock").clone()).unwrap();
        let lines: Vec<&str> = text.lines().collect();
        assert_eq!(lines.len(), 3, "{text}");
        assert!(!text.contains('\u{1b}'), "{text:?}");
        assert!(
            lines[0].ends_with("ERROR lost the raw stream"),
            "{}",
            lines[0]
        );
        assert!(lines[1].ends_with(" WARN recovered"), "{}", lines[1]);
        assert!(lines[2].ends_with(" INFO active"), "{}", lines[2]);
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
