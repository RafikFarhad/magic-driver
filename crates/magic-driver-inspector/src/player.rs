use std::time::Duration;

use crate::timeline::Timeline;

pub const SPEEDS: [f64; 7] = [0.1, 0.25, 0.5, 1.0, 2.0, 4.0, 8.0];
const MIN_SPEED: f64 = 0.01;
const MAX_SPEED: f64 = 64.0;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlayState {
    Paused,
    Playing,
    Following,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlayerKind {
    Offline,
    Live,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Player {
    kind: PlayerKind,
    position: u64,
    state: PlayState,
    speed: f64,
    carry_us: f64,
}

impl Player {
    pub fn offline(timeline: &Timeline) -> Player {
        Player {
            kind: PlayerKind::Offline,
            position: timeline.start_time().unwrap_or(0),
            state: PlayState::Paused,
            speed: 1.0,
            carry_us: 0.0,
        }
    }

    pub fn live() -> Player {
        Player {
            kind: PlayerKind::Live,
            position: 0,
            state: PlayState::Following,
            speed: 1.0,
            carry_us: 0.0,
        }
    }

    pub const fn kind(&self) -> PlayerKind {
        self.kind
    }

    pub const fn position(&self) -> u64 {
        self.position
    }

    pub const fn state(&self) -> PlayState {
        self.state
    }

    pub const fn speed(&self) -> f64 {
        self.speed
    }

    pub fn is_playing(&self) -> bool {
        self.state != PlayState::Paused
    }

    pub fn set_speed(&mut self, speed: f64) {
        self.speed = if speed.is_finite() {
            speed.clamp(MIN_SPEED, MAX_SPEED)
        } else {
            1.0
        };
    }

    pub fn play(&mut self, timeline: &Timeline) {
        let (start, end) = bounds(timeline);
        if self.position >= end {
            match self.kind {
                PlayerKind::Offline => self.position = start,
                PlayerKind::Live => {
                    self.follow(timeline);
                    return;
                }
            }
        }
        self.state = PlayState::Playing;
        self.carry_us = 0.0;
    }

    pub fn pause(&mut self) {
        self.state = PlayState::Paused;
        self.carry_us = 0.0;
    }

    pub fn toggle(&mut self, timeline: &Timeline) {
        if self.is_playing() {
            self.pause();
        } else {
            self.play(timeline);
        }
    }

    pub fn follow(&mut self, timeline: &Timeline) {
        if self.kind == PlayerKind::Live {
            self.state = PlayState::Following;
            self.position = bounds(timeline).1;
        }
    }

    pub fn seek(&mut self, timeline: &Timeline, t: u64) {
        let (start, end) = bounds(timeline);
        self.position = t.clamp(start, end);
        if self.state == PlayState::Following {
            self.state = PlayState::Paused;
        }
        self.carry_us = 0.0;
    }

    pub fn step_forward(&mut self, timeline: &Timeline) {
        self.pause();
        let end = bounds(timeline).1;
        self.position = timeline.next_report_after(self.position).unwrap_or(end);
    }

    pub fn step_back(&mut self, timeline: &Timeline) {
        self.pause();
        let start = bounds(timeline).0;
        self.position = timeline
            .previous_report_before(self.position)
            .unwrap_or(start);
    }

    pub fn tick(&mut self, timeline: &Timeline, elapsed: Duration) {
        let (start, end) = bounds(timeline);
        match self.state {
            PlayState::Following => self.position = end,
            PlayState::Paused => self.position = self.position.clamp(start, end),
            PlayState::Playing => {
                #[allow(clippy::cast_precision_loss)]
                let advance = elapsed.as_micros() as f64 * self.speed + self.carry_us;
                #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
                let whole = advance.floor().max(0.0) as u64;
                #[allow(clippy::cast_precision_loss)]
                let carry = advance - whole as f64;
                self.carry_us = carry;
                self.position = self.position.saturating_add(whole).clamp(start, end);
                if self.position >= end {
                    self.carry_us = 0.0;
                    self.state = match self.kind {
                        PlayerKind::Offline => PlayState::Paused,
                        PlayerKind::Live => PlayState::Following,
                    };
                }
            }
        }
    }
}

fn bounds(timeline: &Timeline) -> (u64, u64) {
    let start = timeline.start_time().unwrap_or(0);
    let end = timeline.end_time().unwrap_or(start);
    (start, end)
}

#[cfg(test)]
mod tests;
