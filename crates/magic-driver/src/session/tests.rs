use std::path::PathBuf;

use md_core::config::Config;
use md_core::profile::MAGIC_MOUSE_2_BT;
use md_core::synth::Frame;
use md_core::testkit::{ReportBuilder, TouchRecord, touch_state};
use md_core::{Button, Micros, Mode};
use nix::errno::Errno;

use super::selfcheck::SelfCheckFailure;
use super::*;
use crate::io::fake::{Call, FakeOpener, SharedWorld, world};

fn nodes(n: u32) -> MouseNodes {
    MouseNodes {
        key: PathBuf::from(format!(
            "/sys/devices/virtual/misc/uhid/0005:004C:0269.000{n}"
        )),
        hidraw: PathBuf::from(format!("/dev/hidraw{n}")),
        evdev: PathBuf::from(format!("/dev/input/event{}", 10 + n)),
        profile: &MAGIC_MOUSE_2_BT,
    }
}

#[derive(Debug, Default)]
struct Recorder {
    lines: Vec<String>,
}

impl SessionObserver for Recorder {
    fn observe(&mut self, event: &SessionEvent<'_>) {
        let line = match event {
            SessionEvent::Acquired { .. } => "acquired".to_owned(),
            SessionEvent::Degraded {
                cause, retry_in, ..
            } => format!("degraded {cause} retry {}", retry_in.as_millis()),
            SessionEvent::Retrying { .. } => "retrying".to_owned(),
            SessionEvent::RawLost {
                failure,
                retry_in,
                attempts,
                ..
            } => format!(
                "raw lost {failure} retry {} attempts {attempts}",
                retry_in.as_millis()
            ),
            SessionEvent::RawReopenFailed {
                error,
                retry_in,
                attempts,
                ..
            } => format!(
                "reopen failed {} retry {} attempt {attempts}",
                error.kind(),
                retry_in.as_millis()
            ),
            SessionEvent::RawRecovered {
                attempts, elapsed, ..
            } => format!(
                "raw recovered after {} attempts {attempts}",
                elapsed.as_millis()
            ),
            SessionEvent::AcquireFailed {
                step,
                error,
                retry_in,
                ..
            } => format!(
                "failed {step}: {} retry {:?}",
                error.kind(),
                retry_in.map(Micros::as_millis)
            ),
            SessionEvent::Released { reason, .. } => format!("released {reason}"),
            SessionEvent::Report { outputs, .. } => format!("report {}", outputs.len()),
            SessionEvent::StockEvents { events, .. } => format!("stock {}", events.len()),
            SessionEvent::Trace { record } => format!("trace {record}"),
        };
        self.lines.push(line);
    }
}

struct Harness {
    world: SharedWorld,
    opener: FakeOpener,
    session: Session<FakeOpener>,
    observer: Recorder,
}

impl Harness {
    fn new(mode: Mode) -> Harness {
        let world = world();
        Harness {
            opener: FakeOpener {
                world: world.clone(),
            },
            world,
            session: Session::new(nodes(5), mode, Config::default(), Micros::ZERO),
            observer: Recorder::default(),
        }
    }

    fn timer(&mut self, ms: u64) {
        self.session.on_timer(
            Micros::from_millis(ms),
            &mut self.opener,
            &mut self.observer,
        );
    }

    fn report(&mut self, bytes: Vec<u8>) {
        self.world.borrow_mut().raw_reports.push_back(Ok(bytes));
    }

    fn read(&mut self, ms: u64) {
        self.session
            .on_raw_readable(Micros::from_millis(ms), &mut self.observer);
    }

    fn calls(&self) -> Vec<Call> {
        self.world.borrow().calls.clone()
    }

    fn take_calls(&self) -> Vec<Call> {
        std::mem::take(&mut self.world.borrow_mut().calls)
    }

    fn active(mode: Mode) -> Harness {
        let mut harness = Harness::new(mode);
        harness.timer(0);
        assert_eq!(harness.session.state(), SessionState::Active);
        harness.take_calls();
        harness
    }
}

fn position(calls: &[Call], wanted: &Call) -> usize {
    calls
        .iter()
        .position(|call| call == wanted)
        .unwrap_or_else(|| panic!("{wanted:?} not in {calls:?}"))
}

#[test]
fn new_session_is_acquiring_and_due_now() {
    let harness = Harness::new(Mode::RawForward);
    assert_eq!(harness.session.state(), SessionState::Acquiring);
    assert_eq!(harness.session.next_deadline(), Some(Micros::ZERO));
    assert!(harness.calls().is_empty());
}

#[test]
fn normal_takeover_opens_raw_then_creates_devices_then_grabs() {
    let mut harness = Harness::new(Mode::RawForward);
    harness.timer(0);
    assert_eq!(harness.session.state(), SessionState::Active);
    assert_eq!(
        harness.calls(),
        vec![
            Call::OpenRaw(PathBuf::from("/dev/hidraw5")),
            Call::CreatePointer,
            Call::CreateTouchpad,
            Call::OpenStock(PathBuf::from("/dev/input/event15")),
            Call::Grab,
        ]
    );
    assert!(harness.world.borrow().grabbed);
    assert_eq!(harness.observer.lines, vec!["acquired"]);
    assert_eq!(harness.session.next_deadline(), None);
    assert_ne!(harness.session.fds().generation, 0);
}

