use std::io::{Read, Write};
use std::os::unix::net::UnixListener;
use std::path::Path;
use std::time::Instant;

use md_core::config::Config;
use md_core::profile::MAGIC_MOUSE_2_BT;
use md_trace::{DroppedRecord, GestureState, RecogniserRecord, ReportRecord, encode};

use super::*;

const WAIT: Duration = Duration::from_secs(5);
const RETRY: Duration = Duration::from_millis(20);

fn hello_with(protocol_version: u32, config: &Config) -> TraceRecord {
    TraceRecord::Hello(Hello {
        protocol_version,
        daemon_version: "0.1.0".to_owned(),
        profile: md_core::trace::profile_info(&MAGIC_MOUSE_2_BT),
        config: serde_json::to_value(config).unwrap(),
        session: "active: Magic Mouse 2 (hidraw5)".to_owned(),
    })
}

fn hello() -> TraceRecord {
    hello_with(PROTOCOL_VERSION, &Config::for_profile(&MAGIC_MOUSE_2_BT))
}

fn report(t: u64) -> TraceRecord {
    TraceRecord::Report(ReportRecord {
        t,
        clicks: 0,
        dx: 1,
        dy: -1,
        touches: Vec::new(),
        error: None,
        processing_ns: Some(2_000),
    })
}

fn send(stream: &mut UnixStream, record: &TraceRecord) {
    let mut buffer = Vec::new();
    encode(record, &mut buffer).unwrap();
    stream.write_all(&buffer).unwrap();
}

fn accept(listener: &UnixListener) -> UnixStream {
    listener.set_nonblocking(false).unwrap();
    let (stream, _) = listener.accept().unwrap();
    stream
}

fn next(client: &LiveClient) -> LiveEvent {
    client.recv_timeout(WAIT).expect("an event in time")
}

fn records(client: &LiveClient, count: usize) -> Vec<TraceRecord> {
    (0..count)
        .map(|_| match next(client) {
            LiveEvent::Record { record, .. } => record,
            other => panic!("expected a record, got {other:?}"),
        })
        .collect()
}

fn spawn(path: &Path) -> LiveClient {
    LiveClient::spawn(path.to_path_buf(), RETRY, LocalClock::new()).unwrap()
}

#[test]
fn socket_path_is_inside_the_runtime_dir() {
    assert_eq!(
        socket_path(Some(OsStr::new("/run/user/1000"))),
        Some(PathBuf::from("/run/user/1000/magic-driver/trace.sock"))
    );
    assert_eq!(socket_path(Some(OsStr::new(""))), None);
    assert_eq!(socket_path(Some(OsStr::new("run"))), None);
    assert_eq!(socket_path(None), None);
}

#[test]
fn the_client_reads_the_hello_then_every_record_in_order() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("trace.sock");
    let listener = UnixListener::bind(&path).unwrap();
    let client = spawn(&path);
    let mut server = accept(&listener);
    send(&mut server, &hello());
    for t in 0..50 {
        send(&mut server, &report(t * 11_111));
    }
    send(
        &mut server,
        &TraceRecord::Dropped(DroppedRecord { count: 7 }),
    );
    let LiveEvent::Connected(hello_record) = next(&client) else {
        panic!("expected the hello first");
    };
    assert_eq!(hello_record.protocol_version, PROTOCOL_VERSION);
    let received = records(&client, 51);
    for (index, record) in received.iter().take(50).enumerate() {
        assert_eq!(record, &report(index as u64 * 11_111));
    }
    assert_eq!(
        received[50],
        TraceRecord::Dropped(DroppedRecord { count: 7 })
    );
    drop(server);
    assert!(matches!(next(&client), LiveEvent::Disconnected(_)));
}

#[test]
fn the_client_never_writes_to_the_daemon() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("trace.sock");
    let listener = UnixListener::bind(&path).unwrap();
    let mut client = spawn(&path);
    let mut server = accept(&listener);
    send(&mut server, &hello());
    send(&mut server, &report(1));
    assert!(matches!(next(&client), LiveEvent::Connected(_)));
    assert_eq!(records(&client, 1), [report(1)]);
    client.stop();
    server.set_read_timeout(Some(WAIT)).unwrap();
    let mut buffer = Vec::new();
    let read = server.read_to_end(&mut buffer).unwrap();
    assert_eq!(read, 0);
    assert!(buffer.is_empty());
}

