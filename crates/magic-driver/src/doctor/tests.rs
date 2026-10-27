use std::fs;
use std::path::{Path, PathBuf};

use md_core::config::{Config, Diagnostic, DiagnosticKind, ParsedConfig};

use super::checks::{self, Check, NodeAcl, Permissions, ServiceState, Verdict};
use super::*;
use crate::config_file::LoadOutcome;

fn fixture(name: &str) -> String {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("src/doctor/fixtures")
        .join(name);
    fs::read_to_string(&path).unwrap_or_else(|error| panic!("{}: {error}", path.display()))
}

fn listed(name: &str) -> NodeAcl {
    NodeAcl::Listed(fixture(name))
}

#[test]
fn the_installed_rule_must_match_the_shipped_one() {
    let missing = checks::udev_rule(None, SHIPPED_RULE);
    assert_eq!(missing.verdict, Verdict::Fail);
    assert!(missing.hint.unwrap().contains("scripts/install.sh"));
    let same = checks::udev_rule(Some(&format!("{SHIPPED_RULE}\n\n")), SHIPPED_RULE);
    assert_eq!(same.verdict, Verdict::Pass);
    let old = SHIPPED_RULE.replace("TAG+=\"uaccess\"", "MODE=\"0666\"");
    assert_eq!(
        checks::udev_rule(Some(&old), SHIPPED_RULE).verdict,
        Verdict::Warn
    );
}

#[test]
fn getfacl_output_is_parsed_with_the_effective_mask() {
    assert_eq!(
        checks::acl_permissions(&fixture("getfacl-uinput-granted.txt"), "owner"),
        Some(Permissions {
            read: true,
            write: true
        })
    );
    assert_eq!(
        checks::acl_permissions(&fixture("getfacl-event-masked.txt"), "owner"),
        Some(Permissions {
            read: false,
            write: false
        })
    );
    assert_eq!(
        checks::acl_permissions(&fixture("getfacl-hidraw-read-only.txt"), "owner"),
        Some(Permissions {
            read: true,
            write: false
        })
    );
    assert_eq!(
        checks::acl_permissions(&fixture("getfacl-hidraw-no-acl.txt"), "owner"),
        None
    );
    assert_eq!(
        checks::acl_permissions(&fixture("getfacl-uinput-granted.txt"), "other"),
        None
    );
}

#[test]
fn acl_checks_pass_warn_or_fail_per_node() {
    let uinput = Path::new("/dev/uinput");
    let hidraw = Path::new("/dev/hidraw5");
    let check = |node: &NodeAcl, path: &Path, write: bool| {
        checks::acl("node", path, node, "owner", write).verdict
    };
    assert_eq!(
        check(&listed("getfacl-uinput-granted.txt"), uinput, true),
        Verdict::Pass
    );
    assert_eq!(
        check(&listed("getfacl-hidraw-read-only.txt"), hidraw, false),
        Verdict::Pass
    );
    assert_eq!(
        check(&listed("getfacl-hidraw-read-only.txt"), uinput, true),
        Verdict::Fail
    );
    assert_eq!(
        check(&listed("getfacl-event-masked.txt"), hidraw, false),
        Verdict::Fail
    );
    let missing = checks::acl(
        "the raw device",
        hidraw,
        &listed("getfacl-hidraw-no-acl.txt"),
        "owner",
        false,
    );
    assert_eq!(missing.verdict, Verdict::Fail);
    assert_eq!(missing.detail, "/dev/hidraw5 has no ACL entry for owner");
    assert_eq!(check(&NodeAcl::NotPresent, hidraw, false), Verdict::Warn);
    let unreadable = checks::acl(
        "node",
        hidraw,
        &NodeAcl::Unreadable("cannot run getfacl".to_owned()),
        "owner",
        false,
    );
    assert_eq!(unreadable.verdict, Verdict::Warn);
    assert!(unreadable.hint.unwrap().contains("acl package"));
}