#[test]
fn reports_drive_pointer_and_touchpad() {
    let mut harness = Harness::active(Mode::RawForward);
    let finger = TouchRecord::new(1, 0, 0).size(12);
    harness.report(ReportBuilder::new().motion(4, -3).build());
    harness.report(
        ReportBuilder::new()
            .touch(finger.state(touch_state::START))
            .build(),
    );
    harness.report(ReportBuilder::new().left(true).touch(finger).build());
    harness.read(10);
    let calls = harness.calls();
    assert_eq!(calls[0], Call::Motion { dx: 4, dy: -3 });
    assert!(matches!(calls[1], Call::Frame(frame) if frame.contact_count() == 1));
    assert_eq!(
        calls[2],
        Call::Button {
            button: Button::Middle,
            pressed: true
        }
    );
    assert_eq!(harness.session.engine().stats().reports, 3);
    assert_eq!(
        harness.observer.lines[1..],
        ["report 1", "report 1", "report 1"]
    );
}

#[test]
fn permission_denied_then_granted_retries_with_backoff() {
    let mut harness = Harness::new(Mode::RawForward);
    harness.world.borrow_mut().open_raw_errors.extend([
        Errno::EACCES,
        Errno::EACCES,
        Errno::EACCES,
    ]);
    harness.timer(0);
    assert_eq!(harness.session.state(), SessionState::Acquiring);
    assert_eq!(
        harness.session.next_deadline(),
        Some(Micros::from_millis(50))
    );
    harness.timer(20);
    assert_eq!(
        harness.session.next_deadline(),
        Some(Micros::from_millis(50))
    );
    harness.timer(50);
    assert_eq!(
        harness.session.next_deadline(),
        Some(Micros::from_millis(150))
    );
    harness.timer(150);
    assert_eq!(
        harness.session.next_deadline(),
        Some(Micros::from_millis(350))
    );
    assert!(harness.calls().is_empty());
    harness.timer(350);
    assert_eq!(harness.session.state(), SessionState::Active);
    assert_eq!(
        harness.observer.lines,
        vec![
            "failed open raw device: permission denied retry Some(50)",
            "failed open raw device: permission denied retry Some(100)",
            "failed open raw device: permission denied retry Some(200)",
            "acquired",
        ]
    );
}

#[test]
fn a_failed_grab_destroys_created_devices_before_retrying() {
    let mut harness = Harness::new(Mode::RawForward);
    harness
        .world
        .borrow_mut()
        .grab_errors
        .push_back(Errno::EBUSY);
    harness.timer(0);
    assert_eq!(
        harness.take_calls(),
        vec![
            Call::OpenRaw(PathBuf::from("/dev/hidraw5")),
            Call::CreatePointer,
            Call::CreateTouchpad,
            Call::OpenStock(PathBuf::from("/dev/input/event15")),
            Call::CloseStock,
            Call::DestroyTouchpad,
            Call::DestroyPointer,
            Call::CloseRaw,
        ]
    );
    assert_eq!(harness.session.state(), SessionState::Acquiring);
    harness.timer(50);
    assert_eq!(harness.session.state(), SessionState::Active);
}

#[test]
fn uinput_failure_closes_the_raw_device() {
    let mut harness = Harness::new(Mode::RawForward);
    harness
        .world
        .borrow_mut()
        .create_touchpad_errors
        .push_back(Errno::EACCES);
    harness.timer(0);
    assert_eq!(
        harness.calls(),
        vec![
            Call::OpenRaw(PathBuf::from("/dev/hidraw5")),
            Call::CreatePointer,
            Call::DestroyPointer,
            Call::CloseRaw,
        ]
    );
    assert!(!harness.world.borrow().grabbed);
}

#[test]
fn acquisition_gives_up_after_two_seconds_and_degrades() {
    let mut harness = Harness::new(Mode::RawForward);
    harness
        .world
        .borrow_mut()
        .open_raw_errors
        .extend(std::iter::repeat_n(Errno::EACCES, 100));
    let mut attempts = Vec::new();
    while harness.session.state() == SessionState::Acquiring {
        let deadline = harness.session.next_deadline().unwrap();
        attempts.push(deadline.as_millis());
        harness.timer(deadline.as_millis());
    }
    assert_eq!(attempts, vec![0, 50, 150, 350, 750, 1550, 2000]);
    assert_eq!(harness.session.state(), SessionState::Degraded);
    assert_eq!(
        harness.observer.lines[harness.observer.lines.len() - 2..],
        [
            "failed open raw device: permission denied retry None",
            "degraded cannot open raw device: permission denied after 2000 ms of retries retry 5000",
        ]
    );
    assert_eq!(
        harness.session.next_deadline(),
        Some(Micros::from_millis(7_000))
    );
    assert!(harness.calls().is_empty());
}

