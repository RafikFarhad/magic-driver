use std::collections::BTreeMap;
use std::fmt;
use std::path::{Path, PathBuf};

use crate::config_file::{LoadOutcome, format_diagnostic};
use crate::io::uinput::{POINTER_IDENTITY, TOUCHPAD_IDENTITY, VirtualDeviceIdentity};

pub const RULE_PATHS: [&str; 2] = [
    "/etc/udev/rules.d/70-magic-driver.rules",
    "/usr/lib/udev/rules.d/70-magic-driver.rules",
];
pub const MODULES_LOAD_PATHS: [&str; 2] = [
    "/etc/modules-load.d/magic-driver.conf",
    "/usr/lib/modules-load.d/magic-driver.conf",
];
pub const HIRES_WARNING: &str = "high-resolution scroll";
pub const MAGIC_MOUSE_KEYD_ID: &str = "004c:0269";
pub const MODULES_DIR: &str = "/lib/modules";
pub const MAGIC_MOUSE_DRIVER: &str = "magicmouse";
pub const MAGIC_MOUSE_MODULE_FILE: &str = "hid-magicmouse.ko";

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Verdict {
    Pass,
    Warn,
    Fail,
}

impl fmt::Display for Verdict {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Verdict::Pass => "pass",
            Verdict::Warn => "warn",
            Verdict::Fail => "fail",
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Check {
    pub name: String,
    pub verdict: Verdict,
    pub detail: String,
    pub hint: Option<String>,
}

impl Check {
    fn pass(name: impl Into<String>, detail: impl Into<String>) -> Check {
        Check {
            name: name.into(),
            verdict: Verdict::Pass,
            detail: detail.into(),
            hint: None,
        }
    }

    fn warn(name: impl Into<String>, detail: impl Into<String>, hint: impl Into<String>) -> Check {
        Check {
            name: name.into(),
            verdict: Verdict::Warn,
            detail: detail.into(),
            hint: Some(hint.into()),
        }
    }

