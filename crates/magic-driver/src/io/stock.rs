use std::fs::OpenOptions;
use std::io;
use std::os::fd::{AsFd, BorrowedFd, OwnedFd};
use std::os::unix::fs::OpenOptionsExt;
use std::path::Path;

use evdev::raw_stream::RawDevice;
use nix::fcntl::OFlag;

use super::{DeviceError, EvdevEvent, StockDevice};

pub struct StockEvdev {
    device: RawDevice,
}

impl std::fmt::Debug for StockEvdev {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("StockEvdev")
            .field("grabbed", &self.device.is_grabbed())
            .finish_non_exhaustive()
    }
}

impl StockEvdev {
    pub fn open(path: &Path) -> Result<StockEvdev, DeviceError> {
        let file = OpenOptions::new()
            .read(true)
            .custom_flags(OFlag::O_NONBLOCK.bits())
            .open(path)?;
        let device = RawDevice::from_fd(OwnedFd::from(file))?;
        Ok(StockEvdev { device })
    }
}

pub fn drain_events<F>(mut fetch: F, out: &mut Vec<EvdevEvent>) -> Result<(), DeviceError>
where
    F: FnMut(&mut Vec<EvdevEvent>) -> io::Result<()>,
{
    loop {
        let before = out.len();
        match fetch(out) {
            Ok(()) => {
                if out.len() == before {
                    return Ok(());
                }
            }
            Err(error) if error.kind() == io::ErrorKind::Interrupted => {}
            Err(error) if error.kind() == io::ErrorKind::WouldBlock => return Ok(()),
            Err(error) => return Err(DeviceError::from(error)),
        }
    }
}

impl StockDevice for StockEvdev {
    fn grab(&mut self) -> Result<(), DeviceError> {
        self.device.grab().map_err(DeviceError::from)
    }

    fn ungrab(&mut self) -> Result<(), DeviceError> {
        self.device.ungrab().map_err(DeviceError::from)
    }

    fn drain(&mut self, out: &mut Vec<EvdevEvent>) -> Result<(), DeviceError> {
        let device = &mut self.device;
        drain_events(
            |out| {
                let events = device.fetch_events()?;
                out.extend(events.map(|event| {
                    EvdevEvent::new(event.event_type().0, event.code(), event.value())
                }));
                Ok(())
            },
            out,
        )
    }

    fn poll_fd(&self) -> Option<BorrowedFd<'_>> {
        Some(self.device.as_fd())
    }
}

#[cfg(test)]
mod tests {
    use std::collections::VecDeque;

    use nix::errno::Errno;

    use super::*;
    use crate::io::DeviceErrorKind;

    fn drain_script(
        steps: Vec<io::Result<Vec<EvdevEvent>>>,
    ) -> (Result<(), DeviceError>, Vec<EvdevEvent>) {
        let mut steps: VecDeque<_> = steps.into();
        let mut out = Vec::new();
        let result = drain_events(
            |out| {
                let events = steps
                    .pop_front()
                    .unwrap_or_else(|| Err(io::Error::from(Errno::EAGAIN)))?;
                out.extend(events);
                Ok(())
            },
            &mut out,
        );
        (result, out)
    }

    fn rel_x(value: i32) -> EvdevEvent {
        EvdevEvent::new(2, 0, value)
    }

    #[test]
    fn drain_reads_until_would_block() {
        let (result, out) = drain_script(vec![Ok(vec![rel_x(1), rel_x(2)]), Ok(vec![rel_x(3)])]);
        assert!(result.is_ok());
        assert_eq!(out, vec![rel_x(1), rel_x(2), rel_x(3)]);
    }

    #[test]
    fn drain_retries_interrupted_reads() {
        let (result, out) =
            drain_script(vec![Err(io::Error::from(Errno::EINTR)), Ok(vec![rel_x(1)])]);
        assert!(result.is_ok());
        assert_eq!(out, vec![rel_x(1)]);
    }

    #[test]
    fn drain_stops_on_an_empty_read() {
        let (result, out) = drain_script(vec![Ok(vec![]), Ok(vec![rel_x(1)])]);
        assert!(result.is_ok());
        assert!(out.is_empty());
    }

    #[test]
    fn drain_maps_enodev_and_eio_to_gone_and_keeps_earlier_events() {
        for errno in [Errno::ENODEV, Errno::EIO] {
            let (result, out) = drain_script(vec![Ok(vec![rel_x(5)]), Err(io::Error::from(errno))]);
            assert_eq!(result.unwrap_err().kind(), DeviceErrorKind::Gone, "{errno}");
            assert_eq!(out, vec![rel_x(5)]);
        }
    }

    fn fake_stock_device() -> (evdev::uinput::VirtualDevice, std::path::PathBuf) {
        use evdev::{AttributeSet, KeyCode, RelativeAxisCode};
        let keys: AttributeSet<KeyCode> = [KeyCode::BTN_LEFT].into_iter().collect();
        let axes: AttributeSet<RelativeAxisCode> =
            [RelativeAxisCode::REL_X, RelativeAxisCode::REL_Y]
                .into_iter()
                .collect();
        let mut device = evdev::uinput::VirtualDevice::builder()
            .unwrap()
            .name("magic-driver test stock device")
            .with_keys(&keys)
            .unwrap()
            .with_relative_axes(&axes)
            .unwrap()
            .build()
            .unwrap();
        for _ in 0..100 {
            let node = device
                .enumerate_dev_nodes_blocking()
                .unwrap()
                .find_map(Result::ok);
            if let Some(node) = node.filter(|node| node.exists()) {
                std::thread::sleep(std::time::Duration::from_millis(100));
                return (device, node);
            }
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        panic!("fake stock device node did not appear");
    }

    fn emit_motion(device: &mut evdev::uinput::VirtualDevice, value: i32) {
        device.emit(&[evdev::InputEvent::new(2, 0, value)]).unwrap();
        std::thread::sleep(std::time::Duration::from_millis(20));
    }

    fn drained(device: &mut StockEvdev) -> Vec<EvdevEvent> {
        let mut out = Vec::new();
        device.drain(&mut out).unwrap();
        out.retain(|event| event.kind == 2);
        out
    }

    #[test]
    #[ignore = "creates a uinput device and grabs it; owner or isolated VM only, needs MAGIC_DRIVER_DEVICE_TESTS=1"]
    fn device_grab_blocks_a_second_reader_and_closing_releases_it() {
        if !crate::io::device_tests_enabled() {
            eprintln!("skipped: set MAGIC_DRIVER_DEVICE_TESTS=1");
            return;
        }
        let (mut fake, node) = fake_stock_device();
        let mut grabber = StockEvdev::open(&node).unwrap();
        let mut other = StockEvdev::open(&node).unwrap();
        grabber.grab().unwrap();
        emit_motion(&mut fake, 1);
        assert_eq!(drained(&mut grabber), vec![rel_x(1)]);
        assert!(drained(&mut other).is_empty());
        drop(grabber);
        emit_motion(&mut fake, 2);
        assert_eq!(drained(&mut other), vec![rel_x(2)]);
        let mut second = StockEvdev::open(&node).unwrap();
        second.grab().unwrap();
        second.ungrab().unwrap();
        emit_motion(&mut fake, 3);
        assert_eq!(drained(&mut other), vec![rel_x(3)]);
    }

    #[test]
    fn drain_reports_other_errors() {
        let (result, _) = drain_script(vec![Err(io::Error::from(Errno::EINVAL))]);
        assert_eq!(result.unwrap_err().kind(), DeviceErrorKind::Other);
    }
}
