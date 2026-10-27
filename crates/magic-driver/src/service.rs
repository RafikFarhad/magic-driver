use std::time::Duration;

use md_core::Micros;
use sd_notify::NotifyState;

pub const MIN_WATCHDOG_PET_INTERVAL: Micros = Micros::from_millis(1);

pub trait ServiceNotifier {
    fn ready(&mut self);

    fn watchdog(&mut self);

    fn status(&mut self, line: &str);

    fn stopping(&mut self);
}

#[derive(Debug, Default)]
pub struct NoServiceManager;

impl ServiceNotifier for NoServiceManager {
    fn ready(&mut self) {}

    fn watchdog(&mut self) {}

    fn status(&mut self, _line: &str) {}

    fn stopping(&mut self) {}
}

#[derive(Debug, Default)]
pub struct SystemdNotifier;

impl SystemdNotifier {
    fn send(states: &[NotifyState<'_>]) {
        if let Err(error) = sd_notify::notify(states) {
            tracing::debug!("cannot notify the service manager: {error}");
        }
    }
}

impl ServiceNotifier for SystemdNotifier {
    fn ready(&mut self) {
        SystemdNotifier::send(&[NotifyState::Ready]);
    }

    fn watchdog(&mut self) {
        SystemdNotifier::send(&[NotifyState::Watchdog]);
    }

    fn status(&mut self, line: &str) {
        SystemdNotifier::send(&[NotifyState::Status(line)]);
    }

    fn stopping(&mut self) {
        SystemdNotifier::send(&[NotifyState::Stopping]);
    }
}

pub fn watchdog_timeout_from_env() -> Option<Duration> {
    sd_notify::watchdog_enabled()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Watchdog {
    interval: Micros,
    next_pet: Micros,
}

impl Watchdog {
    pub fn new(timeout: Duration) -> Option<Watchdog> {
        if timeout.is_zero() {
            return None;
        }
        let half = u64::try_from(timeout.as_micros() / 2).unwrap_or(u64::MAX);
        Some(Watchdog {
            interval: Micros::new(half).max(MIN_WATCHDOG_PET_INTERVAL),
            next_pet: Micros::ZERO,
        })
    }

    pub fn interval(&self) -> Micros {
        self.interval
    }

    pub fn next_pet(&self) -> Micros {
        self.next_pet
    }

    pub fn pet_if_due(&mut self, now: Micros) -> bool {
        if now < self.next_pet {
            return false;
        }
        self.next_pet = now + self.interval;
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_pet_interval_is_half_the_watchdog_timeout() {
        let watchdog = Watchdog::new(Duration::from_secs(2)).unwrap();
        assert_eq!(watchdog.interval(), Micros::from_millis(1_000));
        assert_eq!(watchdog.next_pet(), Micros::ZERO);
        assert_eq!(Watchdog::new(Duration::ZERO), None);
        assert_eq!(
            Watchdog::new(Duration::from_micros(1)).unwrap().interval(),
            MIN_WATCHDOG_PET_INTERVAL
        );
    }

    #[test]
    fn pets_follow_the_schedule_on_a_fake_clock() {
        let mut watchdog = Watchdog::new(Duration::from_secs(2)).unwrap();
        let mut pets = Vec::new();
        for ms in (0..=5_000).step_by(100) {
            if watchdog.pet_if_due(Micros::from_millis(ms)) {
                pets.push(ms);
            }
        }
        assert_eq!(pets, vec![0, 1_000, 2_000, 3_000, 4_000, 5_000]);
    }

    #[test]
    fn a_late_loop_pets_once_and_reschedules_from_now() {
        let mut watchdog = Watchdog::new(Duration::from_secs(2)).unwrap();
        assert!(watchdog.pet_if_due(Micros::ZERO));
        assert!(!watchdog.pet_if_due(Micros::from_millis(999)));
        assert!(watchdog.pet_if_due(Micros::from_millis(1_700)));
        assert_eq!(watchdog.next_pet(), Micros::from_millis(2_700));
        assert!(!watchdog.pet_if_due(Micros::from_millis(1_700)));
    }

    #[test]
    fn no_service_manager_ignores_everything() {
        let mut notifier = NoServiceManager;
        notifier.ready();
        notifier.watchdog();
        notifier.status("active");
        notifier.stopping();
    }
}