#[test]
fn uinput_must_exist_and_load_on_boot() {
    let modules = "uinput\n";
    assert_eq!(
        checks::uinput(true, true, Some(modules)).verdict,
        Verdict::Pass
    );
    assert_eq!(
        checks::uinput(true, false, Some(modules)).verdict,
        Verdict::Pass
    );
    assert_eq!(checks::uinput(true, true, None).verdict, Verdict::Warn);
    assert_eq!(
        checks::uinput(true, true, Some("evdev\n")).verdict,
        Verdict::Warn
    );
    assert_eq!(
        checks::uinput(false, false, Some(modules)).verdict,
        Verdict::Fail
    );
}

#[test]
fn config_validity_is_reported() {
    let path = PathBuf::from("/home/owner/.config/magic-driver/config.toml");
    let diagnostic = |kind| Diagnostic {
        line: Some(3),
        key: Some("scroll.enabled".to_owned()),
        kind,
    };
    let loaded = LoadOutcome::Loaded(ParsedConfig {
        config: Config::default(),
        warnings: vec![],
    });
    assert_eq!(
        checks::config(Some(&path), Some(&loaded)).verdict,
        Verdict::Pass
    );
    let warned = LoadOutcome::Loaded(ParsedConfig {
        config: Config::default(),
        warnings: vec![diagnostic(DiagnosticKind::UnknownKey)],
    });
    let check = checks::config(Some(&path), Some(&warned));
    assert_eq!(check.verdict, Verdict::Warn);
    assert!(
        check
            .detail
            .contains("config.toml:3: scroll.enabled: unknown key")
    );
    let missing = LoadOutcome::MissingUsingDefaults(Config::default());
    assert_eq!(
        checks::config(Some(&path), Some(&missing)).verdict,
        Verdict::Pass
    );
    let invalid = LoadOutcome::Invalid {
        errors: vec![diagnostic(DiagnosticKind::InvalidValue(
            "expected a boolean".to_owned(),
        ))],
        warnings: vec![],
    };
    let check = checks::config(Some(&path), Some(&invalid));
    assert_eq!(check.verdict, Verdict::Fail);
    assert!(
        check
            .detail
            .contains("config.toml:3: scroll.enabled: invalid value")
    );
    let unreadable = LoadOutcome::Unreadable("not a regular file".to_owned());
    assert_eq!(
        checks::config(Some(&path), Some(&unreadable)).verdict,
        Verdict::Fail
    );
    assert_eq!(checks::config(None, None).verdict, Verdict::Warn);
}

fn keyd<const N: usize>(names: [&str; N]) -> Check {
    let configs: Vec<_> = names
        .iter()
        .map(|name| (PathBuf::from(format!("/etc/keyd/{name}")), fixture(name)))
        .collect();
    checks::keyd(true, &configs)
}

#[test]
fn keyd_must_not_match_the_mouse_or_the_virtual_devices() {
    assert_eq!(keyd(["keyd-default.conf"]).verdict, Verdict::Pass);
    assert_eq!(keyd(["keyd-excludes-mouse.conf"]).verdict, Verdict::Pass);
    let mouse = keyd(["keyd-default.conf", "keyd-matches-mouse.conf"]);
    assert_eq!(mouse.verdict, Verdict::Fail);
    assert_eq!(
        mouse.detail,
        "/etc/keyd/keyd-matches-mouse.conf:3 `004c:0269` matches the Magic Mouse 2"
    );
    let virtual_device = keyd(["keyd-matches-virtual.conf"]);
    assert_eq!(virtual_device.verdict, Verdict::Fail);
    assert!(
        virtual_device
            .detail
            .contains("matches magic-driver touchpad")
    );
    assert!(!virtual_device.detail.contains("046d"));
    let every_mouse = keyd(["keyd-matches-every-mouse.conf"]);
    assert_eq!(every_mouse.verdict, Verdict::Fail);
    assert!(every_mouse.detail.contains("every mouse"));
    assert_eq!(checks::keyd(false, &[]).verdict, Verdict::Pass);
}

