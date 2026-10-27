use std::collections::VecDeque;
use std::fs::{self, DirBuilder, Permissions};
use std::io::{self, ErrorKind, Read, Write};
use std::os::fd::{AsFd, BorrowedFd};
use std::os::unix::fs::{DirBuilderExt, FileTypeExt, MetadataExt, PermissionsExt};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};

use md_trace::{DroppedRecord, TraceRecord, encode};
use nix::sys::socket::{getsockopt, sockopt};

pub const TRACE_DIRECTORY: &str = "magic-driver";
pub const TRACE_SOCKET: &str = "trace.sock";
pub const QUEUE_RECORDS: usize = 1_024;
pub const MAX_SUBSCRIBERS: usize = 8;

const DIRECTORY_MODE: u32 = 0o700;
const SOCKET_MODE: u32 = 0o600;
const READ_BUFFER_LEN: usize = 64;

#[derive(Debug, thiserror::Error)]
pub enum TraceSocketError {
    #[error("XDG_RUNTIME_DIR is not set")]
    NoRuntimeDir,
    #[error("{path}: {reason}")]
    Unsafe { path: PathBuf, reason: &'static str },
    #[error("{path}: another magic-driver is already serving the trace")]
    InUse { path: PathBuf },
    #[error("{path}: {source}")]
    Io { path: PathBuf, source: io::Error },
}

pub fn socket_path(runtime_dir: Option<&std::ffi::OsStr>) -> Result<PathBuf, TraceSocketError> {
    let dir = runtime_dir
        .filter(|dir| !dir.is_empty())
        .ok_or(TraceSocketError::NoRuntimeDir)?;
    Ok(Path::new(dir).join(TRACE_DIRECTORY).join(TRACE_SOCKET))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Admission {
    Accepted(u64),
    WrongUser { uid: u32 },
    Full,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Closed {
    HungUp,
    SentData,
    WriteFailed,
}

#[derive(Debug)]
struct Subscriber {
    id: u64,
    stream: UnixStream,
    queue: VecDeque<Vec<u8>>,
    written: usize,
    dropped: u64,
    dropped_total: u64,
}

impl Subscriber {
    fn new(id: u64, stream: UnixStream) -> Subscriber {
        Subscriber {
            id,
            stream,
            queue: VecDeque::with_capacity(QUEUE_RECORDS),
            written: 0,
            dropped: 0,
            dropped_total: 0,
        }
    }

    fn note_drops(&mut self) {
        if self.dropped == 0 || self.queue.len() + 1 >= QUEUE_RECORDS {
            return;
        }
        let mut notice = Vec::new();
        let record = TraceRecord::Dropped(DroppedRecord {
            count: self.dropped,
        });
        if encode(&record, &mut notice).is_ok() {
            self.queue.push_back(notice);
            self.dropped = 0;
        }
    }

    fn enqueue(&mut self, frame: &[u8]) {
        self.note_drops();
        if self.queue.len() < QUEUE_RECORDS {
            self.queue.push_back(frame.to_vec());
        } else {
            self.dropped += 1;
            self.dropped_total += 1;
        }
    }

    fn flush(&mut self) -> Result<(), Closed> {
        while let Some(front) = self.queue.front() {
            match self.stream.write(&front[self.written..]) {
                Ok(0) => return Err(Closed::WriteFailed),
                Ok(written) => {
                    self.written += written;
                    if self.written == front.len() {
                        self.queue.pop_front();
                        self.written = 0;
                    }
                }
                Err(error) if error.kind() == ErrorKind::WouldBlock => return Ok(()),
                Err(error) if error.kind() == ErrorKind::Interrupted => {}
                Err(_) => return Err(Closed::WriteFailed),
            }
        }
        Ok(())
    }

    fn check_input(&mut self) -> Result<(), Closed> {
        let mut buffer = [0_u8; READ_BUFFER_LEN];
        loop {
            match self.stream.read(&mut buffer) {
                Ok(0) => return Err(Closed::HungUp),
                Ok(_) => return Err(Closed::SentData),
                Err(error) if error.kind() == ErrorKind::WouldBlock => return Ok(()),
                Err(error) if error.kind() == ErrorKind::Interrupted => {}
                Err(_) => return Err(Closed::HungUp),
            }
        }
    }
}

#[derive(Debug)]
pub struct TraceServer {
    listener: Option<UnixListener>,
    path: Option<PathBuf>,
    uid: u32,
    subscribers: Vec<Subscriber>,
    next_id: u64,
    frame: Vec<u8>,
}

impl TraceServer {
    pub fn without_listener(uid: u32) -> TraceServer {
        TraceServer {
            listener: None,
            path: None,
            uid,
            subscribers: Vec::new(),
            next_id: 1,
            frame: Vec::with_capacity(1_024),
        }
    }

    pub fn bind(path: &Path, uid: u32) -> Result<TraceServer, TraceSocketError> {
        let dir = path.parent().ok_or_else(|| TraceSocketError::Unsafe {
            path: path.to_owned(),
            reason: "the socket path has no directory",
        })?;
        prepare_directory(dir, uid)?;
        remove_stale_socket(path)?;
        let io_error = |source| TraceSocketError::Io {
            path: path.to_owned(),
            source,
        };
        let listener = UnixListener::bind(path).map_err(io_error)?;
        fs::set_permissions(path, Permissions::from_mode(SOCKET_MODE)).map_err(io_error)?;
        listener.set_nonblocking(true).map_err(io_error)?;
        let mut server = TraceServer::without_listener(uid);
        server.listener = Some(listener);
        server.path = Some(path.to_owned());
        Ok(server)
    }

    pub fn listener_fd(&self) -> Option<BorrowedFd<'_>> {
        self.listener.as_ref().map(AsFd::as_fd)
    }

    pub fn has_subscribers(&self) -> bool {
        !self.subscribers.is_empty()
    }

    pub fn subscriber_count(&self) -> usize {
        self.subscribers.len()
    }

    pub fn subscriber_fds(&self) -> impl Iterator<Item = (u64, BorrowedFd<'_>, bool)> {
        self.subscribers.iter().map(|subscriber| {
            (
                subscriber.id,
                subscriber.stream.as_fd(),
                !subscriber.queue.is_empty(),
            )
        })
    }

    #[cfg(test)]
    pub fn dropped_total(&self, id: u64) -> Option<u64> {
        self.subscribers
            .iter()
            .find(|subscriber| subscriber.id == id)
            .map(|subscriber| subscriber.dropped_total)
    }

    #[cfg(test)]
    pub fn queued(&self, id: u64) -> Option<usize> {
        self.subscribers
            .iter()
            .find(|subscriber| subscriber.id == id)
            .map(|subscriber| subscriber.queue.len())
    }

    pub fn accept_pending(&mut self, hello: &TraceRecord) -> Vec<Admission> {
        let mut admissions = Vec::new();
        let Some(listener) = self.listener.as_ref() else {
            return admissions;
        };
        let mut accepted = Vec::new();
        loop {
            match listener.accept() {
                Ok((stream, _)) => accepted.push(stream),
                Err(error) if error.kind() == ErrorKind::Interrupted => {}
                Err(_) => break,
            }
        }
        for stream in accepted {
            let uid = match getsockopt(&stream, sockopt::PeerCredentials) {
                Ok(credentials) => credentials.uid(),
                Err(_) => u32::MAX,
            };
            admissions.push(self.admit(stream, uid, hello));
        }
        admissions
    }

    pub fn admit(&mut self, stream: UnixStream, peer_uid: u32, hello: &TraceRecord) -> Admission {
        if peer_uid != self.uid {
            return Admission::WrongUser { uid: peer_uid };
        }
        if self.subscribers.len() >= MAX_SUBSCRIBERS || stream.set_nonblocking(true).is_err() {
            return Admission::Full;
        }
        let id = self.next_id;
        self.next_id += 1;
        let mut subscriber = Subscriber::new(id, stream);
        self.frame.clear();
        if encode(hello, &mut self.frame).is_ok() {
            subscriber.enqueue(&self.frame);
        }
        let open = subscriber.flush().is_ok();
        if open {
            self.subscribers.push(subscriber);
        }
        Admission::Accepted(id)
    }

    pub fn publish(&mut self, record: &TraceRecord) -> Vec<(u64, Closed)> {
        let mut closed = Vec::new();
        if self.subscribers.is_empty() {
            return closed;
        }
        self.frame.clear();
        if encode(record, &mut self.frame).is_err() {
            return closed;
        }
        for subscriber in &mut self.subscribers {
            subscriber.enqueue(&self.frame);
            if let Err(reason) = subscriber.flush() {
                closed.push((subscriber.id, reason));
            }
        }
        self.remove(&closed);
        closed
    }

    pub fn on_ready(&mut self, id: u64) -> Option<Closed> {
        let subscriber = self
            .subscribers
            .iter_mut()
            .find(|subscriber| subscriber.id == id)?;
        let result = subscriber.check_input().and_then(|()| subscriber.flush());
        if result.is_ok() {
            subscriber.note_drops();
            let _ = subscriber.flush();
        }
        let reason = result.err()?;
        self.remove(&[(id, reason)]);
        Some(reason)
    }

    fn remove(&mut self, closed: &[(u64, Closed)]) {
        self.subscribers
            .retain(|subscriber| closed.iter().all(|(id, _)| *id != subscriber.id));
    }
}

impl Drop for TraceServer {
    fn drop(&mut self) {
        if let Some(path) = &self.path
            && fs::symlink_metadata(path).is_ok_and(|metadata| metadata.file_type().is_socket())
        {
            let _ = fs::remove_file(path);
        }
    }
}

fn prepare_directory(dir: &Path, uid: u32) -> Result<(), TraceSocketError> {
    let io_error = |source| TraceSocketError::Io {
        path: dir.to_owned(),
        source,
    };
    match fs::symlink_metadata(dir) {
        Ok(metadata) => {
            if !metadata.is_dir() {
                return Err(TraceSocketError::Unsafe {
                    path: dir.to_owned(),
                    reason: "exists and is not a directory",
                });
            }
            if metadata.uid() != uid {
                return Err(TraceSocketError::Unsafe {
                    path: dir.to_owned(),
                    reason: "is owned by another user",
                });
            }
        }
        Err(error) if error.kind() == ErrorKind::NotFound => {
            DirBuilder::new()
                .mode(DIRECTORY_MODE)
                .create(dir)
                .map_err(io_error)?;
        }
        Err(error) => return Err(io_error(error)),
    }
    fs::set_permissions(dir, Permissions::from_mode(DIRECTORY_MODE)).map_err(io_error)
}

fn remove_stale_socket(path: &Path) -> Result<(), TraceSocketError> {
    let metadata = match fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == ErrorKind::NotFound => return Ok(()),
        Err(source) => {
            return Err(TraceSocketError::Io {
                path: path.to_owned(),
                source,
            });
        }
    };
    if !metadata.file_type().is_socket() {
        return Err(TraceSocketError::Unsafe {
            path: path.to_owned(),
            reason: "exists and is not a socket",
        });
    }
    if UnixStream::connect(path).is_ok() {
        return Err(TraceSocketError::InUse {
            path: path.to_owned(),
        });
    }
    fs::remove_file(path).map_err(|source| TraceSocketError::Io {
        path: path.to_owned(),
        source,
    })
}

#[cfg(test)]
mod tests;
