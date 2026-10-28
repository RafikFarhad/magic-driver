use std::collections::BTreeSet;
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::Command;

use tempfile::TempDir;

const GUARDED_COMMANDS: [&str; 7] = [
    "sudo",
    "systemctl",
    "udevadm",
    "modprobe",
    "rmmod",
    "install",
    "cargo",
];

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .nth(2)
        .expect("repository root")
        .to_path_buf()
}

struct Sandbox {
    dir: TempDir,
}

impl Sandbox {
    fn new() -> Sandbox {
        let sandbox = Sandbox {
            dir: tempfile::tempdir().expect("tempdir"),
        };
        let stubs = sandbox.stubs();
        fs::create_dir_all(&stubs).expect("stub dir");
        fs::create_dir_all(sandbox.home()).expect("home");
        for command in GUARDED_COMMANDS {
            let path = stubs.join(command);
            fs::write(
                &path,
                format!(
                    "#!/bin/sh\necho \"{command} $*\" >> '{}'\nexit 97\n",
                    sandbox.calls_log().display()
                ),
            )
            .expect("stub");
            fs::set_permissions(&path, fs::Permissions::from_mode(0o755)).expect("chmod");
        }
        sandbox
    }

    fn stubs(&self) -> PathBuf {
        self.dir.path().join("stubs")
    }

    fn home(&self) -> PathBuf {
        self.dir.path().join("home")
    }

    fn destdir(&self) -> PathBuf {
        self.dir.path().join("destdir")
    }

    fn calls_log(&self) -> PathBuf {
        self.dir.path().join("guarded-calls.log")
    }

    fn guarded_calls(&self) -> String {
        fs::read_to_string(self.calls_log()).unwrap_or_default()
    }

    fn run(&self, script: &str, args: &[&str], destdir: bool) -> (i32, String, String) {
        let path = format!(
            "{}:{}",
            self.stubs().display(),
            std::env::var("PATH").unwrap_or_default()
        );
        let mut command = Command::new("bash");
        command
            .arg(repo_root().join("scripts").join(script))
            .args(args)
            .env_clear()
            .env("PATH", path)
            .env("HOME", self.home());
        if destdir {
            command.env("DESTDIR", self.destdir());
        }
        let output = command.output().expect("bash runs");
        (
            output.status.code().unwrap_or(-1),
            String::from_utf8(output.stdout).expect("utf8"),
            String::from_utf8(output.stderr).expect("utf8"),
        )
    }

    fn write_manifest(&self, entries: &[String]) {
        let path = self
            .destdir()
            .join(self.home().strip_prefix("/").unwrap())
            .join(".local/share/magic-driver/install-manifest");
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(&path, entries.join("\n") + "\n").unwrap();
    }
}

fn prefixed(stdout: &str, prefix: &str) -> Vec<String> {
    stdout
        .lines()
        .filter_map(|line| line.strip_prefix(prefix))
        .map(str::to_owned)
        .collect()
}

fn file_tree(root: &Path) -> Vec<PathBuf> {
    let mut found = Vec::new();
    if let Ok(entries) = fs::read_dir(root) {
        for entry in entries.flatten() {
            let path = entry.path();
            found.push(path.clone());
            found.extend(file_tree(&path));
        }
    }
    found
}