#[test]
fn touchpad_settings_are_read_from_kcminputrc() {
    let verdicts = |text: Option<&str>| {
        checks::touchpad_settings(text)
            .into_iter()
            .map(|check| check.verdict)
            .collect::<Vec<_>>()
    };
    assert_eq!(
        verdicts(Some(&fixture("kcminputrc-owner.txt"))),
        [Verdict::Pass, Verdict::Pass]
    );
    assert_eq!(
        verdicts(Some(&fixture("kcminputrc-touchpad-clean.txt"))),
        [Verdict::Pass, Verdict::Pass]
    );
    let on = checks::touchpad_settings(Some(&fixture("kcminputrc-touchpad-tap-on.txt")));
    assert_eq!(on[0].verdict, Verdict::Warn);
    assert_eq!(on[0].detail, "tap-to-click is on for magic-driver touchpad");
    assert_eq!(on[1].verdict, Verdict::Warn);
    assert!(on[1].detail.contains("scrolling stops"));
    assert_eq!(verdicts(None), [Verdict::Pass, Verdict::Pass]);
}

#[test]
fn high_resolution_scroll_warnings_are_attributed_to_their_device() {
    assert_eq!(
        checks::hires_warnings(Ok(&fixture("journal-empty.txt"))).verdict,
        Verdict::Pass
    );
    let stock = checks::hires_warnings(Ok(&fixture("journal-hires-stock.txt")));
    assert_eq!(stock.verdict, Verdict::Warn);
    assert_eq!(
        stock.detail,
        "2 from Owner’s Magic Mouse since boot, all from the stock device"
    );
    let ours = checks::hires_warnings(Ok(&fixture("journal-hires-virtual.txt")));
    assert_eq!(ours.verdict, Verdict::Fail);
    assert!(ours.detail.contains("1 from magic-driver pointer"));
    assert_eq!(
        checks::hires_warnings(Err("cannot run journalctl")).verdict,
        Verdict::Warn
    );
}

#[test]
fn service_state_is_parsed_from_systemctl_show() {
    let active = ServiceState::parse(&fixture("systemctl-show-active.txt"));
    assert!(active.installed());
    assert_eq!(active.session_state(), Some("active"));
    assert_eq!(
        active.summary(),
        "active (running), daemon reports: active: Apple Magic Mouse 2 (Bluetooth) on hidraw5"
    );
    let degraded = ServiceState::parse(&fixture("systemctl-show-degraded.txt"));
    assert_eq!(degraded.session_state(), Some("degraded"));
    let missing = ServiceState::parse(&fixture("systemctl-show-not-found.txt"));
    assert!(!missing.installed());
    assert_eq!(missing.session_state(), None);
    assert_eq!(missing.summary(), "not installed as a user service");
}

#[test]
fn the_report_lists_every_check_with_hints_and_a_total() {
    let found = vec![
        checks::udev_rule(Some(SHIPPED_RULE), SHIPPED_RULE),
        checks::uinput(true, true, None),
        checks::udev_rule(None, SHIPPED_RULE),
    ];
    let text = render(&found);
    let lines: Vec<_> = text.lines().collect();
    assert!(lines[0].starts_with("pass  udev rule: "));
    assert!(lines[1].starts_with("warn  uinput module: "));
    assert!(lines[2].starts_with("      fix: "));
    assert!(lines[3].starts_with("fail  udev rule: "));
    assert_eq!(lines.last().unwrap(), &"1 passed, 1 warnings, 1 failed");
    assert_eq!(worst(&found), Verdict::Fail);
    assert_eq!(worst(&[]), Verdict::Pass);
}

#[test]
fn kcminputrc_lives_next_to_the_config_directory() {
    assert_eq!(
        kcminputrc_path(None, Some("/home/owner".into())),
        Some(PathBuf::from("/home/owner/.config/kcminputrc"))
    );
    assert_eq!(
        kcminputrc_path(Some("/xdg".into()), Some("/home/owner".into())),
        Some(PathBuf::from("/xdg/kcminputrc"))
    );
}
