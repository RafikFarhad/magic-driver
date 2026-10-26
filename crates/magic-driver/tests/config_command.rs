use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use tempfile::TempDir;

const BINARY: &str = env!("CARGO_BIN_EXE_magic-driver");

struct Run {
    code: Option<i32>,
    stdout: String,
    stderr: String,
}

impl From<Output> for Run {
    fn from(output: Output) -> Self {
        Run {
            code: output.status.code(),
            stdout: String::from_utf8(output.stdout).expect("utf8 stdout"),
            stderr: String::from_utf8(output.stderr).expect("utf8 stderr"),
        }
    }
}

fn run(args: &[&str], xdg_config_home: Option<&Path>, home: Option<&Path>) -> Run {
    let mut command = Command::new(BINARY);
    command.env_clear().args(args);
    if let Some(xdg) = xdg_config_home {
        command.env("XDG_CONFIG_HOME", xdg);
    }
    if let Some(home) = home {
        command.env("HOME", home);
    }
    command.output().expect("binary runs").into()
}

fn write_config(dir: &Path, relative: &str, content: &str) -> PathBuf {
    let path = dir.join(relative);
    fs::create_dir_all(path.parent().expect("parent")).expect("create dirs");
    fs::write(&path, content).expect("write config");
    path
}

fn defaults() -> String {
    let result = run(&["config", "defaults"], None, None);
    assert_eq!(result.code, Some(0));
    result.stdout
}

#[test]
fn config_defaults_prints_a_complete_file() {
    let text = defaults();
    for key in [
        "version = 1",
        "[pointer]",
        "middle_click = true",
        "acceleration = \"medium\"",
        "palm_rejection = \"medium\"",
        "[advanced.scroll]",
        "[advanced.touch]",
        "[advanced.gestures]",
        "level = \"info\"",
    ] {
        assert!(text.contains(key), "missing {key} in\n{text}");
    }
    assert!(!text.contains('#'));
}

#[test]
fn valid_explicit_file_prints_effective_values() {
    let dir = TempDir::new().expect("temp dir");
    let path = write_config(
        dir.path(),
        "custom.toml",
        "version = 1\n[scroll]\nacceleration = \"high\"\n[advanced.touch]\nrear_zone = 0.2\n",
    );
    let result = run(&["config", "test", path.to_str().unwrap()], None, None);
    assert_eq!(result.code, Some(0), "{}", result.stderr);
    assert!(result.stdout.contains("acceleration = \"high\""));
    assert!(result.stdout.contains("gain_max = 4.0"));
    assert!(result.stdout.contains("rear_zone = 0.2"));
    assert!(result.stderr.contains("test is successful"));
    assert!(!result.stderr.contains("warning"));
}

#[test]
fn short_flag_reads_the_xdg_config_home_file() {
    let dir = TempDir::new().expect("temp dir");
    write_config(
        dir.path(),
        "magic-driver/config.toml",
        "[pointer]\nmiddle_click = false\n",
    );
    let result = run(&["-t"], Some(dir.path()), None);
    assert_eq!(result.code, Some(0), "{}", result.stderr);
    assert!(result.stdout.contains("middle_click = false"));
    assert!(
        result.stderr.contains(
            &dir.path()
                .join("magic-driver/config.toml")
                .display()
                .to_string()
        )
    );
}

#[test]
fn short_flag_accepts_a_path() {
    let dir = TempDir::new().expect("temp dir");
    let path = write_config(dir.path(), "c.toml", "[log]\nlevel = \"debug\"\n");
    let result = run(&["-t", path.to_str().unwrap()], None, None);
    assert_eq!(result.code, Some(0), "{}", result.stderr);
    assert!(result.stdout.contains("level = \"debug\""));
}

#[test]
fn home_config_is_used_without_xdg_config_home() {
    let dir = TempDir::new().expect("temp dir");
    write_config(
        dir.path(),
        ".config/magic-driver/config.toml",
        "[gestures]\nsmart_zoom = true\n",
    );
    let result = run(&["config", "test"], None, Some(dir.path()));
    assert_eq!(result.code, Some(0), "{}", result.stderr);
    assert!(result.stdout.contains("smart_zoom = true"));
}

