use std::cell::RefCell;
use std::fs;
use std::path::PathBuf;
use std::rc::Rc;
use std::thread::{self, ThreadId};
use std::time::Duration;

use md_core::config::Config;
use md_core::profile::MAGIC_MOUSE_2_BT;
use md_core::recording::{RecordingHeader, RecordingReader, RecordingWriter};
use md_core::testkit::{ReportBuilder, synthetic_session};
use md_core::{Micros, Mode};
use nix::errno::Errno;
use tracing::Level;
use tracing_subscriber::filter::LevelFilter;

use super::debug::LogObserver;
use super::*;
use crate::daemon_config::ConfigSource;
use crate::discovery::{DiscoveryState, MouseNode, MouseNodes, NodeKind};
use crate::io::EvdevEvent;
use crate::io::fake::{Call, FakeDirectory, FakeOpener, SharedWorld, world};
use crate::logging::capture::{capture, capture_at};
use crate::service::{ServiceNotifier, Watchdog};
use crate::session::{NoObserver, SessionState};

const KEY: &str = "/sys/devices/virtual/misc/uhid/0005:004C:0269.0005";

fn nodes() -> MouseNodes {
    MouseNodes {
        key: PathBuf::from(KEY),
        hidraw: PathBuf::from("/dev/hidraw5"),
        evdev: PathBuf::from("/dev/input/event15"),
        profile: &MAGIC_MOUSE_2_BT,
    }
}

