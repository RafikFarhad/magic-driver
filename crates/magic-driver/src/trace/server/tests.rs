use std::io::{Read, Write};
use std::os::unix::fs::PermissionsExt;
use std::os::unix::net::UnixStream;
use std::time::{Duration, Instant};

use md_core::config::Config;
use md_core::profile::MAGIC_MOUSE_2_BT;
use md_core::testkit::synthetic_session;
use md_core::{Engine, Input, Mode, Output};
use md_trace::{DroppedRecord, SessionRecord, TraceRecord, decode, read_payload};
use tempfile::TempDir;

use super::*;
use crate::trace::hello;

const UID: u32 = 4_242;

fn hello_record() -> TraceRecord {
    hello(&MAGIC_MOUSE_2_BT, &Config::default(), "waiting")
}

fn session(t: u64) -> TraceRecord {
    TraceRecord::Session(SessionRecord {
        t,
        state: "active".to_owned(),
        detail: format!("record {t}"),
    })
}

fn read_all(client: &mut UnixStream) -> Vec<TraceRecord> {
    client
        .set_read_timeout(Some(Duration::from_millis(200)))
        .unwrap();
    let mut records = Vec::new();
    loop {
        match read_payload(client) {
            Ok(Some(payload)) => records.push(decode(&payload).unwrap()),
            Ok(None) | Err(_) => return records,
        }
    }
}

fn subscribed(server: &mut TraceServer) -> (u64, UnixStream) {
    let (ours, theirs) = UnixStream::pair().unwrap();
    match server.admit(ours, UID, &hello_record()) {
        Admission::Accepted(id) => (id, theirs),
        other => panic!("{other:?}"),
    }
}

#[test]
fn socket_path_is_under_the_runtime_dir() {
    assert_eq!(
        socket_path(Some(std::ffi::OsStr::new("/run/user/1000"))).unwrap(),
        PathBuf::from("/run/user/1000/magic-driver/trace.sock")
    );
    assert!(matches!(
        socket_path(None),
        Err(TraceSocketError::NoRuntimeDir)
    ));
    assert!(matches!(
        socket_path(Some(std::ffi::OsStr::new(""))),
        Err(TraceSocketError::NoRuntimeDir)
    ));
}

#[test]
fn hello_comes_first_then_records_in_order() {
    let mut server = TraceServer::without_listener(UID);
    let (_, mut client) = subscribed(&mut server);
    for t in 0..50 {
        server.publish(&session(t));
    }
    let records = read_all(&mut client);
    assert_eq!(records.len(), 51);
    let TraceRecord::Hello(hello) = &records[0] else {
        panic!("{:?}", records[0]);
    };
    assert_eq!(hello.protocol_version, md_trace::PROTOCOL_VERSION);
    assert_eq!(hello.profile.id, "magic-mouse-2-bt");
    assert_eq!(
        hello.config["advanced"]["gestures"]["kwin_swipe_fingers"],
        4
    );
    for (t, record) in (0..).zip(&records[1..]) {
        assert_eq!(*record, session(t));
    }
}

#[test]
fn a_stalled_client_gets_drops_not_backpressure() {
    let mut server = TraceServer::without_listener(UID);
    let (id, mut client) = subscribed(&mut server);
    let total = 20_000_u64;
    let started = Instant::now();
    for t in 0..total {
        assert!(server.publish(&session(t)).is_empty());
    }
    assert!(started.elapsed() < Duration::from_secs(5));
    let dropped = server.dropped_total(id).unwrap();
    assert!(dropped > 0);
    assert!(server.queued(id).unwrap() <= QUEUE_RECORDS);
    let mut records = Vec::new();
    client
        .set_read_timeout(Some(Duration::from_millis(100)))
        .unwrap();
    loop {
        match read_payload(&mut client) {
            Ok(Some(payload)) => records.push(decode(&payload).unwrap()),
            Ok(None) | Err(_) => {
                if server.queued(id).unwrap() == 0 {
                    break;
                }
            }
        }
        server.on_ready(id);
    }
    for t in total..total + 5 {
        assert!(server.publish(&session(t)).is_empty());
    }
    records.extend(read_all(&mut client));
    assert!(matches!(records[0], TraceRecord::Hello(_)));
    let reported: u64 = records
        .iter()
        .filter_map(|record| match record {
            TraceRecord::Dropped(DroppedRecord { count }) => Some(*count),
            _ => None,
        })
        .sum();
    let delivered = records
        .iter()
        .filter(|record| matches!(record, TraceRecord::Session(_)))
        .count();
    assert!(reported > 0);
    assert_eq!(reported + u64::try_from(delivered).unwrap(), total + 5);
    let mut last = None;
    for record in &records {
        if let TraceRecord::Session(session) = record {
            assert!(last.is_none_or(|last| session.t > last));
            last = Some(session.t);
        }
    }
    assert_eq!(last, Some(total + 4));
}

