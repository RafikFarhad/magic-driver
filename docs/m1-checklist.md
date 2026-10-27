# M1 owner checklist

| | |
|---|---|
| Audience | the owner only (ARCHITECTURE 7.5); implementers write this list but never run it |
| Task | E4-T09, closes M1 together with E3-T06 (`docs/m1-scroll-checklist.md`) |
| Needs | the M1 build (E3 and E4 merged), `docs/install.md`, `libinput` tools, a second input device at hand (keyboard or a USB mouse) |
| Results | fill the `Result` column of each table in this file (pass, fail, notes), then the week log in section 11 |

Run every command from the repository root in a terminal inside the Plasma Wayland session. `journalctl --user -u magic-driver` shows the daemon log; `-f` follows it live.

## 1. Install (FR-50, FR-52, NFR-5)

| Step | Do | Expect | Result |
|---|---|---|---|
| 1.1 | Record the "before" state: run the commands of `docs/install.md` section 1 and save the output to `~/m1-before.txt` | file saved | |
| 1.2 | `scripts/install.sh --dry-run` | every file listed with `install:`, ends with `dry run finished, nothing was changed`; nothing changed on disk | |
| 1.3 | `scripts/install.sh` | asks for `sudo` once, ends with `magic-driver installed` | |
| 1.4 | `magic-driver status` | `Service: active (running), daemon reports: active: Apple Magic Mouse 2 (Bluetooth) on hidrawN`, both virtual devices present, config `missing, defaults are used` or `valid` | |
| 1.5 | `magic-driver doctor` | no `fail`; `uinput module`, `udev rule`, every ACL, `config`, `keyd`, tap-to-click and disable-while-mouse lines `pass`; the high-resolution line may `warn` about the stock device from before the install | |
| 1.6 | `ls -l /dev/hidrawN` and `getfacl /dev/hidrawN` (N from `status`) | owner `root:root`, mode `0600` or similar, never world-readable; `user:<you>:rw-` from the ACL only | |
| 1.7 | `systemctl --user show magic-driver -p NoNewPrivileges -p RestrictAddressFamilies -p MemoryMax -p WatchdogUSec` | `yes`, `AF_UNIX AF_NETLINK`, `67108864`, `2s` | |

## 2. Detection, start on login and takeover (FR-1, FR-2, FR-10, FR-11, FR-50)

| Step | Do | Expect | Result |
|---|---|---|---|
| 2.1 | Log out and log in again with the mouse connected | within a few seconds `magic-driver status` shows `active` | |
| 2.2 | `journalctl --user -u magic-driver -b` | `starting magic-driver`, `config ...`, then `active: ... taken over N ms after the mouse appeared` | |
| 2.3 | `libinput debug-events` (add `sudo` if needed) while moving the mouse and clicking left and right | events only from `magic-driver pointer`; nothing from the stock `Magic Mouse` device (no double movement or clicks) | |
| 2.4 | Move the pointer slowly and fast, compare with your memory of the stock feel | same speed and acceleration, still controlled from System Settings, Mouse, device `magic-driver pointer` | |
| 2.5 | Left click with the finger on the left, right click on the right, in Dolphin and Firefox or Brave | the expected button every time, no missed or doubled clicks | |
| 2.6 | Turn the mouse off, start the session, turn the mouse on (detection after start) | `active` within 2 s of the mouse connecting | |

## 3. Scrolling and touch quality (FR-13, FR-20, FR-21, FR-22, FR-40, FR-41)

The detailed scroll steps are in `docs/m1-scroll-checklist.md` (E3-T06). Summarise its results here.

