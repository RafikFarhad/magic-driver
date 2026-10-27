use std::fmt::{self, Write as _};
use std::path::{Path, PathBuf};

use nix::unistd::{AccessFlags, access};

use crate::config_file::LoadOutcome;
use crate::discovery::MouseNodes;
use crate::discovery::udev_directory::{enumerate_input_devices_named, enumerate_mice};
use crate::doctor::checks::ServiceState;
use crate::doctor::{config_location, load_config, service_state};
use crate::io::DeviceError;
use crate::io::uinput::{POINTER_IDENTITY, TOUCHPAD_IDENTITY};

pub const UINPUT_PATH: &str = "/dev/uinput";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Access {
    ReadWrite,
    ReadOnly,
    WriteOnly,
    Denied,
    Missing,
}

impl fmt::Display for Access {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Access::ReadWrite => "read-write",
            Access::ReadOnly => "read-only",
            Access::WriteOnly => "write-only",
            Access::Denied => "no access for this user",
            Access::Missing => "missing",
        })
    }
}

pub fn access_of(path: &Path) -> Access {
    if access(path, AccessFlags::F_OK).is_err() {
        return Access::Missing;
    }
    let readable = access(path, AccessFlags::R_OK).is_ok();
    let writable = access(path, AccessFlags::W_OK).is_ok();
    match (readable, writable) {
        (true, true) => Access::ReadWrite,
        (true, false) => Access::ReadOnly,
        (false, true) => Access::WriteOnly,
        (false, false) => Access::Denied,
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct MouseStatus {
    pub nodes: MouseNodes,
    pub raw_access: Access,
    pub stock_access: Access,
}

#[derive(Debug, Clone, PartialEq)]
pub struct StatusReport {
    pub service: Result<ServiceState, String>,
    pub mice: Vec<MouseStatus>,
    pub uinput_access: Access,
    pub virtual_devices: Vec<(&'static str, Vec<PathBuf>)>,
    pub config: ConfigStatus,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConfigStatus {
    pub path: Option<PathBuf>,
    pub validity: String,
}

impl ConfigStatus {
    pub fn new(path: Option<PathBuf>, outcome: Option<&LoadOutcome>) -> ConfigStatus {
        let validity = match outcome {
            None => "unknown, neither XDG_CONFIG_HOME nor HOME is set".to_owned(),
            Some(LoadOutcome::Loaded(parsed)) if parsed.warnings.is_empty() => "valid".to_owned(),
            Some(LoadOutcome::Loaded(parsed)) => {
                format!("valid, {} warning(s)", parsed.warnings.len())
            }
            Some(LoadOutcome::MissingUsingDefaults(_)) => "missing, defaults are used".to_owned(),
            Some(LoadOutcome::Invalid { errors, .. }) => format!(
                "invalid, {} error(s), see magic-driver config test",
                errors.len()
            ),
            Some(LoadOutcome::Unreadable(reason)) => format!("unreadable: {reason}"),
        };
        ConfigStatus { path, validity }
    }
}

impl StatusReport {
    pub fn render(&self) -> String {
        let mut text = String::new();
        match &self.service {
            Ok(service) => {
                let _ = writeln!(text, "Service: {}", service.summary());
                if let Some(state) = service.session_state() {
                    let _ = writeln!(text, "Session state: {state}");
                }
            }
            Err(reason) => {
                let _ = writeln!(text, "Service: unknown ({reason})");
            }
        }
        let _ = writeln!(text, "Magic Mouse devices:");
        if self.mice.is_empty() {
            let _ = writeln!(text, "  none found");
        }
        for (index, mouse) in self.mice.iter().enumerate() {
            let nodes = &mouse.nodes;
            let role = if index == 0 {
                "taken over by `magic-driver run`"
            } else {
                "ignored in v1, only the first mouse is taken over"
            };
            let _ = writeln!(
                text,
                "  {} [{}], {role}",
                nodes.profile.name, nodes.profile.id
            );
            let _ = writeln!(text, "    hid device:   {}", nodes.key.display());
            let _ = writeln!(
                text,
                "    raw device:   {} ({})",
                nodes.hidraw.display(),
                mouse.raw_access
            );
            let _ = writeln!(
                text,
                "    stock device: {} ({})",
                nodes.evdev.display(),
                mouse.stock_access
            );
        }
        let _ = writeln!(text, "uinput: {UINPUT_PATH} ({})", self.uinput_access);
        let _ = writeln!(text, "Virtual devices:");
        for (name, nodes) in &self.virtual_devices {
            if nodes.is_empty() {
                let _ = writeln!(text, "  {name}: not present");
            } else {
                let nodes: Vec<_> = nodes
                    .iter()
                    .map(|node| node.display().to_string())
                    .collect();
                let _ = writeln!(text, "  {name}: {}", nodes.join(", "));
            }
        }
        match &self.config.path {
            Some(path) => {
                let _ = writeln!(
                    text,
                    "Config: {} ({})",
                    path.display(),
                    self.config.validity
                );
            }
            None => {
                let _ = writeln!(text, "Config: {}", self.config.validity);
            }
        }
        text
    }
}

pub fn collect() -> Result<StatusReport, DeviceError> {
    let mice = enumerate_mice()?
        .into_iter()
        .map(|nodes| MouseStatus {
            raw_access: access_of(&nodes.hidraw),
            stock_access: access_of(&nodes.evdev),
            nodes,
        })
        .collect();
    let names = [POINTER_IDENTITY.name, TOUCHPAD_IDENTITY.name];
    let found = enumerate_input_devices_named(&names)?;
    let virtual_devices = names
        .iter()
        .map(|name| {
            (
                *name,
                found
                    .iter()
                    .filter(|device| device.name == *name)
                    .map(|device| device.devnode.clone())
                    .collect(),
            )
        })
        .collect();
    let config_path = config_location();
    let outcome = load_config(config_path.as_deref());
    Ok(StatusReport {
        service: service_state(),
        mice,
        uinput_access: access_of(Path::new(UINPUT_PATH)),
        virtual_devices,
        config: ConfigStatus::new(config_path, outcome.as_ref()),
    })
}

#[cfg(test)]
mod tests {
    use md_core::config::{Config, Diagnostic, DiagnosticKind, ParsedConfig};
    use md_core::profile::MAGIC_MOUSE_2_BT;

    use super::*;

    fn mouse(n: u32, raw_access: Access) -> MouseStatus {
        MouseStatus {
            nodes: MouseNodes {
                key: PathBuf::from(format!(
                    "/sys/devices/virtual/misc/uhid/0005:004C:0269.000{n}"
                )),
                hidraw: PathBuf::from(format!("/dev/hidraw{n}")),
                evdev: PathBuf::from(format!("/dev/input/event{}", 10 + n)),
                profile: &MAGIC_MOUSE_2_BT,
            },
            raw_access,
            stock_access: Access::ReadWrite,
        }
    }

    #[test]
    fn renders_mice_access_and_virtual_devices() {
        let report = StatusReport {
            service: Ok(ServiceState {
                load: "loaded".to_owned(),
                active: "active".to_owned(),
                sub: "running".to_owned(),
                status_text: "active: Apple Magic Mouse 2 (Bluetooth) on hidraw5".to_owned(),
            }),
            config: ConfigStatus::new(
                Some(PathBuf::from("/home/u/.config/magic-driver/config.toml")),
                Some(&LoadOutcome::MissingUsingDefaults(Config::default())),
            ),
            mice: vec![mouse(5, Access::Denied), mouse(6, Access::ReadWrite)],
            uinput_access: Access::ReadWrite,
            virtual_devices: vec![
                (
                    "magic-driver pointer",
                    vec![PathBuf::from("/dev/input/event30")],
                ),
                ("magic-driver touchpad", vec![]),
            ],
        };
        assert_eq!(
            report.render(),
            "Service: active (running), daemon reports: active: Apple Magic Mouse 2 (Bluetooth) on hidraw5
Session state: active
Magic Mouse devices:
  Apple Magic Mouse 2 (Bluetooth) [magic-mouse-2-bt], taken over by `magic-driver run`
    hid device:   /sys/devices/virtual/misc/uhid/0005:004C:0269.0005
    raw device:   /dev/hidraw5 (no access for this user)
    stock device: /dev/input/event15 (read-write)
  Apple Magic Mouse 2 (Bluetooth) [magic-mouse-2-bt], ignored in v1, only the first mouse is taken over
    hid device:   /sys/devices/virtual/misc/uhid/0005:004C:0269.0006
    raw device:   /dev/hidraw6 (read-write)
    stock device: /dev/input/event16 (read-write)
uinput: /dev/uinput (read-write)
Virtual devices:
  magic-driver pointer: /dev/input/event30
  magic-driver touchpad: not present
Config: /home/u/.config/magic-driver/config.toml (missing, defaults are used)
"
        );
    }

    #[test]
    fn renders_when_nothing_is_found() {
        let report = StatusReport {
            service: Err("cannot run systemctl".to_owned()),
            config: ConfigStatus::new(None, None),
            mice: vec![],
            uinput_access: Access::Missing,
            virtual_devices: vec![("magic-driver pointer", vec![])],
        };
        let text = report.render();
        assert!(text.starts_with(
            "Service: unknown (cannot run systemctl)\nMagic Mouse devices:\n  none found\n"
        ));
        assert!(text.contains("uinput: /dev/uinput (missing)"));
        assert!(text.ends_with("Config: unknown, neither XDG_CONFIG_HOME nor HOME is set\n"));
    }

    #[test]
    fn config_validity_is_summarised() {
        let path = Some(PathBuf::from("/c.toml"));
        let diagnostic = Diagnostic {
            line: Some(1),
            key: Some("x".to_owned()),
            kind: DiagnosticKind::UnknownKey,
        };
        let warned = LoadOutcome::Loaded(ParsedConfig {
            config: Config::default(),
            warnings: vec![diagnostic.clone()],
        });
        assert_eq!(
            ConfigStatus::new(path.clone(), Some(&warned)).validity,
            "valid, 1 warning(s)"
        );
        let invalid = LoadOutcome::Invalid {
            errors: vec![diagnostic],
            warnings: vec![],
        };
        assert_eq!(
            ConfigStatus::new(path.clone(), Some(&invalid)).validity,
            "invalid, 1 error(s), see magic-driver config test"
        );
        let loaded = LoadOutcome::Loaded(ParsedConfig {
            config: Config::default(),
            warnings: vec![],
        });
        assert_eq!(ConfigStatus::new(path, Some(&loaded)).validity, "valid");
    }

    #[test]
    fn access_checks_do_not_need_the_file_to_open() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("node");
        std::fs::write(&file, b"").unwrap();
        assert_eq!(access_of(&file), Access::ReadWrite);
        assert_eq!(access_of(&dir.path().join("absent")), Access::Missing);
    }
}