#[test]
fn wrong_uid_is_rejected_and_the_connection_closed() {
    let mut server = TraceServer::without_listener(UID);
    let (ours, mut theirs) = UnixStream::pair().unwrap();
    assert_eq!(
        server.admit(ours, UID + 1, &hello_record()),
        Admission::WrongUser { uid: UID + 1 }
    );
    assert!(!server.has_subscribers());
    let mut buffer = [0_u8; 8];
    assert_eq!(theirs.read(&mut buffer).unwrap(), 0);
}

#[test]
fn a_client_sending_data_is_disconnected() {
    let mut server = TraceServer::without_listener(UID);
    let (id, mut client) = subscribed(&mut server);
    client.write_all(b"reload").unwrap();
    assert_eq!(server.on_ready(id), Some(Closed::SentData));
    assert!(!server.has_subscribers());
    let records = read_all(&mut client);
    assert_eq!(records.len(), 1);
}

#[test]
fn a_client_hanging_up_is_removed() {
    let mut server = TraceServer::without_listener(UID);
    let (id, client) = subscribed(&mut server);
    assert_eq!(server.on_ready(id), None);
    drop(client);
    assert_eq!(server.on_ready(id), Some(Closed::HungUp));
    assert!(!server.has_subscribers());
    assert!(server.publish(&session(1)).is_empty());
}

#[test]
fn publishing_to_a_closed_client_removes_it() {
    let mut server = TraceServer::without_listener(UID);
    let (id, client) = subscribed(&mut server);
    drop(client);
    let closed = server.publish(&session(1));
    assert_eq!(closed, [(id, Closed::WriteFailed)]);
    assert!(!server.has_subscribers());
}

#[test]
fn subscribers_are_limited() {
    let mut server = TraceServer::without_listener(UID);
    let clients: Vec<_> = (0..MAX_SUBSCRIBERS)
        .map(|_| subscribed(&mut server))
        .collect();
    let (ours, _theirs) = UnixStream::pair().unwrap();
    assert_eq!(server.admit(ours, UID, &hello_record()), Admission::Full);
    assert_eq!(server.subscriber_count(), clients.len());
}

#[test]
fn bind_creates_a_private_directory_and_socket_and_accepts_the_same_user() {
    let temp = TempDir::new().unwrap();
    let path = temp.path().join("magic-driver").join("trace.sock");
    let uid = nix::unistd::getuid().as_raw();
    let mut server = TraceServer::bind(&path, uid).unwrap();
    let dir_mode = fs::metadata(path.parent().unwrap())
        .unwrap()
        .permissions()
        .mode();
    assert_eq!(dir_mode & 0o777, 0o700);
    let socket = fs::symlink_metadata(&path).unwrap();
    assert!(socket.file_type().is_socket());
    assert_eq!(socket.permissions().mode() & 0o777, 0o600);
    let mut client = UnixStream::connect(&path).unwrap();
    let admissions = server.accept_pending(&hello_record());
    assert!(matches!(admissions.as_slice(), [Admission::Accepted(_)]));
    server.publish(&session(7));
    let records = read_all(&mut client);
    assert!(matches!(records[0], TraceRecord::Hello(_)));
    assert_eq!(records[1], session(7));
    assert!(server.accept_pending(&hello_record()).is_empty());
    drop(server);
    assert!(!path.exists());
}

#[test]
fn bind_rejects_a_foreign_directory_and_a_running_daemon() {
    let temp = TempDir::new().unwrap();
    let path = temp.path().join("magic-driver").join("trace.sock");
    let uid = nix::unistd::getuid().as_raw();
    fs::create_dir(path.parent().unwrap()).unwrap();
    assert!(matches!(
        TraceServer::bind(&path, uid.wrapping_add(1)),
        Err(TraceSocketError::Unsafe { .. })
    ));
    let first = TraceServer::bind(&path, uid).unwrap();
    assert!(matches!(
        TraceServer::bind(&path, uid),
        Err(TraceSocketError::InUse { .. })
    ));
    drop(first);
    let file = temp.path().join("plain");
    fs::create_dir(&file).unwrap();
    fs::write(file.join("trace.sock"), b"not a socket").unwrap();
    assert!(matches!(
        TraceServer::bind(&file.join("trace.sock"), uid),
        Err(TraceSocketError::Unsafe { .. })
    ));
}

#[test]
fn bind_replaces_a_stale_socket_and_tightens_the_directory() {
    let temp = TempDir::new().unwrap();
    let dir = temp.path().join("magic-driver");
    fs::create_dir(&dir).unwrap();
    fs::set_permissions(&dir, Permissions::from_mode(0o755)).unwrap();
    let path = dir.join("trace.sock");
    drop(UnixListener::bind(&path).unwrap());
    assert!(path.exists());
    let uid = nix::unistd::getuid().as_raw();
    let _server = TraceServer::bind(&path, uid).unwrap();
    let mode = fs::metadata(&dir).unwrap().permissions().mode();
    assert_eq!(mode & 0o777, 0o700);
}