fn node(kind: NodeKind) -> MouseNode {
    let (syspath, devnode) = match kind {
        NodeKind::Hidraw => (format!("{KEY}/hidraw/hidraw5"), "/dev/hidraw5"),
        NodeKind::Evdev => (format!("{KEY}/input/input21/event15"), "/dev/input/event15"),
    };
    MouseNode {
        key: PathBuf::from(KEY),
        kind,
        syspath: PathBuf::from(syspath),
        devnode: PathBuf::from(devnode),
        profile: &MAGIC_MOUSE_2_BT,
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum Notice {
    Ready,
    Watchdog(ThreadId),
    Status(String),
    Stopping,
}

#[derive(Debug, Clone, Default)]
struct RecordingNotifier {
    notices: Rc<RefCell<Vec<Notice>>>,
}

impl ServiceNotifier for RecordingNotifier {
    fn ready(&mut self) {
        self.notices.borrow_mut().push(Notice::Ready);
    }

    fn watchdog(&mut self) {
        self.notices
            .borrow_mut()
            .push(Notice::Watchdog(thread::current().id()));
    }

    fn status(&mut self, line: &str) {
        self.notices
            .borrow_mut()
            .push(Notice::Status(line.to_owned()));
    }

    fn stopping(&mut self) {
        self.notices.borrow_mut().push(Notice::Stopping);
    }
}

struct Harness {
    world: SharedWorld,
    now: Micros,
    notices: Rc<RefCell<Vec<Notice>>>,
    runtime: Runtime<FakeDirectory, FakeOpener>,
    discovery: DiscoveryState,
}

impl Harness {
    fn new(watchdog: Option<Duration>) -> Harness {
        Harness::with_config(watchdog, Config::default())
    }

    fn with_config(watchdog: Option<Duration>, config: Config) -> Harness {
        let world = world();
        let notifier = RecordingNotifier::default();
        let notices = notifier.notices.clone();
        let runtime = Runtime::new(
            FakeDirectory {
                world: world.clone(),
            },
            FakeOpener {
                world: world.clone(),
            },
            Mode::RawForward,
            config,
        )
        .with_service(Box::new(notifier), watchdog.and_then(Watchdog::new));
        Harness {
            world,
            now: Micros::ZERO,
            notices,
            runtime,
            discovery: DiscoveryState::default(),
        }
    }

    fn at(&mut self, ms: u64) -> &mut Harness {
        self.now = Micros::from_millis(ms);
        self
    }

    fn start(&mut self) {
        self.runtime.start(self.now, &mut NoObserver).unwrap();
    }

    fn dispatch(&mut self, ready: Ready) {
        self.runtime
            .dispatch(self.now, ready, &mut NoObserver)
            .unwrap();
        self.runtime.run_due(self.now, &mut NoObserver);
    }

    fn loop_until(&mut self, end_ms: u64) {
        while let Some(deadline) = self.runtime.next_deadline() {
            if deadline > Micros::from_millis(end_ms) {
                break;
            }
            self.now = deadline.max(self.now);
            self.dispatch(Ready::Timer);
        }
        self.now = Micros::from_millis(end_ms);
    }

    fn node_added(&mut self, kind: NodeKind) {
        if let Some(event) = self.discovery.node_added(node(kind)) {
            self.world.borrow_mut().udev_changes.push_back(event);
        }
        self.dispatch(Ready::Directory);
    }

    fn node_removed(&mut self, kind: NodeKind) {
        if let Some(event) = self.discovery.node_removed(&node(kind).syspath) {
            self.world.borrow_mut().udev_changes.push_back(event);
        }
        self.dispatch(Ready::Directory);
    }

    fn state(&self) -> Option<SessionState> {
        self.runtime
            .manager()
            .session()
            .map(crate::session::Session::state)
    }

    fn statuses(&self) -> Vec<String> {
        self.notices
            .borrow()
            .iter()
            .filter_map(|notice| match notice {
                Notice::Status(line) => Some(line.clone()),
                _ => None,
            })
            .collect()
    }

    fn pets(&self) -> Vec<ThreadId> {
        self.notices
            .borrow()
            .iter()
            .filter_map(|notice| match notice {
                Notice::Watchdog(thread) => Some(*thread),
                _ => None,
            })
            .collect()
    }

    fn take_calls(&self) -> Vec<Call> {
        std::mem::take(&mut self.world.borrow_mut().calls)
    }
}

#[test]
fn ready_is_sent_once_after_initial_enumeration() {
    let mut harness = Harness::new(None);
    harness
        .world
        .borrow_mut()
        .enumerated
        .push(DiscoveryEvent::MouseAppeared(nodes()));
    assert!(harness.notices.borrow().is_empty());
    harness.start();
    let notices = harness.notices.borrow().clone();
    assert_eq!(notices[0], Notice::Ready);
    assert_eq!(
        notices[1],
        Notice::Status("active: Apple Magic Mouse 2 (Bluetooth) on hidraw5".to_owned())
    );
    assert_eq!(
        notices
            .iter()
            .filter(|notice| **notice == Notice::Ready)
            .count(),
        1
    );
}

#[test]
fn the_watchdog_is_petted_on_the_loop_path_on_schedule() {
    let mut harness = Harness::new(Some(Duration::from_secs(2)));
    harness.start();
    assert_eq!(harness.pets().len(), 1);
    assert_eq!(
        harness.runtime.next_deadline(),
        Some(Micros::from_millis(1_000))
    );
    harness.loop_until(10_000);
    let pets = harness.pets();
    assert_eq!(pets.len(), 11);
    let loop_thread = thread::current().id();
    assert!(pets.iter().all(|thread| *thread == loop_thread));
}

#[test]
fn a_busy_loop_still_pets_without_waiting_for_the_timer() {
    let mut harness = Harness::new(Some(Duration::from_secs(2)));
    harness
        .world
        .borrow_mut()
        .enumerated
        .push(DiscoveryEvent::MouseAppeared(nodes()));
    harness.start();
    for ms in (0..=3_000).step_by(10) {
        harness
            .world
            .borrow_mut()
            .raw_reports
            .push_back(Ok(ReportBuilder::new().motion(1, 0).build()));
        harness.at(ms).dispatch(Ready::Raw);
    }
    assert_eq!(harness.pets().len(), 4);
}

#[test]
fn no_watchdog_means_no_pets_and_no_deadline() {
    let mut harness = Harness::new(None);
    harness.start();
    assert_eq!(harness.runtime.next_deadline(), None);
    harness.at(60_000).dispatch(Ready::Timer);
    assert!(harness.pets().is_empty());
}

#[test]
fn a_hung_loop_stops_petting() {
    let mut harness = Harness::new(Some(Duration::from_secs(2)));
    harness.start();
    harness.loop_until(1_000);
    let before = harness.pets().len();
    harness.now = Micros::from_millis(5_000);
    assert_eq!(harness.pets().len(), before);
    harness.dispatch(Ready::Timer);
    assert_eq!(harness.pets().len(), before + 1);
}

#[test]
fn status_lines_follow_every_session_state() {
    let mut harness = Harness::new(None);
    harness
        .world
        .borrow_mut()
        .open_raw_errors
        .extend(std::iter::repeat_n(Errno::EACCES, 8));
    harness.start();
    harness.node_added(NodeKind::Hidraw);
    harness.node_added(NodeKind::Evdev);
    harness.loop_until(8_000);
    harness
        .world
        .borrow_mut()
        .raw_reports
        .push_back(Err(Errno::ENODEV));
    harness.dispatch(Ready::Raw);
    harness.node_removed(NodeKind::Hidraw);
    harness.dispatch(Ready::Signal(StopSignal::Terminate));
    assert_eq!(
        harness.statuses(),
        [
            "waiting: no Magic Mouse 2 connected",
            "acquiring: Apple Magic Mouse 2 (Bluetooth) on hidraw5",
            "degraded: cannot open raw device: permission denied after 2000 ms of retries; stock behaviour, retrying",
            "acquiring: Apple Magic Mouse 2 (Bluetooth) on hidraw5",
            "active: Apple Magic Mouse 2 (Bluetooth) on hidraw5",
            "absent: Apple Magic Mouse 2 (Bluetooth) disconnected",
            "waiting: no Magic Mouse 2 connected",
        ]
    );
    assert_eq!(harness.notices.borrow().last(), Some(&Notice::Stopping));
}

#[test]
fn the_second_node_triggers_acquisition_in_either_order() {
    for (first, second) in [
        (NodeKind::Hidraw, NodeKind::Evdev),
        (NodeKind::Evdev, NodeKind::Hidraw),
    ] {
        let mut harness = Harness::new(None);
        harness.start();
        harness.at(1_000).node_added(first);
        assert_eq!(harness.state(), None);
        assert!(harness.take_calls().is_empty());
        harness.at(1_030).node_added(second);
        assert_eq!(harness.state(), Some(SessionState::Active));
        assert_eq!(harness.take_calls().last(), Some(&Call::Grab));
    }
}

#[test]
fn a_late_acl_is_retried_and_taken_over_well_inside_two_seconds() {
    let mut harness = Harness::new(None);
    harness.start();
    harness.world.borrow_mut().open_raw_errors.extend([
        Errno::EACCES,
        Errno::EACCES,
        Errno::EACCES,
    ]);
    harness.at(5_000).node_added(NodeKind::Evdev);
    let records = capture(|| {
        let mut observer = LogObserver::new(false);
        harness
            .runtime
            .dispatch(harness.now, Ready::Directory, &mut observer)
            .unwrap();
        harness
            .world
            .borrow_mut()
            .udev_changes
            .extend(harness.discovery.node_added(node(NodeKind::Hidraw)));
        harness
            .runtime
            .dispatch(harness.now, Ready::Directory, &mut observer)
            .unwrap();
        while harness
            .runtime
            .manager()
            .session()
            .map(crate::session::Session::state)
            == Some(SessionState::Acquiring)
        {
            let deadline = harness.runtime.next_deadline().unwrap();
            harness.now = deadline;
            harness.runtime.run_due(deadline, &mut observer);
        }
    });
    assert_eq!(harness.state(), Some(SessionState::Active));
    assert_eq!(harness.now, Micros::from_millis(5_350));
    let info: Vec<_> = records
        .iter()
        .filter(|record| record.level == Level::INFO)
        .collect();
    assert_eq!(info.len(), 1, "{records:?}");
    assert!(
        info[0]
            .message
            .ends_with("taken over 350 ms after the mouse appeared, 4 attempts"),
        "{}",
        info[0].message
    );
}

#[test]
fn removal_during_acquisition_stops_the_retries_and_reconnect_starts_fresh() {
    let mut harness = Harness::new(None);
    harness.start();
    harness
        .world
        .borrow_mut()
        .open_stock_errors
        .extend([Errno::EACCES, Errno::EACCES]);
    harness.at(1_000).node_added(NodeKind::Hidraw);
    harness.node_added(NodeKind::Evdev);
    assert_eq!(harness.state(), Some(SessionState::Acquiring));
    harness.at(1_020).node_removed(NodeKind::Evdev);
    assert_eq!(harness.state(), None);
    assert_eq!(harness.runtime.next_deadline(), None);
    let calls = harness.take_calls();
    assert_eq!(calls.iter().filter(|call| **call == Call::Grab).count(), 0);
    harness.at(4_000).node_added(NodeKind::Evdev);
    harness.loop_until(4_500);
    assert_eq!(harness.state(), Some(SessionState::Active));
}

#[test]
fn removal_of_either_node_releases_and_ten_reconnects_take_over_each_time() {
    let mut harness = Harness::new(None);
    harness.start();
    for cycle in 0..10_u64 {
        let base = cycle * 10_000;
        let (first, second) = if cycle % 2 == 0 {
            (NodeKind::Hidraw, NodeKind::Evdev)
        } else {
            (NodeKind::Evdev, NodeKind::Hidraw)
        };
        harness.at(base).node_added(first);
        harness.at(base + 40).node_added(second);
        assert_eq!(harness.state(), Some(SessionState::Active), "cycle {cycle}");
        harness.at(base + 5_000).node_removed(second);
        assert_eq!(harness.state(), None);
        harness.node_removed(first);
        assert!(!harness.world.borrow().grabbed);
    }
}

#[test]
fn sighup_applies_a_valid_config_and_keeps_the_old_one_when_invalid() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("config.toml");
    let mut harness = Harness::new(None);
    harness.runtime = std::mem::replace(
        &mut harness.runtime,
        Runtime::new(
            FakeDirectory {
                world: harness.world.clone(),
            },
            FakeOpener {
                world: harness.world.clone(),
            },
            Mode::RawForward,
            Config::default(),
        ),
    )
    .with_config_source(ConfigSource::new(path.clone()));
    harness
        .world
        .borrow_mut()
        .enumerated
        .push(DiscoveryEvent::MouseAppeared(nodes()));
    harness.start();

    fs::write(&path, "[pointer]\nmiddle_click = false\n").unwrap();
    let records = capture(|| harness.dispatch(Ready::Reload));
    assert!(!harness.runtime.config().pointer.middle_click);
    let engine_config = harness
        .runtime
        .manager()
        .session()
        .unwrap()
        .engine()
        .config()
        .clone();
    assert!(!engine_config.pointer.middle_click);
    assert!(records[0].message.starts_with("config reloaded from "));

    fs::write(&path, "[pointer]\nmiddle_click = \"no\"\n").unwrap();
    let records = capture(|| harness.dispatch(Ready::Reload));
    assert!(!harness.runtime.config().pointer.middle_click);
    assert!(records.iter().any(|record| {
        record.level == Level::ERROR
            && record
                .message
                .contains("config.toml:2: pointer.middle_click")
    }));
    assert_eq!(
        records.last().unwrap().message,
        "config reload rejected, keeping the previous config"
    );
    assert_eq!(harness.state(), Some(SessionState::Active));

    fs::remove_file(&path).unwrap();
    harness.dispatch(Ready::Reload);
    assert_eq!(harness.runtime.config(), &Config::default());
}

#[test]
fn a_config_reload_applies_to_the_next_session_too() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("config.toml");
    fs::write(&path, "[log]\nlevel = \"warn\"\n").unwrap();
    let mut harness = Harness::new(None);
    harness.runtime = std::mem::replace(
        &mut harness.runtime,
        Runtime::new(
            FakeDirectory {
                world: harness.world.clone(),
            },
            FakeOpener {
                world: harness.world.clone(),
            },
            Mode::RawForward,
            Config::default(),
        ),
    )
    .with_config_source(ConfigSource::new(path));
    harness.start();
    harness.dispatch(Ready::Reload);
    harness.node_added(NodeKind::Hidraw);
    harness.node_added(NodeKind::Evdev);
    let session = harness.runtime.manager().session().unwrap();
    assert_eq!(
        session.engine().config().log.level,
        md_core::config::LogLevel::Warn
    );
}