#[test]
fn install_dry_run_lists_every_file_and_changes_nothing() {
    let sandbox = Sandbox::new();
    let (code, stdout, stderr) = sandbox.run("install.sh", &["--dry-run"], true);
    assert_eq!(code, 0, "{stdout}{stderr}");
    let home = sandbox.home();
    let installed: BTreeSet<_> = prefixed(&stdout, "install: ").into_iter().collect();
    let expected: BTreeSet<_> = [
        "/usr/local/bin/magic-driver".to_owned(),
        "/etc/udev/rules.d/70-magic-driver.rules".to_owned(),
        "/etc/modules-load.d/magic-driver.conf".to_owned(),
        format!(
            "{}/.config/systemd/user/magic-driver.service",
            home.display()
        ),
    ]
    .into_iter()
    .collect();
    assert_eq!(installed, expected);
    let manifest: BTreeSet<_> = prefixed(&stdout, "manifest entry: ")
        .into_iter()
        .map(|entry| entry.split_once(' ').unwrap().1.to_owned())
        .collect();
    assert_eq!(manifest, expected);
    assert_eq!(
        prefixed(&stdout, "config: "),
        [format!(
            "{}/.config/magic-driver/config.toml",
            home.display()
        )]
    );
    assert!(stdout.contains("dry-run: cargo build --release --locked"));
    assert!(
        stdout
            .contains("skip (DESTDIR is set): systemctl --user enable --now magic-driver.service")
    );
    assert!(stdout.contains("skip (DESTDIR is set): sudo udevadm control --reload"));
    assert!(stdout.ends_with("dry run finished, nothing was changed\n"));
    assert_eq!(sandbox.guarded_calls(), "");
    assert!(!sandbox.destdir().exists());
}

#[test]
fn uninstall_dry_run_removes_exactly_what_install_lists() {
    let sandbox = Sandbox::new();
    let (code, install_out, _) = sandbox.run("install.sh", &["--dry-run"], true);
    assert_eq!(code, 0);
    let entries = prefixed(&install_out, "manifest entry: ");
    sandbox.write_manifest(&entries);
    let before = file_tree(&sandbox.destdir());

    let (code, stdout, stderr) = sandbox.run("uninstall.sh", &["--dry-run"], true);
    assert_eq!(code, 0, "{stdout}{stderr}");
    let installed: BTreeSet<_> = prefixed(&install_out, "install: ").into_iter().collect();
    let manifest_path = format!(
        "{}/.local/share/magic-driver/install-manifest",
        sandbox.home().display()
    );
    let mut removed: BTreeSet<_> = prefixed(&stdout, "remove: ").into_iter().collect();
    assert!(removed.remove(&manifest_path));
    assert_eq!(removed, installed);
    assert!(stdout.contains("keep: "));
    assert_eq!(sandbox.guarded_calls(), "");
    assert_eq!(file_tree(&sandbox.destdir()), before);

    let (code, stdout, _) = sandbox.run("uninstall.sh", &["--dry-run", "--purge"], true);
    assert_eq!(code, 0);
    let removed: BTreeSet<_> = prefixed(&stdout, "remove: ").into_iter().collect();
    let config = format!(
        "{}/.config/magic-driver/config.toml",
        sandbox.home().display()
    );
    assert!(removed.contains(&config));
    assert_eq!(removed.len(), installed.len() + 2);
    assert_eq!(file_tree(&sandbox.destdir()), before);
}

#[test]
fn dry_runs_without_destdir_print_system_commands_but_never_run_them() {
    let sandbox = Sandbox::new();
    let (code, stdout, stderr) = sandbox.run(
        "install.sh",
        &["--dry-run", "--binary", "/nonexistent"],
        false,
    );
    assert_eq!(code, 0, "{stdout}{stderr}");
    for command in [
        "dry-run: sudo install -D -m 0755 -o root -g root /nonexistent /usr/local/bin/magic-driver",
        "dry-run: sudo udevadm control --reload",
        "dry-run: systemctl --user daemon-reload",
        "dry-run: systemctl --user enable --now magic-driver.service",
    ] {
        assert!(stdout.contains(command), "{command} missing in\n{stdout}");
    }
    assert!(!stdout.contains("cargo build"));
    let entries = prefixed(&stdout, "manifest entry: ");
    let manifest = sandbox
        .home()
        .join(".local/share/magic-driver/install-manifest");
    fs::create_dir_all(manifest.parent().unwrap()).unwrap();
    fs::write(&manifest, entries.join("\n")).unwrap();

    let (code, stdout, stderr) = sandbox.run("uninstall.sh", &["--dry-run"], false);
    assert_eq!(code, 0, "{stdout}{stderr}");
    for command in [
        "dry-run: systemctl --user disable --now magic-driver.service",
        "dry-run: sudo rm -f -- /usr/local/bin/magic-driver",
        "dry-run: sudo rm -f -- /etc/udev/rules.d/70-magic-driver.rules",
        "dry-run: sudo udevadm control --reload",
    ] {
        assert!(stdout.contains(command), "{command} missing in\n{stdout}");
    }
    assert_eq!(sandbox.guarded_calls(), "");
    assert!(manifest.exists());
    assert!(!sandbox.home().join(".config").exists());
}