fn percentile(sorted: &[Duration], percent: usize) -> Duration {
    sorted[(sorted.len() * percent / 100).min(sorted.len() - 1)]
}

fn loop_latency(server: Option<&mut TraceServer>) -> (Duration, Duration) {
    let session = synthetic_session(10_000);
    let mut engine = Engine::new(&MAGIC_MOUSE_2_BT, Config::default(), Mode::Normal);
    engine.set_tracing(server.is_some());
    let mut server = server;
    let mut out: Vec<Output> = Vec::with_capacity(64);
    let mut durations = Vec::with_capacity(session.len());
    for (t, bytes) in &session {
        let started = Instant::now();
        engine.handle(Input::Report { t: *t, bytes }, &mut out);
        if let Some(server) = server.as_deref_mut() {
            for output in &out {
                if let Output::Trace(record) = output {
                    server.publish(record);
                }
            }
        }
        durations.push(started.elapsed());
        out.clear();
    }
    durations.sort_unstable();
    (percentile(&durations, 50), percentile(&durations, 99))
}

#[test]
fn loop_latency_stays_within_budget_with_a_stalled_subscriber() {
    let (_, baseline_p99) = loop_latency(None);
    let mut server = TraceServer::without_listener(UID);
    let (id, _stalled) = subscribed(&mut server);
    let (p50, p99) = loop_latency(Some(&mut server));
    println!(
        "tracing off p99 {baseline_p99:?}; stalled subscriber p50 {p50:?} p99 {p99:?}, dropped {}",
        server.dropped_total(id).unwrap()
    );
    assert!(server.dropped_total(id).unwrap() > 0);
    assert!(p99 < Duration::from_millis(1), "p99 {p99:?}");
}

#[test]
fn trace_client_prints_one_json_line_per_record_until_the_daemon_closes() {
    let temp = TempDir::new().unwrap();
    let path = temp.path().join("magic-driver").join("trace.sock");
    let uid = nix::unistd::getuid().as_raw();
    let mut server = TraceServer::bind(&path, uid).unwrap();
    let client_path = path.clone();
    let client = std::thread::spawn(move || {
        let mut out = Vec::new();
        let records = crate::trace::client::run(&client_path, &mut out).unwrap();
        (records, String::from_utf8(out).unwrap())
    });
    let deadline = Instant::now() + Duration::from_secs(5);
    while !server.has_subscribers() {
        assert!(Instant::now() < deadline, "client never connected");
        server.accept_pending(&hello_record());
        std::thread::sleep(Duration::from_millis(5));
    }
    server.publish(&session(1));
    server.publish(&session(2));
    drop(server);
    let (records, text) = client.join().unwrap();
    assert_eq!(records, 3);
    let lines: Vec<&str> = text.lines().collect();
    assert!(lines[0].starts_with(r#"{"type":"hello","protocol_version":1,"#));
    assert_eq!(
        lines[1],
        r#"{"type":"session","t":1,"state":"active","detail":"record 1"}"#
    );
    assert_eq!(lines.len(), 3);
}

#[test]
fn trace_client_requires_a_hello_with_the_same_protocol() {
    let mut wire = Vec::new();
    encode(&session(1), &mut wire).unwrap();
    let error = crate::trace::client::print_stream(wire.as_slice(), Vec::new()).unwrap_err();
    assert!(matches!(
        error,
        crate::trace::client::TraceClientError::NoHello
    ));
    let mut hello = hello_record();
    if let TraceRecord::Hello(hello) = &mut hello {
        hello.protocol_version = 99;
    }
    let mut wire = Vec::new();
    encode(&hello, &mut wire).unwrap();
    let error = crate::trace::client::print_stream(wire.as_slice(), Vec::new()).unwrap_err();
    assert!(matches!(
        error,
        crate::trace::client::TraceClientError::Protocol(99)
    ));
}

#[test]
fn trace_client_reports_a_missing_daemon() {
    let temp = TempDir::new().unwrap();
    let error = crate::trace::client::run(&temp.path().join("trace.sock"), Vec::new()).unwrap_err();
    assert!(error.to_string().contains("is magic-driver running?"));
}

#[test]
fn session_events_become_session_records() {
    let nodes = crate::discovery::MouseNodes {
        key: PathBuf::from("/sys/x"),
        hidraw: PathBuf::from("/dev/hidraw5"),
        evdev: PathBuf::from("/dev/input/event16"),
        profile: &MAGIC_MOUSE_2_BT,
    };
    let record = crate::trace::session_record(
        md_core::Micros::new(9),
        &crate::session::SessionEvent::Released {
            nodes: &nodes,
            reason: crate::session::ReleaseReason::Shutdown,
        },
    )
    .unwrap();
    assert_eq!(
        record.to_string(),
        "9 session released: /dev/hidraw5: shutdown"
    );
    let trace = session(1);
    assert!(
        crate::trace::session_record(
            md_core::Micros::ZERO,
            &crate::session::SessionEvent::Trace { record: &trace }
        )
        .is_none()
    );
}