fn synthetic_fixture() -> Vec<(Micros, Vec<u8>)> {
    let header = RecordingHeader {
        profile_id: MAGIC_MOUSE_2_BT.id.to_owned(),
        magic_driver_version: "0.0.0-test".to_owned(),
        kernel_version: "none".to_owned(),
        start_unix_s: 0,
    };
    let mut writer = RecordingWriter::new(Vec::new(), &header).unwrap();
    for (t, bytes) in synthetic_session(2_000) {
        writer.write_report(t, &bytes).unwrap();
    }
    let text = writer.into_inner();
    RecordingReader::new(text.as_slice())
        .unwrap()
        .map(Result::unwrap)
        .collect()
}

fn replay_through_the_runtime(
    debug: bool,
    level: LevelFilter,
) -> Vec<crate::logging::capture::Record> {
    let fixture = synthetic_fixture();
    let mut harness = Harness::new(Some(Duration::from_secs(2)));
    let mut observer = LogObserver::new(debug);
    harness.runtime.set_tracing(debug);
    capture_at(level, || {
        harness
            .world
            .borrow_mut()
            .enumerated
            .push(DiscoveryEvent::MouseAppeared(nodes()));
        harness.runtime.start(Micros::ZERO, &mut observer).unwrap();
        for (t, bytes) in &fixture {
            harness
                .world
                .borrow_mut()
                .raw_reports
                .push_back(Ok(bytes.clone()));
            harness
                .world
                .borrow_mut()
                .stock_events
                .push_back(Ok(vec![EvdevEvent::new(0, 0, 0)]));
            harness
                .runtime
                .dispatch(*t, Ready::Stock, &mut observer)
                .unwrap();
            harness
                .runtime
                .dispatch(*t, Ready::Raw, &mut observer)
                .unwrap();
            harness.runtime.run_due(*t, &mut observer);
        }
        let end = fixture.last().unwrap().0;
        harness
            .runtime
            .dispatch(end, Ready::Signal(StopSignal::Terminate), &mut observer)
            .unwrap();
    })
}

