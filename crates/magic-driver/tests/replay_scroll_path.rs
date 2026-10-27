use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use md_core::recording::{RecordingHeader, RecordingWriter};
use md_core::testkit::scenarios::{self, Scenario};
use tempfile::TempDir;

const BINARY: &str = env!("CARGO_BIN_EXE_magic-driver");

fn write_scenario(dir: &Path, name: &str, scenario: &Scenario) -> PathBuf {
    let header = RecordingHeader {
        profile_id: "magic-mouse-2-bt".to_owned(),
        magic_driver_version: "0.0.0-test".to_owned(),
        kernel_version: "none".to_owned(),
        start_unix_s: 0,
    };
    let mut writer = RecordingWriter::new(Vec::new(), &header).expect("header");
    for (t, bytes) in scenario.timed() {
        writer.write_report(t, &bytes).expect("report");
    }
    let path = dir.join(format!("{name}.rec"));
    fs::write(&path, writer.into_inner()).expect("write recording");
    path
}

fn replay(path: &Path) -> (String, String) {
    let output = Command::new(BINARY)
        .env_clear()
        .arg("replay")
        .arg(path)
        .output()
        .expect("binary runs");
    let stderr = String::from_utf8(output.stderr).expect("utf8 stderr");
    assert_eq!(output.status.code(), Some(0), "{stderr}");
    (
        String::from_utf8(output.stdout).expect("utf8 stdout"),
        stderr,
    )
}

#[test]
fn replay_runs_the_scroll_path_and_fires_engine_deadlines() {
    let dir = TempDir::new().expect("temp dir");
    let path = write_scenario(
        dir.path(),
        "click-while-scrolling",
        &scenarios::click_while_scrolling(),
    );
    let (stdout, stderr) = replay(&path);
    assert!(stderr.contains(" 0 malformed"), "{stderr}");
    let press = stdout
        .lines()
        .find(|line| line.ends_with("button left press"))
        .expect("press line");
    let press_t: u64 = press.split(' ').next().unwrap().parse().unwrap();
    let lift = format!("{} frame 0:- 1:- 2:- 3:- 4:-", press_t + 60_000);
    assert!(stdout.lines().any(|line| line == lift), "{stdout}");
    insta::assert_snapshot!("replay_click_while_scrolling", stdout);
}

#[test]
fn every_synthetic_scenario_replays_offline() {
    let dir = TempDir::new().expect("temp dir");
    for (name, scenario) in scenarios::all() {
        let path = write_scenario(dir.path(), name, &scenario);
        let (stdout, stderr) = replay(&path);
        assert!(stderr.contains(" 0 malformed"), "{name}: {stderr}");
        let frames = stdout
            .lines()
            .filter(|line| line.contains(" frame "))
            .count();
        let expects_frames = !matches!(
            name,
            "idle-resting-palm"
                | "move-fingers-resting"
                | "click-left"
                | "click-right"
                | "click-middle"
        );
        assert_eq!(frames > 0, expects_frames, "{name}");
        if expects_frames {
            assert!(
                stdout
                    .lines()
                    .rev()
                    .find(|line| line.contains(" frame "))
                    .unwrap()
                    .ends_with("frame 0:- 1:- 2:- 3:- 4:-"),
                "{name}"
            );
        }
    }
}
