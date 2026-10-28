use std::ffi::{OsStr, OsString};
use std::io;
use std::os::unix::fs::PermissionsExt;
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::Command;

pub const INSPECTOR_BINARY: &str = "magic-driver-inspector";

pub const INSTALL_HINT: &str = "magic-driver-inspector is not installed.
Install it with the optional inspector step:
  scripts/install.sh --with-inspector
or build it and put it next to magic-driver or on PATH:
  cargo build --release --locked -p magic-driver-inspector
  target/release/magic-driver-inspector";

pub fn find_inspector(current_exe: Option<&Path>, path_var: Option<&OsStr>) -> Option<PathBuf> {
    let beside = current_exe
        .and_then(Path::parent)
        .map(|dir| dir.join(INSPECTOR_BINARY))
        .filter(|candidate| is_executable_file(candidate));
    beside.or_else(|| {
        std::env::split_paths(path_var?)
            .filter(|dir| dir.is_absolute())
            .map(|dir| dir.join(INSPECTOR_BINARY))
            .find(|candidate| is_executable_file(candidate))
    })
}

fn is_executable_file(path: &Path) -> bool {
    std::fs::metadata(path)
        .is_ok_and(|metadata| metadata.is_file() && metadata.permissions().mode() & 0o111 != 0)
}

pub fn inspector_args(replay: Option<&Path>) -> Vec<OsString> {
    replay.map_or_else(Vec::new, |file| {
        vec![OsString::from("--replay"), file.as_os_str().to_owned()]
    })
}

pub fn exec_inspector(inspector: &Path, args: &[OsString]) -> io::Error {
    Command::new(inspector).args(args).exec()
}

#[cfg(test)]
mod tests {
    use std::fs;

    use super::*;

    fn write_file(path: &Path, mode: u32) {
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, "#!/bin/sh\n").unwrap();
        fs::set_permissions(path, fs::Permissions::from_mode(mode)).unwrap();
    }

    #[test]
    fn the_inspector_next_to_the_daemon_wins_over_path() {
        let dir = tempfile::tempdir().unwrap();
        let beside = dir.path().join("bin").join(INSPECTOR_BINARY);
        let on_path = dir.path().join("path").join(INSPECTOR_BINARY);
        write_file(&beside, 0o755);
        write_file(&on_path, 0o755);
        let exe = dir.path().join("bin").join("magic-driver");
        let path_var = std::env::join_paths([dir.path().join("path")]).unwrap();
        assert_eq!(find_inspector(Some(&exe), Some(&path_var)), Some(beside));
    }

    #[test]
    fn path_is_searched_in_order_when_nothing_is_next_to_the_daemon() {
        let dir = tempfile::tempdir().unwrap();
        let first = dir.path().join("first");
        let second = dir.path().join("second");
        let third = dir.path().join("third");
        fs::create_dir_all(&first).unwrap();
        write_file(&second.join(INSPECTOR_BINARY), 0o755);
        write_file(&third.join(INSPECTOR_BINARY), 0o755);
        let exe = dir.path().join("bin").join("magic-driver");
        let path_var = std::env::join_paths([&first, &second, &third]).unwrap();
        assert_eq!(
            find_inspector(Some(&exe), Some(&path_var)),
            Some(second.join(INSPECTOR_BINARY))
        );
        assert_eq!(
            find_inspector(None, Some(&path_var)),
            Some(second.join(INSPECTOR_BINARY))
        );
    }

    #[test]
    fn non_executable_files_directories_and_relative_path_entries_are_skipped() {
        let dir = tempfile::tempdir().unwrap();
        let plain = dir.path().join("plain");
        let folder = dir.path().join("folder");
        write_file(&plain.join(INSPECTOR_BINARY), 0o644);
        fs::create_dir_all(folder.join(INSPECTOR_BINARY)).unwrap();
        let exe = plain.join("magic-driver");
        let path_var = std::env::join_paths([
            plain.clone(),
            folder,
            PathBuf::from("relative"),
            PathBuf::new(),
        ])
        .unwrap();
        assert_eq!(find_inspector(Some(&exe), Some(&path_var)), None);
        assert_eq!(find_inspector(Some(&exe), None), None);
        assert_eq!(find_inspector(None, None), None);
        assert_eq!(find_inspector(Some(Path::new("/")), None), None);
    }

    #[test]
    fn replay_is_passed_on_and_live_mode_takes_no_arguments() {
        assert!(inspector_args(None).is_empty());
        assert_eq!(
            inspector_args(Some(Path::new("fixtures/flick.rec"))),
            [
                OsString::from("--replay"),
                OsString::from("fixtures/flick.rec")
            ]
        );
    }

    #[test]
    fn the_install_hint_names_the_install_flag() {
        assert!(INSTALL_HINT.contains("scripts/install.sh --with-inspector"));
        assert!(INSTALL_HINT.contains("-p magic-driver-inspector"));
    }

    #[test]
    fn a_missing_inspector_cannot_be_executed() {
        let dir = tempfile::tempdir().unwrap();
        let error = exec_inspector(&dir.path().join(INSPECTOR_BINARY), &[]);
        assert_eq!(error.kind(), io::ErrorKind::NotFound);
    }
}
