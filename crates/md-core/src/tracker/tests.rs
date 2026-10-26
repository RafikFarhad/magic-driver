use super::*;
use crate::profile::MAGIC_MOUSE_2_BT;
use crate::report::Touch;

const GEOMETRY: SurfaceGeometry = MAGIC_MOUSE_2_BT.surface;
const FRAME: u64 = 11_111;

fn touch(id: u8, x: i32, y: i32, state: TouchState) -> Touch {
    Touch {
        id,
        x,
        y,
        touch_major: 40,
        touch_minor: 30,
        size: 12,
        orientation: 0,
        state,
    }
}

fn at(frame: u64) -> Micros {
    Micros::new(frame * FRAME)
}

fn only(tracker: &Tracker) -> Track {
    let tracks: Vec<Track> = tracker.tracks().copied().collect();
    assert_eq!(tracks.len(), 1, "expected exactly one track: {tracks:?}");
    tracks[0]
}

fn phase_after(states: &[TouchState]) -> Vec<Phase> {
    let mut tracker = Tracker::new(GEOMETRY);
    states
        .iter()
        .zip(0..)
        .map(|(state, frame)| {
            tracker.update(at(frame), &[touch(3, 0, 0, *state)]);
            tracker
                .by_touch_id(3)
                .map_or(Phase::Gone, |track| track.phase)
        })
        .collect()
}

#[test]
fn states_map_to_phases() {
    assert_eq!(
        phase_after(&[
            TouchState::Hover,
            TouchState::Start,
            TouchState::Drag,
            TouchState::Lift(0x50),
            TouchState::None,
        ]),
        vec![
            Phase::Hover,
            Phase::Starting,
            Phase::Active,
            Phase::Lifting,
            Phase::Gone
        ]
    );
    for lift in [0x50, 0x60, 0x70] {
        assert_eq!(
            phase_after(&[TouchState::Drag, TouchState::Lift(lift)])[1],
            Phase::Lifting
        );
    }
}

#[test]
fn hover_is_never_a_contact() {
    let mut tracker = Tracker::new(GEOMETRY);
    for frame in 0..5 {
        let x = i32::try_from(frame).unwrap() * 50;
        tracker.update(at(frame), &[touch(1, x, 0, TouchState::Hover)]);
        let track = only(&tracker);
        assert_eq!(track.phase, Phase::Hover);
        assert!(!track.phase.is_contact());
        assert!(!track.has_contacted());
        assert_eq!(track.delta(), SurfacePoint::new(0, 0));
        assert_eq!(track.velocity, Velocity::ZERO);
    }
    assert_eq!(tracker.contacts().count(), 0);
}

#[test]
fn contact_reference_is_the_first_contact_position_not_the_hover_position() {
    let mut tracker = Tracker::new(GEOMETRY);
    tracker.update(at(0), &[touch(1, -500, 900, TouchState::Hover)]);
    tracker.update(at(1), &[touch(1, 100, 200, TouchState::Start)]);
    let track = only(&tracker);
    assert_eq!(track.start_pos, SurfacePoint::new(100, 200));
    assert_eq!(track.start_t, at(1));
    assert_eq!(track.delta(), SurfacePoint::new(0, 0));
    tracker.update(at(2), &[touch(1, 130, 150, TouchState::Drag)]);
    let track = only(&tracker);
    assert_eq!(track.phase, Phase::Active);
    assert_eq!(track.delta(), SurfacePoint::new(30, -50));
    assert_eq!(track.start_t, at(1));
}

#[test]
fn starting_stays_starting_until_drag() {
    assert_eq!(
        phase_after(&[TouchState::Start, TouchState::Start, TouchState::Drag]),
        vec![Phase::Starting, Phase::Starting, Phase::Active]
    );
}

#[test]
fn phantom_first_touch_without_start_state_has_zero_delta() {
    let mut tracker = Tracker::new(GEOMETRY);
    tracker.update(at(0), &[touch(7, 900, -1200, TouchState::Drag)]);
    let track = only(&tracker);
    assert_eq!(track.phase, Phase::Active);
    assert_eq!(track.start_pos, SurfacePoint::new(900, -1200));
    assert_eq!(track.delta(), SurfacePoint::new(0, 0));
    assert_eq!(track.velocity, Velocity::ZERO);
}