#[test]
fn device_gone_mid_stroke_lifts_every_touch_before_destroy() {
    let mut harness = Harness::active(Mode::ScrollProbe);
    let finger = TouchRecord::new(1, 0, 0).size(4);
    harness.report(
        ReportBuilder::new()
            .touch(finger.state(touch_state::START))
            .build(),
    );
    harness.report(
        ReportBuilder::new()
            .touch(TouchRecord::new(1, 0, 100).size(4))
            .build(),
    );
    harness
        .world
        .borrow_mut()
        .raw_reports
        .push_back(Err(Errno::ENODEV));
    harness.read(10);
    assert_eq!(harness.session.state(), SessionState::Absent);
    let calls = harness.calls();
    let ungrab = position(&calls, &Call::Ungrab);
    let lift = position(&calls, &Call::Frame(Frame::EMPTY));
    let destroy_touchpad = position(&calls, &Call::DestroyTouchpad);
    assert!(ungrab < lift && lift < destroy_touchpad);
    assert_eq!(
        calls[ungrab..],
        [
            Call::Ungrab,
            Call::CloseStock,
            Call::Frame(Frame::EMPTY),
            Call::DestroyTouchpad,
            Call::DestroyPointer,
            Call::CloseRaw,
        ]
    );
    assert_eq!(
        harness.observer.lines.last().unwrap(),
        "released device gone"
    );
    assert!(harness.session.fds().raw.is_none());
}

#[test]
fn device_gone_mid_click_releases_buttons_before_destroy() {
    let mut harness = Harness::active(Mode::RawForward);
    let finger = TouchRecord::new(1, -700, 0).size(12);
    harness.report(ReportBuilder::new().left(true).touch(finger).build());
    harness
        .world
        .borrow_mut()
        .raw_reports
        .push_back(Err(Errno::EIO));
    harness.read(10);
    let calls = harness.calls();
    let press = position(
        &calls,
        &Call::Button {
            button: Button::Left,
            pressed: true,
        },
    );
    let ungrab = position(&calls, &Call::Ungrab);
    let release = position(
        &calls,
        &Call::Button {
            button: Button::Left,
            pressed: false,
        },
    );
    let lift = position(&calls, &Call::Frame(Frame::EMPTY));
    let destroy_pointer = position(&calls, &Call::DestroyPointer);
    let destroy_touchpad = position(&calls, &Call::DestroyTouchpad);
    assert!(press < ungrab);
    assert!(ungrab < release && release < destroy_pointer);
    assert!(lift < destroy_touchpad && destroy_touchpad < destroy_pointer);
    assert_eq!(harness.session.state(), SessionState::Absent);
}

#[test]
fn stock_device_gone_releases_the_session() {
    let mut harness = Harness::active(Mode::RawForward);
    harness
        .world
        .borrow_mut()
        .stock_events
        .extend([Ok(vec![EvdevEvent::new(2, 0, 1)]), Err(Errno::ENODEV)]);
    harness
        .session
        .on_stock_readable(Micros::from_millis(5), &mut harness.observer);
    assert_eq!(harness.session.state(), SessionState::Absent);
    assert_eq!(
        harness.observer.lines[1..],
        ["stock 1", "released device gone"]
    );
}

#[test]
fn other_stock_read_errors_release_and_degrade_instead_of_spinning() {
    let mut harness = Harness::active(Mode::RawForward);
    harness
        .world
        .borrow_mut()
        .stock_events
        .push_back(Err(Errno::EINVAL));
    harness
        .session
        .on_stock_readable(Micros::from_millis(1), &mut harness.observer);
    assert_eq!(harness.session.state(), SessionState::Degraded);
}

#[test]
fn stock_events_are_drained_while_active() {
    let mut harness = Harness::active(Mode::RawForward);
    harness
        .world
        .borrow_mut()
        .stock_events
        .extend([Ok(vec![EvdevEvent::new(2, 0, 1), EvdevEvent::new(0, 0, 0)])]);
    harness
        .session
        .on_stock_readable(Micros::from_millis(5), &mut harness.observer);
    assert_eq!(harness.session.state(), SessionState::Active);
    assert!(harness.world.borrow().stock_events.is_empty());
    assert_eq!(harness.observer.lines[1..], ["stock 2"]);
}

#[test]
fn stale_stock_events_are_discarded_at_takeover() {
    let mut harness = Harness::new(Mode::RawForward);
    harness
        .world
        .borrow_mut()
        .stock_events
        .push_back(Ok(vec![EvdevEvent::new(2, 0, 1)]));
    harness.timer(0);
    assert!(harness.world.borrow().stock_events.is_empty());
    assert_eq!(harness.observer.lines, vec!["acquired"]);
}

#[test]
fn shutdown_releases_in_order_and_ignores_ungrab_errors() {
    let mut harness = Harness::active(Mode::RawForward);
    harness.world.borrow_mut().ungrab_error = Some(Errno::ENODEV);
    harness
        .session
        .release(ReleaseReason::Shutdown, &mut harness.observer);
    assert_eq!(
        harness.calls(),
        vec![
            Call::Ungrab,
            Call::CloseStock,
            Call::DestroyTouchpad,
            Call::DestroyPointer,
            Call::CloseRaw,
        ]
    );
    assert_eq!(harness.session.state(), SessionState::Absent);
}