#[test]
fn the_client_reconnects_when_the_daemon_restarts() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("trace.sock");
    let listener = UnixListener::bind(&path).unwrap();
    let client = spawn(&path);
    let mut session = LiveSession::new();
    let mut server = accept(&listener);
    send(&mut server, &hello());
    send(&mut server, &report(100));
    send(
        &mut server,
        &TraceRecord::Dropped(DroppedRecord { count: 2 }),
    );
    for _ in 0..3 {
        session.apply(next(&client));
    }
    assert_eq!(session.connections, 1);
    assert_eq!(session.timeline.report_count(), 1);
    assert_eq!(session.dropped_total, 2);

    std::fs::remove_file(&path).unwrap();
    drop(listener);
    drop(server);
    assert!(!session.apply(next(&client)));
    assert!(matches!(session.status, ConnectionStatus::Disconnected(_)));
    let unavailable = next(&client);
    assert!(
        matches!(unavailable, LiveEvent::Unavailable(_)),
        "{unavailable:?}"
    );
    session.apply(unavailable);
    assert!(matches!(session.status, ConnectionStatus::Unavailable(_)));
    std::thread::sleep(RETRY * 5);
    assert!(
        client.try_next().is_none(),
        "the same failure is reported once"
    );

    let listener = UnixListener::bind(&path).unwrap();
    let mut server = accept(&listener);
    send(&mut server, &hello());
    send(&mut server, &report(5));
    assert!(session.apply(next(&client)));
    session.apply(next(&client));
    assert_eq!(session.connections, 2);
    assert_eq!(session.status, ConnectionStatus::Connected);
    assert_eq!(session.timeline.report_count(), 1);
    assert_eq!(session.timeline.start_time(), Some(5));
    assert_eq!(session.dropped_total, 2);
    assert_eq!(session.dropped_this_connection, 0);
}

#[test]
fn a_missing_daemon_is_reported_and_picked_up_when_it_starts() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("trace.sock");
    let client = spawn(&path);
    match next(&client) {
        LiveEvent::Unavailable(reason) => assert!(reason.contains("trace.sock"), "{reason}"),
        other => panic!("{other:?}"),
    }
    let listener = UnixListener::bind(&path).unwrap();
    let mut server = accept(&listener);
    send(&mut server, &hello());
    assert!(matches!(next(&client), LiveEvent::Connected(_)));
}

#[test]
fn a_wrong_protocol_or_a_missing_hello_disconnects_and_retries() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("trace.sock");
    let listener = UnixListener::bind(&path).unwrap();
    let client = spawn(&path);

    let mut server = accept(&listener);
    send(
        &mut server,
        &hello_with(
            PROTOCOL_VERSION + 1,
            &Config::for_profile(&MAGIC_MOUSE_2_BT),
        ),
    );
    match next(&client) {
        LiveEvent::Disconnected(reason) => assert!(reason.contains("protocol 2"), "{reason}"),
        other => panic!("{other:?}"),
    }
    drop(server);

    let mut server = accept(&listener);
    send(&mut server, &report(1));
    match next(&client) {
        LiveEvent::Disconnected(reason) => assert!(reason.contains("hello"), "{reason}"),
        other => panic!("{other:?}"),
    }
    drop(server);

    let server = accept(&listener);
    drop(server);
    match next(&client) {
        LiveEvent::Disconnected(reason) => assert!(reason.contains("hello"), "{reason}"),
        other => panic!("{other:?}"),
    }

    let mut server = accept(&listener);
    server.write_all(&[0, 0, 0, 3, b'{', b'x', b'}']).unwrap();
    assert!(matches!(next(&client), LiveEvent::Disconnected(_)));
    drop(server);

    let mut server = accept(&listener);
    send(&mut server, &hello());
    assert!(matches!(next(&client), LiveEvent::Connected(_)));
}

#[test]
fn stopping_the_client_ends_its_thread_while_connected_and_while_retrying() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("trace.sock");
    let listener = UnixListener::bind(&path).unwrap();
    let mut client = spawn(&path);
    let mut server = accept(&listener);
    send(&mut server, &hello());
    assert!(matches!(next(&client), LiveEvent::Connected(_)));
    let started = Instant::now();
    client.stop();
    assert!(started.elapsed() < WAIT);

    let mut retrying = LiveClient::spawn(
        dir.path().join("missing.sock"),
        Duration::from_secs(60),
        LocalClock::new(),
    )
    .unwrap();
    assert!(matches!(next(&retrying), LiveEvent::Unavailable(_)));
    let started = Instant::now();
    retrying.stop();
    assert!(started.elapsed() < WAIT);
}