#[test]
fn phantom_first_touch_after_reset_ignores_previous_positions() {
    let mut tracker = Tracker::new(GEOMETRY);
    tracker.update(at(0), &[touch(2, -1000, -1000, TouchState::Drag)]);
    tracker.reset();
    assert_eq!(tracker.tracks().count(), 0);
    tracker.update(at(100), &[touch(2, 1000, 1500, TouchState::Drag)]);
    let track = only(&tracker);
    assert_eq!(track.delta(), SurfacePoint::new(0, 0));
    assert_eq!(track.velocity, Velocity::ZERO);
    assert_eq!(track.start_t, at(100));
}

#[test]
fn touch_vanishing_without_lift_state_is_gone_then_removed() {
    let mut tracker = Tracker::new(GEOMETRY);
    tracker.update(at(0), &[touch(4, 0, 0, TouchState::Drag)]);
    tracker.update(at(1), &[]);
    let track = only(&tracker);
    assert_eq!(track.phase, Phase::Gone);
    assert_eq!(track.last_t, at(1));
    tracker.update(at(2), &[]);
    assert_eq!(tracker.tracks().count(), 0);
}

#[test]
fn state_none_ends_a_track_and_unknown_ids_with_state_none_are_ignored() {
    let mut tracker = Tracker::new(GEOMETRY);
    tracker.update(at(0), &[touch(1, 0, 0, TouchState::None)]);
    assert_eq!(tracker.tracks().count(), 0);
    tracker.update(at(1), &[touch(1, 0, 0, TouchState::Drag)]);
    tracker.update(at(2), &[touch(1, 5, 5, TouchState::None)]);
    assert_eq!(only(&tracker).phase, Phase::Gone);
    tracker.update(at(3), &[touch(1, 5, 5, TouchState::None)]);
    assert_eq!(tracker.tracks().count(), 0);
}

#[test]
fn id_reuse_after_gone_starts_a_new_track() {
    let mut tracker = Tracker::new(GEOMETRY);
    tracker.update(at(0), &[touch(5, -800, 0, TouchState::Start)]);
    tracker.update(at(1), &[touch(5, -700, 0, TouchState::Drag)]);
    let first = only(&tracker);
    tracker.update(at(2), &[touch(5, -700, 0, TouchState::None)]);
    tracker.update(at(3), &[touch(5, 600, 300, TouchState::Start)]);
    let second = only(&tracker);
    assert_ne!(second.id, first.id);
    assert!(second.id > first.id);
    assert_eq!(second.start_pos, SurfacePoint::new(600, 300));
    assert_eq!(second.delta(), SurfacePoint::new(0, 0));
    assert_eq!(second.velocity, Velocity::ZERO);
}

#[test]
fn id_reuse_straight_after_lift_starts_a_new_track() {
    let mut tracker = Tracker::new(GEOMETRY);
    tracker.update(at(0), &[touch(5, -800, 0, TouchState::Drag)]);
    let first = only(&tracker);
    tracker.update(at(1), &[touch(5, -800, 0, TouchState::Lift(0x60))]);
    assert_eq!(only(&tracker).id, first.id);
    tracker.update(at(2), &[touch(5, 400, 0, TouchState::Drag)]);
    let second = only(&tracker);
    assert_ne!(second.id, first.id);
    assert_eq!(second.phase, Phase::Active);
    assert_eq!(second.start_pos, SurfacePoint::new(400, 0));
}

#[test]
fn contact_back_from_hover_starts_a_new_track() {
    let mut tracker = Tracker::new(GEOMETRY);
    tracker.update(at(0), &[touch(5, 0, 0, TouchState::Drag)]);
    let first = only(&tracker);
    tracker.update(at(1), &[touch(5, 10, 0, TouchState::Hover)]);
    assert_eq!(only(&tracker).id, first.id);
    assert_eq!(only(&tracker).phase, Phase::Hover);
    tracker.update(at(2), &[touch(5, 300, 0, TouchState::Drag)]);
    assert_ne!(only(&tracker).id, first.id);
    assert_eq!(only(&tracker).delta(), SurfacePoint::new(0, 0));
}

#[test]
fn unknown_states_continue_a_contact_but_never_start_one() {
    assert_eq!(
        phase_after(&[
            TouchState::Unknown(0x80),
            TouchState::Drag,
            TouchState::Unknown(0x90)
        ]),
        vec![Phase::Hover, Phase::Active, Phase::Active]
    );
    assert_eq!(
        phase_after(&[
            TouchState::Drag,
            TouchState::Lift(0x50),
            TouchState::Unknown(0x10)
        ]),
        vec![Phase::Active, Phase::Lifting, Phase::Lifting]
    );
}

