# M0 install: udev rule only

| | |
|---|---|
| Audience | the owner only (ARCHITECTURE 7.5); implementers never run these commands |
| Task | E2-T08 |
| Installs | `/etc/udev/rules.d/70-magic-driver.rules` and nothing else |
| Undo | section "Remove the rule" below |

The M0 prototype runs from a terminal as your normal user. It needs three permissions, granted through `uaccess` to the user of the active local seat session (D7):

- read the Magic Mouse 2 raw device (`/dev/hidrawN`, today `root:root 0600`);
- read and grab the Magic Mouse 2 stock device (`/dev/input/eventN`);
- create virtual devices through `/dev/uinput`.

The rule file also marks the virtual touchpad as an external touchpad (`ID_INPUT_TOUCHPAD_INTEGRATION=external`), so libinput never pairs it with the keyboard for disable-while-typing. It contains no `MODE=` or `GROUP=` keys, so nothing becomes world-readable.

## 1. Build

```sh
cd ~/projects/magic-driver
cargo build --release
```

The build needs the libudev headers, which Manjaro ships in `systemd-libs`.

## 2. Check what is there before

```sh
ls -l /dev/uinput /dev/hidraw*
getfacl /dev/uinput
./target/release/magic-driver status
# hid device:   /sys/devices/virtual/misc/uhid/0005:004C:0269.0013
```

`status` is read-only. With the mouse connected it prints the HID device, the raw device and the stock device, and the access your user has to each. Write down the HID device path (the `hid device:` line), you need it in step 4.

If `/dev/uinput` is missing, load the module once for this boot (from M1 on the installer adds `/etc/modules-load.d/magic-driver.conf`):

```sh
sudo modprobe uinput
```

## 3. Install the rule

```sh
sudo install -m 0644 -o root -g root dist/70-magic-driver.rules /etc/udev/rules.d/70-magic-driver.rules
sudo udevadm control --reload
```

## 4. Apply it to the affected nodes only

Replace `<hid device>` with the `hid device:` path from step 2, for example `/sys/devices/.../0005:004C:0269.0006`.

```sh
sudo udevadm trigger --action=change --parent-match=<hid device>
sudo udevadm trigger --action=change /sys/devices/virtual/misc/uinput
```

`--parent-match` re-triggers the HID device and its children (the hidraw node and the stock input nodes) and nothing else. Turning the mouse off and on again also applies the rule, with no trigger at all.

The virtual touchpad line needs no trigger: it applies whenever `magic-driver run` creates the touchpad.

## 5. Verify

```sh
./target/release/magic-driver status
getfacl /dev/hidrawN
getfacl /dev/input/eventN
getfacl /dev/uinput
```

Expected: `status` shows `read-write` for the raw device and the stock device, and `getfacl` lists `user:<you>:rw-` on all three nodes. If the raw device still shows `no access for this user`, turn the mouse off and on and check again.

## Remove the rule

```sh
sudo rm /etc/udev/rules.d/70-magic-driver.rules
sudo udevadm control --reload
sudo udevadm trigger --action=change --parent-match=<hid device>
sudo udevadm trigger --action=change /sys/devices/virtual/misc/uinput
```

After this `getfacl /dev/hidrawN` no longer lists your user. `/dev/uinput` may still carry your ACL from KDE Connect's own `40-kdeconnect-uinput.rules`, which was there before magic-driver. If you ran `sudo modprobe uinput` in step 2 the module stays loaded until the next boot; that is harmless.

## What the rule does not change

- The stock driver `hid_magicmouse`, its parameters in `/etc/modprobe.d/hid_magicmouse.conf`, and its bindings.
- Any other input device, including keyboards and keyd.
- Group memberships.
