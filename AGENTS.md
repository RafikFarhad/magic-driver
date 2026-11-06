# AGENTS.md

Guide for anyone, human or agent, changing magic-driver. Read the safety rules before running anything.

## What this is

magic-driver is a user-space driver for the Apple Magic Mouse 2 over Bluetooth on Linux, aimed at KDE Plasma 6 on Wayland. It runs as a systemd user service:

1. The stock kernel driver `hid_magicmouse` stays bound. It turns on multi-touch reporting and exposes two nodes for the same mouse: the stock evdev device (`/dev/input/eventN`) and the raw HID device (`/dev/hidrawN`).
2. magic-driver finds the mouse by HID identity `0005:004C:0269`, reads the raw reports from hidraw, and grabs the stock evdev device so the desktop stops seeing it.
3. It creates two uinput devices: `magic-driver pointer` (motion and buttons) and `magic-driver touchpad` (synthetic touches).
4. Touches are never forwarded as they are. A pure engine decides which touches are deliberate, recognises the gesture, and synthesises the touchpad finger pattern libinput already understands: one deliberate finger on the mouse becomes a two-finger scroll on the virtual touchpad, a two-finger swipe becomes a multi-finger touchpad swipe. libinput and KWin then provide smooth scrolling, momentum, natural scrolling and desktop gestures.
5. If the process exits, crashes or is killed, the kernel releases the grab and the stock mouse works again immediately.

## Repository layout

| Path | What it is |
|---|---|
| `crates/md-core` | The engine. Pure, deterministic, no I/O, no clock, `#![forbid(unsafe_code)]`. Time is passed in. |
| `crates/md-trace` | Trace record types and the length-prefixed wire format shared by the daemon and the inspector. |
| `crates/magic-driver` | The daemon and CLI (`run`, `status`, `doctor`, `record`, `replay`, `config`, `trace`, `inspect`): udev discovery, hidraw, evdev grab, uinput, the epoll runtime, sessions, logging, the trace socket. |
| `crates/magic-driver-inspector` | Optional diagnostic window (eframe/egui). Separate binary so the daemon never links GUI code. |
| `dist/` | udev rule, `uinput` modules-load file, systemd user unit, example config (generated). |
| `scripts/` | `check.sh` (the full gate), `install.sh` and `uninstall.sh` (script install for non-Arch systems). |
| `packaging/arch/` | `PKGBUILD` and install message for the `magic-driver-git` package. |
| `docs/` | Install guide, inspector guide, the raw report reference, owner checklists (`m0` to `m3`). |
| `spikes/` | Throwaway toolkit experiments, own Cargo workspace, excluded from the main one. |
| `tests/fixtures/` | Raw report recordings used by replay tests (see its README). |

`docs/PRD.md`, `docs/ARCHITECTURE.md` and `docs/epics/` are the owner's private planning notes. They are git-ignored on purpose. Do not commit them, link to them, or rely on them; everything a contributor needs is in this file, the code and the public docs.

## Engine pipeline (md-core)

`Engine::handle(Input, &mut Vec<Output>)` runs once per raw report (about 90 per second while touching) and on timer deadlines (`Engine::next_deadline`).