#[test]
fn release_while_acquiring_stops_retrying() {
    let mut harness = Harness::new(Mode::RawForward);
    harness
        .world
        .borrow_mut()
        .open_raw_errors
        .push_back(Errno::ENOENT);
    harness.timer(0);
    harness
        .session
        .release(ReleaseReason::Removed, &mut harness.observer);
    assert_eq!(harness.session.state(), SessionState::Absent);
    assert_eq!(harness.session.next_deadline(), None);
    harness.timer(50);
    assert!(harness.calls().is_empty());
}

#[test]
fn virtual_device_write_failure_degrades_and_releases() {
    let mut harness = Harness::active(Mode::RawForward);
    harness.world.borrow_mut().pointer_write_error = Some(Errno::EINVAL);
    harness.report(ReportBuilder::new().motion(1, 1).build());
    harness.report(ReportBuilder::new().motion(2, 2).build());
    harness.read(10);
    assert_eq!(harness.session.state(), SessionState::Degraded);
    assert_eq!(harness.calls()[0], Call::Ungrab);
    assert_eq!(
        harness.observer.lines[harness.observer.lines.len() - 2..],
        [
            "released virtual device write failed",
            "degraded virtual device write failed retry 5000"
        ]
    );
    assert_eq!(harness.world.borrow().raw_reports.len(), 1);
}

#[test]
fn reads_are_bounded_per_wake() {
    let mut harness = Harness::active(Mode::PointerOnly);
    for _ in 0..MAX_REPORTS_PER_WAKE + 5 {
        harness.report(ReportBuilder::new().motion(1, 0).build());
    }
    harness.read(1);
    assert_eq!(harness.world.borrow().raw_reports.len(), 5);
    harness.read(2);
    assert!(harness.world.borrow().raw_reports.is_empty());
}

#[test]
fn reads_without_devices_do_nothing() {
    let mut harness = Harness::new(Mode::RawForward);
    harness.report(ReportBuilder::new().motion(1, 0).build());
    harness.read(1);
    harness
        .session
        .on_stock_readable(Micros::from_millis(1), &mut harness.observer);
    assert!(harness.calls().is_empty());
    assert_eq!(harness.world.borrow().raw_reports.len(), 1);
}

fn stock_motion(harness: &mut Harness, ms: u64) {
    harness
        .world
        .borrow_mut()
        .stock_events
        .push_back(Ok(vec![EvdevEvent::new(2, 0, 1), EvdevEvent::new(0, 0, 0)]));
    harness
        .session
        .on_stock_readable(Micros::from_millis(ms), &mut harness.observer);
}

fn raw_motion(harness: &mut Harness, ms: u64) {
    harness.report(ReportBuilder::new().motion(1, 0).build());
    harness.read(ms);
}

fn stock_error(harness: &mut Harness, ms: u64) {
    harness
        .world
        .borrow_mut()
        .stock_events
        .push_back(Err(Errno::EINVAL));
    harness
        .session
        .on_stock_readable(Micros::from_millis(ms), &mut harness.observer);
}

fn malformed_report() -> Vec<u8> {
    let mut bytes = ReportBuilder::new().motion(1, 0).build();
    bytes.push(0);
    bytes
}

fn battery_report() -> Vec<u8> {
    vec![0x90, 0x04, 0x64]
}

fn raw_losses(harness: &Harness) -> Vec<&str> {
    harness
        .observer
        .lines
        .iter()
        .map(String::as_str)
        .filter(|line| line.starts_with("raw lost") || line.starts_with("reopen failed"))
        .collect()
}

fn run_until(harness: &mut Harness, end_ms: u64) {
    while let Some(deadline) = harness.session.next_deadline() {
        if deadline > Micros::from_millis(end_ms) {
            break;
        }
        harness.timer(deadline.as_millis());
    }
}

fn assert_kept_and_never_lost(harness: &Harness) {
    assert_eq!(harness.session.state(), SessionState::Active);
    assert_eq!(harness.session.raw_failure(), None);
    assert!(harness.world.borrow().grabbed);
    let calls = harness.calls();
    assert!(
        !calls.iter().any(|call| matches!(
            call,
            Call::Ungrab | Call::CloseRaw | Call::DestroyPointer | Call::DestroyTouchpad
        )),
        "{calls:?}"
    );
    assert_eq!(
        raw_losses(harness),
        Vec::<&str>::new(),
        "{:?}",
        harness.observer.lines
    );
}

#[test]
fn the_last_raw_report_handled_before_its_stock_copy_is_not_a_failure() {
    let mut harness = Harness::active(Mode::RawForward);
    for step in 0..20 {
        let ms = 1_000 + step * 11;
        raw_motion(&mut harness, ms);
        stock_motion(&mut harness, ms);
    }
    assert_eq!(harness.session.next_deadline(), None);
    for ms in [1_209 + 250, 1_209 + 251, 5_000, 60_000] {
        harness.timer(ms);
    }
    assert_kept_and_never_lost(&harness);
}

#[test]
fn a_stock_copy_handled_a_wake_later_than_its_raw_report_is_not_a_failure() {
    let mut harness = Harness::active(Mode::RawForward);
    for burst in 0..10 {
        let base = 1_000 + burst * 2_000;
        for step in 0..30 {
            let ms = base + step * 11;
            raw_motion(&mut harness, ms);
            stock_motion(&mut harness, ms + 3);
        }
        harness.timer(base + 1_000);
    }
    assert_kept_and_never_lost(&harness);
}

