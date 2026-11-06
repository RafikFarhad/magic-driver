# M2 owner checklist

| | |
|---|---|
| Audience | the owner only; implementers write this list but never run it |
| Needs | the M2 build installed with `scripts/install.sh` (see `docs/install.md`), the M1 checklists done or at least sections 1 and 2 of `docs/m1-checklist.md`, `libinput` tools, Brave (and Firefox if you use it) |
| Results | fill the `Result` column of each table (pass, fail, notes), the week log in section 13, then the results table at the end |

Run every command from the repository root in a terminal inside the Plasma Wayland session. `journalctl --user -u magic-driver -f` follows the daemon log live.

Changing the config always works the same way, below called "edit and reload":

1. edit `~/.config/magic-driver/config.toml` (create it if missing, `magic-driver config defaults` prints every key);
2. `magic-driver config test`, which must end with `test is successful`;
3. `systemctl --user reload magic-driver`.

Real Magic Mouse recordings do not exist yet. The implementer tested every gesture with synthetic scenarios only, so this checklist is the first time the gestures meet the real surface and the real KWin. Section 12 says how to record a fixture when something is wrong.

## 0. Preparation

| Step | Do | Expect | Result |
|---|---|---|---|
| 0.1 | `git log -1 --format=%h` and `uname -r`, `plasmashell --version`, `libinput --version` | write the values in the results table | |
| 0.2 | `scripts/install.sh --dry-run`, then `scripts/install.sh` | the new binary is installed, ends with `magic-driver installed` | |
| 0.3 | `magic-driver status` | `active: Apple Magic Mouse 2 (Bluetooth) on hidrawN` | |
| 0.4 | `magic-driver config test` | effective values printed; `[gestures]` shows `swipe_desktops = true`, `double_tap_overview = true`; `[advanced.gestures]` shows `kwin_swipe_fingers = 4`, `double_tap_ms = 300`, `swipe_distance_mm = 8.0` | |
| 0.5 | Make at least two virtual desktops (System Settings, Virtual Desktops) | two or more desktops | |

## 1. Which finger count KWin binds (A-6, `advanced.gestures.kwin_swipe_fingers`)

M0 saw KWin switch desktops from raw-forward 3-finger and 4-finger swipes but did not record which count is bound to which action. magic-driver turns a 2-finger swipe into `kwin_swipe_fingers` synthetic fingers (default 4) and the double tap into a scripted swipe up with the same count.

| Step | Do | Expect | Result |
|---|---|---|---|
| 1.1 | Terminal A: `sudo libinput debug-events --device /dev/input/eventM` where `eventM` is `magic-driver touchpad` from `magic-driver status` | events from the virtual touchpad only | |
| 1.2 | Rest the palm as usual, put two fingers on the surface and swipe them sideways together about 2 cm | `GESTURE_SWIPE_BEGIN ... 4`, `GESTURE_SWIPE_UPDATE ... 4` lines, then `GESTURE_SWIPE_END`; no `POINTER_SCROLL_FINGER` lines during the swipe | |
| 1.3 | Same swipe without the terminal in front | KWin switches to the next or previous virtual desktop | |
| 1.4 | Double tap with two fingers (two quick light taps, fingers do not move) | `GESTURE_SWIPE_BEGIN ... 4` with updates going up (negative dy) and an end; KWin opens the Overview (or the action bound to a 4-finger swipe up) | |
| 1.5 | If 1.3 or 1.4 did nothing: edit and reload with `[advanced.gestures]` `kwin_swipe_fingers = 3`, repeat 1.2 to 1.4 | write down which count switches desktops and which opens the Overview | |
| 1.6 | Write the confirmed count in the results table (`kwin_swipe_fingers to use`); if it is not 4, keep it in your config and tell the implementer so the default can change with a fixture | value recorded | |

## 2. Desktop swipe from the surface