    fn fail(name: impl Into<String>, detail: impl Into<String>, hint: impl Into<String>) -> Check {
        Check {
            name: name.into(),
            verdict: Verdict::Fail,
            detail: detail.into(),
            hint: Some(hint.into()),
        }
    }
}

const REINSTALL_HINT: &str = "install the magic-driver package, or run scripts/install.sh from the magic-driver repository (see docs/install.md)";

fn normalized(text: &str) -> Vec<&str> {
    text.lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .collect()
}

pub fn udev_rule(installed: Option<(&str, &str)>, shipped: &str) -> Check {
    let name = "udev rule";
    match installed {
        None => Check::fail(
            name,
            format!(
                "{} is not installed, so the mouse nodes and /dev/uinput get no uaccess ACL",
                RULE_PATHS.join(" or ")
            ),
            REINSTALL_HINT,
        ),
        Some((path, text)) if normalized(text) == normalized(shipped) => Check::pass(
            name,
            format!("{path} is installed and matches the shipped rule"),
        ),
        Some((path, _)) => Check::warn(
            name,
            format!("{path} differs from dist/70-magic-driver.rules of this version"),
            REINSTALL_HINT,
        ),
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Permissions {
    pub read: bool,
    pub write: bool,
}

pub fn acl_permissions(getfacl: &str, user: &str) -> Option<Permissions> {
    let prefix = format!("user:{user}:");
    getfacl.lines().find_map(|line| {
        let rest = line.trim().strip_prefix(&prefix)?;
        let mut fields = rest.split_whitespace();
        let mut perms = fields.next()?;
        for field in fields {
            if let Some(effective) = field.strip_prefix("#effective:") {
                perms = effective;
            }
        }
        let bytes = perms.as_bytes();
        Some(Permissions {
            read: bytes.first() == Some(&b'r'),
            write: bytes.get(1) == Some(&b'w'),
        })
    })
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NodeAcl {
    NotPresent,
    Unreadable(String),
    Listed(String),
}

pub fn acl(label: &str, path: &Path, node: &NodeAcl, user: &str, need_write: bool) -> Check {
    let name = format!("ACL on {label}");
    let wanted = if need_write { "rw" } else { "r" };
    match node {
        NodeAcl::NotPresent => Check::warn(
            name,
            format!("{} is not present", path.display()),
            if label == "/dev/uinput" {
                "load the uinput module (it loads on boot after scripts/install.sh) and run magic-driver doctor again"
            } else {
                "connect the Magic Mouse 2 and run magic-driver doctor again"
            },
        ),
        NodeAcl::Unreadable(reason) => Check::warn(
            name,
            format!("cannot read the ACL of {}: {reason}", path.display()),
            "install the acl package (getfacl) and run magic-driver doctor again",
        ),
        NodeAcl::Listed(text) => match acl_permissions(text, user) {
            Some(perms) if perms.read && (perms.write || !need_write) => Check::pass(
                name,
                format!("{} grants {user} {wanted} through an ACL", path.display()),
            ),
            Some(_) => Check::fail(
                name,
                format!(
                    "{} has an ACL entry for {user} but it does not grant {wanted}",
                    path.display()
                ),
                "check the mask with getfacl; reinstall the udev rule and turn the mouse off and on",
            ),
            None => Check::fail(
                name,
                format!("{} has no ACL entry for {user}", path.display()),
                "install the udev rule (scripts/install.sh), make sure this is the active local session, then turn the mouse off and on",
            ),
        },
    }
}

pub fn uinput(dev_exists: bool, module_loaded: bool, modules_load: Option<(&str, &str)>) -> Check {
    let name = "uinput module";
    let loads_on_boot =
        modules_load.filter(|(_, text)| text.lines().any(|line| line.trim() == "uinput"));
    if !dev_exists {
        return Check::fail(
            name,
            "/dev/uinput does not exist, so no virtual devices can be created",
            "reboot after scripts/install.sh, or load it once with: sudo modprobe uinput",
        );
    }
    let state = if module_loaded {
        "loaded"
    } else {
        "available as a static node, loads on first use"
    };
    match loads_on_boot {
        Some((path, _)) => Check::pass(
            name,
            format!("/dev/uinput exists ({state}) and {path} loads it on boot"),
        ),
        None => Check::warn(
            name,
            format!(
                "/dev/uinput exists ({state}) but neither {} loads it on boot",
                MODULES_LOAD_PATHS.join(" nor ")
            ),
            REINSTALL_HINT,
        ),
    }
}

pub fn config(path: Option<&Path>, outcome: Option<&LoadOutcome>) -> Check {
    let name = "config";
    let (Some(path), Some(outcome)) = (path, outcome) else {
        return Check::warn(
            name,
            "cannot locate the config file: neither XDG_CONFIG_HOME nor HOME is set",
            "run magic-driver doctor from your normal login session",
        );
    };
    let shown = path.display();
    match outcome {
        LoadOutcome::Loaded(parsed) if parsed.warnings.is_empty() => {
            Check::pass(name, format!("{shown} is valid"))
        }
        LoadOutcome::Loaded(parsed) => Check::warn(
            name,
            format!(
                "{shown} is valid with warnings: {}",
                parsed
                    .warnings
                    .iter()
                    .map(|warning| format_diagnostic("warning", path, warning))
                    .collect::<Vec<_>>()
                    .join("; ")
            ),
            "remove or fix the unknown keys, then: systemctl --user reload magic-driver",
        ),
        LoadOutcome::MissingUsingDefaults(_) => Check::pass(
            name,
            format!("{shown} does not exist, the defaults are used"),
        ),
        LoadOutcome::Invalid { errors, .. } => Check::fail(
            name,
            format!(
                "{shown} is invalid: {}",
                errors
                    .iter()
                    .map(|error| format_diagnostic("error", path, error))
                    .collect::<Vec<_>>()
                    .join("; ")
            ),
            "fix the file, check it with magic-driver config test, then: systemctl --user reload magic-driver",
        ),
        LoadOutcome::Unreadable(reason) => Check::fail(
            name,
            format!("cannot read {shown}: {reason}"),
            "make the file a readable regular file, or remove it to use the defaults",
        ),
    }
}

fn identity_id(identity: &VirtualDeviceIdentity) -> String {
    format!("{:04x}:{:04x}", identity.vendor, identity.product)
}

fn keyd_ids(text: &str) -> Vec<(usize, String)> {
    let mut in_ids = false;
    let mut ids = Vec::new();
    for (index, raw) in text.lines().enumerate() {
        let line = raw.split('#').next().unwrap_or_default().trim();
        if line.is_empty() {
            continue;
        }
        if line.starts_with('[') {
            in_ids = line == "[ids]";
            continue;
        }
        if in_ids {
            ids.extend(
                line.split_whitespace()
                    .map(|id| (index + 1, id.to_ascii_lowercase())),
            );
        }
    }
    ids
}

fn keyd_match(id: &str, protected: &[(String, &'static str)]) -> Option<&'static str> {
    if id.starts_with('-') {
        return None;
    }
    let (kind, id) = match id.split_once(':') {
        Some(("k" | "m", rest)) => (id.as_bytes()[0], rest),
        _ => (b'k', id),
    };
    if id == "*" {
        return (kind == b'm').then_some("every mouse");
    }
    let mut fields = id.split(':');
    let vendor_product = format!("{}:{}", fields.next()?, fields.next()?);
    protected
        .iter()
        .find(|(candidate, _)| *candidate == vendor_product)
        .map(|(_, name)| *name)
}

pub fn keyd(present: bool, configs: &[(PathBuf, String)]) -> Check {
    let name = "keyd";
    if !present {
        return Check::pass(name, "keyd is not configured (/etc/keyd does not exist)");
    }
    let protected = [
        (MAGIC_MOUSE_KEYD_ID.to_owned(), "the Magic Mouse 2"),
        (identity_id(&POINTER_IDENTITY), POINTER_IDENTITY.name),
        (identity_id(&TOUCHPAD_IDENTITY), TOUCHPAD_IDENTITY.name),
    ];
    let mut matches = Vec::new();
    for (path, text) in configs {
        for (line, id) in keyd_ids(text) {
            if let Some(device) = keyd_match(&id, &protected) {
                matches.push(format!("{}:{line} `{id}` matches {device}", path.display()));
            }
        }
    }
    if matches.is_empty() {
        Check::pass(
            name,
            format!(
                "{} keyd config file{} checked, none matches the mouse or the virtual devices",
                configs.len(),
                if configs.len() == 1 { "" } else { "s" }
            ),
        )
    } else {
        Check::fail(
            name,
            matches.join("; "),
            "remove these ids from the [ids] section (or exclude them with a leading -), then: sudo keyd reload",
        )
    }
}

fn kcminputrc_groups(text: &str) -> BTreeMap<String, BTreeMap<String, String>> {
    let mut groups: BTreeMap<String, BTreeMap<String, String>> = BTreeMap::new();
    let mut current: Option<String> = None;
    for line in text.lines() {
        let line = line.trim();
        if line.starts_with('[') && line.ends_with(']') {
            current = Some(line.to_owned());
            groups.entry(line.to_owned()).or_default();
            continue;
        }
        let (Some(group), Some((key, value))) = (current.as_ref(), line.split_once('=')) else {
            continue;
        };
        groups
            .entry(group.clone())
            .or_default()
            .insert(key.trim().to_owned(), value.trim().to_owned());
    }
    groups
}

pub fn touchpad_settings(kcminputrc: Option<&str>) -> Vec<Check> {
    let groups = kcminputrc.map(kcminputrc_groups).unwrap_or_default();
    let touchpad: Vec<_> = groups
        .iter()
        .filter(|(header, _)| header.contains(TOUCHPAD_IDENTITY.name))
        .map(|(_, keys)| keys)
        .collect();
    let setting = |key: &str| {
        touchpad
            .iter()
            .filter_map(|keys| keys.get(key))
            .any(|value| value.eq_ignore_ascii_case("true"))
    };
    let tap_name = "tap-to-click";
    let tap = if setting("TapToClick") {
        Check::warn(
            tap_name,
            format!("tap-to-click is on for {}", TOUCHPAD_IDENTITY.name),
            format!(
                "turn it off in System Settings, Touchpad, device {}",
                TOUCHPAD_IDENTITY.name
            ),
        )
    } else {
        Check::pass(
            tap_name,
            format!("tap-to-click is off for {}", TOUCHPAD_IDENTITY.name),
        )
    };
    let dwm_name = "disable touchpad while a mouse is connected";
    let dwm = if setting("DisableEventsOnExternalMouse") {
        Check::warn(
            dwm_name,
            format!(
                "on for {}, and magic-driver pointer counts as a mouse, so scrolling stops",
                TOUCHPAD_IDENTITY.name
            ),
            format!(
                "turn it off in System Settings, Touchpad, device {}",
                TOUCHPAD_IDENTITY.name
            ),
        )
    } else {
        Check::pass(dwm_name, format!("off for {}", TOUCHPAD_IDENTITY.name))
    };
    vec![tap, dwm, scroll_method(&touchpad)]
}

fn scroll_method_name(value: &str) -> &'static str {
    match value {
        "0" => "no scrolling",
        "2" => "edge scrolling",
        "4" => "on-button-down scrolling",
        _ => "an unknown scroll method",
    }
}

fn scroll_method(touchpad: &[&BTreeMap<String, String>]) -> Check {
    let name = "touchpad scroll method";
    let other = touchpad
        .iter()
        .filter_map(|keys| keys.get("ScrollMethod"))
        .find(|value| value.as_str() != "1");
    match other {
        Some(value) => Check::warn(
            name,
            format!(
                "{} is set to {} (ScrollMethod={value}) instead of two fingers, so magic-driver scrolling does not work",
                TOUCHPAD_IDENTITY.name,
                scroll_method_name(value)
            ),
            format!(
                "choose Two fingers in System Settings, Touchpad, device {}, Scrolling",
                TOUCHPAD_IDENTITY.name
            ),
        ),
        None => Check::pass(
            name,
            format!("two-finger scrolling for {}", TOUCHPAD_IDENTITY.name),
        ),
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MagicMouseModule {
    File(PathBuf),
    BuiltIn,
    NotFound,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum KernelModules {
    UnknownRelease(String),
    Missing {
        release: String,
    },
    Installed {
        release: String,
        magicmouse: MagicMouseModule,
    },
}

pub fn kernel_modules(modules: &KernelModules) -> Check {
    let name = "kernel modules";
    match modules {
        KernelModules::UnknownRelease(reason) => Check::warn(
            name,
            format!("cannot read the running kernel release: {reason}"),
            format!("compare the output of uname -r with the directories in {MODULES_DIR}"),
        ),
        KernelModules::Missing { release } => Check::fail(
            name,
            format!(
                "{MODULES_DIR}/{release} does not exist, so the running kernel {release} cannot load hid_magicmouse or any other module"
            ),
            "the kernel was upgraded or removed without a reboot; reboot into the installed kernel",
        ),
        KernelModules::Installed {
            release,
            magicmouse: MagicMouseModule::File(path),
        } => Check::pass(
            name,
            format!(
                "{MODULES_DIR}/{release} exists and has hid_magicmouse ({})",
                path.display()
            ),
        ),
        KernelModules::Installed {
            release,
            magicmouse: MagicMouseModule::BuiltIn,
        } => Check::pass(
            name,
            format!("{MODULES_DIR}/{release} exists and hid_magicmouse is built into the kernel"),
        ),
        KernelModules::Installed {
            release,
            magicmouse: MagicMouseModule::NotFound,
        } => Check::warn(
            name,
            format!(
                "{MODULES_DIR}/{release} has no {MAGIC_MOUSE_MODULE_FILE}* and modules.builtin does not list it, so the Magic Mouse 2 stays on hid-generic"
            ),
            "use a kernel built with CONFIG_HID_MAGICMOUSE (most distribution kernels are)",
        ),
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HidBinding {
    pub device: String,
    pub driver: Option<String>,
}

pub fn hid_driver(bindings: Result<&[HidBinding], &str>, modules: &KernelModules) -> Vec<Check> {
    let name = "HID driver";
    let bindings = match bindings {
        Ok(bindings) => bindings,
        Err(reason) => {
            return vec![Check::warn(
                name,
                format!("cannot list the HID devices: {reason}"),
                "run: ls -l /sys/bus/hid/devices/*/driver",
            )];
        }
    };
    if bindings.is_empty() {
        return vec![Check::pass(
            name,
            "no Magic Mouse 2 is connected, nothing to check",
        )];
    }
    let hint = if matches!(modules, KernelModules::Missing { .. }) {
        "the running kernel has no modules directory, see the kernel modules check: reboot into the installed kernel"
    } else {
        "turn the mouse off and on to reconnect it, or load the driver with: sudo modprobe hid_magicmouse"
    };
    bindings
        .iter()
        .map(|binding| match binding.driver.as_deref() {
            Some(MAGIC_MOUSE_DRIVER) => Check::pass(
                name,
                format!("{} is bound to {MAGIC_MOUSE_DRIVER}", binding.device),
            ),
            Some(driver) => Check::fail(
                name,
                format!(
                    "{} is bound to {driver}, not {MAGIC_MOUSE_DRIVER}, so the mouse never reports touches and scrolling cannot work",
                    binding.device
                ),
                hint,
            ),
            None => Check::fail(
                name,
                format!(
                    "{} is not bound to any driver, so the mouse never reports touches and scrolling cannot work",
                    binding.device
                ),
                hint,
            ),
        })
        .collect()
}

fn hires_device(line: &str) -> Option<&str> {
    let after = line.split_once(" - ")?.1;
    Some(
        after
            .split_once(": ")
            .map_or(after, |(device, _)| device)
            .trim(),
    )
}

pub fn hires_warnings(journal: Result<&str, &str>) -> Check {
    let name = "libinput high-resolution scroll warnings";
    let text = match journal {
        Ok(text) => text,
        Err(reason) => {
            return Check::warn(
                name,
                format!("cannot read the journal: {reason}"),
                "run: journalctl -b | grep -i 'high-resolution scroll'",
            );
        }
    };
    let mut devices: BTreeMap<&str, usize> = BTreeMap::new();
    for line in text.lines() {
        if line.to_ascii_lowercase().contains(HIRES_WARNING) {
            *devices
                .entry(hires_device(line).unwrap_or("unknown device"))
                .or_default() += 1;
        }
    }
    if devices.is_empty() {
        return Check::pass(name, "none in the journal since boot");
    }
    let listed = devices
        .iter()
        .map(|(device, count)| format!("{count} from {device}"))
        .collect::<Vec<_>>()
        .join(", ");
    if devices
        .keys()
        .any(|device| device.starts_with("magic-driver"))
    {
        Check::fail(
            name,
            format!("{listed} since boot; magic-driver devices must never declare a wheel axis"),
            "report this as a bug with the output of: libinput list-devices",
        )
    } else {
        Check::warn(
            name,
            format!("{listed} since boot, all from the stock device"),
            "these come from times the stock driver was in use (before takeover or during fallback); none come from magic-driver devices",
        )
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ServiceState {
    pub load: String,
    pub active: String,
    pub sub: String,
    pub status_text: String,
}

impl ServiceState {
    pub fn parse(text: &str) -> ServiceState {
        let mut state = ServiceState::default();
        for line in text.lines() {
            let Some((key, value)) = line.split_once('=') else {
                continue;
            };
            let value = value.trim().to_owned();
            match key.trim() {
                "LoadState" => state.load = value,
                "ActiveState" => state.active = value,
                "SubState" => state.sub = value,
                "StatusText" => state.status_text = value,
                _ => {}
            }
        }
        state
    }

    pub fn installed(&self) -> bool {
        !self.load.is_empty() && self.load != "not-found"
    }

    pub fn session_state(&self) -> Option<&str> {
        let (state, _) = self.status_text.split_once(':')?;
        Some(state.trim())
    }

    pub fn summary(&self) -> String {
        if !self.installed() {
            return "not installed as a user service".to_owned();
        }
        let mut text = format!("{} ({})", self.active, self.sub);
        if !self.status_text.is_empty() {
            text.push_str(", daemon reports: ");
            text.push_str(&self.status_text);
        }
        text
    }
}