#[test]
fn info_logging_over_a_replayed_fixture_has_no_per_report_lines() {
    let records = replay_through_the_runtime(false, LevelFilter::INFO);
    let messages: Vec<_> = records
        .iter()
        .map(|record| record.message.as_str())
        .collect();
    assert_eq!(records.len(), 2, "{messages:?}");
    assert!(messages[0].starts_with("active: Apple Magic Mouse 2"));
    assert!(messages[1].starts_with("released /dev/hidraw5: shutdown"));
    assert!(records.iter().all(|record| record.level <= Level::INFO));
}

#[test]
fn debug_mode_logs_one_line_per_report_at_debug_level() {
    let records = replay_through_the_runtime(true, LevelFilter::DEBUG);
    let reports = records
        .iter()
        .filter(|record| {
            record.level == Level::DEBUG
                && record
                    .message
                    .split(' ')
                    .next()
                    .is_some_and(|t| t.parse::<u64>().is_ok())
                && record.message.split(' ').nth(1) == Some("report")
        })
        .count();
    assert_eq!(reports, 2_000);
    assert!(
        records
            .iter()
            .filter(|record| record.message.split(' ').nth(1) == Some("report"))
            .all(|record| record.message.contains(" ns"))
    );
    let info = records
        .iter()
        .filter(|record| record.level == Level::INFO)
        .count();
    assert_eq!(info, 2);
}