| Step | Do | Expect | Result |
|---|---|---|---|
| 2.1 | Two-finger swipe left, then right, at a normal speed, ten times each | the desktop switches every time in the swipe direction (or the direction KWin uses for natural swiping) | |
| 2.2 | Slow two-finger swipe | the desktop switch follows the fingers and completes when you lift | |
| 2.3 | Short fast two-finger flick (1 to 2 cm) | the desktop switches; note if it needs a longer swipe (tuning key `advanced.gestures.swipe_distance_mm`, default 8 mm) | |
| 2.4 | Two fingers moving forward or backward (vertical) | nothing at all: no scroll, no desktop switch, no overview | |
| 2.5 | One-finger horizontal scroll in a wide page or Dolphin's details view, ten times | horizontal scrolling only, never a desktop switch | |
| 2.6 | During a two-finger swipe, lift one finger and keep moving the other | the swipe ends; the remaining finger does not start a scroll until it is lifted and put down again | |
| 2.7 | Start a two-finger swipe and click during it | the fingers freeze and lift; no scroll; note whether KWin still switched (it may complete a swipe that was already past its threshold) | |
| 2.8 | Move the mouse with the hand resting as usual for a minute | no desktop switch | |

## 3. Overview from the double tap

| Step | Do | Expect | Result |
|---|---|---|---|
| 3.1 | Two-finger double tap, ten times | the Overview opens every time (close it again with the same gesture or Escape) | |
| 3.2 | A single two-finger tap, ten times | nothing | |
| 3.3 | Two-finger tap, then a two-finger tap together with a click | nothing apart from the click | |
| 3.4 | Two two-finger taps with about half a second between them | nothing (the window is `advanced.gestures.double_tap_ms`, 300 ms after the first tap's lift) | |
| 3.5 | Two-finger double tap where the fingers slide a little | nothing if they slide more than about 1.5 mm (that is a swipe or a scroll start); note if real taps are refused | |
| 3.6 | One-finger double tap, ten times | nothing | |
| 3.7 | If 3.1 fails often because your taps are slower: edit and reload with `[advanced.gestures]` `double_tap_ms = 450`, repeat 3.1 to 3.4 | note the value that works | |

## 4. Browser back and forward

No new gesture: one-finger horizontal scroll arrives as finger-source horizontal scroll (A-7), and the browser turns overscroll at the page edge into back and forward.

| Step | Do | Expect | Result |
|---|---|---|---|
| 4.1 | Brave: open a page, follow a link, then one-finger swipe right on the surface at the left edge of the page (or the direction your natural scrolling setting makes "back") | the back arrow appears and the previous page opens | |
| 4.2 | Brave: the opposite swipe | forward | |
| 4.3 | Brave: horizontal scroll inside a wide element (a code block or table) | the element scrolls; back or forward only at its edge | |
| 4.4 | Firefox (optional, M0 notes you do not use it): same as 4.1 and 4.2 | back and forward | |
| 4.5 | `sudo libinput debug-events --device /dev/input/eventM` during 4.1 | `POINTER_SCROLL_FINGER` with `horiz` values and a final `0.00/0.0*` stop; no gesture lines | |

## 5. Acceleration presets

| Step | Do | Expect | Result |
|---|---|---|---|
| 5.1 | Default (`medium`): scroll very slowly in Kate line by line | fine, pixel-level control | |
| 5.2 | Default: a fast flick in a long page | covers clearly more distance than the same flick with `off` (5.4) | |
| 5.3 | Edit and reload with `[scroll]` `acceleration = "high"` | flicks go further than with `medium`; slow scrolling feels the same | |
| 5.4 | Edit and reload with `acceleration = "off"` | constant speed: slow and fast scrolling move the page by the finger distance times KDE's scroll speed only | |
| 5.5 | Edit and reload with `acceleration = "low"` | between `off` and `medium` | |
| 5.6 | Pick the preset you like and write it in the results table | value recorded | |

Very long fast strokes with `high` can reach the edge of the large virtual touchpad. The synthetic fingers then stop until you lift, so the scroll stops early; note it if you see it.

## 6. Axis lock

| Step | Do | Expect | Result |
|---|---|---|---|
| 6.1 | In a page that scrolls both ways (a wide spreadsheet or a map), scroll vertically with a natural slight sideways drift | the page moves vertically only | |
| 6.2 | Scroll horizontally with a slight vertical drift | horizontal only | |
| 6.3 | Edit and reload with `[scroll]` `axis_lock = false`, repeat 6.1 | the page now also drifts sideways | |
| 6.4 | Restore `axis_lock = true` (or delete the line) and reload | 6.1 behaves as before | |

## 7. Each gesture off individually, then reloaded

Do each line, check it, then undo it and reload before the next line.

| Step | Config change, then reload | Expect | Result |
|---|---|---|---|
| 7.1 | `[gestures]` `swipe_desktops = false` | a two-finger swipe does nothing at all (no desktop switch, no scroll); one-finger scroll and the double tap still work | |
| 7.2 | `[gestures]` `double_tap_overview = false` | a two-finger double tap does nothing; the swipe and scrolling still work | |
| 7.3 | `[scroll]` `enabled = false` | no scrolling from the surface; the swipe and the double tap still work | |
| 7.4 | `[scroll]` `horizontal = false` | vertical scrolling only; browser back and forward from the surface stop working, as expected | |
| 7.5 | `[pointer]` `middle_click = false` | a click in the middle zone is a left click | |
| 7.6 | Remove all the changes and reload | everything works as in sections 2 to 6 | |

## 8. First touch after connecting

| Step | Do | Expect | Result |
|---|---|---|---|
| 8.1 | Switch the mouse off and on, wait for `active` in the journal, then put one finger down and scroll slowly | the page starts moving smoothly from the first millimetre, no jump of several lines | |
| 8.2 | Repeat three times | no jump | |

## 9. Suspend and resume ten times

| Step | Do | Expect | Result |
|---|---|---|---|
| 9.1 | Terminal: `journalctl --user -u magic-driver -f -o short-precise` | log follows | |
| 9.2 | Ten times, with the mouse on: `systemctl suspend`, wait at least 10 s, wake the PC, move and click the mouse | after each wake: `released ...` (if the mouse disconnected) then `active: ... taken over N ms after the mouse appeared`; pointer, clicks and scrolling work without any manual action | |
| 9.3 | Write the ten values of N | each under 2 000 ms after the mouse reconnected | |
| 9.4 | After the last cycle: `magic-driver status` | `active`, exactly one `magic-driver pointer` and one `magic-driver touchpad` | |
| 9.5 | After the last cycle: a two-finger swipe and a two-finger double tap | both still work | |
| 9.6 | `journalctl --user -u magic-driver -b -p warning` | no lines, or copy them here | |

## 10. Other adapters

| Step | Do | Expect | Result |
|---|---|---|---|
| 10.1 | Only if a second Bluetooth adapter is available: pair the mouse through it (or switch adapters) and repeat 0.3 | `active` on the new hidraw node | |
| 10.2 | If no second adapter is available | write `not available` | |

## 11. Debug mode, live trace and logging

| Step | Do | Expect | Result |
|---|---|---|---|
| 11.1 | `journalctl --user -u magic-driver -b` after a normal hour of use | only lifecycle lines (starting, config, active, released); no per-report lines | |
| 11.2 | `ls -ld $XDG_RUNTIME_DIR/magic-driver` and `ls -l $XDG_RUNTIME_DIR/magic-driver/trace.sock` | directory `drwx------` owned by you, socket `srw-------` owned by you | |
| 11.3 | `magic-driver trace \| head -n 40` while scrolling | first line `{"type":"hello","protocol_version":1,...}` with the profile and the effective config, then `report`, `decision`, `recogniser` and `output` lines as JSON | |
| 11.4 | `magic-driver trace \| grep -E '"type":"(decision\|recogniser)"'` while resting the palm, scrolling, swiping and double tapping | `rejected` decisions with reasons such as `palm` or `resting`, `promoted` for the scroll finger, recogniser changes `idle` to `scroll`, `swipe`, `tap_pending` | |
| 11.5 | Stalled reader: `magic-driver trace \| (sleep 30; head -n 3)` and keep using the mouse during the 30 s | the mouse never lags or stutters; the daemon keeps running | |
| 11.6 | `magic-driver trace \| grep -m1 dropped` after 11.5 with a second terminal reading slowly (`magic-driver trace \| (sleep 20; grep -m1 dropped)`) | a `{"type":"dropped","count":N}` line (the slow reader lost records instead of slowing the daemon) | |
| 11.7 | `echo x \| socat - UNIX-CONNECT:$XDG_RUNTIME_DIR/magic-driver/trace.sock > /dev/null` (only if `socat` is installed) | the daemon closes that connection (the socket is subscribe-only); the daemon keeps running | |
| 11.8 | `systemctl --user stop magic-driver`, then `./target/release/magic-driver run --debug` (or `/usr/local/bin/magic-driver run --debug`), scroll, swipe and double tap, then Ctrl+C | text lines such as `... report clicks=0 dx=0 dy=0 touches=2 [...] in 2300 ns`, `... track 4 (touch 2) candidate -> deliberate: promoted`, `... recogniser idle -> scroll`, `... output frame 0:0@7600,8000 1:1@8400,8000`; processing times well under 1 000 000 ns | |
| 11.9 | `systemctl --user start magic-driver` | `active` again | |

## 12. Tuning and false triggers

Every threshold change needs a recording that reproduces the problem, so it becomes a regression test.

1. `systemctl --user stop magic-driver` so the stock driver is active.
2. `magic-driver record tests/fixtures/magic-mouse-2-bt/<name>.rec`, do the movement that went wrong (for example a swipe that did not switch, a tap that opened the Overview by accident), Ctrl+C.
3. `magic-driver replay tests/fixtures/magic-mouse-2-bt/<name>.rec`: lines with ` frame ` are the synthetic touchpad output (2 contacts for scrolling, 4 for a swipe or the scripted swipe up).
4. `systemctl --user start magic-driver`, and describe the case in the results with the file name.

| Symptom | Key | Default |
|---|---|---|
| desktop swipe needs too long a swipe, or starts by accident | `advanced.gestures.swipe_distance_mm` | 8.0 |
| KWin ignores the swipe or the double tap | `advanced.gestures.kwin_swipe_fingers` | 4 |
| double tap refused, or two separate taps open the Overview | `advanced.gestures.double_tap_ms` | 300 |
| taps count as scroll starts | `advanced.touch.promote_distance_mm` | 1.5 |
| slow scrolling too fast or too coarse | `scroll.acceleration`, `advanced.scroll.gain_speed_mm_s` | `medium`, [20, 250] |
| flicks do not go far enough | `scroll.acceleration`, `advanced.scroll.gain_max` | `medium`, 2.5 |
| sideways drift while scrolling vertically | `scroll.axis_lock` | true |

The known libinput line `kernel bug: missing right button, assuming it is a clickpad` on device add is expected (M0 A-10). Whether to add `INPUT_PROP_BUTTONPAD` to the virtual touchpad is still an open owner decision; the gesture work did not change the touchpad capabilities.

## 13. One week of daily use with gestures (M2 exit)

Use the gestures daily for a week. Fill one line per day. A false trigger is any scroll, desktop switch or Overview you did not intend.

| Day | Hours used | Unintended scrolls | Unintended desktop switches | Unintended Overview | Intended gestures that did not trigger | Missed or doubled clicks | Notes |
|---|---|---|---|---|---|---|---|
| 1 | | | | | | | |
| 2 | | | | | | | |
| 3 | | | | | | | |
| 4 | | | | | | | |
| 5 | | | | | | | |
| 6 | | | | | | | |
| 7 | | | | | | | |

Targets: fewer than 1 false trigger per hour in total (unintended scrolls, desktop switches and Overview together); 0 missed or doubled clicks; gestures used daily.

## 14. P1 coverage

| Requirement | Steps |
|---|---|
| suspend and resume | 9.1 to 9.6 |
| any adapter, several adapters | 10.1, 10.2 (and `docs/m1-checklist.md` section 7) |
| middle click with an option to turn it off | 7.5 |
| velocity scroll acceleration | 5.1 to 5.6 |
| axis lock | 6.1 to 6.4 |
| no phantom first-touch jump | 8.1, 8.2 |
| browser back and forward | 4.1 to 4.5 |
| desktop switch and Overview gestures | 1.2 to 1.6, 2.1 to 2.6, 3.1 |
| each gesture can be turned off | 7.1 to 7.4, 7.6 |
| no scroll or gesture from a click | 2.7, 3.3 |
| debug mode and live read-only trace | 11.2 to 11.9 |
| journal, quiet by default | 11.1 |

## Results

| Field | Value |
|---|---|
| Date | pending |
| magic-driver commit | pending |
| Kernel, Plasma, libinput versions | pending |
| `kwin_swipe_fingers` to use (desktop switch) | pending |
| Finger count that opens the Overview | pending |
| Preferred `scroll.acceleration` | pending |
| Other tuned values | pending |
| Fixtures recorded | pending |
| False triggers per hour over the week | pending |
| M2 exit (all P1 steps pass, under 1 false trigger per hour) | pending |