#[test]
fn idle_after_a_burst_never_fails_whatever_the_last_order() {
    for raw_first in [true, false] {
        let mut harness = Harness::active(Mode::RawForward);
        for step in 0..50 {
            let ms = 1_000 + step * 11;
            if raw_first {
                raw_motion(&mut harness, ms);
                stock_motion(&mut harness, ms);
            } else {
                stock_motion(&mut harness, ms);
                raw_motion(&mut harness, ms);
            }
        }
        for ms in (2_000..120_000).step_by(997) {
            harness.timer(ms);
        }
        stock_motion(&mut harness, 120_000);
        raw_motion(&mut harness, 120_001);
        harness.timer(121_000);
        assert_kept_and_never_lost(&harness);
    }
}

#[test]
fn interleaved_raw_and_stock_orders_never_fail() {
    let mut harness = Harness::active(Mode::RawForward);
    let mut ms = 1_000;
    for step in 0..300_u64 {
        match step % 4 {
            0 => {
                raw_motion(&mut harness, ms);
                stock_motion(&mut harness, ms);
            }
            1 => {
                stock_motion(&mut harness, ms);
                raw_motion(&mut harness, ms + 1);
            }
            2 => {
                harness.report(ReportBuilder::new().motion(1, 0).build());
                harness.report(ReportBuilder::new().motion(1, 0).build());
                harness.read(ms);
                stock_motion(&mut harness, ms + 2);
                stock_motion(&mut harness, ms + 4);
            }
            _ => {
                stock_motion(&mut harness, ms);
                stock_motion(&mut harness, ms + 1);
                harness.report(ReportBuilder::new().motion(1, 0).build());
                harness.report(ReportBuilder::new().motion(1, 0).build());
                harness.read(ms + 2);
            }
        }
        harness.timer(ms + 5);
        ms += 11;
        if step % 50 == 49 {
            ms += 3_000;
        }
    }
    assert_kept_and_never_lost(&harness);
}

#[test]
fn ignored_reports_prove_the_raw_stream_is_alive() {
    let mut harness = Harness::active(Mode::RawForward);
    for step in 0..100 {
        let ms = 1_000 + step * 11;
        stock_motion(&mut harness, ms);
        harness.report(battery_report());
        harness.read(ms + 1);
    }
    assert_eq!(harness.session.engine().stats().ignored, 100);
    assert_kept_and_never_lost(&harness);
}

#[test]
fn stock_activity_with_matching_hidraw_reports_stays_active() {
    let mut harness = Harness::active(Mode::RawForward);
    for step in 0..100 {
        let ms = 1_000 + step * 11;
        stock_motion(&mut harness, ms);
        raw_motion(&mut harness, ms);
        harness.timer(ms + 5);
    }
    assert_eq!(harness.session.next_deadline(), None);
    assert_kept_and_never_lost(&harness);
}

#[test]
fn stray_stock_events_after_a_long_idle_period_are_not_a_failure() {
    let mut harness = Harness::active(Mode::RawForward);
    raw_motion(&mut harness, 10);
    for ms in (60_000..70_000).step_by(500) {
        stock_motion(&mut harness, ms);
    }
    assert_kept_and_never_lost(&harness);
}

#[test]
fn silent_hidraw_while_the_stock_device_moves_reopens_only_the_raw_device() {
    let mut harness = Harness::active(Mode::RawForward);
    let generation = harness.session.fds().generation;
    raw_motion(&mut harness, 1_000);
    stock_motion(&mut harness, 1_000);
    harness.take_calls();
    let mut ms = 1_010;
    while harness.session.raw_failure().is_none() {
        stock_motion(&mut harness, ms);
        ms += 10;
        assert!(ms < 2_000);
    }
    assert_eq!(ms - 10, 1_310);
    assert_eq!(harness.session.state(), SessionState::Active);
    assert_eq!(
        harness.session.raw_failure(),
        Some(RawFailure::SelfCheck(SelfCheckFailure::NoRawReports))
    );
    assert_eq!(harness.take_calls(), vec![Call::CloseRaw]);
    assert!(harness.world.borrow().grabbed);
    assert_eq!(
        harness.observer.lines.last().unwrap(),
        "raw lost self-check failed: no hidraw report for 250 ms while the stock device kept reporting retry 50 attempts 0"
    );
    assert!(harness.session.fds().raw.is_none());
    assert_eq!(
        harness.session.status_line(),
        "recovering: Apple Magic Mouse 2 (Bluetooth) on hidraw5, self-check failed: no hidraw report for 250 ms while the stock device kept reporting; reopening the raw device"
    );
    assert_eq!(
        harness.session.next_deadline(),
        Some(Micros::from_millis(1_360))
    );
    stock_motion(&mut harness, 1_320);
    assert_eq!(harness.calls(), Vec::<Call>::new());
    harness.timer(1_360);
    assert_eq!(
        harness.take_calls(),
        vec![Call::OpenRaw(PathBuf::from("/dev/hidraw5"))]
    );
    assert_ne!(harness.session.fds().generation, generation);
    assert_eq!(harness.session.next_deadline(), None);
    raw_motion(&mut harness, 1_400);
    assert_eq!(harness.take_calls(), vec![Call::Motion { dx: 1, dy: 0 }]);
    assert_eq!(harness.session.raw_failure(), None);
    assert!(
        harness
            .observer
            .lines
            .iter()
            .any(|line| line == "raw recovered after 90 attempts 1")
    );
    assert_eq!(
        harness.session.status_line(),
        "active: Apple Magic Mouse 2 (Bluetooth) on hidraw5"
    );
    assert!(
        !harness
            .observer
            .lines
            .iter()
            .any(|line| line.starts_with("released") || line.starts_with("degraded"))
    );
}

