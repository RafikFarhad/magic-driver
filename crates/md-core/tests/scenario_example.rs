use md_core::Micros;
use md_core::recording::{RecordingHeader, RecordingReader, RecordingWriter};
use md_core::testkit::{ReportBuilder, TouchRecord, render_reports, render_timeline, touch_state};

const REPORT_INTERVAL_US: u64 = 11_111;

fn one_finger_stroke_then_click() -> Vec<(Micros, Vec<u8>)> {
    let palm = TouchRecord::new(1, 0, 1_600)
        .touch_major(120)
        .touch_minor(90);
    let finger = |state: u8, y: i32| TouchRecord::new(2, -200, y).state(state);
    let reports = [
        ReportBuilder::new().touch(palm),
        ReportBuilder::new()
            .touch(palm)
            .touch(finger(touch_state::START, 400)),
        ReportBuilder::new()
            .touch(palm)
            .touch(finger(touch_state::DRAG, 300)),
        ReportBuilder::new()
            .touch(palm)
            .touch(finger(touch_state::DRAG, 150)),
        ReportBuilder::new()
            .touch(palm)
            .touch(finger(touch_state::LIFT_50, 140)),
        ReportBuilder::new()
            .touch(palm)
            .touch(finger(touch_state::GONE, 140)),
        ReportBuilder::new().motion(3, -2).touch(palm),
        ReportBuilder::new().left(true).touch(palm),
        ReportBuilder::new().left(false).touch(palm),
    ];
    reports
        .iter()
        .zip(0_u64..)
        .map(|(builder, index)| {
            let timestamp = u32::try_from(index * 11).expect("small timestamp");
            (
                Micros::new(index * REPORT_INTERVAL_US),
                builder.clone().timestamp(timestamp).build(),
            )
        })
        .collect()
}

fn header() -> RecordingHeader {
    RecordingHeader {
        profile_id: "magic-mouse-2-bt".to_owned(),
        magic_driver_version: "0.0.0-test".to_owned(),
        kernel_version: "none".to_owned(),
        start_unix_s: 0,
    }
}

#[test]
fn builder_reports_survive_a_recording_round_trip() {
    let reports = one_finger_stroke_then_click();

    let mut writer = RecordingWriter::new(Vec::new(), &header()).expect("header");
    for (t, bytes) in &reports {
        writer.write_report(*t, bytes).expect("report");
    }
    let recording = writer.into_inner();

    let reader = RecordingReader::new(recording.as_slice()).expect("reader");
    assert_eq!(reader.header(), &header());
    let parsed: Vec<(Micros, Vec<u8>)> = reader.collect::<Result<_, _>>().expect("reports");
    assert_eq!(parsed, reports);

    insta::assert_snapshot!(
        "one_finger_stroke_then_click_recording",
        render_reports(&parsed)
    );
    insta::assert_snapshot!(
        "one_finger_stroke_then_click_sizes",
        render_timeline(
            parsed
                .iter()
                .map(|(t, bytes)| (*t, (bytes[0], bytes[1], bytes.len())))
        )
    );
}