#[test]
fn invalid_file_reports_every_error_with_line_numbers() {
    let dir = TempDir::new().expect("temp dir");
    let path = write_config(
        dir.path(),
        "bad.toml",
        "[scroll]\nenabled = \"yes\"\n\n[advanced.touch]\nrear_zone = 2.0\nclick_guard_ms = 5000\n",
    );
    let shown = path.display().to_string();
    let result = run(&["config", "test", &shown], None, None);
    assert_eq!(result.code, Some(1));
    assert!(result.stdout.is_empty());
    assert!(
        result
            .stderr
            .contains(&format!("error: {shown}:2: scroll.enabled: invalid value")),
        "{}",
        result.stderr
    );
    assert!(
        result.stderr.contains(&format!(
            "error: {shown}:5: advanced.touch.rear_zone: value 2 is out of range"
        )),
        "{}",
        result.stderr
    );
    assert!(
        result.stderr.contains(&format!(
            "error: {shown}:6: advanced.touch.click_guard_ms: value 5000 is out of range"
        )),
        "{}",
        result.stderr
    );
    assert!(result.stderr.contains("test failed"));
}

#[test]
fn syntax_error_reports_the_line() {
    let dir = TempDir::new().expect("temp dir");
    let path = write_config(dir.path(), "broken.toml", "version = 1\n[scroll\n");
    let shown = path.display().to_string();
    let result = run(&["-t", &shown], None, None);
    assert_eq!(result.code, Some(1));
    assert!(
        result
            .stderr
            .contains(&format!("error: {shown}:2: invalid TOML")),
        "{}",
        result.stderr
    );
}

#[test]
fn unknown_keys_warn_but_pass() {
    let dir = TempDir::new().expect("temp dir");
    let path = write_config(
        dir.path(),
        "warn.toml",
        "version = 1\ncolour = \"red\"\n[scroll]\nspeed = 2\n",
    );
    let shown = path.display().to_string();
    let result = run(&["config", "test", &shown], None, None);
    assert_eq!(result.code, Some(0), "{}", result.stderr);
    assert!(
        result
            .stderr
            .contains(&format!("warning: {shown}:2: colour: unknown key, ignored"))
    );
    assert!(result.stderr.contains(&format!(
        "warning: {shown}:4: scroll.speed: unknown key, ignored"
    )));
    assert_eq!(result.stdout, defaults());
}

#[test]
fn missing_default_file_means_defaults() {
    let dir = TempDir::new().expect("temp dir");
    let result = run(&["config", "test"], Some(dir.path()), None);
    assert_eq!(result.code, Some(0), "{}", result.stderr);
    assert_eq!(result.stdout, defaults());
    assert!(result.stderr.contains("does not exist, using defaults"));
}

#[test]
fn missing_explicit_file_fails() {
    let dir = TempDir::new().expect("temp dir");
    let path = dir.path().join("absent.toml");
    let result = run(&["config", "test", path.to_str().unwrap()], None, None);
    assert_eq!(result.code, Some(1));
    assert!(result.stderr.contains("file does not exist"));
    assert!(result.stdout.is_empty());
}

#[test]
fn directory_is_not_a_config_file() {
    let dir = TempDir::new().expect("temp dir");
    let result = run(&["-t", dir.path().to_str().unwrap()], None, None);
    assert_eq!(result.code, Some(1));
    assert!(result.stderr.contains("not a regular file"));
}

#[test]
fn no_config_location_fails() {
    let result = run(&["config", "test"], None, None);
    assert_eq!(result.code, Some(1));
    assert!(result.stderr.contains("cannot locate the config file"));
}

#[test]
fn printed_defaults_pass_config_test_unchanged() {
    let dir = TempDir::new().expect("temp dir");
    let path = write_config(dir.path(), "defaults.toml", &defaults());
    let result = run(&["config", "test", path.to_str().unwrap()], None, None);
    assert_eq!(result.code, Some(0), "{}", result.stderr);
    assert_eq!(result.stdout, defaults());
    assert!(!result.stderr.contains("warning"));
}