1. `report`: parses report `0x12` (14-byte header plus 8 bytes per touch, layout from the kernel's `hid-magicmouse.c`, documented in `docs/reference/magicmouse-report.md`). Malformed reports are counted, never a panic.
2. `tracker`: maps device touch ids to tracks. Hover (`0x20`) is never a contact.
3. `context`: decaying mouse-motion energy, button state, last press and release.
4. `classify`: each track goes `Candidate` to `Deliberate` or `Rejected` (sticky until lift). Reasons: `palm` (contact size), `rear_zone`, `mouse_moving`, `click_guard`, `resting`, `promoted`.
5. `recognise`: `Idle`, `Scroll`, `Swipe`, `TapPending`, `Cancelling`, `Suppressed`.
6. `synth`: synthetic touchpad frames, scroll gain curve, axis lock, scripted gestures.
7. `buttons`: left, right, middle from the click bit and touch positions.

Pointer motion and buttons are emitted before touch processing, unbuffered. All thresholds live in `md_core::config` (`[advanced.touch]`, `[advanced.scroll]`, `[advanced.gestures]`); profile defaults are in `md_core::profile`.

## Daemon behaviour worth knowing

- One single-threaded epoll loop: udev monitor, signalfd (`SIGHUP` reloads config), one timerfd, the trace socket, and per session the hidraw and evdev fds. No async runtime.
- Self-check: if stock events keep arriving for 250 ms while hidraw delivers nothing, or reports stop parsing, the session keeps the grab and the virtual devices, logs an error, and reopens only hidraw with backoff. While the process runs it never hands the mouse back to the stock driver; only removal, stop, crash or exit do.
- Logs go to the journal when stderr is the journal stream (`JOURNAL_STREAM` matches), otherwise to the terminal. Problems are `warn` or `error`, so `journalctl --user -u magic-driver -p warning` shows only problems.
- The trace socket `$XDG_RUNTIME_DIR/magic-driver/trace.sock` streams JSON trace records (reports, decisions, recogniser changes, outputs) to `magic-driver trace` and the inspector. It is read-only for clients and owner-only (`0600`).
- `status` and `doctor` are read-only. `doctor` also catches environment problems: missing kernel modules after an upgrade without reboot, the mouse bound to `hid-generic`, KDE set to edge scrolling for `magic-driver touchpad`.

## Toolchain and commands

The toolchain is pinned in `rust-toolchain.toml` (1.98.1 with rustfmt and clippy). Use rustup; a newer system clippy reports lints the pinned one does not, and CI uses the pinned one.

| Task | Command |
|---|---|
| Full gate, run before every commit | `scripts/check.sh` (fmt, clippy pedantic with `-D warnings`, all tests, the two dependency checks, shellcheck) |
| Build | `cargo build --release --locked -p magic-driver -p magic-driver-inspector` |
| Tests | `cargo test --locked --workspace` |
| One crate | `cargo test -p md-core` |
| Review snapshot changes | `INSTA_UPDATE=new cargo test`, then compare each `.snap.new` with its `.snap`; accept only intended changes |
| Regenerate the example config after changing a default | `cargo run -p magic-driver -- config defaults > dist/config.example.toml` |
| Replay a recording offline | `cargo run -p magic-driver -- replay <file>` (no `--emit`) |
| Arch package | see `docs/install.md` section 0 |

`scripts/check.sh` enforces two architectural rules: md-core must not depend on Linux, I/O, async or clock crates, and the `magic-driver` daemon must not depend on any GUI, Wayland or graphics crate. Do not work around them; put the code in the right crate instead.

## Safety rules

The developer's own desktop is never a test target. These rules apply to every contributor and every agent, with or without `sudo`.

Allowed:

- editing code and docs, `cargo build`, `cargo test` (default set), `cargo clippy`, `cargo fmt`, `scripts/check.sh`
- `magic-driver replay <file>` without `--emit`, `magic-driver config test`, `magic-driver config defaults`
- read-only inspection: `magic-driver status`, `magic-driver doctor`, `ls -l /dev/...`, `getfacl`, `udevadm info`, reading `/sys`
- `scripts/install.sh --dry-run`, `scripts/uninstall.sh --dry-run`, `makepkg` without installing

Forbidden unless the person whose machine it is asks for that exact action:

- `magic-driver run`, `magic-driver record`, `magic-driver replay --emit`
- live `magic-driver trace` or `magic-driver inspect` against someone's running daemon
- any test or tool that opens `/dev/uinput`, `/dev/hidraw*` or `/dev/input/event*`, or creates, grabs or destroys an input device
- `scripts/install.sh` or `scripts/uninstall.sh` without `--dry-run`, `pacman -U` or `makepkg -i`
- writing, reloading or triggering udev rules, `modprobe`, `rmmod`, `/etc/modprobe.d`, `hid_*` parameters, driver bind or unbind
- `systemctl --user start`, `stop`, `restart`, `enable` or `disable` of `magic-driver`
- changing KDE input settings (`kcminputrc`, KWin input D-Bus properties) or Bluetooth state

Two device tests exist and are `#[ignore]` plus gated by `MAGIC_DRIVER_DEVICE_TESTS=1`. Run them only in an isolated VM. CI refuses to run when input device nodes are visible.

When a change needs verification on real hardware, write the steps into the matching owner checklist in `docs/` (exact commands and what to observe) and leave them for the owner.

## Testing rules

- Every test must be hardware-free and headless: no device nodes, no window, no Wayland or X display, no real `$XDG_RUNTIME_DIR` socket. Use the fakes in `crates/magic-driver/src/io/fake.rs`, temp directories, and fixture roots for `/sys` and `/proc` reads (see `doctor/fixtures`).
- Engine behaviour is tested by feeding reports. `md_core::testkit` (feature `test-support`) has `ReportBuilder`, `TouchRecord` and ready scenarios (`finger`, `palm`, `grip`, `flick`, `slow_scroll`, ...).
- Bug fixes start with a test that fails for the reported reason, then the fix.
- Tuning a threshold needs evidence from real data: a recording or a captured trace that shows the wrong decision, and a regression test built from its numbers. Explain the before and after in the commit or PR.
- The engine hot path must not allocate (`crates/md-core/tests/no_allocation.rs`).

## Code conventions

- No code comments, in any language or file. Names, types and tests carry the meaning; context goes in the commit message or PR description.
- No `unsafe` outside the inspector's Wayland glue.
- Errors are typed (`thiserror`); a malformed report or an unexpected device state is an error value, never a panic.
- Public docs: no em-dashes, one line per paragraph (no manual hard wraps), tables for reference material.
- Keep the daemon small and boring: no new threads, no async runtime, no new dependency without a reason in the PR.

## Commits and pull requests

- Small, focused commits with an imperative subject, for example `Stop rejecting small scrolls as palm or mouse movement`.
- Run `scripts/check.sh` before pushing; CI runs the same script on the pinned toolchain.
- Describe in the PR why the change is needed and what it changes. For behaviour changes, include the evidence (trace numbers, recording, or test) and anything the owner should verify on a real mouse.
- Never commit recordings or traces that identify a person or device (Bluetooth names, MAC addresses, home paths). Fixtures use neutral names such as `Owner's Magic Mouse`.
