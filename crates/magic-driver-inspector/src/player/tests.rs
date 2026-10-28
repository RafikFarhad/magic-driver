use md_trace::{ReportRecord, TraceRecord};

use super::*;

fn reports(times: &[u64]) -> Timeline {
    Timeline::from_records(times.iter().map(|t| {
        TraceRecord::Report(ReportRecord {
            t: *t,
            clicks: 0,
            dx: 0,
            dy: 0,
            touches: Vec::new(),
            error: None,
            processing_ns: None,
        })
    }))
}

#[test]
fn offline_player_starts_paused_at_the_first_record() {
    let timeline = reports(&[1_000, 2_000, 3_000]);
    let player = Player::offline(&timeline);
    assert_eq!(player.kind(), PlayerKind::Offline);
    assert_eq!(player.state(), PlayState::Paused);
    assert_eq!(player.position(), 1_000);
    assert!(!player.is_playing());
    assert_eq!(Player::offline(&Timeline::new()).position(), 0);
}

#[test]
fn playing_advances_by_elapsed_time_times_speed_and_pauses_at_the_end() {
    let timeline = reports(&[0, 100_000, 200_000]);
    let mut player = Player::offline(&timeline);
    player.play(&timeline);
    player.tick(&timeline, Duration::from_millis(50));
    assert_eq!(player.position(), 50_000);
    player.set_speed(2.0);
    player.tick(&timeline, Duration::from_millis(25));
    assert_eq!(player.position(), 100_000);
    player.tick(&timeline, Duration::from_secs(5));
    assert_eq!(player.position(), 200_000);
    assert_eq!(player.state(), PlayState::Paused);
    player.play(&timeline);
    assert_eq!(player.position(), 0);
    assert!(player.is_playing());
}

#[test]
fn slow_speeds_carry_fractions_of_a_microsecond() {
    let timeline = reports(&[0, 1_000]);
    let mut player = Player::offline(&timeline);
    player.set_speed(0.1);
    player.play(&timeline);
    for _ in 0..10 {
        player.tick(&timeline, Duration::from_micros(5));
    }
    assert_eq!(player.position(), 5);
}

#[test]
fn speed_is_clamped_and_invalid_values_reset_it() {
    let timeline = reports(&[0]);
    let mut player = Player::offline(&timeline);
    player.set_speed(1_000.0);
    assert!((player.speed() - 64.0).abs() < f64::EPSILON);
    player.set_speed(0.0);
    assert!((player.speed() - 0.01).abs() < f64::EPSILON);
    player.set_speed(f64::NAN);
    assert!((player.speed() - 1.0).abs() < f64::EPSILON);
    for speed in SPEEDS {
        player.set_speed(speed);
        assert!((player.speed() - speed).abs() < f64::EPSILON);
    }
}

#[test]
fn seek_clamps_to_the_timeline_and_pausing_stops_time() {
    let timeline = reports(&[1_000, 5_000]);
    let mut player = Player::offline(&timeline);
    player.seek(&timeline, 0);
    assert_eq!(player.position(), 1_000);
    player.seek(&timeline, 9_000);
    assert_eq!(player.position(), 5_000);
    player.seek(&timeline, 3_000);
    player.tick(&timeline, Duration::from_secs(1));
    assert_eq!(player.position(), 3_000);
    player.toggle(&timeline);
    assert!(player.is_playing());
    player.toggle(&timeline);
    assert!(!player.is_playing());
}

#[test]
fn steps_move_report_by_report_and_pause() {
    let timeline = reports(&[1_000, 2_000, 3_000]);
    let mut player = Player::offline(&timeline);
    player.play(&timeline);
    player.step_forward(&timeline);
    assert_eq!(player.position(), 2_000);
    assert_eq!(player.state(), PlayState::Paused);
    player.step_forward(&timeline);
    player.step_forward(&timeline);
    assert_eq!(player.position(), 3_000);
    player.seek(&timeline, 2_500);
    player.step_back(&timeline);
    assert_eq!(player.position(), 2_000);
    player.step_back(&timeline);
    player.step_back(&timeline);
    assert_eq!(player.position(), 1_000);
}

#[test]
fn live_player_follows_the_newest_record_until_paused_or_scrubbed() {
    let mut timeline = reports(&[1_000, 2_000]);
    let mut player = Player::live();
    assert_eq!(player.kind(), PlayerKind::Live);
    assert_eq!(player.state(), PlayState::Following);
    player.tick(&timeline, Duration::ZERO);
    assert_eq!(player.position(), 2_000);
    player.seek(&timeline, 1_500);
    assert_eq!(player.state(), PlayState::Paused);
    timeline = reports(&[1_000, 2_000, 3_000, 4_000]);
    player.tick(&timeline, Duration::from_millis(10));
    assert_eq!(player.position(), 1_500);
    player.play(&timeline);
    player.tick(&timeline, Duration::from_millis(10));
    assert_eq!(player.state(), PlayState::Following);
    assert_eq!(player.position(), 4_000);
    player.pause();
    player.follow(&timeline);
    assert_eq!(player.state(), PlayState::Following);
    player.pause();
    player.seek(&timeline, 4_000);
    player.play(&timeline);
    assert_eq!(player.state(), PlayState::Following);
}

#[test]
fn follow_does_nothing_offline() {
    let timeline = reports(&[1_000, 2_000]);
    let mut player = Player::offline(&timeline);
    player.follow(&timeline);
    assert_eq!(player.state(), PlayState::Paused);
    assert_eq!(player.position(), 1_000);
}
