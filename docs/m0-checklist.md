# M0 owner checklist

| | |
|---|---|
| Audience | the owner only |
| Results go to | `docs/m0-findings.md` (one section per assumption, field names given below) |
| Needs | the udev rule from `docs/install-m0.md`, a release build, `libinput` tools (`libinput list-devices`, `libinput debug-events`, needs the `input` group or `sudo`), `wev` |

Every step says what to run, what to do with the mouse, what to look at and which field of `docs/m0-findings.md` to fill. Run everything from the repository root in a terminal inside the Plasma Wayland session.

## 0. Preparation

1. Build: `cargo build --release`.
2. Install the rule: follow `docs/install-m0.md`, steps 1 to 5.
3. Note the versions in `docs/m0-findings.md`, section "Environment":

   ```sh
   uname -r
   plasmashell --version
   kwin_wayland --version
   libinput --version
   ```

4. Keep a second input device at hand (keyboard shortcuts work, or a USB mouse) in case the Magic Mouse stops responding. The stop procedure is in section 12.

## 1. Start and stop safely (do this first)

1. Terminal A: `./target/release/magic-driver status`. The mouse must be listed with `read-write` for the raw and stock devices.
2. Terminal A: `./target/release/magic-driver run --mode raw-forward`.
3. Expected output: `magic-driver: active: Apple Magic Mouse 2 (Bluetooth) (raw /dev/hidrawN, stock /dev/input/eventM grabbed)`.
4. Move the mouse and click left and right: the pointer moves and clicks work.
5. Press Ctrl+C in terminal A. Expected: `released ... shutdown; stock behaviour restored`, then `stopped by Interrupt`.
6. Move the mouse and scroll: it behaves exactly as before (stock wheel steps).
7. Findings field: "Start and stop" (yes or no, plus anything unexpected).

## 2. A-1: libinput classifies the virtual touchpad, KDE lists it and remembers settings

1. Terminal A: `./target/release/magic-driver run --mode raw-forward`.
2. Terminal B: `libinput list-devices` (add `sudo` if your user cannot read `/dev/input`).
3. Look for the devices `magic-driver pointer` and `magic-driver touchpad`. For the touchpad note: `Capabilities` (expected `pointer gesture`), `Size` (expected about 400x400mm), `Tap-to-click` (expected `disabled`), `Disable-w-typing` (expected `n/a`, which is what `ID_INPUT_TOUCHPAD_INTEGRATION=external` gives). For the pointer note its `Capabilities` (expected `pointer`).
4. Terminal B: `udevadm info /dev/input/eventK` for the touchpad node (from `magic-driver status`, section "Virtual devices"). Note whether `ID_INPUT_TOUCHPAD=1` and `ID_INPUT_TOUCHPAD_INTEGRATION=external` are present.
5. Open System Settings, Touchpad. Check that `magic-driver touchpad` is listed. Change one setting, for example toggle natural scrolling, and apply.
6. Open System Settings, Mouse. Check that `magic-driver pointer` is listed.
7. Stop with Ctrl+C, start again, open the Touchpad page: is the setting from step 5 still applied?
8. Turn the mouse off and on (switch underneath). Wait for `active:` in terminal A. Check the Touchpad page again.
9. Findings fields: "A-1 list-devices output", "A-1 KDE lists touchpad", "A-1 KDE lists pointer", "A-1 setting kept across restart", "A-1 setting kept across reconnect", "A-1 result" (pass, fail, partial).

## 3. A-2: two synthetic fingers give finger-source scroll with axis_stop

1. Terminal A: `./target/release/magic-driver run --mode scroll-probe`. In this mode every touch on the mouse becomes two fingers moving together on the virtual touchpad, without filtering.
2. Terminal B: `libinput debug-events --device /dev/input/eventK` with the touchpad node.
3. Rest the hand off the mouse. Put one finger on the surface, slide it slowly forward, lift it.
4. Expected in terminal B: `POINTER_SCROLL_FINGER` events while moving, and a final event with value `0.00` on the axis after the lift (that is the scroll stop). Copy a short excerpt.
5. Terminal B: stop `libinput`, then run `wev` and focus its window. Repeat step 3 over it. Expected: `wl_pointer.axis_source` with `finger`, `wl_pointer.axis` events, and `wl_pointer.axis_stop` after the lift. Copy a short excerpt.
6. Repeat sideways for horizontal scroll.
7. Findings fields: "A-2 libinput excerpt", "A-2 wev excerpt", "A-2 horizontal", "A-2 result".

## 4. A-3: momentum in the daily apps

