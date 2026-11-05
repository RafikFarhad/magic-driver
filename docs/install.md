# Install, control and uninstall magic-driver

| | |
|---|---|
| Audience | the owner only (ARCHITECTURE 7.5); implementers only ever run the scripts with `--dry-run` |
| Task | E4-T07 |
| Replaces | `docs/install-m0.md` (the M0 rule-only install) |
| Undo | section 6, `scripts/uninstall.sh` |

magic-driver runs as a systemd user service in your Plasma session. One install step needs `sudo`: the binary, the udev rule and the `uinput` modules-load file. Everything else lives in your home directory.

## 1. Before you install

Write down what is there now, so you can compare after uninstalling:

```sh
ls -l /usr/local/bin/magic-driver /etc/udev/rules.d/70-magic-driver.rules /etc/modules-load.d/magic-driver.conf
ls -l ~/.config/systemd/user/magic-driver.service ~/.config/magic-driver ~/.local/share/magic-driver
getfacl /dev/uinput
```

If you installed the M0 rule by hand (`docs/install-m0.md`), that is fine: the installer overwrites `/etc/udev/rules.d/70-magic-driver.rules` with the same file and records it in the manifest, so the uninstaller removes it too.

## 2. Look at what the installer will do

```sh
cd ~/projects/magic-driver
scripts/install.sh --dry-run
```

`--dry-run` prints every action (`install:` for every file, `dry-run:` for every command) and changes nothing.

## 3. Install

```sh
scripts/install.sh
```

It does, in order:

1. `cargo build --release --locked -p magic-driver` (or use `--binary PATH` for a prebuilt binary).
2. With `sudo`: installs `/usr/local/bin/magic-driver`, `/etc/udev/rules.d/70-magic-driver.rules` and `/etc/modules-load.d/magic-driver.conf` (loads `uinput` on boot).
3. With `sudo`: `udevadm control --reload`, then re-triggers only the Magic Mouse 2 HID device (`--parent-match`) and `/sys/devices/virtual/misc/uinput`.
4. Installs the user unit `~/.config/systemd/user/magic-driver.service`.
5. Installs `~/.config/magic-driver/config.toml` from `dist/config.example.toml`, only if you have no config yet.
6. Writes the manifest `~/.local/share/magic-driver/install-manifest` (every file it created, except your config).
7. `systemctl --user daemon-reload` and `systemctl --user enable --now magic-driver.service`.

Then check:

```sh
magic-driver status
magic-driver doctor
```

### Optional: the inspector

The diagnostic window `magic-driver-inspector` (`magic-driver inspect`, see `docs/inspector.md`) is not installed by default. To add it:

```sh
scripts/install.sh --dry-run --with-inspector
scripts/install.sh --with-inspector
```

This also builds `-p magic-driver-inspector` and installs `/usr/local/bin/magic-driver-inspector` with `sudo`, listed in the manifest, so `scripts/uninstall.sh` removes it again. With `--binary PATH` the inspector is taken from the `magic-driver-inspector` file next to that binary, or from `--inspector-binary PATH`. Reinstalling later without `--with-inspector` removes an inspector that an earlier install put there, so the installed files always match the flags.

`status` shows the service state and the daemon's own status line (for example `active: Apple Magic Mouse 2 (Bluetooth) on hidraw5`), the mouse nodes, the virtual devices and the config. `doctor` prints `pass`, `warn` or `fail` for each check with a fix hint. Both only read.

## 4. Everyday control

| What | Command |
|---|---|
| Turn magic-driver off for now (mouse falls back to the stock driver at once) | `systemctl --user stop magic-driver` |
| Turn it back on | `systemctl --user start magic-driver` |
| Turn it off until you enable it again, also after login | `systemctl --user disable --now magic-driver` |
| Turn it on again permanently | `systemctl --user enable --now magic-driver` |
| Check a config change before applying it | `magic-driver config test` |
| Apply a config change | `systemctl --user reload magic-driver` |
| Read the log | `journalctl --user -u magic-driver -b` |
| Show only problems | `journalctl --user -u magic-driver -b -p warning` |

`reload` first runs `magic-driver config test`. If the file is invalid the reload fails with the error and line number, and the running config stays as it was. The daemon validates again on `SIGHUP`, so a direct `kill -HUP` is just as safe. `log.level` in the config (`warn`, `info`, `debug`) sets how much goes to the journal; `info` logs only lifecycle events (taken over, released, degraded, config reloaded). Problems are logged as warnings or errors, so `-p warning` shows only them; normal lifecycle events are logged below that priority.

While magic-driver runs it checks the raw stream of the mouse. If the stock device keeps reporting for 250 ms while no raw report arrives, if the raw device returns a read error, or if 10 raw reports within one second cannot be parsed, the journal shows an error starting with `lost the raw stream of /dev/hidrawN`. magic-driver keeps the stock device grabbed and keeps its virtual devices, so the mouse does not respond until the raw stream works again, and it reopens only the raw device, first after 50 ms and then with a growing delay of up to 5 s. While this lasts, `magic-driver status` shows `recovering: ...`, and a repeated failure is logged at most once every 5 s. When raw reports flow again the journal shows a warning `recovered /dev/hidrawN: raw reports flow again N ms after: ...`. If the reports cannot be parsed at all (for example after a kernel update changed the report format) the mouse stays unresponsive until you stop magic-driver with `systemctl --user stop magic-driver` (or Ctrl+C when it runs in a terminal), which gives the stock behaviour back at once. Only a disconnect of the mouse, a stop, a crash or an exit release the stock device.

When `magic-driver run` is started from a terminal it logs readable text to the terminal; it logs to the journal only when systemd connected its standard error to the journal.

The service restarts on its own one second after any crash, and systemd kills it if its main loop stops answering for 2 seconds (watchdog). In both cases the stock driver takes over immediately, because the kernel releases the grab when the process ends.

## 5. Stock driver options (`/etc/modprobe.d/hid_magicmouse.conf`)

magic-driver never changes the stock driver or its options (NFR-8). While magic-driver is active the stock device is grabbed, so its options have no visible effect; they only shape the fallback, that is the behaviour whenever magic-driver is stopped, restarting or degraded.

| Option | While magic-driver is active | Effect on the fallback |
|---|---|---|
| `emulate_scroll_wheel` | no effect | wheel emulation from the surface, as before |
| `scroll_speed` | no effect | wheel step speed, as before |
| `scroll_acceleration` | no effect | repeated-swipe acceleration, as before |
| `emulate_3button` | no effect; middle click is `pointer.middle_click` in the magic-driver config | middle click zone, as before |

Keep your file as it is. Do not set `emulate_scroll_wheel=0` for magic-driver's sake: it would only make the fallback lose scrolling.

## 6. Uninstall

```sh
scripts/uninstall.sh --dry-run
scripts/uninstall.sh
```

It stops and disables the service, removes every file listed in the manifest (with `sudo` for the system files), reloads udev, re-triggers the Magic Mouse 2 and uinput nodes so the ACLs from the rule go away, and removes the manifest. Your config in `~/.config/magic-driver/` stays; add `--purge` to remove it too.

Afterwards repeat the commands from section 1: the files are gone and `getfacl` shows what it showed before. `/dev/uinput` may still carry your ACL from KDE Connect's own rule, which was there before magic-driver. Directories that existed before, such as `~/.config/systemd/user`, are left alone.
