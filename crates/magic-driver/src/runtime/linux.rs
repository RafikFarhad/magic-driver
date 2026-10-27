use std::collections::HashMap;
use std::env;
use std::os::fd::{AsFd, BorrowedFd};

use md_core::profile::MAGIC_MOUSE_2_BT;
use md_core::{Micros, Mode};
use nix::errno::Errno;
use nix::sys::epoll::{Epoll, EpollCreateFlags, EpollEvent, EpollFlags, EpollTimeout};
use nix::sys::signal::{SigSet, Signal};
use nix::sys::signalfd::{SfdFlags, SignalFd};
use nix::sys::time::TimeSpec;
use nix::sys::timerfd::{ClockId, Expiration, TimerFd, TimerFlags, TimerSetTimeFlags};

use super::debug::LogObserver;
use super::{Clock, MonotonicClock, Ready, Runtime, StopSignal, timer_delay};
use crate::config_file::default_config_path;
use crate::daemon_config::{ConfigSource, initial_config};
use crate::discovery::udev_directory::UdevDirectory;
use crate::io::{DeviceDirectory, DeviceError, DeviceOpener, LinuxDevices};
use crate::logging::Logging;
use crate::service::{SystemdNotifier, Watchdog, watchdog_timeout_from_env};
use crate::session::{SessionEvent, SessionObserver};
use crate::trace::server::{Admission, TraceServer, socket_path};
use crate::trace::{hello, session_record};

const TOKEN_DIRECTORY: u64 = 0;
const TOKEN_SIGNAL: u64 = 1;
const TOKEN_TIMER: u64 = 2;
const TOKEN_RAW: u64 = 3;
const TOKEN_STOCK: u64 = 4;
const TOKEN_TRACE_LISTENER: u64 = 5;
const TOKEN_SUBSCRIBER_BASE: u64 = 1 << 32;