#[test]
fn several_touches_are_tracked_independently() {
    let mut tracker = Tracker::new(GEOMETRY);
    tracker.update(
        at(0),
        &[
            touch(1, 0, 1500, TouchState::Drag),
            touch(2, -300, 0, TouchState::Start),
        ],
    );
    tracker.update(at(1), &[touch(2, -300, -100, TouchState::Drag)]);
    let palm = tracker.by_touch_id(1).unwrap();
    let finger = tracker.by_touch_id(2).unwrap();
    assert_eq!(palm.phase, Phase::Gone);
    assert_eq!(finger.phase, Phase::Active);
    assert_eq!(finger.delta(), SurfacePoint::new(0, -100));
    assert_eq!(tracker.contacts().count(), 1);
    assert_eq!(tracker.get(finger.id), Some(finger));
}

#[test]
fn track_ids_are_unique_across_reset() {
    let mut tracker = Tracker::new(GEOMETRY);
    tracker.update(at(0), &[touch(1, 0, 0, TouchState::Drag)]);
    let before = only(&tracker).id;
    tracker.reset();
    tracker.update(at(1), &[touch(1, 0, 0, TouchState::Drag)]);
    assert_ne!(only(&tracker).id, before);
}

#[test]
fn size_follows_the_latest_record() {
    let mut tracker = Tracker::new(GEOMETRY);
    let mut record = touch(1, 0, 0, TouchState::Drag);
    tracker.update(at(0), &[record]);
    record.touch_major = 200;
    record.touch_minor = 150;
    record.size = 40;
    tracker.update(at(1), &[record]);
    assert_eq!(
        only(&tracker).size,
        TouchSize {
            touch_major: 200,
            touch_minor: 150,
            size: 40
        }
    );
}

fn close(a: f64, b: f64) -> bool {
    (a - b).abs() < 1e-6
}

#[test]
fn velocity_is_measured_in_mm_per_second() {
    let mut tracker = Tracker::new(GEOMETRY);
    let units_per_mm_x = GEOMETRY.x.units_per_mm();
    let step_x = 26;
    for frame in 0..10 {
        let x = i32::try_from(frame).unwrap() * step_x;
        tracker.update(
            Micros::new(frame * 10_000),
            &[touch(1, x, 0, TouchState::Drag)],
        );
    }
    let track = only(&tracker);
    let expected = f64::from(step_x) / units_per_mm_x / 0.010;
    assert!(
        close(track.velocity.x_mm_s, expected),
        "{:?}",
        track.velocity
    );
    assert!(close(track.velocity.y_mm_s, 0.0));
    assert!(close(track.velocity.speed_mm_s(), expected));
}

#[test]
fn velocity_uses_only_the_recent_window() {
    let mut tracker = Tracker::new(GEOMETRY);
    let mut t = 0;
    for x in [0, 500, 1000] {
        tracker.update(Micros::new(t), &[touch(1, x, 0, TouchState::Drag)]);
        t += 10_000;
    }
    for _ in 0..6 {
        tracker.update(Micros::new(t), &[touch(1, 1000, 0, TouchState::Drag)]);
        t += 10_000;
    }
    assert!(close(only(&tracker).velocity.x_mm_s, 0.0));
}

#[test]
fn velocity_is_zero_after_a_gap_longer_than_the_window() {
    let mut tracker = Tracker::new(GEOMETRY);
    tracker.update(Micros::new(0), &[touch(1, 0, 0, TouchState::Drag)]);
    tracker.update(
        VELOCITY_WINDOW + Micros::new(1),
        &[touch(1, 700, 0, TouchState::Drag)],
    );
    assert_eq!(only(&tracker).velocity, Velocity::ZERO);
}

#[test]
fn velocity_ignores_reports_with_the_same_timestamp() {
    let mut tracker = Tracker::new(GEOMETRY);
    tracker.update(Micros::new(0), &[touch(1, 0, 0, TouchState::Drag)]);
    tracker.update(Micros::new(0), &[touch(1, 100, 0, TouchState::Drag)]);
    let velocity = only(&tracker).velocity;
    assert!(velocity.x_mm_s.is_finite());
    assert_eq!(velocity, Velocity::ZERO);
}

#[test]
fn reset_drops_all_tracks() {
    let mut tracker = Tracker::new(GEOMETRY);
    tracker.update(
        at(0),
        &[
            touch(1, 0, 0, TouchState::Drag),
            touch(2, 0, 0, TouchState::Hover),
            touch(3, 0, 0, TouchState::Lift(0x70)),
        ],
    );
    assert_eq!(tracker.tracks().count(), 3);
    tracker.reset();
    assert_eq!(tracker.tracks().count(), 0);
    assert_eq!(tracker.contacts().count(), 0);
}
