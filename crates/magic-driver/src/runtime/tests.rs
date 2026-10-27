use std::path::PathBuf;
use std::time::Duration;

use md_core::config::Config;
use md_core::profile::MAGIC_MOUSE_2_BT;
use md_core::synth::Frame;
use md_core::testkit::{ReportBuilder, TouchRecord, touch_state};
use md_core::{Button, Micros, Mode};
use nix::errno::Errno;

use super::*;
use crate::discovery::MouseNodes;
use crate::io::fake::{Call, FakeDirectory, FakeOpener, SharedWorld, world};
use crate::session::{NoObserver, SessionState};

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
struct FakeClock {
    now: std::cell::Cell<u64>,
}

impl FakeClock {
    fn advance_ms(&self, ms: u64) {
        self.now.set(self.now.get() + ms * 1_000);
    }
}

impl Clock for FakeClock {
    fn now(&self) -> Micros {
        Micros::new(self.now.get())
    }
}

struct Harness {
    world: SharedWorld,
    clock: FakeClock,
    runtime: Runtime<FakeDirectory, FakeOpener>,
}

impl Harness {
    fn new(mode: Mode) -> Harness {
        let world = world();
        Harness {
            runtime: Runtime::new(
                FakeDirectory {
                    world: world.clone(),
                },
                FakeOpener {
                    world: world.clone(),
                },
                mode,
                Config::default(),
            ),
            world,
            clock: FakeClock::default(),
        }
    }

    fn start(&mut self) {
        self.runtime
            .start(self.clock.now(), &mut NoObserver)
            .unwrap();
    }

    fn dispatch(&mut self, ready: Ready) {
        self.runtime
            .dispatch(self.clock.now(), ready, &mut NoObserver)
            .unwrap();
        self.runtime.run_due(self.clock.now(), &mut NoObserver);
    }

    fn fire_timer_at_deadline(&mut self) -> Option<Micros> {
        let deadline = self.runtime.next_deadline()?;
        let delay = timer_delay(self.clock.now(), deadline);
        self.clock
            .now
            .set(self.clock.now.get() + u64::try_from(delay.as_micros()).unwrap());
        self.dispatch(Ready::Timer);
        Some(deadline)
    }

    fn state(&self) -> Option<SessionState> {
        self.runtime
            .manager()
            .session()
            .map(crate::session::Session::state)
    }

    fn take_calls(&self) -> Vec<Call> {
        std::mem::take(&mut self.world.borrow_mut().calls)
    }

    fn udev(&mut self, event: DiscoveryEvent) {
        self.world.borrow_mut().udev_changes.push_back(event);
        self.dispatch(Ready::Directory);
    }
}

#[test]
fn timer_delay_is_positive_and_saturates() {
    assert_eq!(
        timer_delay(Micros::new(1_000), Micros::new(51_000)),
        Duration::from_millis(50)
    );
    assert_eq!(
        timer_delay(Micros::new(10), Micros::new(10)),
        MIN_TIMER_DELAY
    );
    assert_eq!(
        timer_delay(Micros::new(20), Micros::new(10)),
        MIN_TIMER_DELAY
    );
    assert_eq!(
        timer_delay(Micros::ZERO, Micros::MAX),
        Duration::from_micros(u64::MAX)
    );
}

#[test]
fn monotonic_clock_never_goes_backwards() {
    let clock = MonotonicClock::new();
    let first = clock.now();
    let second = clock.now();
    assert!(second >= first);
}

#[test]
fn mouse_present_at_start_is_taken_over_immediately() {
    let mut harness = Harness::new(Mode::RawForward);
    harness
        .world
        .borrow_mut()
        .enumerated
        .push(DiscoveryEvent::MouseAppeared(nodes(5)));
    harness.start();
    assert_eq!(harness.state(), Some(SessionState::Active));
    assert_eq!(harness.take_calls().last(), Some(&Call::Grab));
    assert_eq!(harness.runtime.next_deadline(), None);
}

#[test]
fn no_mouse_at_start_waits_without_a_deadline() {
    let mut harness = Harness::new(Mode::RawForward);
    harness.start();
    assert_eq!(harness.state(), None);
    assert_eq!(harness.runtime.next_deadline(), None);
    assert!(harness.take_calls().is_empty());
}

#[test]
fn udev_add_after_start_takes_over_and_retries_follow_the_timer() {
    let mut harness = Harness::new(Mode::RawForward);
    harness.start();
    harness
        .world
        .borrow_mut()
        .open_raw_errors
        .extend([Errno::EACCES, Errno::EACCES]);
    harness.clock.advance_ms(1_000);
    harness.udev(DiscoveryEvent::MouseAppeared(nodes(5)));
    assert_eq!(harness.state(), Some(SessionState::Acquiring));
    assert_eq!(
        harness.runtime.next_deadline(),
        Some(Micros::from_millis(1_050))
    );
    assert_eq!(
        harness.fire_timer_at_deadline(),
        Some(Micros::from_millis(1_050))
    );
    assert_eq!(
        harness.runtime.next_deadline(),
        Some(Micros::from_millis(1_150))
    );
    assert_eq!(
        harness.fire_timer_at_deadline(),
        Some(Micros::from_millis(1_150))
    );
    assert_eq!(harness.state(), Some(SessionState::Active));
    assert_eq!(harness.runtime.next_deadline(), None);
}

