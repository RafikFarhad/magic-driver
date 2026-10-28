use std::collections::VecDeque;

use md_trace::{GestureState, TraceRecord};

#[derive(Debug, Clone, PartialEq)]
pub struct Entry {
    pub t: u64,
    pub record: TraceRecord,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Timeline {
    entries: VecDeque<Entry>,
    first_seq: u64,
    report_times: VecDeque<u64>,
    recogniser_changes: VecDeque<(u64, GestureState)>,
    initial_state: Option<GestureState>,
}

pub fn record_time(record: &TraceRecord) -> Option<u64> {
    match record {
        TraceRecord::Report(report) => Some(report.t),
        TraceRecord::Decision(decision) => Some(decision.t),
        TraceRecord::Recogniser(recogniser) => Some(recogniser.t),
        TraceRecord::Output(output) => Some(output.t),
        TraceRecord::Session(session) => Some(session.t),
        TraceRecord::Hello(_) | TraceRecord::Dropped(_) => None,
    }
}

impl Timeline {
    pub fn new() -> Timeline {
        Timeline::default()
    }

    pub fn from_records<I: IntoIterator<Item = TraceRecord>>(records: I) -> Timeline {
        let mut timeline = Timeline::new();
        for record in records {
            timeline.push(record);
        }
        timeline
    }

    pub fn push(&mut self, record: TraceRecord) {
        let last = self.entries.back().map_or(0, |entry| entry.t);
        let t = record_time(&record).map_or(last, |t| t.max(last));
        match &record {
            TraceRecord::Report(_) => self.report_times.push_back(t),
            TraceRecord::Recogniser(change) => {
                if self.recogniser_changes.is_empty() && self.initial_state.is_none() {
                    self.initial_state = Some(change.from);
                }
                self.recogniser_changes.push_back((t, change.to));
            }
            _ => {}
        }
        self.entries.push_back(Entry { t, record });
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    pub const fn first_seq(&self) -> u64 {
        self.first_seq
    }

    pub fn end_seq(&self) -> u64 {
        self.first_seq + self.entries.len() as u64
    }

    pub fn get(&self, seq: u64) -> Option<&Entry> {
        let index = usize::try_from(seq.checked_sub(self.first_seq)?).ok()?;
        self.entries.get(index)
    }

    pub fn entries(&self) -> impl Iterator<Item = &Entry> {
        self.entries.iter()
    }

    pub fn start_time(&self) -> Option<u64> {
        self.entries.front().map(|entry| entry.t)
    }

    pub fn end_time(&self) -> Option<u64> {
        self.entries.back().map(|entry| entry.t)
    }

    pub fn report_count(&self) -> usize {
        self.report_times.len()
    }

    pub fn seq_at(&self, t: u64) -> u64 {
        self.first_seq + self.entries.partition_point(|entry| entry.t <= t) as u64
    }

    pub fn next_report_after(&self, t: u64) -> Option<u64> {
        let index = self.report_times.partition_point(|time| *time <= t);
        self.report_times.get(index).copied()
    }

    pub fn previous_report_before(&self, t: u64) -> Option<u64> {
        let index = self.report_times.partition_point(|time| *time < t);
        index
            .checked_sub(1)
            .and_then(|index| self.report_times.get(index).copied())
    }

    pub fn reports_between(&self, after: u64, until: u64) -> usize {
        let from = self.report_times.partition_point(|time| *time <= after);
        let to = self.report_times.partition_point(|time| *time <= until);
        to.saturating_sub(from)
    }

    pub fn report_rate_hz(&self, t: u64, window_us: u64) -> f64 {
        if window_us == 0 {
            return 0.0;
        }
        let count = self.reports_between(t.saturating_sub(window_us), t);
        #[allow(clippy::cast_precision_loss)]
        let rate = count as f64 * 1_000_000.0 / window_us as f64;
        rate
    }

    pub fn recogniser_state_at(&self, t: u64) -> GestureState {
        let index = self
            .recogniser_changes
            .partition_point(|(time, _)| *time <= t);
        match index.checked_sub(1) {
            Some(index) => self.recogniser_changes[index].1,
            None => self.initial_state.unwrap_or(GestureState::Idle),
        }
    }

    pub fn recogniser_segments(&self, from: u64, to: u64) -> Vec<StateSegment> {
        let mut segments = Vec::new();
        if to < from {
            return segments;
        }
        let mut start = from;
        let mut state = self.recogniser_state_at(from);
        let first = self
            .recogniser_changes
            .partition_point(|(time, _)| *time <= from);
        for (time, next) in self.recogniser_changes.iter().skip(first) {
            if *time > to {
                break;
            }
            if *next != state {
                segments.push(StateSegment {
                    start,
                    end: *time,
                    state,
                });
                start = *time;
                state = *next;
            }
        }
        segments.push(StateSegment {
            start,
            end: to,
            state,
        });
        segments
    }

    pub fn evict_before(&mut self, cutoff: u64) {
        while self.entries.front().is_some_and(|entry| entry.t < cutoff) {
            self.pop_front();
        }
    }

    pub fn truncate_front(&mut self, max_len: usize) {
        while self.entries.len() > max_len {
            self.pop_front();
        }
    }

    fn pop_front(&mut self) {
        let Some(entry) = self.entries.pop_front() else {
            return;
        };
        self.first_seq += 1;
        match entry.record {
            TraceRecord::Report(_) => {
                self.report_times.pop_front();
            }
            TraceRecord::Recogniser(_) => {
                if let Some((_, state)) = self.recogniser_changes.pop_front() {
                    self.initial_state = Some(state);
                }
            }
            _ => {}
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StateSegment {
    pub start: u64,
    pub end: u64,
    pub state: GestureState,
}

#[cfg(test)]
mod tests;
