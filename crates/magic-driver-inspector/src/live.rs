use std::ffi::OsStr;
use std::io;
use std::net::Shutdown;
use std::os::unix::net::UnixStream;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, SyncSender, sync_channel};
use std::sync::{Arc, Mutex, PoisonError};
use std::thread::JoinHandle;
use std::time::Duration;

use md_core::profile::Profile;
use md_trace::{Hello, PROTOCOL_VERSION, TraceRecord, decode, read_payload};

use crate::clock::{ClockAlignment, LocalClock};
use crate::timeline::{Timeline, record_time};
use crate::view::Thresholds;

pub const SOCKET_DIR: &str = "magic-driver";
pub const SOCKET_NAME: &str = "trace.sock";
pub const CHANNEL_CAPACITY: usize = 16_384;
pub const DEFAULT_RETRY: Duration = Duration::from_millis(500);
pub const BUFFER_WINDOW_US: u64 = 120_000_000;
pub const BUFFER_MAX_RECORDS: usize = 400_000;
const STOP_POLL: Duration = Duration::from_millis(20);

#[derive(Debug, Clone, PartialEq)]
pub enum LiveEvent {
    Connected(Box<Hello>),
    Record { record: TraceRecord, local_us: u64 },
    Disconnected(String),
    Unavailable(String),
}

#[derive(Debug, thiserror::Error)]
pub enum SessionError {
    #[error("cannot connect: {0}")]
    Connect(io::Error),
    #[error("{0}")]
    Wire(#[from] md_trace::WireError),
    #[error("the daemon did not start with a hello record")]
    NoHello,
    #[error("the daemon speaks trace protocol {0}, the inspector speaks {PROTOCOL_VERSION}")]
    Protocol(u32),
    #[error("the inspector stopped")]
    Stopped,
}

pub fn socket_path(runtime_dir: Option<&OsStr>) -> Option<PathBuf> {
    let dir = PathBuf::from(runtime_dir.filter(|dir| !dir.is_empty())?);
    dir.is_absolute()
        .then(|| dir.join(SOCKET_DIR).join(SOCKET_NAME))
}

pub struct LiveClient {
    stop: Arc<AtomicBool>,
    stream: Arc<Mutex<Option<UnixStream>>>,
    events: Receiver<LiveEvent>,
    thread: Option<JoinHandle<()>>,
}

impl LiveClient {
    pub fn spawn(path: PathBuf, retry: Duration, clock: LocalClock) -> io::Result<LiveClient> {
        LiveClient::spawn_with_waker(path, retry, clock, || {})
    }

    pub fn spawn_with_waker<F: Fn() + Send + 'static>(
        path: PathBuf,
        retry: Duration,
        clock: LocalClock,
        wake: F,
    ) -> io::Result<LiveClient> {
        let stop = Arc::new(AtomicBool::new(false));
        let stream = Arc::new(Mutex::new(None));
        let (sender, events) = sync_channel(CHANNEL_CAPACITY);
        let worker = Worker {
            path,
            retry,
            clock,
            stop: Arc::clone(&stop),
            stream: Arc::clone(&stream),
            sender,
            wake: Box::new(wake),
        };
        let thread = std::thread::Builder::new()
            .name("trace-client".to_owned())
            .spawn(move || worker.run())?;
        Ok(LiveClient {
            stop,
            stream,
            events,
            thread: Some(thread),
        })
    }

    pub fn try_next(&self) -> Option<LiveEvent> {
        self.events.try_recv().ok()
    }

    pub fn recv_timeout(&self, timeout: Duration) -> Option<LiveEvent> {
        self.events.recv_timeout(timeout).ok()
    }

    pub fn drain(&self, max: usize) -> Vec<LiveEvent> {
        std::iter::from_fn(|| self.try_next()).take(max).collect()
    }

    pub fn stop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        if let Some(stream) = self
            .stream
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .as_ref()
        {
            let _ = stream.shutdown(Shutdown::Both);
        }
        while self.events.try_recv().is_ok() {}
        if let Some(thread) = self.thread.take() {
            while !thread.is_finished() {
                while self.events.try_recv().is_ok() {}
                std::thread::sleep(Duration::from_millis(1));
            }
            let _ = thread.join();
        }
    }
}

impl Drop for LiveClient {
    fn drop(&mut self) {
        self.stop();
    }
}

struct Worker {
    path: PathBuf,
    retry: Duration,
    clock: LocalClock,
    stop: Arc<AtomicBool>,
    stream: Arc<Mutex<Option<UnixStream>>>,
    sender: SyncSender<LiveEvent>,
    wake: Box<dyn Fn() + Send>,
}

impl Worker {
    fn run(self) {
        let mut last_unavailable = None;
        while !self.stopped() {
            match self.session() {
                Ok(()) => {
                    last_unavailable = None;
                    if !self.send(LiveEvent::Disconnected(
                        "the daemon closed the trace".to_owned(),
                    )) {
                        return;
                    }
                }
                Err(SessionError::Stopped) => return,
                Err(SessionError::Connect(error)) => {
                    let message = format!("cannot connect to {}: {error}", self.path.display());
                    if last_unavailable.as_ref() != Some(&message) {
                        last_unavailable = Some(message.clone());
                        if !self.send(LiveEvent::Unavailable(message)) {
                            return;
                        }
                    }
                }
                Err(error) => {
                    last_unavailable = None;
                    if !self.send(LiveEvent::Disconnected(error.to_string())) {
                        return;
                    }
                }
            }
            self.pause();
        }
    }

