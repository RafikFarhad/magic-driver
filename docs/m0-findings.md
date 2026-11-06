# M0 findings

| | |
|---|---|
| Status | filled from the owner's results; fixtures and some A-6 details deferred to the inspector |
| Filled by | the owner, following `docs/m0-checklist.md` |
| Signed off | owner, go |

Replace every `pending` value with the observed result. Keep excerpts short.

## Environment

| Field | Value |
|---|---|
| Date | 2026-10-26 |
| Kernel (`uname -r`) | 7.1.13-2-MANJARO |
| Plasma version | plasmashell 6.7.4 |
| KWin version | 6.7.4 |
| libinput version | 1.31.3 |
| magic-driver commit | f0167d2 |
| `hid_magicmouse` parameters in `/etc/modprobe.d/hid_magicmouse.conf` | not recorded |
| Bluetooth adapter | TP-Link UB500 |

## Start and stop

| Field | Value |
|---|---|
| Start and stop | pass (owner reported working as described) |
| Stop procedure | pass (owner reported working as described) |

## A-1 Device classification and KDE settings

| Field | Value |
|---|---|
| A-1 list-devices output | not recorded |
| A-1 udev properties of the touchpad | not recorded |
| A-1 KDE lists touchpad | pass (owner reported working as described) |
| A-1 KDE lists pointer | pass (owner reported working as described) |
| A-1 setting kept across restart | pass (owner reported working as described) |
| A-1 setting kept across reconnect | pass after the udev rule was reinstalled; the first failure was caused by the rule missing from `/etc/udev/rules.d` |
| A-1 result | pass |

## A-2 Scroll source and axis_stop

| Field | Value |
|---|---|
| A-2 libinput excerpt | `POINTER_SCROLL_FINGER ... vert 2.37/0.0* horiz -1.31/0.0* (finger)` while moving, ending with `vert 0.00/0.0* horiz 0.00/0.0* (finger)` after the lift |
| A-2 wev excerpt | not recorded |
| A-2 horizontal | yes, `horiz` values up to -4.21 with a final `0.00/0.0*` scroll stop |
| A-2 result | pass; a forward slide also produced a large horizontal component, to verify with the inspector |

## A-3, A-4, A-5 Per-app momentum matrix

| App | Momentum after flick (A-3) | Hold stops momentum (A-4) | Hold then lift, no momentum (A-5) | Notes |
|---|---|---|---|---|
| Firefox | yes | yes | yes | |
| Chromium app (Brave) | yes | yes | yes | |
| Slack | yes | yes | yes | |
| VS Code | yes | yes | yes | |
| Dolphin | yes | yes | yes | |
| Kate | yes | yes | yes | |
| Konsole | yes | yes | yes | |

| Field | Value |
|---|---|
| A-4 hold events seen | yes, GESTURE_HOLD_BEGIN 2 and GESTURE_HOLD_END 2 in libinput debug-events |
| A-3 result | pass |
| A-4 result | pass |
| A-5 result | pass |

## A-6 KWin swipe gestures

| Field | Value |
|---|---|
| A-6 Plasma version | 6.7.4 |
| A-6 finger count for desktop switch | not recorded; KWin switched desktop from a raw-forward 3 and 4 finger swipe |
| A-6 finger count for overview | not recorded |
| A-6 triggered by synthetic swipe | yes, libinput reports `GESTURE_SWIPE_BEGIN 3` and `4` from raw-forward and KWin switched desktop |
| A-6 fast swipe | deferred to the gesture work |
| `advanced.gestures.kwin_swipe_fingers` to use | keep the default 4 until the gesture work confirms it |
| A-6 result | pass (KWin reacts to synthetic swipes), finger counts to confirm in the gesture work |

## A-7 Browser back and forward

| Field | Value |
|---|---|
| A-7 Firefox back | not tested, the owner does not use Firefox |
| A-7 Firefox forward | not tested, the owner does not use Firefox |
| A-7 Chromium back | yes (Brave) |
| A-7 Chromium forward | yes (Brave) |
| A-7 notes | Brave back and forward work |

## A-8 hidraw while grabbed, cross-check with the stock device

| Field | Value |
|---|---|
| A-8 report counts | hidraw reports 1156, stock frames 1154 |
| A-8 cross-check line | `hidraw reports=1156 dx=200 dy=336 left=17/17 right=17/17 middle=0/0 \| stock frames=1154 dx=200 dy=336 left=17/17 right=17/17 middle=0/0 \| motion match \| buttons match` |
| A-8 differences | none in motion or buttons; 2 more hidraw reports than stock frames |
| A-8 result | pass |

## A-9 uaccess timing on reconnect

| Field | Value |
|---|---|
| A-9 reconnect times | about 1 s from mouse on to takeover |
| A-9 permission retries seen | yes, but only while the udev rule was missing from `/etc/udev/rules.d`; after reinstalling the rule the reconnect worked |
| A-9 result | pass, with the udev rule installed |

## A-10 Geometry side effects

| Field | Value |
|---|---|
| A-10 warnings | on device add: `kernel bug: missing right button, assuming it is a clickpad` (device shows `click-buttonareas-clickfinger`); in raw-forward mode: 5 x `kernel bug: Touch jump detected and discarded`, then rate limited |
| A-10 unexpected events | in raw-forward mode single touches move the pointer (`POINTER_MOTION`, one jump of +193 units), expected for unfiltered forwarding |
| A-10 result | partial: clickpad warning on device add, touch jumps in raw-forward mode |

## Fixtures recorded

| Scenario file | Recorded | Replays offline |
|---|---|---|
| `idle-resting-palm.rec` | deferred to the inspector | deferred to the inspector |
| `move-fingers-resting.rec` | deferred to the inspector | deferred to the inspector |
| `slow-scroll.rec` | deferred to the inspector | deferred to the inspector |
| `flick.rec` | deferred to the inspector | deferred to the inspector |
| `horizontal-scroll.rec` | deferred to the inspector | deferred to the inspector |
| `click-left.rec` | deferred to the inspector | deferred to the inspector |
| `click-right.rec` | deferred to the inspector | deferred to the inspector |
| `click-middle.rec` | deferred to the inspector | deferred to the inspector |
| `click-while-touching.rec` | deferred to the inspector | deferred to the inspector |
| `two-finger-swipe.rec` | deferred to the inspector | deferred to the inspector |
| `two-finger-double-tap.rec` | deferred to the inspector | deferred to the inspector |
| `connect-first-touch.rec` | deferred to the inspector | deferred to the inspector |
| real `magic-mouse-2-bt.udev` capture handed over | deferred to the inspector | |

## Decision

| Field | Value |
|---|---|
| Decision (go, adjust, stop) | go |
| A-2 or A-3 failed broadly | no |
| Fallback decision (only if A-2 or A-3 failed broadly) | not needed |
| Design adjustments | open: whether to add `INPUT_PROP_BUTTONPAD` to the virtual touchpad to silence the clickpad warning; none decided |
| Signed off by and date | Farhad, 2026-10-26 |