1. Terminal A: `./target/release/magic-driver run --mode scroll-probe`.
2. For each app in the matrix in `docs/m0-findings.md` (Firefox, a Chromium app, Slack, VS Code, Dolphin, Kate, Konsole), open a long document or page, and flick one finger forward on the mouse and lift quickly.
3. Observe whether the content keeps gliding after the lift (momentum) and whether it slows down smoothly.
4. Findings fields: matrix column "Momentum after flick" (yes, no, partial) and "Notes".

## 5. A-4: stationary fingers stop momentum (hold gesture)

1. Still in scroll-probe mode, in each app: flick, and while the content glides put one finger down on the mouse and keep it still.
2. Observe whether the glide stops immediately.
3. Optional: in terminal B run `libinput debug-events --device /dev/input/eventK` and look for `GESTURE_HOLD_BEGIN` and `GESTURE_HOLD_END` when the finger rests.
4. Findings fields: matrix column "Hold stops momentum", field "A-4 hold events seen".

## 6. A-5: hold then lift cancels without momentum

1. Still in scroll-probe mode, in each app: move a finger quickly, then stop the finger completely for about a tenth of a second, then lift.
2. Observe whether the content stays where it is after the lift (no glide).
3. Findings fields: matrix column "Hold then lift, no momentum".

## 7. A-6: KWin swipe gestures from synthesised fingers

The prototype modes produce at most as many synthetic fingers as fingers on the mouse (raw forward) or exactly two (scroll probe), so this step uses raw forward with several fingers.

1. Note the gesture bindings: System Settings, Workspace, Touchpad gestures (or the KWin effect settings). Write down which finger count switches desktops and which opens the overview, and the Plasma version.
2. Terminal A: `./target/release/magic-driver run --mode raw-forward`.
3. Terminal B: `libinput debug-events --device /dev/input/eventK`.
4. Put three fingers flat on the mouse surface and swipe them sideways together, then four if the surface allows it. Watch terminal B for `GESTURE_SWIPE_BEGIN` with the finger count, and watch whether KWin switches desktop.
5. Swipe up with the same count: does the overview open?
6. Fast scripted swipes are tested later; here note only whether a slow and a fast manual swipe both trigger.
7. Findings fields: "A-6 Plasma version", "A-6 finger count for desktop switch", "A-6 finger count for overview", "A-6 triggered by synthetic swipe", "A-6 fast swipe", "A-6 result". Copy the count into `advanced.gestures.kwin_swipe_fingers` in the findings.

## 8. A-7: horizontal finger scroll gives browser back and forward

1. Terminal A: `./target/release/magic-driver run --mode scroll-probe`.
2. In Firefox, open a page, follow a link, so that back is possible. Put the page at its left edge. Slide one finger sideways to the right on the mouse (and to the left for forward).
3. Observe whether Firefox shows the back arrow and navigates.
4. Repeat in a Chromium browser (Chrome, Chromium or Brave).
5. Findings fields: "A-7 Firefox back", "A-7 Firefox forward", "A-7 Chromium back", "A-7 Chromium forward", "A-7 notes" (for example a browser setting that had to be enabled).

## 9. A-8: hidraw delivers every report while grabbed, buttons and motion match the stock device

1. Terminal A: `./target/release/magic-driver run --mode raw-forward --debug 2> /tmp/m0-a8.log`.
2. Move the mouse in circles for about ten seconds. Then click left five times, right five times, and middle five times (finger in the middle of the surface). Then click left while two fingers rest on the surface.
3. Press Ctrl+C.
4. `grep -c ' hidraw ' /tmp/m0-a8.log` and `grep -c ' stock ' /tmp/m0-a8.log`: both are non-zero, and the hidraw count is at least the number of stock lines that contain `REL_X` or `REL_Y`.
5. `grep '^cross-check' /tmp/m0-a8.log`: the summary compares the sum of hidraw `dx`/`dy` with the sum of stock `REL_X`/`REL_Y`, and the press and release counts per button. Expected: `motion match | buttons match`.
6. If the summary says `differ`, find the first difference: `grep -E ' (hidraw|stock) ' /tmp/m0-a8.log | less` and compare the hidraw line (`clicks=`, `dx=`, `dy=`, emitted `button ... press`) with the stock line written right after it (`REL_X=`, `left=1`).
7. If `/etc/modprobe.d/hid_magicmouse.conf` sets `emulate_3button=0` or `middle_click_3finger`, the stock device decides middle clicks differently from magic-driver; then a `differ` on middle clicks only is expected. Note the parameters in the findings.
8. Findings fields: "A-8 report counts", "A-8 cross-check line", "A-8 differences", "A-8 result". Attach `/tmp/m0-a8.log` only if it contains no personal data (it holds only motion, buttons and touch coordinates).

## 10. A-9: uaccess applies in time for reconnect within 2 s

