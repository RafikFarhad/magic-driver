use std::fs::{File, OpenOptions};
use std::io::{self, BufWriter, Write};
use std::os::fd::AsFd;
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

use md_core::Micros;
use md_core::profile::Profile;
use md_core::recording::{RecordingError, RecordingHeader, RecordingWriter};
use nix::errno::Errno;
use nix::poll::{PollFd, PollFlags, PollTimeout, poll};
use nix::sys::signal::{SigSet, Signal};
use nix::sys::signalfd::{SfdFlags, SignalFd};

use crate::discovery::udev_directory::enumerate_mice;
use crate::io::hidraw::{HidrawReader, REPORT_BUFFER_LEN};
use crate::io::{DeviceError, RawSource};
use crate::runtime::{Clock, MonotonicClock};

#[derive(Debug, thiserror::Error)]
pub enum RecordError {
    #[error("{0} already exists, refusing to overwrite it")]
    Exists(String),
    #[error("cannot create {path}: {source}")]
    Create { path: String, source: io::Error },
    #[error("no Magic Mouse 2 found; connect it and check `magic-driver status`")]
    NoMouse,
    #[error("cannot open the raw device {path}: {source}")]
    OpenRaw { path: String, source: DeviceError },
    #[error("raw device read failed: {0}")]
    Read(DeviceError),
    #[error("cannot write the recording: {0}")]
    Write(#[from] RecordingError),
    #[error("device discovery failed: {0}")]
    Discovery(DeviceError),
    #[error("cannot set up signal handling: {0}")]
    Signals(Errno),
}

pub fn ensure_absent(path: &Path) -> Result<(), RecordError> {
    match std::fs::symlink_metadata(path) {
        Ok(_) => Err(RecordError::Exists(path.display().to_string())),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(RecordError::Create {
            path: path.display().to_string(),
            source: error,
        }),
    }
}

pub fn create_new(path: &Path) -> Result<File, RecordError> {
    OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .map_err(|source| {
            if source.kind() == io::ErrorKind::AlreadyExists {
                RecordError::Exists(path.display().to_string())
            } else {
                RecordError::Create {
                    path: path.display().to_string(),
                    source,
                }
            }
        })
}

pub fn header_for(profile: &Profile, kernel: &str, start_unix_s: u64) -> RecordingHeader {
    let kernel = kernel.trim();
    RecordingHeader {
        profile_id: profile.id.to_owned(),
        magic_driver_version: env!("CARGO_PKG_VERSION").to_owned(),
        kernel_version: if kernel.is_empty() || kernel.contains(char::is_whitespace) {
            "unknown".to_owned()
        } else {
            kernel.replace('=', "-")
        },
        start_unix_s,
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PumpOutcome {
    Drained,
    DeviceGone,
}

pub struct Recorder<W: Write> {
    writer: RecordingWriter<W>,
    start: Micros,
    reports: u64,
    buffer: Vec<u8>,
}

impl<W: Write> Recorder<W> {
    pub fn new(
        out: W,
        header: &RecordingHeader,
        start: Micros,
    ) -> Result<Recorder<W>, RecordError> {
        let mut writer = RecordingWriter::new(out, header)?;
        writer.flush()?;
        Ok(Recorder {
            writer,
            start,
            reports: 0,
            buffer: vec![0; REPORT_BUFFER_LEN],
        })
    }

    pub fn reports(&self) -> u64 {
        self.reports
    }

    pub fn into_inner(self) -> W {
        self.writer.into_inner()
    }

    pub fn pump<S: RawSource>(
        &mut self,
        source: &mut S,
        now: Micros,
    ) -> Result<PumpOutcome, RecordError> {
        let t = now.saturating_sub(self.start);
        let outcome = loop {
            match source.read_report(&mut self.buffer) {
                Ok(Some(len)) => {
                    self.writer.write_report(t, &self.buffer[..len])?;
                    self.reports += 1;
                }
                Ok(None) => break PumpOutcome::Drained,
                Err(error) if error.is_gone() => break PumpOutcome::DeviceGone,
                Err(error) => return Err(RecordError::Read(error)),
            }
        };
        self.writer.flush()?;
        Ok(outcome)
    }
}

pub fn run(path: &Path) -> Result<u64, RecordError> {
    ensure_absent(path)?;
    let mouse = enumerate_mice()
        .map_err(RecordError::Discovery)?
        .into_iter()
        .next()
        .ok_or(RecordError::NoMouse)?;
    let mut mask = SigSet::empty();
    mask.add(Signal::SIGINT);
    mask.add(Signal::SIGTERM);
    mask.thread_block().map_err(RecordError::Signals)?;
    let signals = SignalFd::with_flags(&mask, SfdFlags::SFD_NONBLOCK | SfdFlags::SFD_CLOEXEC)
        .map_err(RecordError::Signals)?;
    let mut source = HidrawReader::open(&mouse.hidraw).map_err(|source| RecordError::OpenRaw {
        path: mouse.hidraw.display().to_string(),
        source,
    })?;
    let kernel = std::fs::read_to_string("/proc/sys/kernel/osrelease").unwrap_or_default();
    let start_unix_s = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_secs());
    let header = header_for(mouse.profile, &kernel, start_unix_s);
    let file = create_new(path)?;
    let clock = MonotonicClock::new();
    let mut recorder = Recorder::new(BufWriter::new(file), &header, clock.now())?;
    eprintln!(
        "magic-driver: recording {} (not grabbed) to {}; press Ctrl+C to stop",
        mouse.hidraw.display(),
        path.display()
    );
    while let Some(raw_fd) = source.poll_fd() {
        let mut fds = [
            PollFd::new(raw_fd, PollFlags::POLLIN),
            PollFd::new(signals.as_fd(), PollFlags::POLLIN),
        ];
        match poll(&mut fds, PollTimeout::NONE) {
            Ok(_) | Err(Errno::EINTR) => {}
            Err(error) => return Err(RecordError::Signals(error)),
        }
        let stop = fds[1].revents().is_some_and(|events| !events.is_empty());
        if recorder.pump(&mut source, clock.now())? == PumpOutcome::DeviceGone {
            eprintln!("magic-driver: the mouse disconnected, stopping");
            break;
        }
        if stop {
            break;
        }
    }
    let reports = recorder.reports();
    recorder
        .into_inner()
        .flush()
        .map_err(|error| RecordError::Write(RecordingError::Io(error)))?;
    Ok(reports)
}

#[cfg(test)]
mod tests {
    use std::collections::VecDeque;
    use std::io::Cursor;
    use std::os::fd::BorrowedFd;

    use md_core::profile::MAGIC_MOUSE_2_BT;
    use md_core::recording::RecordingReader;
    use md_core::testkit::ReportBuilder;

    use super::*;

    struct FakeSource {
        steps: VecDeque<Result<Option<Vec<u8>>, Errno>>,
    }

    impl RawSource for FakeSource {
        fn read_report(&mut self, buf: &mut [u8]) -> Result<Option<usize>, DeviceError> {
            match self.steps.pop_front() {
                None | Some(Ok(None)) => Ok(None),
                Some(Ok(Some(bytes))) => {
                    buf[..bytes.len()].copy_from_slice(&bytes);
                    Ok(Some(bytes.len()))
                }
                Some(Err(errno)) => Err(DeviceError::from(errno)),
            }
        }

        fn poll_fd(&self) -> Option<BorrowedFd<'_>> {
            None
        }
    }

    fn source(steps: Vec<Result<Option<Vec<u8>>, Errno>>) -> FakeSource {
        FakeSource {
            steps: steps.into(),
        }
    }

    fn header() -> RecordingHeader {
        header_for(&MAGIC_MOUSE_2_BT, "7.1.13-2-MANJARO\n", 1_767_225_600)
    }

    #[test]
    fn header_names_profile_version_and_kernel() {
        let header = header();
        assert_eq!(header.profile_id, "magic-mouse-2-bt");
        assert_eq!(header.kernel_version, "7.1.13-2-MANJARO");
        assert_eq!(header.magic_driver_version, env!("CARGO_PKG_VERSION"));
        assert_eq!(
            header_for(&MAGIC_MOUSE_2_BT, "", 0).kernel_version,
            "unknown"
        );
        assert_eq!(
            header_for(&MAGIC_MOUSE_2_BT, "a b", 0).kernel_version,
            "unknown"
        );
        assert_eq!(
            header_for(&MAGIC_MOUSE_2_BT, "a=b", 0).kernel_version,
            "a-b"
        );
        assert!(header.render().is_ok());
    }

    #[test]
    fn reports_are_written_with_times_since_start_and_read_back() {
        let first = ReportBuilder::new().motion(1, 2).build();
        let second = ReportBuilder::new().left(true).build();
        let mut recorder = Recorder::new(Vec::new(), &header(), Micros::from_millis(500)).unwrap();
        let mut fake = source(vec![
            Ok(Some(first.clone())),
            Ok(None),
            Ok(Some(second.clone())),
        ]);
        assert_eq!(
            recorder.pump(&mut fake, Micros::from_millis(511)).unwrap(),
            PumpOutcome::Drained
        );
        assert_eq!(
            recorder.pump(&mut fake, Micros::from_millis(522)).unwrap(),
            PumpOutcome::Drained
        );
        assert_eq!(recorder.reports(), 2);
        let bytes = recorder.into_inner();
        let reader = RecordingReader::new(Cursor::new(bytes)).unwrap();
        assert_eq!(reader.header(), &header());
        let reports: Vec<_> = reader.map(Result::unwrap).collect();
        assert_eq!(
            reports,
            vec![
                (Micros::from_millis(11), first),
                (Micros::from_millis(22), second)
            ]
        );
    }

    #[test]
    fn several_reports_in_one_wake_share_the_timestamp() {
        let report = ReportBuilder::new().build();
        let mut recorder = Recorder::new(Vec::new(), &header(), Micros::ZERO).unwrap();
        let mut fake = source(vec![Ok(Some(report.clone())), Ok(Some(report.clone()))]);
        recorder.pump(&mut fake, Micros::new(7)).unwrap();
        let text = String::from_utf8(recorder.into_inner()).unwrap();
        let times: Vec<_> = text
            .lines()
            .skip(1)
            .map(|line| line.split(' ').next().unwrap())
            .collect();
        assert_eq!(times, vec!["7", "7"]);
    }

    #[test]
    fn device_gone_ends_the_recording_and_keeps_earlier_reports() {
        let report = ReportBuilder::new().build();
        let mut recorder = Recorder::new(Vec::new(), &header(), Micros::ZERO).unwrap();
        let mut fake = source(vec![Ok(Some(report)), Err(Errno::ENODEV)]);
        assert_eq!(
            recorder.pump(&mut fake, Micros::new(1)).unwrap(),
            PumpOutcome::DeviceGone
        );
        assert_eq!(recorder.reports(), 1);
        let text = String::from_utf8(recorder.into_inner()).unwrap();
        assert_eq!(text.lines().count(), 2);
    }

    #[test]
    fn other_read_errors_are_reported() {
        let mut recorder = Recorder::new(Vec::new(), &header(), Micros::ZERO).unwrap();
        let mut fake = source(vec![Err(Errno::EINVAL)]);
        assert!(matches!(
            recorder.pump(&mut fake, Micros::new(1)),
            Err(RecordError::Read(_))
        ));
    }

    #[test]
    fn the_header_is_flushed_before_any_report() {
        let recorder = Recorder::new(Vec::new(), &header(), Micros::ZERO).unwrap();
        let text = String::from_utf8(recorder.into_inner()).unwrap();
        assert!(text.starts_with("magic-driver-recording format=1 profile=magic-mouse-2-bt"));
        assert_eq!(text.lines().count(), 1);
    }

    #[test]
    fn existing_files_are_never_overwritten() {
        let dir = tempfile::tempdir().unwrap();
        let existing = dir.path().join("taken.rec");
        std::fs::write(&existing, b"keep me").unwrap();
        assert!(matches!(
            ensure_absent(&existing),
            Err(RecordError::Exists(_))
        ));
        assert!(matches!(create_new(&existing), Err(RecordError::Exists(_))));
        assert_eq!(std::fs::read(&existing).unwrap(), b"keep me");

        let link = dir.path().join("link.rec");
        std::os::unix::fs::symlink(dir.path().join("missing-target"), &link).unwrap();
        assert!(matches!(ensure_absent(&link), Err(RecordError::Exists(_))));

        let fresh = dir.path().join("fresh.rec");
        assert!(ensure_absent(&fresh).is_ok());
        create_new(&fresh).unwrap();
        assert!(fresh.exists());
    }
}
