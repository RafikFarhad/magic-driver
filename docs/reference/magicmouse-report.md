# Magic Mouse 2 raw report reference

This document is the reference for `md-core::profile::MAGIC_MOUSE_2_BT` and for the report parser in E1. Everything here is taken from the stock kernel driver unless it is marked as an assumption.

| | |
|---|---|
| Source | `drivers/hid/hid-magicmouse.c` and `drivers/hid/hid-ids.h` |
| Kernel version | Linux `v7.1` tag (the owner runs `7.1.13-2-MANJARO`) |
| Retrieved from | `https://raw.githubusercontent.com/torvalds/linux/v7.1/drivers/hid/hid-magicmouse.c` |
| Checked against | `master` at the time of writing: report `0x12` handling, touch decoding and geometry are identical; `master` only adds a recursion guard for `DOUBLE_REPORT_ID` (`0xf7`) and a null check in `magicmouse_event` |

## HID identity

| Field | Value | Kernel name |
|---|---|---|
| bus | `0x0005` (Bluetooth) | `BUS_BLUETOOTH` |
| vendor | `0x004c` | `BT_VENDOR_ID_APPLE` |
| product | `0x0269` | `USB_DEVICE_ID_APPLE_MAGICMOUSE2` |

Related identities that the kernel driver also binds and v1 does not support: Magic Mouse 2 over USB (vendor `0x05ac`, product `0x0269`), USB-C Magic Mouse (product `0x0323`), first generation Magic Mouse (product `0x030d`, report `0x29`). They are candidates for later profiles (E7).

## Report `0x12`

`magicmouse_raw_event` accepts a report `0x12` only if its size is exactly 8 bytes, or 14 bytes plus 8 bytes per touch. More than 15 touches is rejected with a warning (`npoints > 15`). The 8-byte form carries the header fields up to byte 5 and no touches.

| Constant | Value | Profile field |
|---|---|---|
| report ID | `0x12` | `report.report_id` |
| header length | 14 | `report.header_len` |
| short report length | 8 | `report.short_report_len` |
| touch record length | 8 | `report.touch_len` |
| maximum touches per report | 15 | `report.max_touches` |
| kernel multitouch slots | 16 | not used by md-core |

### Header

