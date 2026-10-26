use std::io::{self, Stderr};
use std::os::fd::{AsFd, BorrowedFd};

use md_core::Mode;
use nix::errno::Errno;
use nix::sys::epoll::{Epoll, EpollCreateFlags, EpollEvent, EpollFlags, EpollTimeout};
use nix::sys::signal::{SigSet, Signal};
use nix::sys::signalfd::{SfdFlags, SignalFd};
use nix::sys::time::TimeSpec;
use nix::sys::timerfd::{ClockId, Expiration, TimerFd, TimerFlags, TimerSetTimeFlags};

use super::debug::TerminalObserver;
use super::{Clock, MonotonicClock, Ready, Runtime, StopSignal, timer_delay};
use crate::discovery::udev_directory::UdevDirectory;
use crate::io::{DeviceDirectory, DeviceError, DeviceOpener, LinuxDevices};

const TOKEN_DIRECTORY: u64 = 0;
const TOKEN_SIGNAL: u64 = 1;
const TOKEN_TIMER: u64 = 2;
const TOKEN_RAW: u64 = 3;
const TOKEN_STOCK: u64 = 4;

#[derive(Debug, thiserror::Error)]
pub enum RunError {
    #[error("cannot set up the event loop: {0}")]
    Setup(#[from] Errno),
    #[error("device discovery failed: {0}")]
    Discovery(#[from] DeviceError),
}

pub fn stop_signal(signo: u32) -> Option<StopSignal> {
    let signal = Signal::try_from(i32::try_from(signo).ok()?).ok()?;
    match signal {
        Signal::SIGINT => Some(StopSignal::Interrupt),
        Signal::SIGTERM => Some(StopSignal::Terminate),
        _ => None,
    }
}

pub fn ready_for(token: u64) -> Option<Ready> {
    match token {
        TOKEN_DIRECTORY => Some(Ready::Directory),
        TOKEN_TIMER => Some(Ready::Timer),
        TOKEN_RAW => Some(Ready::Raw),
        TOKEN_STOCK => Some(Ready::Stock),
        _ => None,
    }
}

fn register(epoll: &Epoll, fd: BorrowedFd<'_>, token: u64) -> Result<(), Errno> {
    let event = EpollEvent::new(EpollFlags::EPOLLIN, token);
    match epoll.add(fd, event) {
        Err(Errno::EEXIST) => epoll.modify(fd, &mut EpollEvent::new(EpollFlags::EPOLLIN, token)),
        result => result,
    }
}

fn sync_session_fds<D: DeviceDirectory, O: DeviceOpener>(
    epoll: &Epoll,
    runtime: &Runtime<D, O>,
    registered: &mut Option<u64>,
) -> Result<(), Errno> {
    let Some(fds) = runtime.session_fds() else {
        *registered = None;
        return Ok(());
    };
    let (Some(raw), Some(stock)) = (fds.raw, fds.stock) else {
        *registered = None;
        return Ok(());
    };
    if *registered != Some(fds.generation) {
        register(epoll, raw, TOKEN_RAW)?;
        register(epoll, stock, TOKEN_STOCK)?;
        *registered = Some(fds.generation);
    }
    Ok(())
}

fn arm_timer(
    timer: &TimerFd,
    clock: &MonotonicClock,
    deadline: Option<md_core::Micros>,
) -> Result<(), Errno> {
    match deadline {
        Some(deadline) => timer.set(
            Expiration::OneShot(TimeSpec::from_duration(timer_delay(clock.now(), deadline))),
            TimerSetTimeFlags::empty(),
        ),
        None => timer.unset(),
    }
}

pub fn run(mode: Mode, debug: bool) -> Result<Option<StopSignal>, RunError> {
    let mut mask = SigSet::empty();
    mask.add(Signal::SIGINT);
    mask.add(Signal::SIGTERM);
    mask.thread_block()?;
    let signals = SignalFd::with_flags(&mask, SfdFlags::SFD_NONBLOCK | SfdFlags::SFD_CLOEXEC)?;
    let timer = TimerFd::new(
        ClockId::CLOCK_MONOTONIC,
        TimerFlags::TFD_NONBLOCK | TimerFlags::TFD_CLOEXEC,
    )?;
    let epoll = Epoll::new(EpollCreateFlags::EPOLL_CLOEXEC)?;
    let directory = UdevDirectory::new()?;
    if let Some(fd) = directory.poll_fd() {
        register(&epoll, fd, TOKEN_DIRECTORY)?;
    }
    register(&epoll, signals.as_fd(), TOKEN_SIGNAL)?;
    register(&epoll, timer.as_fd(), TOKEN_TIMER)?;

    let clock = MonotonicClock::new();
    let mut observer: TerminalObserver<Stderr> = TerminalObserver::new(io::stderr(), debug);
    let mut runtime = Runtime::new(directory, LinuxDevices, mode);
    let result = event_loop(
        &epoll,
        &signals,
        &timer,
        &clock,
        &mut runtime,
        &mut observer,
    );
    runtime.shutdown(&mut observer);
    result.map(|()| runtime.stop_signal())
}

fn event_loop(
    epoll: &Epoll,
    signals: &SignalFd,
    timer: &TimerFd,
    clock: &MonotonicClock,
    runtime: &mut Runtime<UdevDirectory, LinuxDevices>,
    observer: &mut TerminalObserver<Stderr>,
) -> Result<(), RunError> {
    runtime.start(clock.now(), observer)?;
    if runtime.manager().known().is_empty() {
        eprintln!("magic-driver: no Magic Mouse 2 found yet, waiting for it to connect");
    }
    let mut registered = None;
    let mut events = [EpollEvent::empty(); 16];
    while !runtime.is_stopped() {
        sync_session_fds(epoll, runtime, &mut registered)?;
        arm_timer(timer, clock, runtime.next_deadline())?;
        let count = match epoll.wait(&mut events, EpollTimeout::NONE) {
            Ok(count) => count,
            Err(Errno::EINTR) => continue,
            Err(error) => return Err(RunError::Setup(error)),
        };
        let now = clock.now();
        for event in &events[..count] {
            let ready = if event.data() == TOKEN_SIGNAL {
                let Some(info) = signals.read_signal()? else {
                    continue;
                };
                let Some(signal) = stop_signal(info.ssi_signo) else {
                    continue;
                };
                Ready::Signal(signal)
            } else if event.data() == TOKEN_TIMER {
                let _ = timer.wait();
                Ready::Timer
            } else if let Some(ready) = ready_for(event.data()) {
                ready
            } else {
                continue;
            };
            if let Err(error) = runtime.dispatch(now, ready, observer) {
                eprintln!("magic-driver: udev monitor error: {error}");
            }
            if runtime.is_stopped() {
                break;
            }
        }
        runtime.run_due(clock.now(), observer);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sigint_and_sigterm_stop_the_loop() {
        assert_eq!(stop_signal(2), Some(StopSignal::Interrupt));
        assert_eq!(stop_signal(15), Some(StopSignal::Terminate));
        assert_eq!(stop_signal(1), None);
        assert_eq!(stop_signal(10_000), None);
    }

    #[test]
    fn tokens_map_to_ready_sources() {
        assert_eq!(ready_for(TOKEN_DIRECTORY), Some(Ready::Directory));
        assert_eq!(ready_for(TOKEN_TIMER), Some(Ready::Timer));
        assert_eq!(ready_for(TOKEN_RAW), Some(Ready::Raw));
        assert_eq!(ready_for(TOKEN_STOCK), Some(Ready::Stock));
        assert_eq!(ready_for(TOKEN_SIGNAL), None);
        assert_eq!(ready_for(99), None);
    }
}
