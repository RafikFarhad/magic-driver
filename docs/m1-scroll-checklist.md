# M1 scroll owner checklist

| | |
|---|---|
| Audience | the owner only |
| Status | waiting for the owner |
| Results go to | the "Results" section at the end of this file |
| Needs | the udev rule from `docs/install-m0.md` (already installed in M0), a release build, `libinput` tools, `journalctl` |

Every step says what to run, what to do with the mouse and what to look at. Run everything from the repository root in a terminal inside the Plasma Wayland session. The stop procedure of `docs/m0-checklist.md` section 12 still applies: Ctrl+C in the terminal running magic-driver always gives the stock mouse back.

`magic-driver run` without `--mode` is the scroll path: deliberate one-finger movement becomes a two-finger scroll on `magic-driver touchpad`, and the palm, gripping fingers, resting fingers, mouse movement and clicks produce nothing. `--mode raw-forward` and `--mode scroll-probe` remain for diagnostics only.

## 0. Preparation

1. Build: `cargo build --release`.
2. Check that the rule is still active: `./target/release/magic-driver status` lists the mouse with `read-write` for the raw and stock devices.
3. Note the commit you test: `git rev-parse --short HEAD`.
4. Terminal A: `./target/release/magic-driver run`. Expected output: `magic-driver: active: Apple Magic Mouse 2 (Bluetooth) ...`.
5. Terminal B, keep it open for the whole checklist: `libinput debug-events --device /dev/input/eventK`, where `eventK` is the `magic-driver touchpad` node shown by `magic-driver status` in section "Virtual devices" (add `sudo` if your user cannot read `/dev/input`).
6. Note the time, so the journal check in section 10 covers exactly this session: `date '+%F %T'`.

## 1. Slow scroll

1. In each app of the matrix in "Results" (Firefox, a Chromium app such as Brave, Dolphin or Kate), open a long page or document.
2. Rest the hand on the mouse as usual. Put one finger on the front half of the surface and slide it slowly forward and back.
3. Expected: the content moves smoothly with the finger, pixel by pixel, never in wheel steps. Terminal B shows `POINTER_SCROLL_FINGER` events while moving and a final `0.00` value after the lift.
4. Results: column "Slow scroll".

## 2. Flick and glide

1. In each app: flick one finger forward quickly and lift.
2. Expected: the content keeps gliding after the lift and slows down smoothly (toolkit momentum).
3. Results: column "Flick and glide".

## 3. Stop by touch

1. In each app: flick so the content glides, then put one finger on the mouse surface and keep it still.
2. Expected: the glide stops at once. Terminal B shows `GESTURE_HOLD_BEGIN 2` and later `GESTURE_HOLD_END 2`.
3. Lift the finger without moving it. Expected: the content stays where it is, no new glide.
4. Also try putting the finger down and moving it straight away: the glide stops and a new scroll follows the finger.
5. Results: column "Stop by touch".

## 4. Horizontal scroll

1. Open wide content: a wide spreadsheet or table in the browser, or a long line in Kate with word wrap off.
2. Slide one finger sideways (left and right) on the surface.
3. Expected: the content scrolls sideways smoothly; terminal B shows `horiz` values and a final `0.00`.
4. Note whether a forward slide alone also scrolls sideways a little (seen in M0 A-2). Axis lock (`scroll.axis_lock`) should keep this small; note it if it is disturbing.
5. Results: column "Horizontal scroll".

## 5. Click while touching

1. In each app: left click, right click and middle click as you normally do, with the hand resting on the mouse. Expected: exactly the same button as with the stock driver, and no scroll at all, not even a small jump.
2. Start a slow scroll, and while the finger is still moving press to click. Expected: the scroll stops dead at the click, nothing glides after it, and the click is delivered once.
3. After the click, keep the same finger on the surface and move it. Expected: nothing scrolls until that finger has been lifted and put down again.
4. Click and then immediately start a scroll with another finger. Expected: a touch that starts within about 100 ms after the release is ignored; a touch shortly after that scrolls normally.
5. Results: column "Click while touching".

## 6. Moving the mouse with a resting hand

1. Hold the mouse with the palm and gripping fingers resting on it, as in daily use. Move it around for at least a minute over the open apps: small precise moves, large fast moves, lifting and placing it again.
2. Expected: nothing scrolls, terminal B shows no `POINTER_SCROLL_FINGER` and no `GESTURE_HOLD` events while you only move the mouse.
3. Rest the hand still on the mouse for 30 s without moving. Expected: nothing scrolls.
4. Results: column "Resting hand", plus the count of unintended scrolls you saw.

## 7. KDE Touchpad page: scroll speed and natural scrolling

