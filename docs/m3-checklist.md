# M3 owner checklist

| | |
|---|---|
| Audience | the owner only (ARCHITECTURE 7.5); implementers write this list but never run it |
| Tasks | owner steps of E6-T01 (spikes), E6-T04 (live mode), E6-T05 (Received panel), E6-T06 (install), and the M3 exit criterion |
| Needs | the E6 build, the M2 checklist done or at least `magic-driver status` showing `active`, a Plasma 6 Wayland session |
| Results | fill the `Result` column (pass, fail, notes) and the results table at the end |
| Status | waiting for the owner |

Run every command from the repository root in a terminal inside the Plasma Wayland session. The implementer built the spikes and the inspector and tested the inspector's logic headless with synthetic scenarios and a fake trace server; no real recording exists yet, and nobody has opened the window on a desktop.

## 0. Preparation and install (E6-T06)

| Step | Do | Expect | Result |
|---|---|---|---|
| 0.1 | `git log -1 --format=%h`, `uname -r`, `plasmashell --version` | write the values in the results table | |
| 0.2 | `scripts/install.sh --dry-run` | no line mentions `magic-driver-inspector` | |
| 0.3 | `scripts/install.sh --dry-run --with-inspector` | `build: cargo build --release --locked -p magic-driver -p magic-driver-inspector`, `install: /usr/local/bin/magic-driver-inspector`, `manifest entry: system /usr/local/bin/magic-driver-inspector` | |
| 0.4 | `scripts/install.sh --with-inspector` | ends with `magic-driver installed`; `ls -l /usr/local/bin/magic-driver-inspector` shows `root root` and mode `-rwxr-xr-x` | |
| 0.5 | `grep inspector ~/.local/share/magic-driver/install-manifest` | `system /usr/local/bin/magic-driver-inspector` | |
| 0.6 | `magic-driver status` | `active: Apple Magic Mouse 2 (Bluetooth) on hidrawN` | |

## 1. Toolkit spikes (E6-T01)

Compare what you see with the table in `docs/inspector-toolkit.md`.

| Step | Do | Expect | Result |
|---|---|---|---|
| 1.1 | `cd spikes && cargo build --release` | both spikes build | |
| 1.2 | `cargo run --release -p eframe-events`, pointer over the window, scroll slowly with one finger on the Magic Mouse, then lift | `MouseWheel unit=Point ... phase=Start`, then `phase=Move` lines, then one `phase=End`; `source=Unknown` on every line | |
| 1.3 | Same window: a quick flick | `Move` lines, then `End` at the lift; no line names the source or the axis that stopped | |
| 1.4 | Same window: touch the surface again while the page would still be moving, then two-finger swipe and double tap | no `Zoom`, `Rotate` or gesture lines (KWin may switch desktops or open the Overview, that is expected) | |
| 1.5 | If you have a wheel mouse: one wheel notch over the window | `unit=Line` with delta 1 or -1 | |
| 1.6 | Close it; `cargo run --release -p sctk-events` | first lines: `zwp_pointer_gestures_v1 version 3`, `pointer on wl_seat version N`; write N down | |
| 1.7 | One-finger scroll over the window, then lift | `enter (wl_pointer vN)`, then `axis ... source=Some(Finger) vertical={value ...}` lines, then one line with `stop true` on the scrolled axis | |
| 1.8 | Flick, then touch the surface again before the scroll would end | after the flick's stop line: `hold Begin { ... fingers: 2 }` and `hold End` (from magic-driver's hold to stop) | |
| 1.9 | One-finger horizontal scroll | `horizontal={value ...}` lines and a horizontal stop | |
| 1.10 | If you have a wheel mouse: one notch | `source=Some(Wheel)`, `discrete 1` (or -1) and, with `wl_seat` 8 or later, `value120 120` | |
| 1.11 | Two-finger swipe sideways | either nothing (KWin switches desktops and keeps the gesture) or `swipe Begin ... fingers: N`, `Update`, `End` lines | |

## 2. Offline replay (E6-T03)

| Step | Do | Expect | Result |
|---|---|---|---|
| 2.1 | `magic-driver record /tmp/m3-scroll.txt`, rest the palm, scroll up and down with one finger, click once, flick once, Ctrl+C | `recorded N reports` | |
| 2.2 | `magic-driver inspect --replay /tmp/m3-scroll.txt` | the window opens with the file name, the report count and `config:` with your config file | |
| 2.3 | Play, pause, step, step back, drag the slider, change the speed | the touch dots move with the recording; step moves one report; the time label follows | |
| 2.4 | Look at the Raw panel while the palm rests | the palm dot is large, in or near the shaded rear zone, with a red ring and reason `palm` or `rear zone`; the finger dot is small with a green ring once it moves | |
| 2.5 | Look at the Decisions panel during the scroll | recogniser `scroll`, a blue segment in the state timeline, two synthetic fingers moving together, decisions with `promoted` for the finger | |
| 2.6 | At the click | a button row in the pointer events, the touches around it rejected with `click guard`, recogniser `cancelling` then `idle` | |
| 2.7 | Switch palm rejection to `low` and `high`, acceleration to `off` and `high` | the decisions and the synthetic finger travel change; `reset config` restores them | |