#[test]
fn uninstall_refuses_unexpected_manifest_entries() {
    let sandbox = Sandbox::new();
    sandbox.write_manifest(&["system /etc/passwd".to_owned()]);
    let (code, stdout, stderr) = sandbox.run("uninstall.sh", &["--dry-run"], true);
    assert_eq!(code, 1, "{stdout}");
    assert!(stderr.contains("refusing to act on unexpected manifest entry: system /etc/passwd"));
    assert!(!stdout.contains("remove:"));
    assert_eq!(sandbox.guarded_calls(), "");
}

#[test]
fn uninstall_without_a_manifest_does_nothing() {
    let sandbox = Sandbox::new();
    let (code, stdout, stderr) = sandbox.run("uninstall.sh", &["--dry-run"], true);
    assert_eq!(code, 1);
    assert!(stderr.contains("no install manifest"));
    assert_eq!(stdout, "");
}

#[test]
fn unknown_arguments_are_rejected_before_any_action() {
    let sandbox = Sandbox::new();
    for script in ["install.sh", "uninstall.sh"] {
        let (code, stdout, stderr) = sandbox.run(script, &["--now"], true);
        assert_eq!(code, 1);
        assert_eq!(stdout, "");
        assert!(stderr.contains("unknown argument: --now"));
        let (code, stdout, _) = sandbox.run(script, &["--help"], true);
        assert_eq!(code, 0);
        assert!(stdout.contains("--dry-run"));
    }
    assert_eq!(sandbox.guarded_calls(), "");
}

#[test]
fn install_dry_run_includes_the_inspector_only_with_the_flag() {
    let sandbox = Sandbox::new();
    let inspector = "/usr/local/bin/magic-driver-inspector".to_owned();

    let (code, plain, stderr) = sandbox.run("install.sh", &["--dry-run"], true);
    assert_eq!(code, 0, "{plain}{stderr}");
    assert!(!prefixed(&plain, "install: ").contains(&inspector));
    assert!(!plain.contains("magic-driver-inspector"));
    assert!(plain.contains("dry-run: cargo build --release --locked --manifest-path"));
    assert!(plain.contains(" -p magic-driver\n"));

    let (code, stdout, stderr) =
        sandbox.run("install.sh", &["--dry-run", "--with-inspector"], true);
    assert_eq!(code, 0, "{stdout}{stderr}");
    let installed: BTreeSet<_> = prefixed(&stdout, "install: ").into_iter().collect();
    let without: BTreeSet<_> = prefixed(&plain, "install: ").into_iter().collect();
    let added: Vec<_> = installed.difference(&without).cloned().collect();
    assert_eq!(added, std::slice::from_ref(&inspector));
    assert!(prefixed(&stdout, "manifest entry: ").contains(&format!("system {inspector}")));
    assert!(stdout.contains(" -p magic-driver -p magic-driver-inspector\n"));
    let build_inspector = format!(
        "{}/target/release/magic-driver-inspector {}{inspector}",
        repo_root().display(),
        sandbox.destdir().display()
    );
    assert!(
        stdout.contains(&build_inspector),
        "{build_inspector} missing in\n{stdout}"
    );
    assert_eq!(sandbox.guarded_calls(), "");
    assert!(!sandbox.destdir().exists());
}