1. Terminal A: `./target/release/magic-driver run --mode raw-forward 2>&1 | ts '%.s'` (`ts` is in the `moreutils` package; without it, watch the clock).
2. Turn the mouse off with its switch. Expected: `released ...: device removed` (or `device gone`).
3. Turn it on again. Note the time between the moment the mouse connects (KDE shows it, or the first `cannot ...` line) and the `active:` line.
4. Lines such as `cannot open raw device ...: permission denied; retrying in 50 ms` show that the ACL came late; note how many appeared.
5. Repeat three times.
6. Findings fields: "A-9 reconnect times", "A-9 permission retries seen", "A-9 result" (pass if every reconnect became `active:` within 2 s).

## 11. A-10: the large virtual surface has no libinput side effects

1. Terminal A: `./target/release/magic-driver run --mode scroll-probe`.
2. Terminal B: `libinput debug-events --verbose --device /dev/input/eventK`.
3. Scroll slowly, flick, and rest fingers on the mouse for a few seconds.
4. Look for any warning or error lines from libinput, for example about touch jumps, kernel bugs, size or resolution, palm or thumb detection, and for unexpected pointer motion events (`POINTER_MOTION`) from the touchpad.
5. Check the journal for libinput messages: `journalctl --user -b -g libinput` and `journalctl -b -g libinput --since "-10 min"`.
6. Findings fields: "A-10 warnings", "A-10 unexpected events", "A-10 result".

## 12. Stop procedure (always works)

1. Press Ctrl+C in the terminal that runs `magic-driver run`. The output ends with `stock behaviour restored` and `stopped`.
2. If the terminal is gone: `pkill -INT -x magic-driver`. If that does not stop it: `pkill -KILL -x magic-driver`. The kernel releases the grab when the process ends, whatever the cause.
3. Confirm stock behaviour: move the mouse, click, scroll (wheel steps as before). `./target/release/magic-driver status` shows both virtual devices `not present`.
4. If the pointer still does not move, turn the mouse off and on.
5. Findings field: "Stop procedure" (did it ever need step 2 or 4).

## 13. Fixtures

Record each scenario into its own file. `record` reads the raw device without grabbing it, so the mouse keeps its stock behaviour while recording, and it refuses to overwrite a file. Stop each recording with Ctrl+C. `magic-driver run` must not be running at the same time.

```sh
mkdir -p tests/fixtures/magic-mouse-2-bt
./target/release/magic-driver record tests/fixtures/magic-mouse-2-bt/<scenario>.rec
```

| Scenario file | What to do during the recording (about 5 to 10 s each) |
|---|---|
| `idle-resting-palm.rec` | rest the hand on the mouse as when working, do not move |
| `move-fingers-resting.rec` | move the mouse around with fingers resting on the surface |
| `slow-scroll.rec` | hand off, one finger slowly forward and back a few times |
| `flick.rec` | several quick flicks forward with one finger |
| `horizontal-scroll.rec` | one finger sideways, left and right |
| `click-left.rec` | three left clicks |
| `click-right.rec` | three right clicks |
| `click-middle.rec` | three clicks with one finger in the middle of the surface |
| `click-while-touching.rec` | left and right clicks while another finger rests on the surface |
| `two-finger-swipe.rec` | two fingers swipe sideways, left and right |
| `two-finger-double-tap.rec` | two fingers tap twice quickly, three times |
| `connect-first-touch.rec` | start the recording with the mouse off, turn it on, wait for it to connect, then touch the surface once and scroll a little. Start `record` right after the mouse connects if `record` reports that no mouse was found |

After recording, check each file offline (safe, no device access):

```sh
for f in tests/fixtures/magic-mouse-2-bt/*.rec; do ./target/release/magic-driver replay "$f" > /dev/null || echo "FAILED: $f"; done
```

Also capture the udev records of the real mouse for the discovery tests (read-only):

```sh
H=$(./target/release/magic-driver status | sed -n 's/^    hid device:   \/sys//p' | head -1)
{ udevadm info -p "$H"/hidraw/hidraw*; udevadm info -p "$H"; udevadm info -p "$H"/input/input*; udevadm info -p "$H"/input/input*/event*; udevadm info -p "$H"/input/input*/mouse*; } > /tmp/magic-mouse-2-bt.udev
```

Replace the Bluetooth addresses (`HID_PHYS`, `HID_UNIQ`, `PHYS`, `UNIQ`, `LIBINPUT_DEVICE_GROUP`) with `00:00:00:00:00:01` and `00:00:00:00:00:02`, then hand the file to the implementer to replace `crates/magic-driver/src/discovery/fixtures/magic-mouse-2-bt.udev`.

Findings field: "Fixtures recorded" (list of files, any that failed to replay).

## 14. Decision

Fill the section "Decision" in `docs/m0-findings.md`: go, adjust or stop. If A-2 or A-3 failed broadly, the fallback (pointer-only with high-resolution wheel events plus self-generated momentum) needs your decision; note it there.
