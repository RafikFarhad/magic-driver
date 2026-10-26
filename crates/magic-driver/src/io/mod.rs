pub mod hidraw;
pub mod stock;
pub mod uinput;

#[cfg(test)]
pub mod fake;

use std::fmt;
use std::io;
use std::os::fd::BorrowedFd;
use std::path::Path;

use md_core::Button;
use md_core::synth::Frame;
use nix::errno::Errno;

use crate::discovery::DiscoveryEvent;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeviceErrorKind {
    Gone,
    PermissionDenied,
    NotFound,
    Busy,
    Other,
}

impl DeviceErrorKind {
    pub fn of(error: &io::Error) -> DeviceErrorKind {
        let Some(code) = error.raw_os_error() else {
            return match error.kind() {
                io::ErrorKind::PermissionDenied => DeviceErrorKind::PermissionDenied,
                io::ErrorKind::NotFound => DeviceErrorKind::NotFound,
                _ => DeviceErrorKind::Other,
            };
        };
        match Errno::from_raw(code) {
            Errno::ENODEV | Errno::EIO | Errno::ENXIO => DeviceErrorKind::Gone,
            Errno::EACCES | Errno::EPERM => DeviceErrorKind::PermissionDenied,
            Errno::ENOENT => DeviceErrorKind::NotFound,
            Errno::EBUSY => DeviceErrorKind::Busy,
            _ => DeviceErrorKind::Other,
        }
    }
}

impl fmt::Display for DeviceErrorKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            DeviceErrorKind::Gone => "device gone",
            DeviceErrorKind::PermissionDenied => "permission denied",
            DeviceErrorKind::NotFound => "not found",
            DeviceErrorKind::Busy => "busy",
            DeviceErrorKind::Other => "I/O error",
        })
    }
}

#[derive(Debug, thiserror::Error)]
#[error("{kind} ({source})")]
pub struct DeviceError {
    kind: DeviceErrorKind,
    source: io::Error,
}

impl DeviceError {
    pub fn kind(&self) -> DeviceErrorKind {
        self.kind
    }

    pub fn is_gone(&self) -> bool {
        self.kind() == DeviceErrorKind::Gone
    }

    pub fn from_errno(errno: Errno) -> DeviceError {
        DeviceError::from(io::Error::from(errno))
    }
}

impl From<io::Error> for DeviceError {
    fn from(source: io::Error) -> DeviceError {
        DeviceError {
            kind: DeviceErrorKind::of(&source),
            source,
        }
    }
}

impl From<Errno> for DeviceError {
    fn from(errno: Errno) -> DeviceError {
        DeviceError::from_errno(errno)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct EvdevEvent {
    pub kind: u16,
    pub code: u16,
    pub value: i32,
}

impl EvdevEvent {
    pub const fn new(kind: u16, code: u16, value: i32) -> EvdevEvent {
        EvdevEvent { kind, code, value }
    }
}

pub trait RawSource {
    fn read_report(&mut self, buf: &mut [u8]) -> Result<Option<usize>, DeviceError>;

    fn poll_fd(&self) -> Option<BorrowedFd<'_>>;
}

pub trait StockDevice {
    fn grab(&mut self) -> Result<(), DeviceError>;

    fn ungrab(&mut self) -> Result<(), DeviceError>;

    fn drain(&mut self, out: &mut Vec<EvdevEvent>) -> Result<(), DeviceError>;

    fn poll_fd(&self) -> Option<BorrowedFd<'_>>;
}

pub trait VirtualPointer {
    fn motion(&mut self, dx: i32, dy: i32) -> Result<(), DeviceError>;

    fn button(&mut self, button: Button, pressed: bool) -> Result<(), DeviceError>;

    fn destroy(self)
    where
        Self: Sized,
    {
        drop(self);
    }
}

pub trait VirtualTouchpad {
    fn frame(&mut self, frame: &Frame) -> Result<(), DeviceError>;

    fn destroy(self)
    where
        Self: Sized,
    {
        drop(self);
    }
}

pub trait DeviceDirectory {
    fn enumerate(&mut self, out: &mut Vec<DiscoveryEvent>) -> Result<(), DeviceError>;

    fn changes(&mut self, out: &mut Vec<DiscoveryEvent>) -> Result<(), DeviceError>;

    fn poll_fd(&self) -> Option<BorrowedFd<'_>>;
}

pub trait DeviceOpener {
    type Raw: RawSource;
    type Stock: StockDevice;
    type Pointer: VirtualPointer;
    type Touchpad: VirtualTouchpad;

    fn open_raw(&mut self, path: &Path) -> Result<Self::Raw, DeviceError>;

    fn open_stock(&mut self, path: &Path) -> Result<Self::Stock, DeviceError>;

    fn create_pointer(&mut self) -> Result<Self::Pointer, DeviceError>;

    fn create_touchpad(&mut self) -> Result<Self::Touchpad, DeviceError>;
}

#[derive(Debug, Default)]
pub struct LinuxDevices;

impl DeviceOpener for LinuxDevices {
    type Raw = hidraw::HidrawReader;
    type Stock = stock::StockEvdev;
    type Pointer = uinput::UinputPointer;
    type Touchpad = uinput::UinputTouchpad;

    fn open_raw(&mut self, path: &Path) -> Result<Self::Raw, DeviceError> {
        hidraw::HidrawReader::open(path)
    }

    fn open_stock(&mut self, path: &Path) -> Result<Self::Stock, DeviceError> {
        stock::StockEvdev::open(path)
    }

    fn create_pointer(&mut self) -> Result<Self::Pointer, DeviceError> {
        uinput::UinputPointer::create()
    }

    fn create_touchpad(&mut self) -> Result<Self::Touchpad, DeviceError> {
        uinput::UinputTouchpad::create()
    }
}

#[cfg(test)]
pub fn device_tests_enabled() -> bool {
    std::env::var_os("MAGIC_DRIVER_DEVICE_TESTS").is_some_and(|value| value == "1")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn errno_maps_to_error_kinds() {
        let cases = [
            (Errno::ENODEV, DeviceErrorKind::Gone),
            (Errno::EIO, DeviceErrorKind::Gone),
            (Errno::ENXIO, DeviceErrorKind::Gone),
            (Errno::EACCES, DeviceErrorKind::PermissionDenied),
            (Errno::EPERM, DeviceErrorKind::PermissionDenied),
            (Errno::ENOENT, DeviceErrorKind::NotFound),
            (Errno::EBUSY, DeviceErrorKind::Busy),
            (Errno::EINVAL, DeviceErrorKind::Other),
        ];
        for (errno, kind) in cases {
            let error = DeviceError::from(errno);
            assert_eq!(error.kind(), kind, "{errno}");
            assert_eq!(error.is_gone(), kind == DeviceErrorKind::Gone);
        }
    }

    #[test]
    fn io_errors_without_errno_map_by_kind() {
        let denied = io::Error::new(io::ErrorKind::PermissionDenied, "denied");
        assert_eq!(
            DeviceError::from(denied).kind(),
            DeviceErrorKind::PermissionDenied
        );
        let missing = io::Error::new(io::ErrorKind::NotFound, "missing");
        assert_eq!(DeviceError::from(missing).kind(), DeviceErrorKind::NotFound);
        let other = io::Error::other("other");
        assert_eq!(DeviceError::from(other).kind(), DeviceErrorKind::Other);
    }

    #[test]
    fn errors_display_kind_and_cause() {
        let error = DeviceError::from(Errno::ENODEV);
        assert!(error.to_string().starts_with("device gone ("));
    }
}