| Byte | Content | Kernel expression |
|---|---|---|
| 0 | report ID `0x12` | `data[0]` |
| 1 | buttons: bit 0 left, bit 1 right (the mouse's own guess) | `clicks = data[1]`, used as `clicks & 3` |
| 2-3 | relative X, signed 16-bit little endian | `x = (int)((data[3] << 24) \| (data[2] << 16)) >> 16` |
| 4-5 | relative Y, signed 16-bit little endian | `y = (int)((data[5] << 24) \| (data[4] << 16)) >> 16` |
| 6-10 | unknown, not used by the kernel | |
| 11-13 | device timestamp, 18 bits, unused by the kernel | `ts = data[11] >> 6 \| data[12] << 2 \| data[13] << 10` |

Relative X and Y are reported unchanged as `REL_X` and `REL_Y` on the stock device. The unit of the timestamp is not documented.

### Touch record (8 bytes, `tdata`)

| Field | Bits | Kernel expression |
|---|---|---|
| x | 12-bit signed: low byte `tdata[0]`, high nibble `tdata[1] & 0x0f` | `x = (tdata[1] << 28 \| tdata[0] << 20) >> 20` |
| y | 12-bit signed from `tdata[2]` (high 8 bits) and `tdata[1] >> 4` (low 4 bits), then negated | `y = -((tdata[2] << 24 \| tdata[1] << 16) >> 20)` |
| touch major | `tdata[3]`, 0 to 255 | `touch_major = tdata[3]` |
| touch minor | `tdata[4]`, 0 to 255 | `touch_minor = tdata[4]` |
| size | `tdata[5] & 0x3f`, 0 to 63 | `size = tdata[5] & 0x3f` |
| touch ID | 4 bits: `tdata[5] >> 6` (low 2 bits) and `tdata[6] & 0x03` (high 2 bits) | `id = (tdata[6] << 2 \| tdata[5] >> 6) & 0xf` |
| orientation | `tdata[6] >> 2` minus 32, range -32 to 31 | `orientation = (tdata[6] >> 2) - 32` |
| state | `tdata[7] & 0xf0` | `state = tdata[7] & TOUCH_STATE_MASK` |

The low bits of `tdata[7]` are not decoded by the kernel; its comment says bits `0x03` seem to indicate the aspect ratio of the touch. With `report_undeciphered=1` the kernel forwards the whole `tdata[7]` as `MSC_RAW`.

On the stock device the kernel reports `ABS_MT_TOUCH_MAJOR` and `ABS_MT_TOUCH_MINOR` as the raw value shifted left by 2 (range 0 to 1020), and `ABS_MT_ORIENTATION` as `-orientation`. The profile keeps this factor as `touch_size_scale = 4`.

### Touch states

| State | Kernel meaning | Observed on the owner's mouse (PRD section 10) |
|---|---|---|
| `0x00` | `TOUCH_STATE_NONE`, not down | gone |
| `0x20` | possibly a near-field reading (kernel comment) | near or hovering |
| `0x30` | `TOUCH_STATE_START` | start |
| `0x40` | `TOUCH_STATE_DRAG` | drag |
| `0x50`, `0x60`, `0x70` | not named | lifting |

The kernel treats every state other than `0x00` as down, including `0x20`. magic-driver deliberately differs: hover is never a contact (ARCHITECTURE 4.1 step 2).

## Surface geometry

The kernel declares the touch surface in hundredths of a millimetre and in device units:

| Axis | Min | Max | Span (units) | Size | Kernel resolution (truncated) | Exact units per mm |
|---|---|---|---|---|---|---|
| X | -1100 | 1258 | 2358 | 9056 (90.56 mm) | 26 | 26.038 |
| Y | -1589 | 2047 | 3636 | 5152 (51.52 mm) | 70 | 70.575 |

The kernel computes `MOUSE_RES_X = (MOUSE_MAX_X - MOUSE_MIN_X) / (MOUSE_DIMENSION_X / 100)` in floating point and passes it to `input_abs_set_res`, which truncates to 26 and 70 units per mm. md-core uses the exact ratio (`SurfaceAxis::units_per_mm`) for millimetre conversions. The Y range applies to the negated Y value, which is what the kernel reports as `ABS_MT_POSITION_Y`. md-core uses the same convention: surface coordinates in device units are the kernel's values after negation (`raw_y_inverted = true`).

The kernel comment states the sizes are "not precise, but close enough".

### Axis orientation

| Axis | Direction of increasing values | Evidence |
|---|---|---|
| X | toward the right side of the mouse | `emulate_3button`: a firm touch with `x < -350` is a left click, `x > 350` a right click |
| Y (after negation) | toward the rear of the mouse, where the palm rests | kernel comment: the device's Y is inverted relative to pointer motion, and the driver keeps the origin and negates Y; pointer motion `REL_Y` grows toward the user |

The Y direction is derived from the kernel comment, not measured. It is to be confirmed with the first real fixtures (E2-T10).

## Button emulation (reference for E1-T04)

The kernel decides buttons in `magicmouse_emit_buttons(msc, clicks & 3)`:

- `middle_button_start = -350`, `middle_button_stop = +350`, compared against the touch's X in device units.
- If the new state is 0, all buttons are released.
- Otherwise, if any of left, right or middle was already down, the previous state is kept (the button is latched until release).
- Otherwise, if there is exactly one "firm" touch (`size >= 8`) among the touches of the current report: X below -350 gives left, above +350 gives right, otherwise middle.
- Otherwise the mouse's own guess from byte 1 is used (bit 0 left, bit 1 right).
- Middle is only reported when the `emulate_3button` module parameter is on (default on).

The kernel updates touches before buttons within the same report, so the zone decision uses the touches of the report that carries the click edge.

## Values not taken from the kernel

| Profile field | Value | Status |
|---|---|---|
| `default_palm_size_mm` | 30.0 mm | default for `advanced.touch.palm_size_mm`; a scrolling finger pressed while holding the mouse measured up to about 29 mm, a resting palm about 31 mm (in the rear zone) |
| `default_mouse_motion_threshold` | 150.0 mouse counts per second | indicative default for `advanced.touch.mouse_motion_threshold`, tuned in E3 |
| `id` | `magic-mouse-2-bt` | profile identifier used in recording headers |
