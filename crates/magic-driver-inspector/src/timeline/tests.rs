use md_trace::{
    DecisionReason, DecisionRecord, DroppedRecord, OutputEvent, OutputRecord, RecogniserRecord,
    ReportRecord, SessionRecord, TrackClass,
};

use super::*;

pub(crate) fn report(t: u64) -> TraceRecord {
    TraceRecord::Report(ReportRecord {
        t,
        clicks: 0,
        dx: 0,
        dy: 0,
        touches: Vec::new(),
        error: None,
        processing_ns: None,
    })
}

pub(crate) fn change(t: u64, from: GestureState, to: GestureState) -> TraceRecord {
    TraceRecord::Recogniser(RecogniserRecord { t, from, to })
}

fn decision(t: u64) -> TraceRecord {
    TraceRecord::Decision(DecisionRecord {
        t,
        track: 1,
        touch_id: Some(2),
        previous: TrackClass::Candidate,
        class: TrackClass::Deliberate,
        reason: DecisionReason::Promoted,
    })
}

fn motion(t: u64) -> TraceRecord {
    TraceRecord::Output(OutputRecord {
        t,
        output: OutputEvent::PointerMotion { dx: 1, dy: 0 },
    })
}

fn session(t: u64) -> TraceRecord {
    TraceRecord::Session(SessionRecord {
        t,
        state: "active".to_owned(),
        detail: String::new(),
    })
}

fn dropped() -> TraceRecord {
    TraceRecord::Dropped(DroppedRecord { count: 3 })
}

#[test]
fn record_time_reads_the_engine_time_of_every_timed_record() {
    assert_eq!(record_time(&report(5)), Some(5));
    assert_eq!(record_time(&decision(6)), Some(6));
    assert_eq!(
        record_time(&change(7, GestureState::Idle, GestureState::Scroll)),
        Some(7)
    );
    assert_eq!(record_time(&motion(8)), Some(8));
    assert_eq!(record_time(&session(9)), Some(9));
    assert_eq!(record_time(&dropped()), None);
}

#[test]
fn entries_keep_time_order_and_untimed_records_take_the_last_time() {
    let timeline = Timeline::from_records([report(100), dropped(), motion(50), report(200)]);
    let times: Vec<u64> = timeline.entries().map(|entry| entry.t).collect();
    assert_eq!(times, [100, 100, 100, 200]);
    assert_eq!(timeline.start_time(), Some(100));
    assert_eq!(timeline.end_time(), Some(200));
    assert_eq!(timeline.len(), 4);
    assert_eq!(timeline.report_count(), 2);
    assert!(Timeline::new().is_empty());
    assert_eq!(Timeline::new().start_time(), None);
}

#[test]
fn seq_at_counts_every_entry_at_or_before_the_time() {
    let timeline = Timeline::from_records([report(10), motion(10), report(20), report(30)]);
    assert_eq!(timeline.first_seq(), 0);
    assert_eq!(timeline.end_seq(), 4);
    assert_eq!(timeline.seq_at(0), 0);
    assert_eq!(timeline.seq_at(10), 2);
    assert_eq!(timeline.seq_at(25), 3);
    assert_eq!(timeline.seq_at(1_000), 4);
    assert_eq!(timeline.get(2).map(|entry| entry.t), Some(20));
    assert_eq!(timeline.get(4), None);
}

#[test]
fn report_navigation_finds_the_neighbouring_reports() {
    let timeline = Timeline::from_records([report(10), motion(15), report(20), report(30)]);
    assert_eq!(timeline.next_report_after(0), Some(10));
    assert_eq!(timeline.next_report_after(10), Some(20));
    assert_eq!(timeline.next_report_after(15), Some(20));
    assert_eq!(timeline.next_report_after(30), None);
    assert_eq!(timeline.previous_report_before(30), Some(20));
    assert_eq!(timeline.previous_report_before(25), Some(20));
    assert_eq!(timeline.previous_report_before(10), None);
    assert_eq!(timeline.reports_between(10, 30), 2);
    assert_eq!(timeline.reports_between(0, 30), 3);
}

#[test]
fn report_rate_counts_reports_in_the_window() {
    let timeline = Timeline::from_records((0..=90).map(|index| report(index * 11_111)));
    let rate = timeline.report_rate_hz(1_000_000, 1_000_000);
    assert!((rate - 90.0).abs() < 1.5, "{rate}");
    assert!(timeline.report_rate_hz(1_000_000, 0).abs() < f64::EPSILON);
}

#[test]
fn recogniser_state_and_segments_follow_the_changes() {
    let timeline = Timeline::from_records([
        report(0),
        change(100, GestureState::Idle, GestureState::Scroll),
        change(300, GestureState::Scroll, GestureState::Cancelling),
        change(400, GestureState::Cancelling, GestureState::Idle),
        report(500),
    ]);
    assert_eq!(timeline.recogniser_state_at(50), GestureState::Idle);
    assert_eq!(timeline.recogniser_state_at(100), GestureState::Scroll);
    assert_eq!(timeline.recogniser_state_at(350), GestureState::Cancelling);
    assert_eq!(timeline.recogniser_state_at(450), GestureState::Idle);
    assert_eq!(
        timeline.recogniser_segments(50, 350),
        [
            StateSegment {
                start: 50,
                end: 100,
                state: GestureState::Idle
            },
            StateSegment {
                start: 100,
                end: 300,
                state: GestureState::Scroll
            },
            StateSegment {
                start: 300,
                end: 350,
                state: GestureState::Cancelling
            },
        ]
    );
    assert!(timeline.recogniser_segments(10, 5).is_empty());
}

#[test]
fn eviction_moves_the_first_sequence_and_keeps_the_state_before_the_window() {
    let mut timeline = Timeline::from_records([
        report(0),
        change(100, GestureState::Idle, GestureState::Scroll),
        report(200),
        report(300),
    ]);
    timeline.evict_before(150);
    assert_eq!(timeline.first_seq(), 2);
    assert_eq!(timeline.end_seq(), 4);
    assert_eq!(timeline.start_time(), Some(200));
    assert_eq!(timeline.report_count(), 2);
    assert_eq!(timeline.recogniser_state_at(0), GestureState::Scroll);
    assert_eq!(timeline.get(1), None);
    assert_eq!(timeline.get(2).map(|entry| entry.t), Some(200));
    timeline.truncate_front(1);
    assert_eq!(timeline.first_seq(), 3);
    assert_eq!(timeline.report_count(), 1);
    assert_eq!(timeline.previous_report_before(300), None);
}

#[test]
fn a_change_before_any_other_sets_the_initial_state() {
    let timeline =
        Timeline::from_records([change(100, GestureState::TapPending, GestureState::Idle)]);
    assert_eq!(timeline.recogniser_state_at(50), GestureState::TapPending);
    assert_eq!(timeline.recogniser_state_at(100), GestureState::Idle);
}
