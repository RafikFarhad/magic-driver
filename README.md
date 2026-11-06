# magic-driver

A user-space driver for the Apple Magic Mouse 2 (Bluetooth) on Linux with KDE Plasma 6 on Wayland. It reads the mouse's raw multi-touch reports, filters out palms and resting fingers, and drives a virtual pointer and a virtual touchpad, so scrolling, momentum and gestures behave like a touchpad.

## Status

Early, personal project. It is tested on one machine with one mouse.

## Install

See `docs/install.md`. The optional diagnostic window is described in `docs/inspector.md`.

## License

magic-driver is free software: you can redistribute it and/or modify it under the terms of the GNU General Public License as published by the Free Software Foundation, either version 3 of the License, or (at your option) any later version. See `LICENSE`.

The report format comes from the Linux kernel driver `drivers/hid/hid-magicmouse.c` (GPL-2.0-or-later), whose layout is described in `docs/reference/magicmouse-report.md`.

The inspector bundles no fonts. It uses the fonts already installed on the system through fontconfig.

## Trademarks

This project is not affiliated with, endorsed by or sponsored by Apple Inc. Apple and Magic Mouse are trademarks of Apple Inc., registered in the U.S. and other countries. The names are used only to say which device this software works with.
