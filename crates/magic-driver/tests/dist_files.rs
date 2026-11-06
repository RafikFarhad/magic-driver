use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

const BINARY: &str = env!("CARGO_BIN_EXE_magic-driver");

fn dist(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .nth(2)
        .expect("repository root")
        .join("dist")
        .join(name)
}

fn read(name: &str) -> String {
    fs::read_to_string(dist(name)).unwrap_or_else(|error| panic!("{name}: {error}"))
}

fn service_lines() -> Vec<String> {
    read("magic-driver.service")
        .lines()
        .map(str::to_owned)
        .collect()
}

#[test]
fn the_service_unit_has_the_expected_hardening_and_watchdog() {
    let lines = service_lines();
    for expected in [
        "[Unit]",
        "PartOf=graphical-session.target",
        "After=graphical-session.target",
        "[Service]",
        "Type=notify",
        "ExecStart=/usr/local/bin/magic-driver run",
        "WatchdogSec=2",
        "Restart=always",
        "RestartSec=1",
        "NoNewPrivileges=yes",
        "RestrictAddressFamilies=AF_UNIX AF_NETLINK",
        "LockPersonality=yes",
        "MemoryMax=64M",
        "[Install]",
        "WantedBy=graphical-session.target",
    ] {
        assert!(
            lines.iter().any(|line| line == expected),
            "{expected} missing"
        );
    }
    assert!(!lines.iter().any(|line| line.starts_with("PrivateNetwork")));
}

#[test]
fn reload_tests_the_config_before_sending_sighup() {
    let reloads: Vec<_> = service_lines()
        .into_iter()
        .filter(|line| line.starts_with("ExecReload="))
        .collect();
    assert_eq!(
        reloads,
        [
            "ExecReload=/usr/local/bin/magic-driver config test",
            "ExecReload=/bin/kill -HUP $MAINPID",
        ]
    );
}

#[test]
fn systemd_analyze_verifies_the_unit_when_available() {
    if Command::new("systemd-analyze")
        .arg("--version")
        .output()
        .is_err()
    {
        eprintln!("systemd-analyze not installed, skipping the unit verification");
        return;
    }
    let dir = tempfile::tempdir().expect("tempdir");
    let unit = dir.path().join("magic-driver.service");
    fs::write(
        &unit,
        read("magic-driver.service").replace("/usr/local/bin/magic-driver", BINARY),
    )
    .expect("write unit");
    let output = Command::new("systemd-analyze")
        .args(["--user", "verify"])
        .arg(&unit)
        .output()
        .expect("systemd-analyze runs");
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn the_example_config_is_generated_from_config_defaults() {
    let output = Command::new(BINARY)
        .args(["config", "defaults"])
        .output()
        .expect("binary runs");
    assert!(output.status.success());
    assert_eq!(
        read("config.example.toml"),
        String::from_utf8(output.stdout).expect("utf8"),
        "regenerate with: magic-driver config defaults > dist/config.example.toml"
    );
}

#[test]
fn the_example_config_passes_config_test() {
    let output = Command::new(BINARY)
        .args(["config", "test"])
        .arg(dist("config.example.toml"))
        .output()
        .expect("binary runs");
    assert!(output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("test is successful"));
    assert!(!String::from_utf8_lossy(&output.stderr).contains("warning"));
}

#[test]
fn the_modules_load_file_loads_only_uinput() {
    assert_eq!(read("modules-load.d/magic-driver.conf"), "uinput\n");
}
