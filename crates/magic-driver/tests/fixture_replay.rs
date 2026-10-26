use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

const BINARY: &str = env!("CARGO_BIN_EXE_magic-driver");

fn fixtures_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures")
}

fn recordings(dir: &Path, found: &mut Vec<PathBuf>) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries.map(Result::unwrap) {
        let path = entry.path();
        if path.is_dir() {
            recordings(&path, found);
        } else if path.extension().is_some_and(|extension| extension == "rec") {
            found.push(path);
        }
    }
}

#[test]
fn every_committed_fixture_replays_offline_without_errors() {
    let mut found = Vec::new();
    recordings(&fixtures_dir(), &mut found);
    found.sort();
    for path in &found {
        let output = Command::new(BINARY)
            .env_clear()
            .arg("replay")
            .arg(path)
            .output()
            .expect("binary runs");
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert_eq!(
            output.status.code(),
            Some(0),
            "{}: {stderr}",
            path.display()
        );
        assert!(!stderr.contains("error:"), "{}: {stderr}", path.display());
        assert!(
            stderr.contains(" 0 malformed"),
            "{}: {stderr}",
            path.display()
        );
    }
    eprintln!("replayed {} fixtures", found.len());
}