#[test]
fn a_full_channel_does_not_stop_the_client_from_stopping() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("trace.sock");
    let listener = UnixListener::bind(&path).unwrap();
    let mut client = spawn(&path);
    let mut server = accept(&listener);
    send(&mut server, &hello());
    let writer = std::thread::spawn(move || {
        for t in 0..(CHANNEL_CAPACITY as u64 + 2_000) {
            let mut buffer = Vec::new();
            encode(&report(t), &mut buffer).unwrap();
            if server.write_all(&buffer).is_err() {
                return;
            }
        }
    });
    std::thread::sleep(Duration::from_millis(200));
    let started = Instant::now();
    client.stop();
    assert!(started.elapsed() < WAIT);
    writer.join().unwrap();
}

#[test]
fn the_waker_is_called_for_every_event() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("trace.sock");
    let listener = UnixListener::bind(&path).unwrap();
    let wakes = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let counter = Arc::clone(&wakes);
    let client = LiveClient::spawn_with_waker(path, RETRY, LocalClock::new(), move || {
        counter.fetch_add(1, Ordering::SeqCst);
    })
    .unwrap();
    let mut server = accept(&listener);
    send(&mut server, &hello());
    send(&mut server, &report(1));
    assert!(matches!(next(&client), LiveEvent::Connected(_)));
    records(&client, 1);
    assert!(wakes.load(Ordering::SeqCst) >= 2);
    assert!(client.drain(10).is_empty());
}

#[test]
fn the_hello_profile_and_config_drive_the_surface_and_thresholds() {
    let mut config = Config::for_profile(&MAGIC_MOUSE_2_BT);
    config.advanced.touch.rear_zone = 0.2;
    config.advanced.touch.palm_size_mm = 12.5;
    let TraceRecord::Hello(hello_record) = hello_with(PROTOCOL_VERSION, &config) else {
        unreachable!();
    };
    let mut session = LiveSession::new();
    assert_eq!(session.status, ConnectionStatus::Connecting);
    assert!(session.apply(LiveEvent::Connected(Box::new(hello_record.clone()))));
    assert_eq!(
        session.profile.map(|profile| profile.id),
        Some(MAGIC_MOUSE_2_BT.id)
    );
    assert_eq!(session.thresholds, Some(Thresholds::from_config(&config)));
    assert_eq!(session.hello.as_ref(), Some(&hello_record));

    let mut unknown = hello_record;
    unknown.profile.id = "trackpad-9".to_owned();
    let mut session = LiveSession::new();
    session.apply(LiveEvent::Connected(Box::new(unknown)));
    assert!(session.profile.is_none());
    assert!(session.thresholds.is_none());
}

#[test]
fn the_rolling_buffer_keeps_a_time_window_and_a_record_limit() {
    let mut session = LiveSession::with_limits(1_000_000, 1_000);
    let TraceRecord::Hello(hello_record) = hello() else {
        unreachable!();
    };
    session.apply(LiveEvent::Connected(Box::new(hello_record)));
    for index in 0..500_u64 {
        session.apply(LiveEvent::Record {
            record: report(index * 10_000),
            local_us: 1_000_000 + index * 10_000,
        });
    }
    let start = session.timeline.start_time().unwrap();
    let end = session.timeline.end_time().unwrap();
    assert_eq!(end, 4_990_000);
    assert_eq!(start, 3_990_000);
    assert!(session.timeline.first_seq() > 0);
    assert_eq!(session.alignment.to_local(end), Some(5_990_000));

    let mut capped = LiveSession::with_limits(u64::MAX, 10);
    for index in 0..100_u64 {
        capped.apply(LiveEvent::Record {
            record: TraceRecord::Recogniser(RecogniserRecord {
                t: index,
                from: GestureState::Idle,
                to: GestureState::Scroll,
            }),
            local_us: index,
        });
    }
    assert_eq!(capped.timeline.len(), 10);
    assert_eq!(capped.timeline.first_seq(), 90);
    assert!(LiveSession::default().timeline.is_empty());
}