#[test]
fn a_raw_device_still_silent_after_reopen_is_reopened_with_growing_backoff() {
    let mut harness = Harness::active(Mode::RawForward);
    let mut reopen_times = Vec::new();
    let mut ms = 1_000;
    while ms <= 40_000 {
        stock_motion(&mut harness, ms);
        harness.timer(ms);
        if harness
            .take_calls()
            .contains(&Call::OpenRaw(PathBuf::from("/dev/hidraw5")))
        {
            reopen_times.push(ms);
        }
        ms += 10;
    }
    let gaps: Vec<u64> = reopen_times
        .windows(2)
        .map(|pair| pair[1] - pair[0])
        .collect();
    assert_eq!(
        &gaps[..7],
        &[
            100 + 260,
            200 + 260,
            400 + 260,
            800 + 260,
            1_600 + 260,
            3_200 + 260,
            5_000 + 260
        ]
    );
    assert!(gaps[7..].iter().all(|gap| *gap == 5_000 + 260), "{gaps:?}");
    assert_eq!(harness.session.state(), SessionState::Active);
    assert!(harness.world.borrow().grabbed);
    let losses = raw_losses(&harness);
    assert!(losses.len() <= 1 + 39 / 5, "{losses:?}");
    assert!(losses[0].ends_with("retry 50 attempts 0"));
    assert!(
        losses[1].starts_with("raw lost self-check failed: no hidraw report"),
        "{losses:?}"
    );
    run_until(&mut harness, ms + 5_000);
    assert!(harness.session.fds().raw.is_some() || harness.session.raw_failure().is_some());
    raw_motion(&mut harness, ms + 5_000);
    let lines = &harness.observer.lines;
    let recovered = &lines[lines.len() - 2];
    assert!(recovered.starts_with("raw recovered after "), "{recovered}");
    assert_eq!(harness.session.raw_failure(), None);
}

#[test]
fn a_failing_raw_reopen_backs_off_to_the_cap_and_keeps_the_grab() {
    let mut harness = Harness::active(Mode::RawForward);
    harness
        .world
        .borrow_mut()
        .raw_reports
        .push_back(Err(Errno::EINVAL));
    harness.read(1_000);
    assert_eq!(
        harness.session.raw_failure(),
        Some(RawFailure::ReadFailed(DeviceErrorKind::Other))
    );
    harness
        .world
        .borrow_mut()
        .open_raw_errors
        .extend(std::iter::repeat_n(Errno::EACCES, 12));
    let mut attempts = Vec::new();
    while !harness.world.borrow().open_raw_errors.is_empty() {
        let deadline = harness.session.next_deadline().unwrap();
        attempts.push(deadline.as_millis());
        harness.timer(deadline.as_millis());
    }
    assert_eq!(
        attempts,
        vec![
            1_050, 1_150, 1_350, 1_750, 2_550, 4_150, 7_350, 12_350, 17_350, 22_350, 27_350, 32_350
        ]
    );
    assert_eq!(
        raw_losses(&harness),
        [
            "raw lost hidraw read failed: I/O error retry 50 attempts 0",
            "reopen failed permission denied retry 5000 attempt 7",
            "reopen failed permission denied retry 5000 attempt 8",
            "reopen failed permission denied retry 5000 attempt 9",
            "reopen failed permission denied retry 5000 attempt 10",
            "reopen failed permission denied retry 5000 attempt 11",
            "reopen failed permission denied retry 5000 attempt 12",
        ]
    );
    assert!(harness.world.borrow().grabbed);
    assert_eq!(harness.session.state(), SessionState::Active);
    harness.timer(37_350);
    assert!(harness.session.fds().generation != 0);
    harness.report(battery_report());
    harness.read(37_400);
    assert_eq!(
        harness.observer.lines[harness.observer.lines.len() - 2],
        "raw recovered after 36400 attempts 13"
    );
    let calls = harness.calls();
    assert!(!calls.contains(&Call::Ungrab), "{calls:?}");
    assert!(!calls.contains(&Call::DestroyPointer), "{calls:?}");
}

