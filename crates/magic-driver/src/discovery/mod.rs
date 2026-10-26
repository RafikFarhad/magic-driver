pub mod udev_directory;

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use md_core::profile::Profile;

pub const HIDRAW_SUBSYSTEM: &str = "hidraw";
pub const INPUT_SUBSYSTEM: &str = "input";
pub const HID_SUBSYSTEM: &str = "hid";
pub const HID_ID_PROPERTY: &str = "HID_ID";
const SYSFS_ROOT: &str = "/sys";

#[derive(Debug, Clone, PartialEq)]
pub struct MouseNodes {
    pub key: PathBuf,
    pub hidraw: PathBuf,
    pub evdev: PathBuf,
    pub profile: &'static Profile,
}

#[derive(Debug, Clone, PartialEq)]
pub enum DiscoveryEvent {
    MouseAppeared(MouseNodes),
    MouseGone { key: PathBuf },
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct UdevRecord {
    pub devpath: String,
    pub subsystem: Option<String>,
    pub devnode: Option<PathBuf>,
    pub properties: BTreeMap<String, String>,
}

impl UdevRecord {
    pub fn syspath(&self) -> PathBuf {
        PathBuf::from(format!("{SYSFS_ROOT}{}", self.devpath))
    }

    pub fn sysname(&self) -> &str {
        self.devpath.rsplit('/').next().unwrap_or_default()
    }

    pub fn property(&self, name: &str) -> Option<&str> {
        self.properties.get(name).map(String::as_str)
    }

    fn subsystem_is(&self, subsystem: &str) -> bool {
        self.subsystem.as_deref() == Some(subsystem)
    }
}

#[cfg(test)]
pub fn parse_udevadm_info(text: &str) -> Vec<UdevRecord> {
    let mut records = Vec::new();
    let mut current: Option<UdevRecord> = None;
    for line in text.lines() {
        let line = line.trim_end();
        if line.is_empty() {
            records.extend(current.take());
            continue;
        }
        let Some((tag, value)) = line.split_once(": ") else {
            continue;
        };
        if tag == "P" {
            records.extend(current.take());
            current = Some(UdevRecord {
                devpath: value.to_owned(),
                ..UdevRecord::default()
            });
            continue;
        }
        let Some(record) = current.as_mut() else {
            continue;
        };
        match tag {
            "U" => record.subsystem = Some(value.to_owned()),
            "N" => record.devnode = Some(Path::new("/dev").join(value)),
            "E" => {
                if let Some((name, value)) = value.split_once('=') {
                    record.properties.insert(name.to_owned(), value.to_owned());
                }
            }
            _ => {}
        }
    }
    records.extend(current);
    for record in &mut records {
        if record.subsystem.is_none() {
            record.subsystem = record.property("SUBSYSTEM").map(str::to_owned);
        }
        if record.devnode.is_none() {
            record.devnode = record.property("DEVNAME").map(PathBuf::from);
        }
    }
    records
}

pub fn parse_hid_id(value: &str) -> Option<(u16, u16, u16)> {
    let mut parts = value.split(':');
    let mut field = || {
        let part = parts.next()?;
        let number = u32::from_str_radix(part, 16).ok()?;
        u16::try_from(number).ok()
    };
    let id = (field()?, field()?, field()?);
    parts.next().is_none().then_some(id)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NodeKind {
    Hidraw,
    Evdev,
}

#[derive(Debug, Clone, PartialEq)]
pub struct MouseNode {
    pub key: PathBuf,
    pub kind: NodeKind,
    pub syspath: PathBuf,
    pub devnode: PathBuf,
    pub profile: &'static Profile,
}

pub fn node_kind(node: &UdevRecord) -> Option<NodeKind> {
    if node.subsystem_is(HIDRAW_SUBSYSTEM) {
        Some(NodeKind::Hidraw)
    } else if node.subsystem_is(INPUT_SUBSYSTEM) && node.sysname().starts_with("event") {
        Some(NodeKind::Evdev)
    } else {
        None
    }
}

pub fn hid_profile(hid_parent: &UdevRecord) -> Option<&'static Profile> {
    if !hid_parent.subsystem_is(HID_SUBSYSTEM) {
        return None;
    }
    let (bus, vendor, product) = parse_hid_id(hid_parent.property(HID_ID_PROPERTY)?)?;
    Profile::lookup(bus, vendor, product)
}

pub fn classify(node: &UdevRecord, hid_parent: &UdevRecord) -> Option<MouseNode> {
    let kind = node_kind(node)?;
    let profile = hid_profile(hid_parent)?;
    let parent_prefix = format!("{}/", hid_parent.devpath);
    if !node.devpath.starts_with(&parent_prefix) {
        return None;
    }
    Some(MouseNode {
        key: hid_parent.syspath(),
        kind,
        syspath: node.syspath(),
        devnode: node.devnode.clone()?,
        profile,
    })
}

#[cfg(test)]
pub fn hid_parent_in<'a>(node: &UdevRecord, records: &'a [UdevRecord]) -> Option<&'a UdevRecord> {
    records
        .iter()
        .filter(|candidate| {
            candidate.subsystem_is(HID_SUBSYSTEM)
                && node.devpath.starts_with(&format!("{}/", candidate.devpath))
        })
        .max_by_key(|candidate| candidate.devpath.len())
}

