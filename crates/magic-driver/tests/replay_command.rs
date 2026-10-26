use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use md_core::Micros;
use md_core::recording::{RecordingHeader, RecordingWriter};
use md_core::testkit::{ReportBuilder, TouchRecord, touch_state};
use tempfile::TempDir;

const BINARY: &str = env!("CARGO_BIN_EXE_magic-driver");
const REPORT_INTERVAL_US: u64 = 11_111;

struct Run {
    code: Option<i32>,
    stdout: String,
    stderr: String,
}

fn run(args: &[&str]) -> Run {
    let output = Command::new(BINARY)
        .env_clear()
        .args(args)
        .output()
        .expect("binary runs");
    Run {
        code: output.status.code(),
        stdout: String::from_utf8(output.stdout).expect("utf8 stdout"),
        stderr: String::from_utf8(output.stderr).expect("utf8 stderr"),
    }
}

fn header(profile_id: &str) -> RecordingHeader {
    RecordingHeader {
        profile_id: profile_id.to_owned(),
        magic_driver_version: "0.0.0-test".to_owned(),
        kernel_version: "none".to_owned(),
        start_unix_s: 0,
    }
}

fn clicks_and_motion() -> Vec<Vec<u8>> {
    let palm = TouchRecord::new(1, 0, 1600)
        .touch_major(120)
        .touch_minor(90)
        .size(30);
    let left = TouchRecord::new(2, -700, 200).size(12);
    let right = TouchRecord::new(2, 700, 200).size(12);
    let middle = TouchRecord::new(2, 0, 200).size(12);
    let builders = [
        ReportBuilder::new().motion(3, -2),
        ReportBuilder::new().motion(-40, 25),
        ReportBuilder::new().touch(left.state(touch_state::START)),
        ReportBuilder::new().left(true).touch(left),
        ReportBuilder::new().left(true).touch(right),
        ReportBuilder::new().touch(left),
        ReportBuilder::new().touch(right.state(touch_state::GONE)),
        ReportBuilder::new().left(true).touch(right),
        ReportBuilder::new().touch(right),
        ReportBuilder::new().right(true).touch(middle),
        ReportBuilder::new().motion(1, 1).right(true).touch(middle),
        ReportBuilder::new().touch(middle),
        ReportBuilder::new().right(true),
        ReportBuilder::new(),
        ReportBuilder::new().left(true).touch(palm).touch(left),
        ReportBuilder::new().touch(palm).touch(left),
    ];
    let mut reports: Vec<Vec<u8>> = builders.iter().map(ReportBuilder::build).collect();
    reports.insert(2, vec![0x29, 0x00, 0x00]);
    reports.insert(3, vec![0x12, 0x01, 0x02]);
    reports
}

fn write_fixture(dir: &Path, profile_id: &str, reports: &[Vec<u8>]) -> PathBuf {
    let mut writer = RecordingWriter::new(Vec::new(), &header(profile_id)).expect("header");
    for (index, bytes) in (0_u64..).zip(reports) {
        writer
            .write_report(Micros::new(index * REPORT_INTERVAL_US), bytes)
            .expect("report");
    }
    let path = dir.join("clicks-and-motion.rec");
    fs::write(&path, writer.into_inner()).expect("write fixture");
    path
}

#[test]
fn replay_prints_the_expected_outputs() {
    let dir = TempDir::new().expect("temp dir");
    let path = write_fixture(dir.path(), "magic-mouse-2-bt", &clicks_and_motion());
    let result = run(&["replay", path.to_str().unwrap()]);
    assert_eq!(result.code, Some(0), "{}", result.stderr);
    assert!(
        result
            .stderr
            .contains("replayed 18 reports (1 ignored, 1 malformed), 13 outputs"),
        "{}",
        result.stderr
    );
    insta::assert_snapshot!("replay_clicks_and_motion_text", result.stdout);
}

#[test]
fn replay_json_prints_one_object_per_line() {
    let dir = TempDir::new().expect("temp dir");
    let path = write_fixture(dir.path(), "magic-mouse-2-bt", &clicks_and_motion());
    let result = run(&["replay", "--json", path.to_str().unwrap()]);
    assert_eq!(result.code, Some(0), "{}", result.stderr);
    let text = run(&["replay", path.to_str().unwrap()]).stdout;
    assert_eq!(result.stdout.lines().count(), text.lines().count());
    for line in result.stdout.lines() {
        let value: serde_json::Value = serde_json::from_str(line).expect("json line");
        assert!(value["t"].is_u64(), "{line}");
        assert!(value["type"].is_string(), "{line}");
    }
    insta::assert_snapshot!("replay_clicks_and_motion_json", result.stdout);
}

#[test]
fn replay_refuses_device_nodes_and_other_non_files() {
    for path in ["/dev/null", "/dev", "/"] {
        let result = run(&["replay", path]);
        assert_eq!(result.code, Some(1), "{path}");
        assert!(result.stdout.is_empty());
        assert!(
            result.stderr.contains("not a regular file"),
            "{path}: {}",
            result.stderr
        );
    }
}

#[test]
fn replay_of_a_missing_file_fails() {
    let dir = TempDir::new().expect("temp dir");
    let path = dir.path().join("missing.rec");
    let result = run(&["replay", path.to_str().unwrap()]);
    assert_eq!(result.code, Some(1));
    assert!(result.stderr.contains("cannot open the recording"));
}

#[test]
fn replay_rejects_an_unknown_profile() {
    let dir = TempDir::new().expect("temp dir");
    let path = write_fixture(dir.path(), "magic-trackpad", &clicks_and_motion());
    let result = run(&["replay", path.to_str().unwrap()]);
    assert_eq!(result.code, Some(1));
    assert!(result.stdout.is_empty());
    assert!(
        result
            .stderr
            .contains("unknown device profile \"magic-trackpad\"")
    );
}

#[test]
fn replay_stops_at_a_malformed_line_with_its_number() {
    let dir = TempDir::new().expect("temp dir");
    let path = write_fixture(
        dir.path(),
        "magic-mouse-2-bt",
        &[ReportBuilder::new().motion(5, 5).build()],
    );
    let mut text = fs::read_to_string(&path).unwrap();
    text.push_str("20 12XY\n30 1200\n");
    fs::write(&path, text).unwrap();
    let result = run(&["replay", path.to_str().unwrap()]);
    assert_eq!(result.code, Some(1));
    assert_eq!(result.stdout, "0 pointer dx=5 dy=5\n");
    assert!(result.stderr.contains("line 3"), "{}", result.stderr);
}

#[test]
fn replay_emit_is_not_available_offline() {
    let dir = TempDir::new().expect("temp dir");
    let path = write_fixture(dir.path(), "magic-mouse-2-bt", &clicks_and_motion());
    let result = run(&["replay", "--emit", path.to_str().unwrap()]);
    assert_eq!(result.code, Some(1));
    assert!(result.stdout.is_empty());
    assert!(result.stderr.contains("replay --emit: not implemented"));
}