| Step | Do | Expect | Result |
|---|---|---|---|
| 3.1 | One-finger slow scroll, vertical and horizontal, in Firefox or Brave, Dolphin, Kate | smooth pixel scrolling in both axes (FR-20) | |
| 3.2 | Flick, then touch the surface again | glide after the flick, clean stop on touch (FR-22) | |
| 3.3 | System Settings, Touchpad, device `magic-driver touchpad`: change scroll speed and natural scrolling | both take effect at once (FR-21) | |
| 3.4 | Click while a finger rests on the surface, ten times | no scroll or gesture from any click (FR-13) | |
| 3.5 | Move the mouse around for a minute with the palm and fingers resting as usual | no scroll at all (FR-40, FR-41) | |
| 3.6 | `journalctl -b \| grep -i 'high-resolution scroll'` after 3.1 to 3.5 | no line naming `magic-driver pointer` or `magic-driver touchpad` | |

## 4. Stop and fallback (FR-4, FR-52, US-9)

| Step | Do | Expect | Result |
|---|---|---|---|
| 4.1 | `systemctl --user stop magic-driver` | the mouse keeps working at once with the stock behaviour (wheel-step scrolling); `magic-driver status` shows the virtual devices `not present` | |
| 4.2 | `systemctl --user start magic-driver` | `active` again within 2 s | |
| 4.3 | `systemctl --user disable --now magic-driver`, log out and in | stock behaviour after login, service not running | |
| 4.4 | `systemctl --user enable --now magic-driver` | `active` again | |

## 5. Crash fallback (FR-4, NFR-3)

| Step | Do | Expect | Result |
|---|---|---|---|
| 5.1 | `systemctl --user kill -s KILL magic-driver` (same as `kill -9` on the main PID) | the stock behaviour is back immediately (move and scroll right away) | |
| 5.2 | Wait and watch `journalctl --user -u magic-driver -f` | systemd restarts the service after about 1 s; `active: ...` again well inside 5 s of the kill | |
| 5.3 | `systemctl --user show magic-driver -p NRestarts` | the counter went up by one | |

## 6. Hang fallback with the watchdog (FR-4, R-5, E4-T01)

| Step | Do | Expect | Result |
|---|---|---|---|
| 6.1 | `systemctl --user show magic-driver -p MainPID` and note the PID | a PID | |
| 6.2 | `kill -STOP <PID>`, start a stopwatch, keep moving the mouse | the pointer freezes first (the grab is still held by the stopped process) | |
| 6.3 | Keep moving the mouse | the stock mouse works again within 3 s of step 6.2 (watchdog after 2 s, then the kill) | |
| 6.4 | `journalctl --user -u magic-driver -b \| tail` | `Watchdog timeout` from systemd, then a restart and `active: ...` again | |

If 6.3 takes longer than 3 s, note the time and the journal lines; the fix to discuss is `WatchdogSignal=SIGKILL` in the unit.

## 7. Reconnect ten times (FR-3, US-10, E4-T03)

| Step | Do | Expect | Result |
|---|---|---|---|
| 7.1 | Terminal: `journalctl --user -u magic-driver -f -o short-precise` | log follows | |
| 7.2 | Ten times: switch the mouse off, wait 5 s, switch it on, move it | each time `released ...: device gone` (or `device removed`), then `active: ... taken over N ms after the mouse appeared`; the pointer works after each | |
| 7.3 | For each of the ten cycles compare the kernel line of the mouse appearing (`journalctl -k -o short-precise \| grep -i 'magic mouse'`) with the `active:` line | every takeover under 2 s; write the ten values of N and the worst delta here | |
| 7.4 | `magic-driver status` after the last cycle | `active`, exactly one `magic-driver pointer` and one `magic-driver touchpad` | |

## 8. Config reload (FR-51, D8, E4-T04)

