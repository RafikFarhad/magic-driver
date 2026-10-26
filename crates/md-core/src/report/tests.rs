use proptest::prelude::*;

use super::*;
use crate::profile::MAGIC_MOUSE_2_BT;
use crate::testkit::{ReportBuilder, TouchRecord, touch_state};

const HEADER: [u8; 14] = [
    0x12, 0x01, 0xfd, 0xff, 0x07, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x40, 0x69, 0xa9,
];
const TOUCH_A: [u8; 8] = [0xa2, 0x0e, 0xe7, 0x28, 0x20, 0x4c, 0x81, 0x40];
const TOUCH_B: [u8; 8] = [0xe8, 0xc3, 0x5d, 0xff, 0x00, 0xbf, 0x02, 0x30];
const TOUCH_C: [u8; 8] = [0xff, 0xff, 0xff, 0x01, 0x02, 0xc0, 0xff, 0x23];

const HEADER_DECODED: Header = Header {
    clicks: 0x01,
    dx: -3,
    dy: 7,
    timestamp: Some(0x2_a5a5),
};
const TOUCH_A_DECODED: Touch = Touch {
    id: 5,
    x: -350,
    y: 400,
    touch_major: 40,
    touch_minor: 32,
    size: 12,
    orientation: 0,
    state: TouchState::Drag,
};
const TOUCH_B_DECODED: Touch = Touch {
    id: 10,
    x: 1000,
    y: -1500,
    touch_major: 255,
    touch_minor: 0,
    size: 63,
    orientation: -32,
    state: TouchState::Start,
};
const TOUCH_C_DECODED: Touch = Touch {
    id: 15,
    x: -1,
    y: 1,
    touch_major: 1,
    touch_minor: 2,
    size: 0,
    orientation: 31,
    state: TouchState::Hover,
};

fn golden(touches: &[[u8; 8]]) -> Vec<u8> {
    let mut bytes = HEADER.to_vec();
    for touch in touches {
        bytes.extend_from_slice(touch);
    }
    bytes
}

fn parse_mm2(bytes: &[u8]) -> Result<Report, ParseError> {
    parse(&MAGIC_MOUSE_2_BT, bytes)
}

#[test]
fn golden_header_only_report_has_no_touches() {
    let report = parse_mm2(&golden(&[])).expect("report");
    assert_eq!(report.format, ReportFormat::Full);
    assert_eq!(report.header, HEADER_DECODED);
    assert!(report.touches.is_empty());
    assert!(report.has_touch_area());
}

#[test]
fn golden_report_with_one_touch() {
    let report = parse_mm2(&golden(&[TOUCH_A])).expect("report");
    assert_eq!(report.header, HEADER_DECODED);
    assert_eq!(report.touches.as_slice(), &[TOUCH_A_DECODED]);
}

#[test]
fn golden_report_with_two_touches() {
    let report = parse_mm2(&golden(&[TOUCH_A, TOUCH_B])).expect("report");
    assert_eq!(
        report.touches.as_slice(),
        &[TOUCH_A_DECODED, TOUCH_B_DECODED]
    );
}

#[test]
fn golden_report_with_three_touches() {
    let report = parse_mm2(&golden(&[TOUCH_A, TOUCH_B, TOUCH_C])).expect("report");
    assert_eq!(
        report.touches.as_slice(),
        &[TOUCH_A_DECODED, TOUCH_B_DECODED, TOUCH_C_DECODED]
    );
}

#[test]
fn golden_report_with_profile_maximum_of_touches() {
    let records: Vec<TouchRecord> = (0..15_u8)
        .map(|id| {
            TouchRecord::new(id, i32::from(id) * 100 - 700, 1500 - i32::from(id) * 200)
                .touch_major(id * 10)
                .touch_minor(id * 5)
                .size(id * 4)
                .orientation(i8::try_from(id).unwrap() * 4 - 32)
                .state(
                    [touch_state::START, touch_state::DRAG, touch_state::LIFT_60]
                        [usize::from(id % 3)],
                )
        })
        .collect();
    let bytes = records
        .iter()
        .fold(
            ReportBuilder::new().right(true).motion(-12, 34),
            |builder, touch| builder.touch(*touch),
        )
        .timestamp(0x3_ffff)
        .build();
    assert_eq!(bytes.len(), 134);
    let report = parse_mm2(&bytes).expect("report");
    assert_eq!(
        report.header,
        Header {
            clicks: 0x02,
            dx: -12,
            dy: 34,
            timestamp: Some(0x3_ffff)
        }
    );
    assert_eq!(report.touches.len(), 15);
    for (touch, record) in report.touches.iter().zip(&records) {
        assert_eq!(
            *touch,
            Touch {
                id: record.id,
                x: record.x,
                y: record.y,
                touch_major: record.touch_major,
                touch_minor: record.touch_minor,
                size: record.size,
                orientation: record.orientation,
                state: TouchState::from_bits(record.state),
            }
        );
    }
}

#[test]
fn short_report_carries_header_without_touch_area() {
    let report = parse_mm2(&[0x12, 0x03, 0x02, 0x00, 0xfe, 0xff, 0xaa, 0xbb]).expect("report");
    assert_eq!(report.format, ReportFormat::Short);
    assert!(!report.has_touch_area());
    assert_eq!(
        report.header,
        Header {
            clicks: 0x03,
            dx: 2,
            dy: -2,
            timestamp: None
        }
    );
    assert!(report.touches.is_empty());
}