    fn stopped(&self) -> bool {
        self.stop.load(Ordering::SeqCst)
    }

    fn pause(&self) {
        let mut waited = Duration::ZERO;
        while waited < self.retry && !self.stopped() {
            let step = STOP_POLL.min(self.retry.checked_sub(waited).unwrap());
            std::thread::sleep(step);
            waited += step;
        }
    }

    fn send(&self, event: LiveEvent) -> bool {
        let sent = self.sender.send(event).is_ok();
        (self.wake)();
        sent && !self.stopped()
    }

    fn session(&self) -> Result<(), SessionError> {
        let stream = UnixStream::connect(&self.path).map_err(SessionError::Connect)?;
        let mut reader = stream.try_clone().map_err(SessionError::Connect)?;
        *self.stream.lock().unwrap_or_else(PoisonError::into_inner) = Some(stream);
        let result = self.read_stream(&mut reader);
        *self.stream.lock().unwrap_or_else(PoisonError::into_inner) = None;
        if self.stopped() {
            return Err(SessionError::Stopped);
        }
        result
    }

    fn read_stream(&self, reader: &mut UnixStream) -> Result<(), SessionError> {
        if self.stopped() {
            return Err(SessionError::Stopped);
        }
        let Some(payload) = read_payload(reader)? else {
            return Err(SessionError::NoHello);
        };
        let hello = match decode(&payload)? {
            TraceRecord::Hello(hello) if hello.protocol_version == PROTOCOL_VERSION => hello,
            TraceRecord::Hello(hello) => {
                return Err(SessionError::Protocol(hello.protocol_version));
            }
            _ => return Err(SessionError::NoHello),
        };
        if !self.send(LiveEvent::Connected(Box::new(hello))) {
            return Err(SessionError::Stopped);
        }
        while let Some(payload) = read_payload(reader)? {
            let record = decode(&payload)?;
            let event = LiveEvent::Record {
                record,
                local_us: self.clock.now_us(),
            };
            if !self.send(event) {
                return Err(SessionError::Stopped);
            }
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConnectionStatus {
    Connecting,
    Connected,
    Disconnected(String),
    Unavailable(String),
}

#[derive(Debug, Clone)]
pub struct LiveSession {
    pub status: ConnectionStatus,
    pub hello: Option<Hello>,
    pub profile: Option<&'static Profile>,
    pub thresholds: Option<Thresholds>,
    pub timeline: Timeline,
    pub dropped_total: u64,
    pub dropped_this_connection: u64,
    pub connections: u32,
    pub alignment: ClockAlignment,
    window_us: u64,
    max_records: usize,
}

impl LiveSession {
    pub fn new() -> LiveSession {
        LiveSession::with_limits(BUFFER_WINDOW_US, BUFFER_MAX_RECORDS)
    }

    pub fn with_limits(window_us: u64, max_records: usize) -> LiveSession {
        LiveSession {
            status: ConnectionStatus::Connecting,
            hello: None,
            profile: None,
            thresholds: None,
            timeline: Timeline::new(),
            dropped_total: 0,
            dropped_this_connection: 0,
            connections: 0,
            alignment: ClockAlignment::new(),
            window_us,
            max_records: max_records.max(1),
        }
    }

    pub fn apply(&mut self, event: LiveEvent) -> bool {
        match event {
            LiveEvent::Connected(hello) => {
                self.connections += 1;
                self.status = ConnectionStatus::Connected;
                self.timeline = Timeline::new();
                self.alignment = ClockAlignment::new();
                self.dropped_this_connection = 0;
                self.profile = Profile::by_id(&hello.profile.id);
                self.thresholds = self.profile.map(|profile| {
                    Thresholds::from_config_value(
                        &hello.config,
                        Thresholds::from_config(&md_core::config::Config::for_profile(profile)),
                    )
                });
                self.hello = Some(*hello);
                true
            }
            LiveEvent::Record { record, local_us } => {
                if let TraceRecord::Dropped(dropped) = &record {
                    self.dropped_total += dropped.count;
                    self.dropped_this_connection += dropped.count;
                }
                if let Some(t) = record_time(&record) {
                    self.alignment.observe(t, local_us);
                }
                self.timeline.push(record);
                if let Some(end) = self.timeline.end_time() {
                    self.timeline
                        .evict_before(end.saturating_sub(self.window_us));
                }
                self.timeline.truncate_front(self.max_records);
                false
            }
            LiveEvent::Disconnected(reason) => {
                self.status = ConnectionStatus::Disconnected(reason);
                false
            }
            LiveEvent::Unavailable(reason) => {
                self.status = ConnectionStatus::Unavailable(reason);
                false
            }
        }
    }
}

impl Default for LiveSession {
    fn default() -> Self {
        LiveSession::new()
    }
}

#[cfg(test)]
mod tests;
