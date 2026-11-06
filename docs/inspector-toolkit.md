# Inspector toolkit decision

| | |
|---|---|
| Question | can eframe (egui) give the Received panel axis source, axis value, `axis_stop`, discrete steps and pointer-gesture hold, swipe and pinch events, or does the panel need smithay-client-toolkit directly? |
| Spikes | `spikes/eframe-events`, `spikes/sctk-events` (own Cargo workspace, excluded from the main one) |
| Status | spikes built by the implementer; running them needs the real mouse through magic-driver and a desktop session, so the owner runs them (`docs/m3-checklist.md` section 1, waiting for the owner) |

## Spikes

Both are throwaway programs that open one window and print every scroll and gesture event they get, with a millisecond timestamp, to stdout.

| Spike | Stack | Prints |
|---|---|---|
| `eframe-events` | eframe 0.37 (glow, winit 0.30 Wayland backend) | every `egui::Event::MouseWheel` (unit, delta, phase, source, modifiers), `Zoom`, `Rotate`, `Touch`, `PointerGone` |
| `sctk-events` | smithay-client-toolkit 0.20 window on wl_shm, plus `zwp_pointer_gestures_v1` from wayland-protocols 0.32 | the bound pointer-gestures, `wl_seat` and `wl_pointer` versions; every `wl_pointer` frame as one axis line (time, source, per axis value, discrete, value120, stop, relative direction); every swipe, pinch and hold event |

Build (implementer and owner):

```sh
cd spikes
cargo build --release
```

Run (owner only, in the Plasma Wayland session):

```sh
cd spikes
cargo run --release -p eframe-events
cargo run --release -p sctk-events
```

## What each candidate exposes

From the spike code and the sources of the exact versions in the lockfiles (winit 0.30.13, egui-winit 0.37.0, egui 0.37.0, smithay-client-toolkit 0.20.0, wayland-protocols 0.32.13). The owner's spike run in `docs/m3-checklist.md` section 1 confirms it on the desktop.

| Wayland detail | eframe 0.37 (winit 0.30 on Wayland) | smithay-client-toolkit 0.20 |
|---|---|---|
| axis source (`wheel`, `finger`, `continuous`, `wheel tilt`) | no: winit drops it; `MouseWheelSource` is `Unknown` on Linux | yes, per frame |
| axis value | partly: `MouseWheelUnit::Point` delta, sign inverted and divided by the scale; replaced by the line count when discrete steps are present | yes, per axis, unchanged |
| `axis_stop` | partly: `TouchPhase::End` when either axis stops; which axis stopped is lost | yes, per axis |
| discrete steps | partly: `MouseWheelUnit::Line` from `axis_discrete`; winit's sctk 0.19 binds `wl_seat` at most version 7, so `axis_value120` never arrives | yes: `axis_discrete` and, with `wl_seat` 8 or later, `axis_value120` |
| relative direction (natural scrolling, `wl_pointer` 9) | no | yes |
| event timestamps | no | yes (`time` in ms) |
| pointer-gesture swipe | no: winit has gesture events on macOS and iOS only | not wrapped by sctk; available through `zwp_pointer_gestures_v1` from wayland-protocols on the same connection |
| pointer-gesture pinch | no (egui `Zoom` comes only from Ctrl+wheel or macOS pinch) | same, through wayland-protocols, with scale and rotation |
| pointer-gesture hold | no | same, through wayland-protocols, with `zwp_pointer_gestures_v1` version 3 |

eframe cannot feed the Received panel: it loses the source, the per-axis stop, value120 and every gesture, which are exactly the details the panel exists to show.

## Chosen combination

- eframe (glow renderer, Wayland and X11 backends) draws the whole window and all three panels.
- The Received panel uses smithay-client-toolkit 0.20 directly: `SeatState` and `PointerHandler` on a second event queue on eframe's own Wayland connection (`Backend::from_foreign_display` on winit's `wl_display`). It creates its own `wl_seat` and `wl_pointer`. The compositor sends pointer events to every `wl_pointer` the client holds for that seat, so this pointer sees the same scroll frames as winit's, for the inspector's own surface, with every detail intact.
- Pointer gestures come from wayland-protocols `zwp_pointer_gestures_v1` (swipe, pinch, hold) on the same queue, because sctk has no wrapper for them.
- The inspector dispatches that queue once per egui frame; winit reads the socket as usual, so no extra thread or event loop is needed.
- Under X11 or XWayland the Received panel shows that it has no data; Raw and Decisions still work.

Cost: the inspector's dependency tree carries two sctk versions, 0.19 through winit and 0.20 for the panel. Neither reaches the daemon crate; `scripts/check.sh` fails if the `magic-driver` crate ever depends on a GUI, Wayland or graphics crate.