#[test]
fn header_click_bits() {
    let header = |clicks| Header {
        clicks,
        ..Header::default()
    };
    assert!(!header(0x00).is_clicked());
    assert!(!header(0x80).is_clicked());
    assert!(header(0x01).left() && !header(0x01).right());
    assert!(header(0x02).right() && !header(0x02).left());
    assert_eq!(header(0xff).click_bits(), 0x03);
}

#[test]
fn touch_states_follow_the_reference() {
    assert_eq!(TouchState::from_bits(0x00), TouchState::None);
    assert_eq!(TouchState::from_bits(0x0f), TouchState::None);
    assert_eq!(TouchState::from_bits(0x20), TouchState::Hover);
    assert_eq!(TouchState::from_bits(0x33), TouchState::Start);
    assert_eq!(TouchState::from_bits(0x40), TouchState::Drag);
    for lift in [0x50, 0x60, 0x70] {
        assert_eq!(TouchState::from_bits(lift), TouchState::Lift(lift));
    }
    for unknown in [0x10, 0x80, 0x90, 0xf0] {
        assert_eq!(TouchState::from_bits(unknown), TouchState::Unknown(unknown));
    }
    for bits in (0..=0xf0).step_by(0x10) {
        assert_eq!(TouchState::from_bits(bits).bits(), bits);
    }
    assert!(!TouchState::None.is_down());
    assert!(TouchState::Hover.is_down());
}

#[test]
fn other_report_ids_are_ignored() {
    for report_id in [0x00, 0x10, 0x11, 0x13, 0x29, 0x90, 0xf7, 0xff] {
        let mut bytes = golden(&[TOUCH_A]);
        bytes[0] = report_id;
        let error = parse_mm2(&bytes).unwrap_err();
        assert_eq!(error, ParseError::Ignored { report_id });
        assert!(!error.is_malformed());
    }
    assert_eq!(
        parse_mm2(&[0x90, 0x04]),
        Err(ParseError::Ignored { report_id: 0x90 })
    );
}

#[test]
fn wrong_lengths_are_errors() {
    let full = golden(&[TOUCH_A, TOUCH_B]);
    for len in [0, 1, 2, 5, 6, 7, 9, 13, 15, 21, 23, 29] {
        let error = parse_mm2(&full[..len.min(full.len())]).unwrap_err();
        assert_eq!(error, ParseError::InvalidLength { len }, "length {len}");
        assert!(error.is_malformed());
    }
}

#[test]
fn more_than_fifteen_touches_is_an_error() {
    let mut bytes = golden(&[]);
    for _ in 0..16 {
        bytes.extend_from_slice(&TOUCH_A);
    }
    let error = parse_mm2(&bytes).unwrap_err();
    assert_eq!(
        error,
        ParseError::TooManyTouches {
            touches: 16,
            max: 15
        }
    );
    assert!(error.is_malformed());
}

#[test]
fn builder_reports_parse_back_to_their_fields() {
    let touch = TouchRecord::new(9, -1100, 2047)
        .state(touch_state::LIFT_50)
        .orientation(-7);
    let bytes = ReportBuilder::new()
        .left(true)
        .motion(i16::MIN, i16::MAX)
        .timestamp(5)
        .touch(touch)
        .build();
    let report = parse_mm2(&bytes).expect("report");
    assert_eq!(report.header.dx, -32768);
    assert_eq!(report.header.dy, 32767);
    assert_eq!(report.header.timestamp, Some(5));
    assert_eq!(report.touches[0].position(), SurfacePoint::new(-1100, 2047));
    assert_eq!(report.touches[0].state, TouchState::Lift(0x50));
    assert_eq!(report.touches[0].orientation, -7);
}

proptest! {
    #[test]
    fn arbitrary_bytes_never_panic(bytes in proptest::collection::vec(any::<u8>(), 0..200)) {
        let _ = parse_mm2(&bytes);
    }

    #[test]
    fn arbitrary_mm2_reports_never_panic_and_decode_every_record(
        touches in 0_usize..=16,
        extra in 0_usize..8,
        body in proptest::collection::vec(any::<u8>(), 200),
    ) {
        let len = 14 + touches * 8 + extra;
        let mut bytes = body[..len].to_vec();
        bytes[0] = 0x12;
        match parse_mm2(&bytes) {
            Ok(report) => {
                prop_assert_eq!(extra, 0);
                prop_assert!(touches <= 15);
                prop_assert_eq!(report.touches.len(), touches);
                for touch in &report.touches {
                    prop_assert!(touch.id < 16);
                    prop_assert!((-2048..=2047).contains(&touch.x));
                    prop_assert!((-2047..=2048).contains(&touch.y));
                    prop_assert!(touch.size < 64);
                    prop_assert!((-32..=31).contains(&touch.orientation));
                    prop_assert!(report.header.timestamp.is_some_and(|ts| ts < 1 << 18));
                }
            }
            Err(error) => {
                prop_assert!(error.is_malformed());
                prop_assert!(extra != 0 || touches > 15);
            }
        }
    }
}