#[test]
fn malformed_reports_above_the_threshold_reopen_the_raw_device() {
    let mut harness = Harness::active(Mode::RawForward);
    for _ in 0..selfcheck::PARSE_ERROR_LIMIT - 1 {
        harness.report(malformed_report());
    }
    harness.read(100);
    assert_eq!(harness.session.raw_failure(), None);
    harness.report(malformed_report());
    harness.report(ReportBuilder::new().motion(1, 0).build());
    harness.read(200);
    assert_eq!(harness.session.state(), SessionState::Active);
    assert_eq!(
        harness.session.raw_failure(),
        Some(RawFailure::SelfCheck(SelfCheckFailure::ParseErrors {
            count: 10
        }))
    );
    assert_eq!(harness.world.borrow().raw_reports.len(), 1);
    harness.world.borrow_mut().raw_reports.clear();
    assert_eq!(harness.take_calls(), vec![Call::CloseRaw]);
    assert!(harness.world.borrow().grabbed);
    harness.timer(250);
    assert_eq!(
        harness.take_calls(),
        vec![Call::OpenRaw(PathBuf::from("/dev/hidraw5"))]
    );
    for _ in 0..selfcheck::PARSE_ERROR_LIMIT {
        harness.report(malformed_report());
    }
    harness.read(260);
    assert_eq!(harness.session.engine().stats().malformed, 20);
    assert_eq!(harness.take_calls(), vec![Call::CloseRaw]);
    assert_eq!(
        harness.session.next_deadline(),
        Some(Micros::from_millis(360))
    );
    assert_eq!(raw_losses(&harness).len(), 1);
    assert!(harness.world.borrow().grabbed);
}

#[test]
fn occasional_malformed_reports_are_tolerated() {
    let mut harness = Harness::active(Mode::RawForward);
    for step in 0..50 {
        harness.report(malformed_report());
        harness.report(ReportBuilder::new().motion(1, 0).build());
        harness.read(step * 200);
    }
    assert_kept_and_never_lost(&harness);
}

#[test]
fn losing_the_raw_stream_mid_click_releases_buttons_on_the_kept_devices() {
    let mut harness = Harness::active(Mode::RawForward);
    let finger = TouchRecord::new(1, -700, 0).size(12);
    harness.report(ReportBuilder::new().left(true).touch(finger).build());
    harness
        .world
        .borrow_mut()
        .raw_reports
        .push_back(Err(Errno::EINVAL));
    harness.read(10);
    let calls = harness.calls();
    let press = position(
        &calls,
        &Call::Button {
            button: Button::Left,
            pressed: true,
        },
    );
    let close = position(&calls, &Call::CloseRaw);
    let release = position(
        &calls,
        &Call::Button {
            button: Button::Left,
            pressed: false,
        },
    );
    let lift = position(&calls, &Call::Frame(Frame::EMPTY));
    assert!(press < close && close < release && close < lift);
    assert!(!calls.contains(&Call::Ungrab));
    assert!(!calls.contains(&Call::DestroyPointer));
    assert!(!calls.contains(&Call::DestroyTouchpad));
    assert_eq!(harness.session.state(), SessionState::Active);
}

#[test]
fn removal_shutdown_and_stock_gone_during_raw_recovery_release_everything() {
    for ending in 0..3 {
        let mut harness = Harness::active(Mode::RawForward);
        harness
            .world
            .borrow_mut()
            .raw_reports
            .push_back(Err(Errno::EINVAL));
        harness.read(10);
        harness.take_calls();
        match ending {
            0 => harness
                .session
                .release(ReleaseReason::Removed, &mut harness.observer),
            1 => harness
                .session
                .release(ReleaseReason::Shutdown, &mut harness.observer),
            _ => {
                harness
                    .world
                    .borrow_mut()
                    .stock_events
                    .push_back(Err(Errno::ENODEV));
                harness
                    .session
                    .on_stock_readable(Micros::from_millis(20), &mut harness.observer);
            }
        }
        assert_eq!(harness.session.state(), SessionState::Absent);
        assert_eq!(
            harness.take_calls(),
            vec![
                Call::Ungrab,
                Call::CloseStock,
                Call::DestroyTouchpad,
                Call::DestroyPointer,
            ]
        );
        assert!(!harness.world.borrow().grabbed);
        assert_eq!(harness.session.next_deadline(), None);
        assert_eq!(harness.session.raw_failure(), None);
        harness.timer(1_000);
        assert_eq!(harness.calls(), Vec::<Call>::new());
    }
}

#[test]
fn degraded_retries_after_the_minimum_dwell_and_recovers() {
    let mut harness = Harness::active(Mode::RawForward);
    stock_error(&mut harness, 1_000);
    assert_eq!(harness.session.state(), SessionState::Degraded);
    assert_eq!(harness.calls()[0], Call::Ungrab);
    assert_eq!(
        harness.observer.lines[harness.observer.lines.len() - 2..],
        [
            "released device read failed",
            "degraded device read failed retry 5000"
        ]
    );
    harness.take_calls();
    harness.timer(2_000);
    harness.timer(5_999);
    assert_eq!(harness.session.state(), SessionState::Degraded);
    assert_eq!(harness.calls(), Vec::<Call>::new());
    harness.timer(6_000);
    assert_eq!(harness.session.state(), SessionState::Active);
    assert_eq!(
        harness.take_calls(),
        vec![
            Call::OpenRaw(PathBuf::from("/dev/hidraw5")),
            Call::CreatePointer,
            Call::CreateTouchpad,
            Call::OpenStock(PathBuf::from("/dev/input/event15")),
            Call::Grab,
        ]
    );
    assert_eq!(
        harness.observer.lines[harness.observer.lines.len() - 2..],
        ["retrying", "acquired"]
    );
    assert_eq!(harness.session.degrade_cause(), None);
}

