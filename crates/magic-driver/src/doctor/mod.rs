pub mod checks;

use std::env;
use std::ffi::OsString;
use std::fmt::Write as _;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use nix::unistd::{Uid, User};

use md_core::profile::Profile;

use self::checks::{
    Check, HidBinding, KernelModules, MAGIC_MOUSE_MODULE_FILE, MODULES_DIR, MODULES_LOAD_PATHS,
    MagicMouseModule, NodeAcl, RULE_PATHS, ServiceState, Verdict,
};
use crate::config_file::{self, ConfigLocation, LoadOutcome, default_config_path};
use crate::discovery::parse_hid_id;
use crate::discovery::udev_directory::enumerate_mice;
use crate::status::UINPUT_PATH;

pub const SHIPPED_RULE: &str = include_str!("../../../../dist/70-magic-driver.rules");
pub const SERVICE_NAME: &str = "magic-driver.service";
const KEYD_DIR: &str = "/etc/keyd";
const OSRELEASE_PATH: &str = "proc/sys/kernel/osrelease";
const HID_DEVICES_PATH: &str = "sys/bus/hid/devices";
const SKIPPED_MODULE_TREES: [&str; 2] = ["build", "source"];

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

fn find_module_file(dir: &Path, relative: &Path) -> Option<PathBuf> {
    let mut entries: Vec<_> = fs::read_dir(dir).ok()?.flatten().collect();
    entries.sort_by_key(fs::DirEntry::file_name);
    for entry in entries {
        let file_name = entry.file_name();
        let Ok(file_type) = entry.file_type() else {
            continue;
        };
        if file_type.is_dir() {
            if relative.as_os_str().is_empty()
                && SKIPPED_MODULE_TREES
                    .iter()
                    .any(|skipped| file_name == *skipped)
            {
                continue;
            }
            if let Some(found) = find_module_file(&entry.path(), &relative.join(&file_name)) {
                return Some(found);
            }
        } else if file_name
            .to_string_lossy()
            .starts_with(MAGIC_MOUSE_MODULE_FILE)
        {
            return Some(relative.join(file_name));
        }
    }
    None
}

fn kernel_modules_state(root: &Path) -> KernelModules {
    let release = match fs::read_to_string(root.join(OSRELEASE_PATH)) {
        Ok(text) if !text.trim().is_empty() => text.trim().to_owned(),
        Ok(_) => return KernelModules::UnknownRelease(format!("/{OSRELEASE_PATH} is empty")),
        Err(error) => {
            return KernelModules::UnknownRelease(format!("/{OSRELEASE_PATH}: {error}"));
        }
    };
    let dir = root
        .join(MODULES_DIR.trim_start_matches('/'))
        .join(&release);
    if !dir.is_dir() {
        return KernelModules::Missing { release };
    }
    let built_in = fs::read_to_string(dir.join("modules.builtin")).is_ok_and(|text| {
        text.lines().any(|line| {
            Path::new(line.trim())
                .file_name()
                .is_some_and(|name| name == MAGIC_MOUSE_MODULE_FILE)
        })
    });
    let magicmouse = if built_in {
        MagicMouseModule::BuiltIn
    } else {
        find_module_file(&dir, Path::new(""))
            .map_or(MagicMouseModule::NotFound, MagicMouseModule::File)
    };
    KernelModules::Installed {
        release,
        magicmouse,
    }
}

fn is_known_mouse(device: &str) -> bool {
    device
        .split_once('.')
        .and_then(|(id, _)| parse_hid_id(id))
        .and_then(|(bus, vendor, product)| Profile::lookup(bus, vendor, product))
        .is_some()
}

fn hid_bindings(root: &Path) -> Result<Vec<HidBinding>, String> {
    let dir = root.join(HID_DEVICES_PATH);
    let entries = fs::read_dir(&dir).map_err(|error| format!("/{HID_DEVICES_PATH}: {error}"))?;
    let mut bindings: Vec<_> = entries
        .flatten()
        .filter_map(|entry| {
            let device = entry.file_name().into_string().ok()?;
            is_known_mouse(&device).then(|| HidBinding {
                driver: fs::read_link(entry.path().join("driver"))
                    .ok()
                    .and_then(|target| Some(target.file_name()?.to_string_lossy().into_owned())),
                device,
            })
        })
        .collect();
    bindings.sort_by(|left, right| left.device.cmp(&right.device));
    Ok(bindings)
}

fn first_readable(paths: &[&'static str]) -> Option<(&'static str, String)> {
    paths
        .iter()
        .find_map(|path| fs::read_to_string(path).ok().map(|text| (*path, text)))
}

pub fn collect() -> Vec<Check> {
    let mut found = Vec::new();
    let rule = first_readable(&RULE_PATHS);
    found.push(checks::udev_rule(
        rule.as_ref().map(|(path, text)| (*path, text.as_str())),
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
        first_readable(&MODULES_LOAD_PATHS)
            .as_ref()
            .map(|(path, text)| (*path, text.as_str())),
    ));
    let root = Path::new("/");
    let modules = kernel_modules_state(root);
    found.push(checks::kernel_modules(&modules));
    found.extend(checks::hid_driver(
        hid_bindings(root).as_deref().map_err(String::as_str),
        &modules,
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