fn push_raw_motion(harness: &Harness) {
    harness
        .world
        .borrow_mut()
        .raw_reports
        .push_back(Ok(ReportBuilder::new().motion(1, 0).build()));
}

fn push_stock_motion(harness: &Harness) {
    harness
        .world
        .borrow_mut()
        .stock_events
        .push_back(Ok(vec![EvdevEvent::new(2, 0, 1), EvdevEvent::new(0, 0, 0)]));
}

fn active_harness() -> Harness {
    let mut harness = Harness::new(Some(Duration::from_secs(2)));
    harness
        .world
        .borrow_mut()
        .enumerated
        .push(DiscoveryEvent::MouseAppeared(nodes()));
    harness.start();
    assert_eq!(harness.state(), Some(SessionState::Active));
    harness.take_calls();
    harness
}

#[test]
fn raw_dispatched_before_stock_at_the_end_of_every_burst_keeps_the_takeover() {
    let mut harness = active_harness();
    let records = capture_at(LevelFilter::WARN, || {
        for burst in 0..20_u64 {
            let base = 1_000 + burst * 3_000;
            for step in 0..40 {
                push_raw_motion(&harness);
                push_stock_motion(&harness);
                let ms = base + step * 11;
                harness.at(ms).dispatch(Ready::Raw);
                harness.at(ms).dispatch(Ready::Stock);
            }
            harness.loop_until(base + 3_000);
        }
    });
    assert_eq!(records, Vec::<crate::logging::capture::Record>::new());
    assert_eq!(harness.state(), Some(SessionState::Active));
    assert!(harness.world.borrow().grabbed);
    let calls = harness.take_calls();
    assert!(
        !calls.iter().any(|call| matches!(
            call,
            Call::Ungrab | Call::CloseRaw | Call::DestroyPointer | Call::DestroyTouchpad
        )),
        "{calls:?}"
    );
    assert!(
        harness
            .statuses()
            .iter()
            .all(|line| !line.starts_with("recovering") && !line.starts_with("degraded"))
    );
}