#[derive(Debug, thiserror::Error)]
pub enum RunError {
    #[error("cannot set up the event loop: {0}")]
    Setup(#[from] Errno),
    #[error("device discovery failed: {0}")]
    Discovery(#[from] DeviceError),
}

pub fn signal_ready(signo: u32) -> Option<Ready> {
    let signal = Signal::try_from(i32::try_from(signo).ok()?).ok()?;
    match signal {
        Signal::SIGINT => Some(Ready::Signal(StopSignal::Interrupt)),
        Signal::SIGTERM => Some(Ready::Signal(StopSignal::Terminate)),
        Signal::SIGHUP => Some(Ready::Reload),
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

pub struct DaemonObserver {
    log: LogObserver,
    trace: Option<TraceServer>,
    debug: bool,
    now: Micros,
}

impl DaemonObserver {
    pub fn new(debug: bool, trace: Option<TraceServer>) -> DaemonObserver {
        DaemonObserver {
            log: LogObserver::new(debug),
            trace,
            debug,
            now: Micros::ZERO,
        }
    }

    pub fn wants_tracing(&self) -> bool {
        self.debug || self.has_subscribers()
    }

    pub fn set_now(&mut self, now: Micros) {
        self.now = now;
    }

    pub fn trace(&self) -> Option<&TraceServer> {
        self.trace.as_ref()
    }

    pub fn trace_mut(&mut self) -> Option<&mut TraceServer> {
        self.trace.as_mut()
    }

    pub fn has_subscribers(&self) -> bool {
        self.trace
            .as_ref()
            .is_some_and(TraceServer::has_subscribers)
    }
}

impl SessionObserver for DaemonObserver {
    fn observe(&mut self, event: &SessionEvent<'_>) {
        self.log.observe(event);
        let Some(server) = self.trace.as_mut() else {
            return;
        };
        if !server.has_subscribers() {
            return;
        }
        match event {
            SessionEvent::Trace { record } => {
                server.publish(record);
            }
            event => {
                if let Some(record) = session_record(self.now, event) {
                    server.publish(&record);
                }
            }
        }
    }
}

fn open_trace_server() -> Option<TraceServer> {
    let path = match socket_path(env::var_os("XDG_RUNTIME_DIR").as_deref()) {
        Ok(path) => path,
        Err(error) => {
            tracing::warn!("trace socket disabled: {error}");
            return None;
        }
    };
    match TraceServer::bind(&path, nix::unistd::getuid().as_raw()) {
        Ok(server) => {
            tracing::debug!("trace socket at {}", path.display());
            Some(server)
        }
        Err(error) => {
            tracing::warn!("trace socket disabled: {error}");
            None
        }
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

fn sync_trace_fds(
    epoll: &Epoll,
    server: Option<&TraceServer>,
    registered: &mut HashMap<u64, bool>,
) -> Result<(), Errno> {
    let Some(server) = server else {
        registered.clear();
        return Ok(());
    };
    let mut present = Vec::with_capacity(server.subscriber_count());
    for (id, fd, wants_write) in server.subscriber_fds() {
        present.push(id);
        let mut flags = EpollFlags::EPOLLIN | EpollFlags::EPOLLRDHUP;
        if wants_write {
            flags |= EpollFlags::EPOLLOUT;
        }
        let mut event = EpollEvent::new(flags, TOKEN_SUBSCRIBER_BASE + id);
        match registered.get(&id) {
            None => {
                epoll.add(fd, event)?;
                registered.insert(id, wants_write);
            }
            Some(previous) if *previous != wants_write => {
                epoll.modify(fd, &mut event)?;
                registered.insert(id, wants_write);
            }
            Some(_) => {}
        }
    }
    registered.retain(|id, _| present.contains(id));
    Ok(())
}

fn handle_trace_event<D: DeviceDirectory, O: DeviceOpener>(
    token: u64,
    runtime: &Runtime<D, O>,
    observer: &mut DaemonObserver,
) {
    let Some(server) = observer.trace_mut() else {
        return;
    };
    if token == TOKEN_TRACE_LISTENER {
        let profile = runtime.manager().profile().unwrap_or(&MAGIC_MOUSE_2_BT);
        let hello = hello(profile, runtime.config(), &runtime.manager().status_line());
        for admission in server.accept_pending(&hello) {
            match admission {
                Admission::Accepted(id) => tracing::debug!("trace subscriber {id} connected"),
                Admission::WrongUser { uid } => {
                    tracing::warn!("trace subscriber from uid {uid} rejected");
                }
                Admission::Full => {
                    tracing::warn!("trace subscriber rejected: too many subscribers");
                }
            }
        }
    } else if let Some(reason) = server.on_ready(token - TOKEN_SUBSCRIBER_BASE) {
        tracing::debug!(
            "trace subscriber {} disconnected: {reason:?}",
            token - TOKEN_SUBSCRIBER_BASE
        );
    }
}

fn arm_timer(
    timer: &TimerFd,
    clock: &MonotonicClock,
    deadline: Option<Micros>,
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
    let mut source = default_config_path(env::var_os("XDG_CONFIG_HOME"), env::var_os("HOME"))
        .map(ConfigSource::new);
    let outcome = source.as_ref().map(ConfigSource::load);
    let config = outcome
        .as_ref()
        .map_or_else(md_core::config::Config::default, initial_config);
    let logging = Logging::init(config.log.level, debug);
    let debug_note = if debug { " with --debug" } else { "" };
    tracing::info!(
        "starting magic-driver {}{debug_note}",
        env!("CARGO_PKG_VERSION")
    );
    if let (Some(source), Some(outcome)) = (source.as_mut(), outcome.as_ref()) {
        source.report_initial(outcome);
    } else {
        tracing::warn!(
            "cannot locate the config file: neither XDG_CONFIG_HOME nor HOME is set; using defaults"
        );
    }

    let mut mask = SigSet::empty();
    mask.add(Signal::SIGINT);
    mask.add(Signal::SIGTERM);
    mask.add(Signal::SIGHUP);
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

    let watchdog = watchdog_timeout_from_env().and_then(Watchdog::new);
    if let Some(watchdog) = watchdog {
        tracing::debug!(
            "systemd watchdog enabled, petting every {} ms",
            watchdog.interval().as_millis()
        );
    }
    let trace = open_trace_server();
    if let Some(fd) = trace.as_ref().and_then(TraceServer::listener_fd) {
        register(&epoll, fd, TOKEN_TRACE_LISTENER)?;
    }
    let clock = MonotonicClock::new();
    let mut observer = DaemonObserver::new(debug, trace);
    let mut runtime = Runtime::new(directory, LinuxDevices, mode, config)
        .with_service(Box::new(SystemdNotifier), watchdog);
    if let Some(source) = source {
        runtime = runtime.with_config_source(source);
    }
    runtime.set_tracing(observer.wants_tracing());
    let result = event_loop(
        &epoll,
        &signals,
        &timer,
        &clock,
        &logging,
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
    logging: &Logging,
    runtime: &mut Runtime<UdevDirectory, LinuxDevices>,
    observer: &mut DaemonObserver,
) -> Result<(), RunError> {
    observer.set_now(clock.now());
    runtime.start(clock.now(), observer)?;
    if runtime.manager().known().is_empty() {
        tracing::info!("no Magic Mouse 2 found yet, waiting for it to connect");
    }
    let mut registered = None;
    let mut subscribers = HashMap::new();
    let mut events = [EpollEvent::empty(); 16];
    while !runtime.is_stopped() {
        sync_session_fds(epoll, runtime, &mut registered)?;
        sync_trace_fds(epoll, observer.trace(), &mut subscribers)?;
        runtime.set_tracing(observer.wants_tracing());
        arm_timer(timer, clock, runtime.next_deadline())?;
        let count = match epoll.wait(&mut events, EpollTimeout::NONE) {
            Ok(count) => count,
            Err(Errno::EINTR) => continue,
            Err(error) => return Err(RunError::Setup(error)),
        };
        let now = clock.now();
        observer.set_now(now);
        for event in &events[..count] {
            if event.data() == TOKEN_TRACE_LISTENER || event.data() >= TOKEN_SUBSCRIBER_BASE {
                handle_trace_event(event.data(), runtime, observer);
                continue;
            }
            let ready = if event.data() == TOKEN_SIGNAL {
                let Some(info) = signals.read_signal()? else {
                    continue;
                };
                let Some(ready) = signal_ready(info.ssi_signo) else {
                    continue;
                };
                ready
            } else if event.data() == TOKEN_TIMER {
                let _ = timer.wait();
                Ready::Timer
            } else if let Some(ready) = ready_for(event.data()) {
                ready
            } else {
                continue;
            };
            if let Err(error) = runtime.dispatch(now, ready, observer) {
                tracing::warn!("udev monitor error: {error}");
            }
            if ready == Ready::Reload {
                logging.set_level(runtime.config().log.level);
            }
            if runtime.is_stopped() {
                break;
            }
        }
        observer.set_now(clock.now());
        runtime.run_due(clock.now(), observer);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sigint_and_sigterm_stop_the_loop_and_sighup_reloads() {
        assert_eq!(signal_ready(2), Some(Ready::Signal(StopSignal::Interrupt)));
        assert_eq!(signal_ready(15), Some(Ready::Signal(StopSignal::Terminate)));
        assert_eq!(signal_ready(1), Some(Ready::Reload));
        assert_eq!(signal_ready(10), None);
        assert_eq!(signal_ready(10_000), None);
    }

    #[test]
    fn tokens_map_to_ready_sources() {
        assert_eq!(ready_for(TOKEN_DIRECTORY), Some(Ready::Directory));
        assert_eq!(ready_for(TOKEN_TIMER), Some(Ready::Timer));
        assert_eq!(ready_for(TOKEN_RAW), Some(Ready::Raw));
        assert_eq!(ready_for(TOKEN_STOCK), Some(Ready::Stock));
        assert_eq!(ready_for(TOKEN_SIGNAL), None);
        assert_eq!(ready_for(TOKEN_TIMER), Some(Ready::Timer));
        assert_eq!(ready_for(99), None);
    }
}