## 3. Live mode (E6-T04)

| Step | Do | Expect | Result |
|---|---|---|---|
| 3.1 | `magic-driver inspect` | the connection line shows `connected to magic-driver <version> (Apple Magic Mouse 2 (Bluetooth)), active: ...`, `dropped records: 0` | |
| 3.2 | Move the mouse and touch the surface | the Raw panel follows in real time, about 90 reports per second while touching, processing times in µs | |
| 3.3 | Press pause, then drag the slider back | the view scrubs back up to 120 s; `go live` returns to the newest record | |
| 3.4 | `systemctl --user restart magic-driver` | the line shows `disconnected ...; reconnecting`, then `connected` again within a second; `connections: 2` | |
| 3.5 | `systemctl --user stop magic-driver`, wait 3 s, `systemctl --user start magic-driver` | `cannot connect to ...trace.sock ...; retrying` while stopped, then `connected`; the mouse falls back and returns as usual | |
| 3.6 | `ls -l $XDG_RUNTIME_DIR/magic-driver/` while the inspector runs | still only `trace.sock` with mode `srw-------` | |

## 4. Received panel (E6-T05)

Keep the pointer over the inspector window for these steps.

| Step | Do | Expect | Result |
|---|---|---|---|
| 4.1 | Look at the Received header | `listening on this window's Wayland pointer`, `wl_seat vN, wl_pointer vN, pointer gestures v3, hold` | |
| 4.2 | Scroll slowly with one finger, then lift | rows `scroll finger v: <value>` while moving and `scroll finger v: stop` at the lift; the graph draws a blue line and a red stop line | |
| 4.3 | Flick | finger rows with larger values, then a stop row at the lift (the flick's momentum is the app's job, so no rows follow) | |
| 4.4 | Flick, then touch again quickly | `hold begin, 2 fingers` and `hold end` rows | |
| 4.5 | One-finger horizontal scroll | `scroll finger h: ...` rows and an `h: stop`; orange line in the graph | |
| 4.6 | In live mode, compare the `trace t` column of a scroll row with the report times in the Raw panel at that moment (pause, step) | the times agree within a few tens of ms | |
| 4.7 | With natural scrolling switched on for the magic-driver touchpad in KDE | rows show `inverted` | |

## 5. M3 exit criterion: tuning and end-to-end check

| Step | Do | Expect | Result |
|---|---|---|---|
| 5.1 | Live, use the mouse normally for 10 minutes with the inspector open beside your work | every scroll you meant shows `promoted` for one finger; resting palm and grip fingers show `palm`, `rear zone` or `resting`; no scroll starts while you only move the mouse | |
| 5.2 | Any wrong decision: pause, note the time and the reason; then `magic-driver record /tmp/m3-case.txt` and repeat it | the replay of the recording shows the same wrong decision | |
| 5.3 | In that replay, try the palm rejection presets | write down which preset fixes it, or which advanced threshold the thresholds line suggests | |
| 5.4 | If a config change helps: edit `~/.config/magic-driver/config.toml`, `magic-driver config test`, `systemctl --user reload magic-driver`, then repeat 5.1 | the new values appear in the inspector after the reconnect (thresholds line) | |
| 5.5 | End to end: scroll, flick, touch to stop, two-finger swipe, two-finger double tap with the inspector live | each intent shows its recogniser state and synthetic fingers in Decisions; scroll and hold reach the Received panel; KWin performs the swipe and the Overview | |
| 5.6 | Keep the recordings that show a problem | attach them to the report; they become the first real fixtures | |

## 6. Uninstall check (E6-T06)

| Step | Do | Expect | Result |
|---|---|---|---|
| 6.1 | `scripts/uninstall.sh --dry-run` | `remove: /usr/local/bin/magic-driver-inspector` among the removals | |
| 6.2 | Optional: `scripts/install.sh` (without the flag) | `remove: /usr/local/bin/magic-driver-inspector (installed before, not requested now)`; afterwards `magic-driver inspect` prints the install hint | |

## Results

| Item | Value |
|---|---|
| Commit | |
| Kernel | |
| Plasma | |
| `wl_seat` version seen by the spike | |
| Spike results match `docs/inspector-toolkit.md` | |
| Palm rejection preset chosen | |
| Recordings kept as fixtures | |
| Open problems | |