#[test]
fn an_early_timer_wake_does_not_attempt_acquisition() {
    let mut harness = Harness::new(Mode::RawForward);
    harness
        .world
        .borrow_mut()
        .open_raw_errors
        .push_back(Errno::EACCES);
    harness
        .world
        .borrow_mut()
        .enumerated
        .push(DiscoveryEvent::MouseAppeared(nodes(5)));
    harness.start();
    harness.clock.advance_ms(10);
    harness.dispatch(Ready::Timer);
    assert_eq!(harness.state(), Some(SessionState::Acquiring));
    assert!(harness.take_calls().is_empty());
    harness.clock.advance_ms(40);
    harness.dispatch(Ready::Timer);
    assert_eq!(harness.state(), Some(SessionState::Active));
}

#[test]
fn signal_releases_in_order_and_stops() {
    let mut harness = Harness::new(Mode::ScrollProbe);
    harness
        .world
        .borrow_mut()
        .enumerated
        .push(DiscoveryEvent::MouseAppeared(nodes(5)));
    harness.start();
    let finger = TouchRecord::new(1, 0, 0).size(4);
    harness.world.borrow_mut().raw_reports.extend([
        Ok(ReportBuilder::new()
            .touch(finger.state(touch_state::START))
            .build()),
        Ok(ReportBuilder::new()
            .left(true)
            .touch(TouchRecord::new(1, 0, 80).size(4))
            .build()),
    ]);
    harness.dispatch(Ready::Raw);
    harness.take_calls();
    harness.dispatch(Ready::Signal(StopSignal::Interrupt));
    assert!(harness.runtime.is_stopped());
    assert_eq!(harness.runtime.stop_signal(), Some(StopSignal::Interrupt));
    let calls = harness.take_calls();
    assert_eq!(calls[0], Call::Ungrab);
    assert!(calls.contains(&Call::Button {
        button: Button::Left,
        pressed: false
    }));
    assert!(calls.contains(&Call::Frame(Frame::EMPTY)));
    assert_eq!(
        calls[calls.len() - 3..],
        [Call::DestroyTouchpad, Call::DestroyPointer, Call::CloseRaw]
    );
    assert_eq!(harness.state(), None);
    assert_eq!(harness.runtime.next_deadline(), None);
    harness.dispatch(Ready::Raw);
    harness.runtime.shutdown(&mut NoObserver);
    assert!(harness.take_calls().is_empty());
}

#[test]
fn removal_releases_and_reconnect_takes_over_again() {
    let mut harness = Harness::new(Mode::RawForward);
    harness
        .world
        .borrow_mut()
        .enumerated
        .push(DiscoveryEvent::MouseAppeared(nodes(5)));
    harness.start();
    let first_generation = harness.runtime.session_fds().unwrap().generation;
    harness
        .world
        .borrow_mut()
        .raw_reports
        .push_back(Err(Errno::ENODEV));
    harness.dispatch(Ready::Raw);
    assert_eq!(harness.state(), Some(SessionState::Absent));
    harness.udev(DiscoveryEvent::MouseGone { key: nodes(5).key });
    assert_eq!(harness.state(), None);
    harness.take_calls();

    harness.clock.advance_ms(3_000);
    harness.udev(DiscoveryEvent::MouseAppeared(nodes(6)));
    assert_eq!(harness.state(), Some(SessionState::Active));
    assert_eq!(
        harness.take_calls(),
        vec![
            Call::OpenRaw(PathBuf::from("/dev/hidraw6")),
            Call::CreatePointer,
            Call::CreateTouchpad,
            Call::OpenStock(PathBuf::from("/dev/input/event16")),
            Call::Grab,
        ]
    );
    assert_ne!(
        harness.runtime.session_fds().unwrap().generation,
        first_generation
    );
}

#[test]
fn reconnect_under_the_same_key_before_removal_replaces_the_absent_session() {
    let mut harness = Harness::new(Mode::RawForward);
    harness
        .world
        .borrow_mut()
        .enumerated
        .push(DiscoveryEvent::MouseAppeared(nodes(5)));
    harness.start();
    harness
        .world
        .borrow_mut()
        .raw_reports
        .push_back(Err(Errno::EIO));
    harness.dispatch(Ready::Raw);
    assert_eq!(harness.state(), Some(SessionState::Absent));
    harness.take_calls();
    let mut renumbered = nodes(5);
    renumbered.hidraw = PathBuf::from("/dev/hidraw9");
    harness.udev(DiscoveryEvent::MouseAppeared(renumbered));
    assert_eq!(harness.state(), Some(SessionState::Active));
    assert_eq!(
        harness.take_calls()[0],
        Call::OpenRaw(PathBuf::from("/dev/hidraw9"))
    );
}