#[test]
fn acquisition_give_up_is_retried_periodically_until_permission_is_granted() {
    let mut harness = Harness::new(Mode::RawForward);
    harness
        .world
        .borrow_mut()
        .open_raw_errors
        .extend(std::iter::repeat_n(Errno::EACCES, 14));
    while harness.session.state() == SessionState::Acquiring {
        let deadline = harness.session.next_deadline().unwrap();
        harness.timer(deadline.as_millis());
    }
    assert_eq!(harness.session.state(), SessionState::Degraded);
    assert!(matches!(
        harness.session.degrade_cause(),
        Some(DegradeCause::AcquireGaveUp {
            step: AcquireStep::OpenRaw,
            kind: DeviceErrorKind::PermissionDenied
        })
    ));
    assert_eq!(
        harness.session.next_deadline(),
        Some(Micros::from_millis(7_000))
    );
    harness.timer(7_000);
    while harness.session.state() == SessionState::Acquiring {
        let deadline = harness.session.next_deadline().unwrap();
        harness.timer(deadline.as_millis());
    }
    assert_eq!(harness.session.state(), SessionState::Degraded);
    assert_eq!(
        harness.session.next_deadline(),
        Some(Micros::from_millis(9_000 + 10_000))
    );
    harness.timer(19_000);
    assert_eq!(harness.session.state(), SessionState::Active);
    assert!(harness.world.borrow().grabbed);
}

#[test]
fn repeated_failures_back_off_without_flapping() {
    let mut harness = Harness::active(Mode::RawForward);
    let mut now = 0;
    let mut dwell = Vec::new();
    let mut transitions = 0;
    let mut previous = harness.session.state();
    let mut degraded_at = None;
    while now <= 300_000 {
        if harness.session.state() == SessionState::Active {
            stock_error(&mut harness, now);
        }
        harness.timer(now);
        let state = harness.session.state();
        if state != previous {
            transitions += 1;
            if state == SessionState::Degraded {
                degraded_at = Some(now);
            } else if previous == SessionState::Degraded {
                dwell.push(now - degraded_at.unwrap());
            }
            previous = state;
        }
        now += 50;
    }
    assert!(dwell.iter().all(|ms| *ms >= DEGRADED_MIN_DWELL.as_millis()));
    assert_eq!(&dwell[..5], &[5_000, 10_000, 20_000, 40_000, 60_000]);
    assert!(dwell[5..].iter().all(|ms| *ms == 60_000));
    assert!(transitions <= 20, "{transitions} transitions");
}

#[test]
fn a_long_stable_period_resets_the_backoff() {
    let mut harness = Harness::active(Mode::RawForward);
    stock_error(&mut harness, 1_000);
    harness.timer(6_000);
    assert_eq!(harness.session.state(), SessionState::Active);
    stock_error(&mut harness, 7_000);
    assert_eq!(
        harness.session.next_deadline(),
        Some(Micros::from_millis(17_000))
    );
    harness.timer(17_000);
    assert_eq!(harness.session.state(), SessionState::Active);
    stock_error(&mut harness, 100_000);
    assert_eq!(
        harness.session.next_deadline(),
        Some(Micros::from_millis(105_000))
    );
}

#[test]
fn removal_while_degraded_cancels_the_retry() {
    let mut harness = Harness::active(Mode::RawForward);
    stock_error(&mut harness, 0);
    assert_eq!(harness.session.state(), SessionState::Degraded);
    harness
        .session
        .release(ReleaseReason::Removed, &mut harness.observer);
    assert_eq!(harness.session.state(), SessionState::Absent);
    assert_eq!(harness.session.next_deadline(), None);
    assert_eq!(harness.session.degrade_cause(), None);
}

#[test]
fn retry_delays_double_up_to_the_cap() {
    let delays: Vec<u64> = (0..8)
        .map(|level| degraded_retry_delay(level).as_millis())
        .collect();
    assert_eq!(
        delays,
        vec![
            5_000, 10_000, 20_000, 40_000, 60_000, 60_000, 60_000, 60_000
        ]
    );
    assert_eq!(degraded_retry_delay(u32::MAX), DEGRADED_MAX_RETRY_DELAY);
}

#[test]
fn a_config_change_reaches_the_engine() {
    let mut harness = Harness::active(Mode::RawForward);
    let mut config = Config::default();
    config.pointer.middle_click = false;
    harness
        .session
        .apply_config(Micros::from_millis(1), &config, &mut harness.observer);
    assert!(!harness.session.engine().config().pointer.middle_click);
    assert_eq!(harness.session.state(), SessionState::Active);
}

#[test]
fn status_lines_name_each_state() {
    let mut harness = Harness::new(Mode::RawForward);
    assert_eq!(
        harness.session.status_line(),
        "acquiring: Apple Magic Mouse 2 (Bluetooth) on hidraw5"
    );
    harness.timer(0);
    assert_eq!(
        harness.session.status_line(),
        "active: Apple Magic Mouse 2 (Bluetooth) on hidraw5"
    );
    harness
        .session
        .release(ReleaseReason::DeviceGone, &mut harness.observer);
    assert_eq!(
        harness.session.status_line(),
        "absent: Apple Magic Mouse 2 (Bluetooth) disconnected"
    );
}
