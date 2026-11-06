# magic-driver inspector

| | |
|---|---|
| Binary | `magic-driver-inspector` (crate `crates/magic-driver-inspector`), optional, installed with `scripts/install.sh --with-inspector` |
| Launcher | `magic-driver inspect [--replay <file>]` runs `magic-driver-inspector` from the directory of `magic-driver` or else from `PATH`, or prints how to install it |

A read-only diagnostic window. It never writes to the trace socket, never changes the config of the running daemon and never opens an input device.

## Modes

| Command | Source |
|---|---|
| `magic-driver inspect --replay rec.txt` | offline: loads a recording (`magic-driver record`), runs `md-core` in-process and plays the trace it produces; no daemon and no device needed |
| `magic-driver inspect` | live: subscribes to `$XDG_RUNTIME_DIR/magic-driver/trace.sock` of the running daemon (owner only) |
| `magic-driver-inspector --replay rec.txt --config other.toml` | offline with another config file; without `--config` the user config `~/.config/magic-driver/config.toml` is used if it exists, else the defaults |
| `magic-driver-inspector --socket PATH` | live from another socket path |

## Window

- Controls: play or pause, step back and step (one report), speed (0.1x to 8x), the time slider. Live mode follows the newest record; pausing or moving the slider scrubs through the rolling buffer (the last 120 s, at most 400,000 records) and `go live` returns to the newest record.
- Offline only: palm rejection (`low`, `medium`, `high`) and acceleration (`off` to `high`) buttons rerun the engine on the whole recording with that preset; `reset config` returns to the loaded config.
- Live only: the connection line (daemon version, profile, session status from the hello record), the dropped-record counter (total and since the last connection) and the connection count. When the daemon restarts the window reconnects on its own and starts a new buffer.
- Raw panel: the touch surface seen from above (front at the top, rear zone shaded), one dot per touch sized by the contact and coloured by the touch state (hover, start, touching, lift), with a ring for the class (green deliberate, red rejected, grey candidate); buttons; clicks, dx and dy; reports per second; per-report processing time (live); a table of the touches with sizes over the palm threshold in red, class and reason; the thresholds from the config.
- Decisions and output panel: the recogniser state and a 5 s state timeline; the last decisions with their reasons (palm, rear zone, mouse moving, click guard, resting, promoted); the synthetic touchpad fingers around the virtual touchpad centre; the pointer motion and button events.
- Received panel: what this window itself receives from KWin while the pointer is over it: one row per scroll frame (source, value per axis, discrete, value120, inverted, stop) and per gesture event (swipe, pinch, hold), and a graph of the last 5 s (vertical blue, horizontal orange, stops as red lines). In live mode each row also shows the trace time it lines up with, so a scroll frame can be matched with the report that caused it.

KWin consumes the swipes it binds to its own actions (desktop switching, Overview), so those never reach the Received panel. Hold gestures and swipes with a finger count KWin does not bind do.
