use std::fmt;

use md_core::Micros;

pub const RAW_SILENCE_LIMIT: Micros = Micros::from_millis(250);
pub const PARSE_ERROR_WINDOW: Micros = Micros::from_millis(1_000);
pub const PARSE_ERROR_LIMIT: u32 = 10;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SelfCheckFailure {
    NoRawReports,
    ParseErrors { count: u32 },
}

impl fmt::Display for SelfCheckFailure {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SelfCheckFailure::NoRawReports => write!(
                f,
                "no valid hidraw report for {} ms while the stock device is active",
                RAW_SILENCE_LIMIT.as_millis()
            ),
            SelfCheckFailure::ParseErrors { count } => write!(
                f,
                "{count} malformed hidraw reports within {} ms, the report format may have changed",
                PARSE_ERROR_WINDOW.as_millis()
            ),
        }
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct SelfCheck {
    stock_unmatched_since: Option<Micros>,
    error_window_start: Option<Micros>,
    errors_in_window: u32,
}

impl SelfCheck {
    pub fn reset(&mut self) {
        *self = SelfCheck::default();
    }

    pub fn on_valid_report(&mut self) {
        self.stock_unmatched_since = None;
    }

    pub fn on_malformed_report(&mut self, now: Micros) -> Option<SelfCheckFailure> {
        let window_open = self
            .error_window_start
            .is_some_and(|start| now.saturating_sub(start) < PARSE_ERROR_WINDOW);
        if !window_open {
            self.error_window_start = Some(now);
            self.errors_in_window = 0;
        }
        self.errors_in_window += 1;
        (self.errors_in_window >= PARSE_ERROR_LIMIT).then_some(SelfCheckFailure::ParseErrors {
            count: self.errors_in_window,
        })
    }

    pub fn on_stock_activity(&mut self, now: Micros) -> Option<SelfCheckFailure> {
        let since = *self.stock_unmatched_since.get_or_insert(now);
        silent_too_long(since, now)
    }

    pub fn on_timer(&self, now: Micros) -> Option<SelfCheckFailure> {
        silent_too_long(self.stock_unmatched_since?, now)
    }

    pub fn deadline(&self) -> Option<Micros> {
        self.stock_unmatched_since
            .map(|since| since + RAW_SILENCE_LIMIT)
    }
}

fn silent_too_long(since: Micros, now: Micros) -> Option<SelfCheckFailure> {
    (now.saturating_sub(since) >= RAW_SILENCE_LIMIT).then_some(SelfCheckFailure::NoRawReports)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ms(value: u64) -> Micros {
        Micros::from_millis(value)
    }

    #[test]
    fn stock_activity_without_raw_reports_fails_after_the_limit() {
        let mut check = SelfCheck::default();
        assert_eq!(check.on_stock_activity(ms(1_000)), None);
        assert_eq!(check.deadline(), Some(ms(1_250)));
        assert_eq!(check.on_stock_activity(ms(1_100)), None);
        assert_eq!(check.on_timer(ms(1_249)), None);
        assert_eq!(
            check.on_timer(ms(1_250)),
            Some(SelfCheckFailure::NoRawReports)
        );
        assert_eq!(
            check.on_stock_activity(ms(1_300)),
            Some(SelfCheckFailure::NoRawReports)
        );
    }

    #[test]
    fn a_valid_report_clears_the_pending_silence() {
        let mut check = SelfCheck::default();
        check.on_stock_activity(ms(0));
        check.on_valid_report();
        assert_eq!(check.deadline(), None);
        assert_eq!(check.on_timer(ms(10_000)), None);
        assert_eq!(check.on_stock_activity(ms(10_000)), None);
        assert_eq!(check.deadline(), Some(ms(10_250)));
    }

    #[test]
    fn an_idle_mouse_never_fails() {
        let check = SelfCheck::default();
        assert_eq!(check.deadline(), None);
        assert_eq!(check.on_timer(ms(60_000)), None);
    }

    #[test]
    fn parse_errors_fail_only_above_the_limit_within_the_window() {
        let mut check = SelfCheck::default();
        for n in 0..PARSE_ERROR_LIMIT - 1 {
            assert_eq!(check.on_malformed_report(ms(u64::from(n) * 50)), None);
        }
        assert_eq!(
            check.on_malformed_report(ms(900)),
            Some(SelfCheckFailure::ParseErrors {
                count: PARSE_ERROR_LIMIT
            })
        );
    }

    #[test]
    fn sparse_parse_errors_never_fail() {
        let mut check = SelfCheck::default();
        for n in 0..100 {
            assert_eq!(check.on_malformed_report(ms(n * 150)), None);
        }
    }

    #[test]
    fn reset_forgets_everything() {
        let mut check = SelfCheck::default();
        check.on_stock_activity(ms(0));
        check.on_malformed_report(ms(0));
        check.reset();
        assert_eq!(check, SelfCheck::default());
    }

    #[test]
    fn failures_name_their_cause() {
        assert_eq!(
            SelfCheckFailure::NoRawReports.to_string(),
            "no valid hidraw report for 250 ms while the stock device is active"
        );
        assert!(
            SelfCheckFailure::ParseErrors { count: 10 }
                .to_string()
                .starts_with("10 malformed hidraw reports within 1000 ms")
        );
    }
}