#[derive(Debug, Clone, PartialEq)]
struct PartialMouse {
    profile: &'static Profile,
    hidraw: Option<(PathBuf, PathBuf)>,
    evdev: Option<(PathBuf, PathBuf)>,
    announced: Option<MouseNodes>,
}

impl PartialMouse {
    fn complete(&self, key: &Path) -> Option<MouseNodes> {
        Some(MouseNodes {
            key: key.to_path_buf(),
            hidraw: self.hidraw.as_ref()?.1.clone(),
            evdev: self.evdev.as_ref()?.1.clone(),
            profile: self.profile,
        })
    }
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct DiscoveryState {
    mice: BTreeMap<PathBuf, PartialMouse>,
}

impl DiscoveryState {
    pub fn node_added(&mut self, node: MouseNode) -> Option<DiscoveryEvent> {
        let mouse = self
            .mice
            .entry(node.key.clone())
            .or_insert_with(|| PartialMouse {
                profile: node.profile,
                hidraw: None,
                evdev: None,
                announced: None,
            });
        let entry = Some((node.syspath, node.devnode));
        match node.kind {
            NodeKind::Hidraw => mouse.hidraw = entry,
            NodeKind::Evdev => mouse.evdev = entry,
        }
        let complete = mouse.complete(&node.key)?;
        if mouse.announced.as_ref() == Some(&complete) {
            return None;
        }
        mouse.announced = Some(complete.clone());
        Some(DiscoveryEvent::MouseAppeared(complete))
    }

    pub fn node_changed(&mut self, node: MouseNode) -> Option<DiscoveryEvent> {
        let key = node.key.clone();
        self.node_added(node).or_else(|| {
            self.mice
                .get(&key)
                .and_then(|mouse| mouse.complete(&key))
                .map(DiscoveryEvent::MouseAppeared)
        })
    }

    pub fn node_removed(&mut self, syspath: &Path) -> Option<DiscoveryEvent> {
        let key = self.mice.iter_mut().find_map(|(key, mouse)| {
            let mut matched = false;
            for slot in [&mut mouse.hidraw, &mut mouse.evdev] {
                if slot.as_ref().is_some_and(|(path, _)| path == syspath) {
                    *slot = None;
                    matched = true;
                }
            }
            matched.then(|| key.clone())
        })?;
        let mouse = self.mice.get_mut(&key)?;
        let was_announced = mouse.announced.take().is_some();
        if mouse.hidraw.is_none() && mouse.evdev.is_none() {
            self.mice.remove(&key);
        }
        was_announced.then_some(DiscoveryEvent::MouseGone { key })
    }

    pub fn mice(&self) -> Vec<MouseNodes> {
        self.mice
            .iter()
            .filter_map(|(key, mouse)| mouse.complete(key))
            .collect()
    }
}

#[cfg(test)]
mod tests;
