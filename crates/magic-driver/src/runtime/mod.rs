pub mod debug;
pub mod linux;

use std::time::{Duration, Instant};

use md_core::{Micros, Mode};

use crate::discovery::DiscoveryEvent;
use crate::io::{DeviceDirectory, DeviceError, DeviceOpener};
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
    Timer,
    Raw,
    Stock,
}

pub struct Runtime<D: DeviceDirectory, O: DeviceOpener> {
    directory: D,
    manager: SessionManager<O>,
    discovered: Vec<DiscoveryEvent>,
    stop: Option<StopSignal>,
}

impl<D: DeviceDirectory, O: DeviceOpener> std::fmt::Debug for Runtime<D, O> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Runtime")
            .field("manager", &self.manager)
            .field("stop", &self.stop)
            .finish_non_exhaustive()
    }
}

impl<D: DeviceDirectory, O: DeviceOpener> Runtime<D, O> {
    pub fn new(directory: D, opener: O, mode: Mode) -> Runtime<D, O> {
        Runtime {
            directory,
            manager: SessionManager::new(opener, mode),
            discovered: Vec::new(),
            stop: None,
        }
    }

    pub fn manager(&self) -> &SessionManager<O> {
        &self.manager
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
        self.manager.next_deadline()
    }

    pub fn start(
        &mut self,
        now: Micros,
        observer: &mut dyn SessionObserver,
    ) -> Result<(), DeviceError> {
        self.discovered.clear();
        self.directory.enumerate(&mut self.discovered)?;
        self.apply_discovered(now, observer);
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
        match ready {
            Ready::Directory => {
                self.discovered.clear();
                let result = self.directory.changes(&mut self.discovered);
                self.apply_discovered(now, observer);
                result?;
            }
            Ready::Signal(signal) => {
                self.stop = Some(signal);
                self.manager.shutdown(observer);
            }
            Ready::Timer => self.run_due(now, observer),
            Ready::Raw => self.manager.on_raw_readable(now, observer),
            Ready::Stock => self.manager.on_stock_readable(now, observer),
        }
        Ok(())
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
    }

    pub fn shutdown(&mut self, observer: &mut dyn SessionObserver) {
        self.manager.shutdown(observer);
    }
}

#[cfg(test)]
mod tests;
