use crate::Micros;
use crate::testkit::report_builder::{ReportBuilder, TouchRecord, touch_state};

pub const SYNTHETIC_REPORT_INTERVAL_US: u64 = 11_111;

pub fn synthetic_session(reports: usize) -> Vec<(Micros, Vec<u8>)> {
    (0..reports)
        .zip(0_u64..)
        .map(|(index, step)| {
            (
                Micros::new(step * SYNTHETIC_REPORT_INTERVAL_US),
                synthetic_report(index).build(),
            )
        })
        .collect()
}

fn synthetic_report(index: usize) -> ReportBuilder {
    let phase = index % 120;
    let stroke = index / 120;
    let offset = i32::try_from(phase).expect("phase fits") * 6;
    let palm = TouchRecord::new(1, 0, 1600)
        .touch_major(120)
        .touch_minor(90);
    let mut builder = ReportBuilder::new().touch(palm);
    if stroke.is_multiple_of(2) {
        builder = builder.motion(
            i16::try_from(phase % 7).expect("small") - 3,
            i16::try_from(phase % 5).expect("small") - 2,
        );
    }
    let finger_state = match phase {
        0 => Some(touch_state::START),
        1..=89 => Some(touch_state::DRAG),
        90 => Some(touch_state::LIFT_50),
        91 => Some(touch_state::GONE),
        _ => None,
    };
    if let Some(state) = finger_state {
        builder = builder.touch(TouchRecord::new(2, -200, 600 - offset).state(state));
    }
    if (30..60).contains(&phase) {
        builder =
            builder.touch(TouchRecord::new(3, 400 + offset / 2, 200).state(touch_state::DRAG));
    }
    if (100..110).contains(&phase) {
        builder = builder.left(true);
    }
    builder
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn session_has_the_requested_length_and_increasing_timestamps() {
        let session = synthetic_session(500);
        assert_eq!(session.len(), 500);
        assert!(session.windows(2).all(|pair| pair[0].0 < pair[1].0));
        assert!(session.iter().all(|(_, bytes)| bytes[0] == 0x12));
        let max_touches = session
            .iter()
            .map(|(_, bytes)| (bytes.len() - 14) / 8)
            .max()
            .unwrap();
        assert_eq!(max_touches, 3);
    }
}
