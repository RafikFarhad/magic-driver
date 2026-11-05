use std::fmt;

use md_core::Micros;

pub const RAW_SILENCE_LIMIT: Micros = Micros::from_millis(250);
pub const RAW_MATCH_GRACE: Micros = Micros::from_millis(50);
pub const STOCK_ACTIVITY_GAP: Micros = Micros::from_millis(100);
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
                "no hidraw report for {} ms while the stock device kept reporting",
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct StockSilence {
    since: Micros,
    last_stock: Micros,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct SelfCheck {
    last_raw: Option<Micros>,
    silence: Option<StockSilence>,
    error_window_start: Option<Micros>,
    errors_in_window: u32,
}

impl SelfCheck {
    pub fn reset(&mut self) {
        *self = SelfCheck::default();
    }

    pub fn on_raw_report(&mut self, now: Micros) {
        self.last_raw = Some(now);
        self.silence = None;
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
        if self
            .last_raw
            .is_some_and(|raw| now.saturating_sub(raw) <= RAW_MATCH_GRACE)
        {
            return None;
        }
        match self.silence.as_mut() {
            Some(silence) if now.saturating_sub(silence.last_stock) <= STOCK_ACTIVITY_GAP => {
                silence.last_stock = now;
                (now.saturating_sub(silence.since) >= RAW_SILENCE_LIMIT)
                    .then_some(SelfCheckFailure::NoRawReports)
            }
            _ => {
                self.silence = Some(StockSilence {
                    since: now,
                    last_stock: now,
                });
                None
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ms(value: u64) -> Micros {
        Micros::from_millis(value)
    }

    fn stock_stream(check: &mut SelfCheck, from: u64, to: u64, step: u64) -> Vec<u64> {
        let mut failures = Vec::new();
        let mut t = from;
        while t <= to {
            if check.on_stock_activity(ms(t)).is_some() {
                failures.push(t);
            }
            t += step;
        }
        failures
    }

    #[test]
    fn sustained_stock_activity_without_raw_reports_fails_after_the_limit() {
        let mut check = SelfCheck::default();
        let failures = stock_stream(&mut check, 1_000, 1_300, 10);
        assert_eq!(failures.first(), Some(&1_250));
        assert_eq!(
            check.on_stock_activity(ms(1_305)),
            Some(SelfCheckFailure::NoRawReports)
        );
    }

    #[test]
    fn a_true_silence_right_after_the_last_raw_report_still_fails() {
        let mut check = SelfCheck::default();
        check.on_raw_report(ms(1_000));
        let failures = stock_stream(&mut check, 1_000, 1_400, 10);
        assert_eq!(failures.first(), Some(&1_310));
    }

    #[test]
    fn a_stock_copy_handled_after_the_last_raw_report_never_fails() {
        let mut check = SelfCheck::default();
        for t in (1_000..1_200).step_by(11) {
            check.on_raw_report(ms(t));
            assert_eq!(check.on_stock_activity(ms(t)), None);
        }
        check.on_raw_report(ms(1_200));
        assert_eq!(check.on_stock_activity(ms(1_203)), None);
        assert_eq!(check.silence, None);
    }

    #[test]
    fn any_order_within_the_grace_never_fails() {
        let mut check = SelfCheck::default();
        for (index, t) in (0..5_000).step_by(11).enumerate() {
            if index % 3 == 0 {
                assert_eq!(check.on_stock_activity(ms(t)), None);
                check.on_raw_report(ms(t + 2));
            } else {
                check.on_raw_report(ms(t));
                assert_eq!(check.on_stock_activity(ms(t + 4)), None);
            }
        }
    }

    #[test]
    fn stray_stock_events_with_gaps_never_fail() {
        let mut check = SelfCheck::default();
        check.on_raw_report(ms(0));
        let failures = stock_stream(&mut check, 1_000, 10_000, 150);
        assert_eq!(failures, Vec::<u64>::new());
    }

    #[test]
    fn a_gap_in_stock_activity_restarts_the_window() {
        let mut check = SelfCheck::default();
        assert_eq!(stock_stream(&mut check, 0, 200, 10), Vec::<u64>::new());
        assert_eq!(stock_stream(&mut check, 400, 600, 10), Vec::<u64>::new());
        assert_eq!(stock_stream(&mut check, 610, 700, 10).first(), Some(&650));
    }

    #[test]
    fn a_raw_report_clears_the_pending_silence() {
        let mut check = SelfCheck::default();
        assert_eq!(stock_stream(&mut check, 0, 200, 10), Vec::<u64>::new());
        check.on_raw_report(ms(205));
        assert_eq!(stock_stream(&mut check, 210, 1_000, 10).first(), Some(&510));
    }

    #[test]
    fn an_idle_mouse_never_fails() {
        let mut check = SelfCheck::default();
        assert_eq!(check.on_stock_activity(ms(60_000)), None);
        assert_eq!(check.on_stock_activity(ms(120_000)), None);
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
        check.on_raw_report(ms(0));
        check.on_stock_activity(ms(100));
        check.on_malformed_report(ms(0));
        check.reset();
        assert_eq!(check, SelfCheck::default());
    }

    #[test]
    fn failures_name_their_cause() {
        assert_eq!(
            SelfCheckFailure::NoRawReports.to_string(),
            "no hidraw report for 250 ms while the stock device kept reporting"
        );
        assert!(
            SelfCheckFailure::ParseErrors { count: 10 }
                .to_string()
                .starts_with("10 malformed hidraw reports within 1000 ms")
        );
    }
}