1. Open System Settings, Touchpad, select `magic-driver touchpad`.
2. Change the scrolling speed to a clearly different value and apply. Repeat a slow scroll: the speed follows the setting.
3. Toggle natural scrolling (inverted scroll direction) and apply. Repeat a slow scroll: the direction flips.
4. Check that tap-to-click is off and that two-finger scrolling is the scroll method.
5. Set both settings back to your preference.
6. Results: field "KDE Touchpad page".

## 8. Momentum in the other daily apps (optional)

1. Repeat sections 1 to 3 quickly in Slack, VS Code and Konsole.
2. Results: field "Other apps".

## 9. Stop and fall back

1. Press Ctrl+C in terminal A. Expected: `released ... shutdown; stock behaviour restored`.
2. Move and scroll: the mouse behaves as with the stock driver.

## 10. Journal check (success metric: zero high-resolution scroll warnings)

1. Show libinput and KWin messages since the start time noted in section 0 step 6:

   ```sh
   journalctl --user --since 'YYYY-MM-DD HH:MM:SS' | grep -iE 'libinput|magic-driver'
   journalctl --since 'YYYY-MM-DD HH:MM:SS' _COMM=kwin_wayland | grep -iE 'libinput|magic-driver|high.resolution|wheel|jump'
   ```

2. Expected:
   - no line about high-resolution scroll or wheel events for `magic-driver pointer` or `magic-driver touchpad`;
   - no `Touch jump detected and discarded` for `magic-driver touchpad` (the scroll path limits every synthetic finger step to well under libinput's jump threshold; the M0 jumps came from raw forwarding);
   - the `missing right button, assuming it is a clickpad` line on device add is known from M0 and expected; whether to silence it with `INPUT_PROP_BUTTONPAD` is still an open owner decision.
3. Results: field "Journal", with any unexpected lines copied.

## 11. When something is wrong: record a fixture

Every threshold change has to come with a recording that reproduces the problem, so it becomes a regression test.

1. Stop `magic-driver run` (Ctrl+C) so the stock driver is active.
2. Record the problem: `./target/release/magic-driver record tests/fixtures/magic-mouse-2-bt/<scenario>.rec`, perform the movement that went wrong (for example a palm that scrolled, a scroll that did not start, a click that scrolled), then Ctrl+C. Use the names from `tests/fixtures/README.md`, with a suffix such as `palm-scrolls-1.rec` for a new case.
3. Check it offline: `./target/release/magic-driver replay tests/fixtures/magic-mouse-2-bt/<scenario>.rec`. Lines with ` frame ` are synthetic touchpad output; there should be none for a palm or a click.
4. Describe in "Results" what you did and what went wrong, with the file name.

Try the setting in `~/.config/magic-driver/config.toml` first (`magic-driver config test`, then reload the service). If a new default is needed, it is changed in md-core together with the recording as a fixture test.

The thresholds that matter most, with their current defaults (`magic-driver config defaults` prints all of them):

| Symptom | Key | Default |
|---|---|---|
| palm or thick finger scrolls | `advanced.touch.palm_size_mm` | 30.0 |
| touches near the rear scroll | `advanced.touch.rear_zone` | 0.35 |
| scroll cancels while scrolling, or scrolls while moving the mouse | `advanced.touch.mouse_motion_threshold` | 150.0 |
| scroll starts too late or too easily | `advanced.touch.promote_distance_mm`, `advanced.touch.promote_window_ms` | 1.5, 150 |
| resting finger scrolls, or stop-by-touch lasts too long | `advanced.touch.rest_dwell_ms` | 300 |
| scroll right after a click is ignored, or a click scrolls | `advanced.touch.click_guard_ms` | 100 |
| glide after a click-cancelled scroll | `advanced.scroll.cancel_hold_ms` | 60 |

## Results

| Field | Value |
|---|---|
| Date | pending |
| Commit | pending |
| Plasma, libinput versions | pending |

| App | Slow scroll | Flick and glide | Stop by touch | Horizontal scroll | Click while touching | Resting hand | Notes |
|---|---|---|---|---|---|---|---|
| Firefox (if used) | pending | pending | pending | pending | pending | pending | |
| Chromium app (Brave) | pending | pending | pending | pending | pending | pending | |
| Dolphin | pending | pending | pending | pending | pending | pending | |
| Kate | pending | pending | pending | pending | pending | pending | |

| Field | Value |
|---|---|
| Unintended scrolls while moving the mouse (count, duration) | pending |
| KDE Touchpad page (speed follows, natural scrolling flips, tap-to-click off) | pending |
| Other apps (Slack, VS Code, Konsole) | pending |
| Journal (high-resolution warnings, touch jumps, other lines) | pending |
| Recorded fixtures and what they show | pending |
| Overall (pass, fail, partial) | pending |