#[test]
fn first_matching_mouse_wins_and_the_second_takes_over_when_it_leaves() {
    let mut harness = Harness::new(Mode::RawForward);
    harness.world.borrow_mut().enumerated.extend([
        DiscoveryEvent::MouseAppeared(nodes(5)),
        DiscoveryEvent::MouseAppeared(nodes(6)),
    ]);
    harness.start();
    let session = harness.runtime.manager().session().unwrap();
    assert_eq!(session.nodes(), &nodes(5));
    assert_eq!(harness.runtime.manager().known().len(), 2);
    let calls = harness.take_calls();
    assert_eq!(calls.iter().filter(|call| **call == Call::Grab).count(), 1);

    harness.udev(DiscoveryEvent::MouseGone { key: nodes(5).key });
    let session = harness.runtime.manager().session().unwrap();
    assert_eq!(session.nodes(), &nodes(6));
    assert_eq!(session.state(), SessionState::Active);
    let calls = harness.take_calls();
    let close = calls
        .iter()
        .position(|call| *call == Call::CloseRaw)
        .unwrap();
    let open = calls
        .iter()
        .position(|call| *call == Call::OpenRaw(PathBuf::from("/dev/hidraw6")))
        .unwrap();
    assert!(close < open);
}

#[test]
fn duplicate_appearances_do_not_disturb_an_active_session() {
    let mut harness = Harness::new(Mode::RawForward);
    harness
        .world
        .borrow_mut()
        .enumerated
        .push(DiscoveryEvent::MouseAppeared(nodes(5)));
    harness.start();
    harness.take_calls();
    harness.udev(DiscoveryEvent::MouseAppeared(nodes(5)));
    assert!(harness.take_calls().is_empty());
    assert_eq!(harness.state(), Some(SessionState::Active));
}

#[test]
fn changed_nodes_for_the_active_mouse_restart_the_session() {
    let mut harness = Harness::new(Mode::RawForward);
    harness
        .world
        .borrow_mut()
        .enumerated
        .push(DiscoveryEvent::MouseAppeared(nodes(5)));
    harness.start();
    harness.take_calls();
    let first_generation = harness.runtime.session_fds().unwrap().generation;
    let mut renumbered = nodes(5);
    renumbered.evdev = PathBuf::from("/dev/input/event40");
    harness.udev(DiscoveryEvent::MouseAppeared(renumbered));
    let calls = harness.take_calls();
    assert_eq!(calls[0], Call::Ungrab);
    assert!(calls.contains(&Call::OpenStock(PathBuf::from("/dev/input/event40"))));
    assert_eq!(harness.state(), Some(SessionState::Active));
    assert_ne!(
        harness.runtime.session_fds().unwrap().generation,
        first_generation
    );
}

#[test]
fn a_change_event_restarts_acquisition_after_giving_up() {
    let mut harness = Harness::new(Mode::RawForward);
    harness
        .world
        .borrow_mut()
        .open_raw_errors
        .extend(std::iter::repeat_n(Errno::EACCES, 7));
    harness
        .world
        .borrow_mut()
        .enumerated
        .push(DiscoveryEvent::MouseAppeared(nodes(5)));
    harness.start();
    while harness.state() == Some(SessionState::Acquiring) {
        harness.fire_timer_at_deadline();
    }
    assert_eq!(harness.state(), Some(SessionState::Degraded));
    harness.udev(DiscoveryEvent::MouseAppeared(nodes(5)));
    assert_eq!(harness.state(), Some(SessionState::Active));
}

#[test]
fn removal_of_an_unknown_or_inactive_mouse_is_ignored() {
    let mut harness = Harness::new(Mode::RawForward);
    harness
        .world
        .borrow_mut()
        .enumerated
        .push(DiscoveryEvent::MouseAppeared(nodes(5)));
    harness.start();
    harness.take_calls();
    harness.udev(DiscoveryEvent::MouseGone { key: nodes(7).key });
    assert!(harness.take_calls().is_empty());
    assert_eq!(harness.state(), Some(SessionState::Active));
}

#[test]
fn removal_during_acquisition_cancels_the_retry_timer() {
    let mut harness = Harness::new(Mode::RawForward);
    harness
        .world
        .borrow_mut()
        .open_raw_errors
        .push_back(Errno::EACCES);
    harness
        .world
        .borrow_mut()
        .enumerated
        .push(DiscoveryEvent::MouseAppeared(nodes(5)));
    harness.start();
    assert!(harness.runtime.next_deadline().is_some());
    harness.udev(DiscoveryEvent::MouseGone { key: nodes(5).key });
    assert_eq!(harness.runtime.next_deadline(), None);
    assert_eq!(harness.state(), None);
}
