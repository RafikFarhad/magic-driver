use std::os::fd::{AsFd, BorrowedFd};
use std::path::PathBuf;

use udev::{Device, Enumerator, EventType, MonitorBuilder, MonitorSocket};

use super::{
    DiscoveryEvent, DiscoveryState, HID_SUBSYSTEM, HIDRAW_SUBSYSTEM, INPUT_SUBSYSTEM, MouseNode,
    MouseNodes, UdevRecord, classify, node_kind,
};
use crate::io::{DeviceDirectory, DeviceError};

pub fn record_of(device: &Device) -> UdevRecord {
    UdevRecord {
        devpath: device.devpath().to_string_lossy().into_owned(),
        subsystem: device
            .subsystem()
            .map(|subsystem| subsystem.to_string_lossy().into_owned()),
        devnode: device.devnode().map(PathBuf::from),
        properties: device
            .properties()
            .map(|entry| {
                (
                    entry.name().to_string_lossy().into_owned(),
                    entry.value().to_string_lossy().into_owned(),
                )
            })
            .collect(),
    }
}

fn mouse_node(device: &Device) -> Option<MouseNode> {
    let record = record_of(device);
    node_kind(&record)?;
    let parent = device.parent_with_subsystem(HID_SUBSYSTEM).ok()??;
    classify(&record, &record_of(&parent))
}

fn add_device(state: &mut DiscoveryState, device: &Device) -> Option<DiscoveryEvent> {
    state.node_added(mouse_node(device)?)
}

fn scan(state: &mut DiscoveryState, out: &mut Vec<DiscoveryEvent>) -> Result<(), DeviceError> {
    for subsystem in [HIDRAW_SUBSYSTEM, INPUT_SUBSYSTEM] {
        let mut enumerator = Enumerator::new()?;
        enumerator.match_subsystem(subsystem)?;
        enumerator.match_is_initialized()?;
        for device in enumerator.scan_devices()? {
            out.extend(add_device(state, &device));
        }
    }
    Ok(())
}

pub fn enumerate_mice() -> Result<Vec<MouseNodes>, DeviceError> {
    let mut state = DiscoveryState::default();
    scan(&mut state, &mut Vec::new())?;
    Ok(state.mice())
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InputDeviceNode {
    pub name: String,
    pub devnode: PathBuf,
}

pub fn enumerate_input_devices_named(names: &[&str]) -> Result<Vec<InputDeviceNode>, DeviceError> {
    let mut enumerator = Enumerator::new()?;
    enumerator.match_subsystem(INPUT_SUBSYSTEM)?;
    enumerator.match_sysname("event*")?;
    let mut found = Vec::new();
    for device in enumerator.scan_devices()? {
        let Some(devnode) = device.devnode() else {
            continue;
        };
        let Some(name) = device.parent().and_then(|parent| {
            parent
                .attribute_value("name")
                .map(|name| name.to_string_lossy().trim().to_owned())
        }) else {
            continue;
        };
        if names.contains(&name.as_str()) {
            found.push(InputDeviceNode {
                name,
                devnode: devnode.to_path_buf(),
            });
        }
    }
    Ok(found)
}

pub struct UdevDirectory {
    monitor: MonitorSocket,
    state: DiscoveryState,
}

impl std::fmt::Debug for UdevDirectory {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("UdevDirectory")
            .field("state", &self.state)
            .finish_non_exhaustive()
    }
}

impl UdevDirectory {
    pub fn new() -> Result<UdevDirectory, DeviceError> {
        let monitor = MonitorBuilder::new()?
            .match_subsystem(HIDRAW_SUBSYSTEM)?
            .match_subsystem(INPUT_SUBSYSTEM)?
            .listen()?;
        Ok(UdevDirectory {
            monitor,
            state: DiscoveryState::default(),
        })
    }
}

impl DeviceDirectory for UdevDirectory {
    fn enumerate(&mut self, out: &mut Vec<DiscoveryEvent>) -> Result<(), DeviceError> {
        scan(&mut self.state, out)
    }

    fn changes(&mut self, out: &mut Vec<DiscoveryEvent>) -> Result<(), DeviceError> {
        for event in self.monitor.iter() {
            match event.event_type() {
                EventType::Add => {
                    out.extend(add_device(&mut self.state, &event.device()));
                }
                EventType::Change => {
                    if let Some(node) = mouse_node(&event.device()) {
                        out.extend(self.state.node_changed(node));
                    }
                }
                EventType::Remove => {
                    out.extend(self.state.node_removed(event.device().syspath()));
                }
                _ => {}
            }
        }
        Ok(())
    }

    fn poll_fd(&self) -> Option<BorrowedFd<'_>> {
        Some(self.monitor.as_fd())
    }
}
