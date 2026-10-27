use std::time::{Duration, Instant};

use md_core::config::Config;
use md_core::profile::MAGIC_MOUSE_2_BT;
use md_core::testkit::synthetic_session;
use md_core::{Engine, Input, Mode, Output};

const REPORTS: usize = 10_000;
const BUDGET: Duration = Duration::from_millis(1);

fn per_report_durations(mode: Mode) -> Vec<Duration> {
    let session = synthetic_session(REPORTS);
    let mut engine = Engine::new(&MAGIC_MOUSE_2_BT, Config::default(), mode);
    let mut out: Vec<Output> = Vec::with_capacity(16);
    let mut durations = Vec::with_capacity(REPORTS);
    for (t, bytes) in &session {
        let start = Instant::now();
        engine.handle(Input::Report { t: *t, bytes }, &mut out);
        durations.push(start.elapsed());
        out.clear();
    }
    durations.sort_unstable();
    durations
}

fn percentile(sorted: &[Duration], percent: usize) -> Duration {
    sorted[(sorted.len() * percent / 100).min(sorted.len() - 1)]
}

#[test]
fn p99_per_report_is_well_under_one_millisecond() {
    for mode in [
        Mode::Normal,
        Mode::PointerOnly,
        Mode::RawForward,
        Mode::ScrollProbe,
    ] {
        let durations = per_report_durations(mode);
        let p50 = percentile(&durations, 50);
        let p99 = percentile(&durations, 99);
        let max = durations[durations.len() - 1];
        println!("{mode:?}: {REPORTS} reports, p50 {p50:?}, p99 {p99:?}, max {max:?}");
        assert!(p99 < BUDGET, "{mode:?}: p99 {p99:?} exceeds {BUDGET:?}");
    }
}
