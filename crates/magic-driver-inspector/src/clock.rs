use std::time::Instant;

#[derive(Debug, Clone, Copy)]
pub struct LocalClock {
    start: Instant,
}

impl LocalClock {
    pub fn new() -> LocalClock {
        LocalClock {
            start: Instant::now(),
        }
    }

    pub fn now_us(&self) -> u64 {
        u64::try_from(self.start.elapsed().as_micros()).unwrap_or(u64::MAX)
    }
}

impl Default for LocalClock {
    fn default() -> Self {
        LocalClock::new()
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ClockAlignment {
    offset: Option<i128>,
}

impl ClockAlignment {
    pub fn new() -> ClockAlignment {
        ClockAlignment::default()
    }

    pub fn observe(&mut self, remote_us: u64, local_us: u64) {
        let offset = i128::from(local_us) - i128::from(remote_us);
        self.offset = Some(self.offset.map_or(offset, |current| current.min(offset)));
    }

    pub const fn is_known(&self) -> bool {
        self.offset.is_some()
    }

    pub fn to_local(&self, remote_us: u64) -> Option<u64> {
        let offset = self.offset?;
        u64::try_from((i128::from(remote_us) + offset).max(0)).ok()
    }

    pub fn to_remote(&self, local_us: u64) -> Option<u64> {
        let offset = self.offset?;
        u64::try_from((i128::from(local_us) - offset).max(0)).ok()
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct WrappingMillis {
    last: Option<u64>,
}

const WRAP: u64 = 1 << 32;
const HALF_WRAP: u64 = 1 << 31;

impl WrappingMillis {
    pub fn new() -> WrappingMillis {
        WrappingMillis::default()
    }

    pub fn extend(&mut self, ms: u32) -> u64 {
        let ms = u64::from(ms);
        let extended = match self.last {
            None => ms,
            Some(last) => {
                let base = last & !(WRAP - 1);
                let candidate = base | ms;
                if candidate + HALF_WRAP < last {
                    candidate + WRAP
                } else if candidate > last + HALF_WRAP && candidate >= WRAP {
                    candidate - WRAP
                } else {
                    candidate
                }
            }
        };
        self.last = Some(self.last.map_or(extended, |last| last.max(extended)));
        extended
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn alignment_keeps_the_smallest_delay() {
        let mut alignment = ClockAlignment::new();
        assert!(!alignment.is_known());
        assert_eq!(alignment.to_local(5), None);
        alignment.observe(1_000, 51_000);
        alignment.observe(2_000, 50_500);
        alignment.observe(3_000, 60_000);
        assert!(alignment.is_known());
        assert_eq!(alignment.to_local(10_000), Some(58_500));
        assert_eq!(alignment.to_remote(58_500), Some(10_000));
        assert_eq!(alignment.to_remote(10), Some(0));
    }

    #[test]
    fn alignment_handles_a_remote_clock_ahead_of_the_local_one() {
        let mut alignment = ClockAlignment::new();
        alignment.observe(5_000_000, 1_000);
        assert_eq!(alignment.to_local(5_000_000), Some(1_000));
        assert_eq!(alignment.to_local(4_000_000), Some(0));
        assert_eq!(alignment.to_remote(2_000), Some(5_001_000));
    }

    #[test]
    fn wrapping_millis_unwraps_the_u32_wayland_time() {
        let mut clock = WrappingMillis::new();
        assert_eq!(clock.extend(u32::MAX - 10), u64::from(u32::MAX - 10));
        assert_eq!(clock.extend(u32::MAX), u64::from(u32::MAX));
        assert_eq!(clock.extend(5), WRAP + 5);
        assert_eq!(clock.extend(u32::MAX - 2), u64::from(u32::MAX - 2));
        assert_eq!(clock.extend(20), WRAP + 20);
        assert_eq!(clock.extend(1_000), WRAP + 1_000);
    }

    #[test]
    fn wrapping_millis_keeps_small_steps_unchanged() {
        let mut clock = WrappingMillis::new();
        for ms in [10_u32, 20, 15, 4_000, 3_990] {
            assert_eq!(clock.extend(ms), u64::from(ms));
        }
    }

    #[test]
    fn local_clock_moves_forward() {
        let clock = LocalClock::new();
        let first = clock.now_us();
        assert!(clock.now_us() >= first);
    }
}