#[test]
fn a_silent_hidraw_keeps_the_takeover_and_logs_only_problems_at_warn_or_error() {
    let mut harness = active_harness();
    let mut observer = LogObserver::new(false);
    let records = capture_at(LevelFilter::INFO, || {
        let mut ms = 1_000;
        while ms < 1_700 {
            push_stock_motion(&harness);
            harness.now = Micros::from_millis(ms);
            harness
                .runtime
                .dispatch(harness.now, Ready::Stock, &mut observer)
                .unwrap();
            harness.runtime.run_due(harness.now, &mut observer);
            ms += 10;
        }
        push_raw_motion(&harness);
        harness
            .runtime
            .dispatch(Micros::from_millis(1_700), Ready::Raw, &mut observer)
            .unwrap();
    });
    assert_eq!(harness.state(), Some(SessionState::Active));
    assert!(harness.world.borrow().grabbed);
    let calls = harness.take_calls();
    assert!(!calls.contains(&Call::Ungrab), "{calls:?}");
    assert!(!calls.contains(&Call::DestroyTouchpad), "{calls:?}");
    assert_eq!(
        calls
            .iter()
            .filter(|call| matches!(call, Call::OpenRaw(_)))
            .count(),
        2
    );
    let levels: Vec<_> = records
        .iter()
        .map(|record| (record.level, record.message.split(':').next().unwrap_or("")))
        .collect();
    assert_eq!(
        levels,
        [
            (Level::ERROR, "lost the raw stream of /dev/hidraw5"),
            (Level::WARN, "recovered /dev/hidraw5"),
        ],
        "{records:?}"
    );
    assert!(
        records[1]
            .message
            .contains("raw reports flow again 450 ms after")
    );
    let statuses = harness.statuses();
    assert!(
        statuses
            .iter()
            .any(|line| line.starts_with("recovering: Apple Magic Mouse 2 (Bluetooth) on hidraw5")),
        "{statuses:?}"
    );
    assert_eq!(
        statuses.last().unwrap(),
        "active: Apple Magic Mouse 2 (Bluetooth) on hidraw5"
    );
}
