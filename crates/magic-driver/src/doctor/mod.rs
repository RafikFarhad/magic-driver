pub mod checks;

use std::env;
use std::ffi::OsString;
use std::fmt::Write as _;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use nix::unistd::{Uid, User};

use self::checks::{Check, MODULES_LOAD_PATH, NodeAcl, RULE_PATH, ServiceState, Verdict};
use crate::config_file::{self, ConfigLocation, LoadOutcome, default_config_path};
use crate::discovery::udev_directory::enumerate_mice;
use crate::status::UINPUT_PATH;

pub const SHIPPED_RULE: &str = include_str!("../../../../dist/70-magic-driver.rules");
pub const SERVICE_NAME: &str = "magic-driver.service";
const KEYD_DIR: &str = "/etc/keyd";

pub fn render(checks: &[Check]) -> String {
    let mut text = String::new();
    for check in checks {
        let _ = writeln!(
            text,
            "{:<5} {}: {}",
            check.verdict.to_string(),
            check.name,
            check.detail
        );
        if let Some(hint) = &check.hint {
            let _ = writeln!(text, "      fix: {hint}");
        }
    }
    let count = |verdict| {
        checks
            .iter()
            .filter(|check| check.verdict == verdict)
            .count()
    };
    let _ = writeln!(
        text,
        "{} passed, {} warnings, {} failed",
        count(Verdict::Pass),
        count(Verdict::Warn),
        count(Verdict::Fail)
    );
    text
}

pub fn worst(checks: &[Check]) -> Verdict {
    checks
        .iter()
        .map(|check| check.verdict)
        .max()
        .unwrap_or(Verdict::Pass)
}

pub fn config_location() -> Option<PathBuf> {
    default_config_path(env::var_os("XDG_CONFIG_HOME"), env::var_os("HOME"))
}

pub fn load_config(path: Option<&Path>) -> Option<LoadOutcome> {
    path.map(|path| config_file::load(&ConfigLocation::Default(path.to_path_buf())))
}

pub fn kcminputrc_path(
    xdg_config_home: Option<OsString>,
    home: Option<OsString>,
) -> Option<PathBuf> {
    let config = default_config_path(xdg_config_home, home)?;
    Some(config.parent()?.parent()?.join("kcminputrc"))
}

fn command_output(program: &str, args: &[&str]) -> Result<String, String> {
    let output = Command::new(program)
        .args(args)
        .output()
        .map_err(|error| format!("cannot run {program}: {error}"))?;
    let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
    let stderr = String::from_utf8_lossy(&output.stderr).trim().to_owned();
    if output.status.success() || stderr.is_empty() {
        Ok(stdout)
    } else {
        Err(stderr)
    }
}

pub fn service_state() -> Result<ServiceState, String> {
    command_output(
        "systemctl",
        &[
            "--user",
            "show",
            SERVICE_NAME,
            "--property=LoadState,ActiveState,SubState,StatusText",
        ],
    )
    .map(|text| ServiceState::parse(&text))
}

fn node_acl(path: &Path) -> NodeAcl {
    if !path.exists() {
        return NodeAcl::NotPresent;
    }
    let shown = path.to_string_lossy();
    match command_output("getfacl", &["-p", "-c", "--", &shown]) {
        Ok(text) => NodeAcl::Listed(text),
        Err(reason) => NodeAcl::Unreadable(reason),
    }
}

fn current_user() -> String {
    User::from_uid(Uid::current())
        .ok()
        .flatten()
        .map(|user| user.name)
        .or_else(|| env::var("USER").ok())
        .unwrap_or_default()
}

fn keyd_configs() -> Vec<(PathBuf, String)> {
    let Ok(entries) = fs::read_dir(KEYD_DIR) else {
        return Vec::new();
    };
    let mut configs: Vec<_> = entries
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| {
            path.extension()
                .is_some_and(|extension| extension == "conf")
        })
        .filter_map(|path| fs::read_to_string(&path).ok().map(|text| (path, text)))
        .collect();
    configs.sort();
    configs
}

pub fn collect() -> Vec<Check> {
    let mut found = Vec::new();
    found.push(checks::udev_rule(
        fs::read_to_string(RULE_PATH).ok().as_deref(),
        SHIPPED_RULE,
    ));
    let user = current_user();
    match enumerate_mice() {
        Ok(mice) if !mice.is_empty() => {
            let mouse = &mice[0];
            found.push(checks::acl(
                "the raw device",
                &mouse.hidraw,
                &node_acl(&mouse.hidraw),
                &user,
                false,
            ));
            found.push(checks::acl(
                "the stock device",
                &mouse.evdev,
                &node_acl(&mouse.evdev),
                &user,
                false,
            ));
        }
        Ok(_) => found.push(checks::acl(
            "the Magic Mouse 2 nodes",
            Path::new("/dev/hidraw*"),
            &NodeAcl::NotPresent,
            &user,
            false,
        )),
        Err(error) => found.push(checks::acl(
            "the Magic Mouse 2 nodes",
            Path::new("/dev/hidraw*"),
            &NodeAcl::Unreadable(format!("cannot list udev devices: {error}")),
            &user,
            false,
        )),
    }
    let uinput = Path::new(UINPUT_PATH);
    found.push(checks::acl(
        UINPUT_PATH,
        uinput,
        &node_acl(uinput),
        &user,
        true,
    ));
    found.push(checks::uinput(
        uinput.exists(),
        Path::new("/sys/module/uinput").exists(),
        fs::read_to_string(MODULES_LOAD_PATH).ok().as_deref(),
    ));
    let config_path = config_location();
    let outcome = load_config(config_path.as_deref());
    found.push(checks::config(config_path.as_deref(), outcome.as_ref()));
    found.push(checks::keyd(Path::new(KEYD_DIR).exists(), &keyd_configs()));
    let kcminputrc = kcminputrc_path(env::var_os("XDG_CONFIG_HOME"), env::var_os("HOME"))
        .and_then(|path| fs::read_to_string(path).ok());
    found.extend(checks::touchpad_settings(kcminputrc.as_deref()));
    let journal = command_output(
        "journalctl",
        &[
            "-b",
            "-q",
            "--no-pager",
            "-o",
            "cat",
            "--case-sensitive=false",
            "-g",
            checks::HIRES_WARNING,
        ],
    );
    found.push(checks::hires_warnings(
        journal.as_deref().map_err(String::as_str),
    ));
    found
}

#[cfg(test)]
mod tests;
