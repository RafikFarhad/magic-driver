pub mod debug;
pub mod linux;

use std::time::{Duration, Instant};

use md_core::config::Config;
use md_core::{Micros, Mode};

use crate::daemon_config::ConfigSource;
use crate::discovery::DiscoveryEvent;
use crate::io::{DeviceDirectory, DeviceError, DeviceOpener};
use crate::service::{NoServiceManager, ServiceNotifier, Watchdog};
use crate::session::manager::SessionManager;
use crate::session::{SessionFds, SessionObserver};

pub const MIN_TIMER_DELAY: Duration = Duration::from_micros(1);

pub trait Clock {
    fn now(&self) -> Micros;
}

#[derive(Debug, Clone, Copy)]
pub struct MonotonicClock {
    start: Instant,
}

impl MonotonicClock {
    pub fn new() -> MonotonicClock {
        MonotonicClock {
            start: Instant::now(),
        }
    }
}

impl Clock for MonotonicClock {
    fn now(&self) -> Micros {
        Micros::new(u64::try_from(self.start.elapsed().as_micros()).unwrap_or(u64::MAX))
    }
}

pub fn timer_delay(now: Micros, deadline: Micros) -> Duration {
    Duration::from_micros(deadline.saturating_sub(now).as_u64()).max(MIN_TIMER_DELAY)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StopSignal {
    Interrupt,
    Terminate,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Ready {
    Directory,
    Signal(StopSignal),
    Reload,
    Timer,
    Raw,
    Stock,
}

pub struct Runtime<D: DeviceDirectory, O: DeviceOpener> {
    directory: D,
    manager: SessionManager<O>,
    discovered: Vec<DiscoveryEvent>,
    stop: Option<StopSignal>,
    notifier: Box<dyn ServiceNotifier>,
    watchdog: Option<Watchdog>,
    published_status: Option<String>,
    config_source: Option<ConfigSource>,
}

impl<D: DeviceDirectory, O: DeviceOpener> std::fmt::Debug for Runtime<D, O> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Runtime")
            .field("manager", &self.manager)
            .field("stop", &self.stop)
            .field("watchdog", &self.watchdog)
            .field("published_status", &self.published_status)
            .finish_non_exhaustive()
    }
}

impl<D: DeviceDirectory, O: DeviceOpener> Runtime<D, O> {
    pub fn new(directory: D, opener: O, mode: Mode, config: Config) -> Runtime<D, O> {
        Runtime {
            directory,
            manager: SessionManager::new(opener, mode, config),
            discovered: Vec::new(),
            stop: None,
            notifier: Box::new(NoServiceManager),
            watchdog: None,
            published_status: None,
            config_source: None,
        }
    }

    #[must_use]
    pub fn with_service(
        mut self,
        notifier: Box<dyn ServiceNotifier>,
        watchdog: Option<Watchdog>,
    ) -> Runtime<D, O> {
        self.notifier = notifier;
        self.watchdog = watchdog;
        self
    }

    #[must_use]
    pub fn with_config_source(mut self, source: ConfigSource) -> Runtime<D, O> {
        self.config_source = Some(source);
        self
    }

    pub fn config(&self) -> &Config {
        self.manager.config()
    }

    pub fn manager(&self) -> &SessionManager<O> {
        &self.manager
    }

    pub fn set_tracing(&mut self, enabled: bool) {
        if self.manager.is_tracing() != enabled {
            self.manager.set_tracing(enabled);
        }
    }

    pub fn session_fds(&self) -> Option<SessionFds<'_>> {
        self.manager.fds()
    }

    pub fn stop_signal(&self) -> Option<StopSignal> {
        self.stop
    }

    pub fn is_stopped(&self) -> bool {
        self.stop.is_some()
    }

    pub fn next_deadline(&self) -> Option<Micros> {
        if self.is_stopped() {
            return None;
        }
        let pet = self.watchdog.map(|watchdog| watchdog.next_pet());
        match (self.manager.next_deadline(), pet) {
            (Some(session), Some(pet)) => Some(session.min(pet)),
            (session, pet) => session.or(pet),
        }
    }

    fn publish_status(&mut self) {
        let line = self.manager.status_line();
        if self.published_status.as_deref() != Some(line.as_str()) {
            self.notifier.status(&line);
            self.published_status = Some(line);
        }
    }

    fn pet_watchdog(&mut self, now: Micros) {
        if self
            .watchdog
            .as_mut()
            .is_some_and(|watchdog| watchdog.pet_if_due(now))
        {
            self.notifier.watchdog();
        }
    }

    pub fn reload_config(&mut self, now: Micros, observer: &mut dyn SessionObserver) {
        let Some(source) = self.config_source.as_mut() else {
            tracing::error!("config reload requested but no config file location is known");
            return;
        };
        let outcome = source.load();
        if let Some(config) = source.reloaded(outcome) {
            self.manager.apply_config(now, config, observer);
        }
    }

    pub fn start(
        &mut self,
        now: Micros,
        observer: &mut dyn SessionObserver,
    ) -> Result<(), DeviceError> {
        self.discovered.clear();
        self.directory.enumerate(&mut self.discovered)?;
        self.apply_discovered(now, observer);
        self.notifier.ready();
        self.publish_status();
        self.pet_watchdog(now);
        Ok(())
    }

    fn apply_discovered(&mut self, now: Micros, observer: &mut dyn SessionObserver) {
        for event in self.discovered.drain(..) {
            self.manager.on_discovery(now, event, observer);
        }
    }

    pub fn dispatch(
        &mut self,
        now: Micros,
        ready: Ready,
        observer: &mut dyn SessionObserver,
    ) -> Result<(), DeviceError> {
        if self.is_stopped() {
            return Ok(());
        }
        let mut outcome = Ok(());
        match ready {
            Ready::Directory => {
                self.discovered.clear();
                outcome = self.directory.changes(&mut self.discovered);
                self.apply_discovered(now, observer);
            }
            Ready::Signal(signal) => {
                self.stop = Some(signal);
                self.notifier.stopping();
                self.manager.shutdown(observer);
            }
            Ready::Reload => self.reload_config(now, observer),
            Ready::Timer => self.run_due(now, observer),
            Ready::Raw => self.manager.on_raw_readable(now, observer),
            Ready::Stock => self.manager.on_stock_readable(now, observer),
        }
        self.publish_status();
        outcome
    }

    pub fn run_due(&mut self, now: Micros, observer: &mut dyn SessionObserver) {
        if self.is_stopped() {
            return;
        }
        if self
            .manager
            .next_deadline()
            .is_some_and(|deadline| deadline <= now)
        {
            self.manager.on_timer(now, observer);
        }
        self.pet_watchdog(now);
        self.publish_status();
    }

    pub fn shutdown(&mut self, observer: &mut dyn SessionObserver) {
        self.manager.shutdown(observer);
    }
}

#[cfg(test)]
mod robustness_tests;
#[cfg(test)]
mod tests;