#[test]
fn a_prebuilt_inspector_is_taken_next_to_the_binary_or_from_its_option() {
    let sandbox = Sandbox::new();
    let (code, stdout, stderr) = sandbox.run(
        "install.sh",
        &[
            "--dry-run",
            "--binary",
            "/opt/md/magic-driver",
            "--with-inspector",
        ],
        false,
    );
    assert_eq!(code, 0, "{stdout}{stderr}");
    assert!(stdout.contains(
        "dry-run: sudo install -D -m 0755 -o root -g root /opt/md/magic-driver-inspector /usr/local/bin/magic-driver-inspector"
    ));
    assert!(!stdout.contains("cargo build"));

    let (code, stdout, stderr) = sandbox.run(
        "install.sh",
        &[
            "--dry-run",
            "--binary",
            "/opt/md/magic-driver",
            "--with-inspector",
            "--inspector-binary",
            "/opt/other/inspector",
        ],
        false,
    );
    assert_eq!(code, 0, "{stdout}{stderr}");
    assert!(stdout.contains(
        "dry-run: sudo install -D -m 0755 -o root -g root /opt/other/inspector /usr/local/bin/magic-driver-inspector"
    ));

    let (code, stdout, stderr) = sandbox.run(
        "install.sh",
        &["--dry-run", "--inspector-binary", "/opt/other/inspector"],
        false,
    );
    assert_eq!(code, 1);
    assert_eq!(stdout, "");
    assert!(stderr.contains("--inspector-binary needs --with-inspector"));
    assert_eq!(sandbox.guarded_calls(), "");
}

#[test]
fn uninstall_dry_run_removes_the_inspector_when_it_was_installed() {
    let sandbox = Sandbox::new();
    let (code, install_out, _) =
        sandbox.run("install.sh", &["--dry-run", "--with-inspector"], true);
    assert_eq!(code, 0);
    sandbox.write_manifest(&prefixed(&install_out, "manifest entry: "));
    let before = file_tree(&sandbox.destdir());

    let (code, stdout, stderr) = sandbox.run("uninstall.sh", &["--dry-run"], true);
    assert_eq!(code, 0, "{stdout}{stderr}");
    let removed: BTreeSet<_> = prefixed(&stdout, "remove: ").into_iter().collect();
    assert!(removed.contains("/usr/local/bin/magic-driver-inspector"));
    let installed: BTreeSet<_> = prefixed(&install_out, "install: ").into_iter().collect();
    assert!(installed.is_subset(&removed));
    assert!(stdout.contains(&format!(
        "dry-run: sudo rm -f -- {}/usr/local/bin/magic-driver-inspector",
        sandbox.destdir().display()
    )));
    assert_eq!(sandbox.guarded_calls(), "");
    assert_eq!(file_tree(&sandbox.destdir()), before);
}

#[test]
fn reinstalling_without_the_flag_removes_an_earlier_inspector() {
    let sandbox = Sandbox::new();
    let (code, install_out, _) =
        sandbox.run("install.sh", &["--dry-run", "--with-inspector"], true);
    assert_eq!(code, 0);
    sandbox.write_manifest(&prefixed(&install_out, "manifest entry: "));
    let before = file_tree(&sandbox.destdir());

    let (code, stdout, stderr) = sandbox.run("install.sh", &["--dry-run"], true);
    assert_eq!(code, 0, "{stdout}{stderr}");
    assert!(stdout.contains(
        "remove: /usr/local/bin/magic-driver-inspector (installed before, not requested now)"
    ));
    assert!(stdout.contains(&format!(
        "dry-run: sudo rm -f -- {}/usr/local/bin/magic-driver-inspector",
        sandbox.destdir().display()
    )));
    assert!(
        !prefixed(&stdout, "manifest entry: ")
            .iter()
            .any(|entry| entry.contains("inspector"))
    );

    let (code, stdout, _) = sandbox.run("install.sh", &["--dry-run", "--with-inspector"], true);
    assert_eq!(code, 0);
    assert!(!stdout.contains("not requested now"));
    assert_eq!(sandbox.guarded_calls(), "");
    assert_eq!(file_tree(&sandbox.destdir()), before);
}