| Step | Do | Expect | Result |
|---|---|---|---|
| 8.1 | Add `[pointer]` and `middle_click = false` to `~/.config/magic-driver/config.toml`; `magic-driver config test` | `test is successful` | |
| 8.2 | `systemctl --user reload magic-driver` | succeeds; journal `config reloaded from ...`; a middle-zone click is now a left click | |
| 8.3 | Change the line to `middle_click = "no"`; `systemctl --user reload magic-driver` | the reload fails; `systemctl --user status magic-driver` and the journal show `error: ...config.toml:2: pointer.middle_click: invalid value`; the service stays active and the middle zone still clicks left (old config kept) | |
| 8.4 | `kill -HUP $(systemctl --user show -p MainPID --value magic-driver)` with the invalid file still there | journal `config reload rejected, keeping the previous config` | |
| 8.5 | Add an unknown key `colour = "red"`, reload twice | the `unknown key, ignored` warning appears once in the daemon log, not on every reload | |
| 8.6 | Restore the file (or delete it) and reload | `config reloaded` or `reloaded the defaults` | |

## 9. Self-check (NFR-6, E4-T02)

This cannot be provoked safely on a working mouse. Check only that it stayed quiet:

| Step | Do | Expect | Result |
|---|---|---|---|
| 9.1 | After a day of use: `journalctl --user -u magic-driver -b \| grep -E 'degraded\|self-check'` | no lines; if there are any, copy them here (they name the cause and the retry time) | |

## 10. Uninstall leaves nothing (FR-52)

| Step | Do | Expect | Result |
|---|---|---|---|
| 10.1 | `scripts/uninstall.sh --dry-run` | every installed file listed with `remove:`, config shown as `keep:` | |
| 10.2 | `scripts/uninstall.sh` | the mouse goes back to the stock behaviour at once | |
| 10.3 | Run the commands of `docs/install.md` section 1 again and compare with `~/m1-before.txt` | identical, apart from `~/.config/magic-driver/config.toml` (kept on purpose) | |
| 10.4 | `systemctl --user list-unit-files \| grep magic` and `ls /etc/udev/rules.d /etc/modules-load.d \| grep magic` | nothing | |
| 10.5 | Install again with `scripts/install.sh` for the week of daily use | `active` | |

## 11. One week of daily use (PRD section 9 success metrics)

Use magic-driver as the default for a full week. Fill one line per day.

| Day | Hours used | Unintended scrolls or gestures (count) | Missed or doubled clicks | Manual fixes needed (restart, reconnect, re-login) | Momentum and smooth scroll fine in Firefox or Brave, a Chromium app, Dolphin or Kate | `journalctl -b \| grep -ci 'high-resolution scroll'` lines naming magic-driver devices | Notes |
|---|---|---|---|---|---|---|---|
| 1 | | | | | | | |
| 2 | | | | | | | |
| 3 | | | | | | | |
| 4 | | | | | | | |
| 5 | | | | | | | |
| 6 | | | | | | | |
| 7 | | | | | | | |

Targets: magic-driver stays the default after the week; fewer than 1 unintended scroll or gesture per hour; 0 missed or doubled clicks; 0 manual fixes; momentum in at least Firefox or Brave, one Chromium-based app and Dolphin or Kate; 0 high-resolution warnings from magic-driver devices.

## 12. P0 coverage

| PRD P0 requirement | Steps |
|---|---|
| FR-1 detect on connect and at start | 2.1, 2.6, 7.2 |
| FR-2 no parallel stock events | 2.3, 7.4 |
| FR-3 reconnect within 2 s | 2.6, 4.2, 7.2, 7.3 |
| FR-4 fallback on exit, crash, stop | 4.1, 5.1, 6.3, 10.2 |
| FR-10 pointer passthrough | 2.4 |
| FR-11 left and right click | 2.5 |
| FR-13 click never scrolls | 3.4 |
| FR-20 one-finger smooth scrolling | 3.1 |
| FR-21 scroll speed and natural scrolling in KDE | 3.3 |
| FR-22 momentum and clean stop | 3.2, 11 |
| FR-40 palm and grip rejection | 3.5, 11 |
| FR-41 resting fingers while moving | 3.5, 11 |
| FR-50 user service, start on login, restart on failure | 1.3, 2.1, 5.2 |
| FR-51 config file, explicit validated reload | 8.1 to 8.6 |
| FR-52 install, uninstall, temporary off | 1.2, 1.3, 4.1 to 4.4, 10.1 to 10.4 |
