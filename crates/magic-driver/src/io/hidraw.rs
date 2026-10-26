use std::fs::{File, OpenOptions};
use std::io::{self, Read};
use std::os::fd::{AsFd, BorrowedFd};
use std::os::unix::fs::OpenOptionsExt;
use std::path::Path;

use nix::fcntl::OFlag;

use super::{DeviceError, RawSource};

pub const REPORT_BUFFER_LEN: usize = 512;

#[derive(Debug)]
pub struct HidrawReader {
    file: File,
}

impl HidrawReader {
    pub fn open(path: &Path) -> Result<HidrawReader, DeviceError> {
        let file = OpenOptions::new()
            .read(true)
            .custom_flags(OFlag::O_NONBLOCK.bits())
            .open(path)?;
        Ok(HidrawReader { file })
    }
}

pub fn read_whole_report<R: Read>(
    reader: &mut R,
    buf: &mut [u8],
) -> Result<Option<usize>, DeviceError> {
    loop {
        match reader.read(buf) {
            Ok(0) => return Ok(None),
            Ok(len) => return Ok(Some(len)),
            Err(error) if error.kind() == io::ErrorKind::Interrupted => {}
            Err(error) if error.kind() == io::ErrorKind::WouldBlock => return Ok(None),
            Err(error) => return Err(DeviceError::from(error)),
        }
    }
}

impl RawSource for HidrawReader {
    fn read_report(&mut self, buf: &mut [u8]) -> Result<Option<usize>, DeviceError> {
        read_whole_report(&mut self.file, buf)
    }

    fn poll_fd(&self) -> Option<BorrowedFd<'_>> {
        Some(self.file.as_fd())
    }
}

#[cfg(test)]
mod tests {
    use std::collections::VecDeque;

    use nix::errno::Errno;

    use super::*;
    use crate::io::DeviceErrorKind;

    struct ScriptedReader {
        steps: VecDeque<io::Result<Vec<u8>>>,
    }

    impl Read for ScriptedReader {
        fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
            match self.steps.pop_front() {
                Some(Ok(bytes)) => {
                    buf[..bytes.len()].copy_from_slice(&bytes);
                    Ok(bytes.len())
                }
                Some(Err(error)) => Err(error),
                None => Err(io::Error::from(Errno::EAGAIN)),
            }
        }
    }

    fn reader(steps: Vec<io::Result<Vec<u8>>>) -> ScriptedReader {
        ScriptedReader {
            steps: steps.into(),
        }
    }

    #[test]
    fn whole_reports_are_returned_one_per_read() {
        let mut source = reader(vec![Ok(vec![0x12; 14]), Ok(vec![0x12; 22])]);
        let mut buf = [0_u8; REPORT_BUFFER_LEN];
        assert_eq!(read_whole_report(&mut source, &mut buf).unwrap(), Some(14));
        assert_eq!(read_whole_report(&mut source, &mut buf).unwrap(), Some(22));
        assert_eq!(read_whole_report(&mut source, &mut buf).unwrap(), None);
    }

    #[test]
    fn would_block_means_no_report() {
        let mut source = reader(vec![Err(io::Error::from(io::ErrorKind::WouldBlock))]);
        let mut buf = [0_u8; REPORT_BUFFER_LEN];
        assert_eq!(read_whole_report(&mut source, &mut buf).unwrap(), None);
    }

    #[test]
    fn interrupted_reads_are_retried() {
        let mut source = reader(vec![Err(io::Error::from(Errno::EINTR)), Ok(vec![0x12; 14])]);
        let mut buf = [0_u8; REPORT_BUFFER_LEN];
        assert_eq!(read_whole_report(&mut source, &mut buf).unwrap(), Some(14));
    }

    #[test]
    fn enodev_and_eio_mean_device_gone() {
        for errno in [Errno::ENODEV, Errno::EIO] {
            let mut source = reader(vec![Err(io::Error::from(errno))]);
            let mut buf = [0_u8; REPORT_BUFFER_LEN];
            let error = read_whole_report(&mut source, &mut buf).unwrap_err();
            assert_eq!(error.kind(), DeviceErrorKind::Gone, "{errno}");
        }
    }

    #[test]
    fn other_errors_are_not_gone() {
        let mut source = reader(vec![Err(io::Error::from(Errno::EINVAL))]);
        let mut buf = [0_u8; REPORT_BUFFER_LEN];
        let error = read_whole_report(&mut source, &mut buf).unwrap_err();
        assert_eq!(error.kind(), DeviceErrorKind::Other);
    }

    #[test]
    fn the_buffer_holds_the_largest_report() {
        let layout = md_core::profile::MAGIC_MOUSE_2_BT.report;
        assert!(layout.report_len(layout.max_touches) <= REPORT_BUFFER_LEN);
    }
}
