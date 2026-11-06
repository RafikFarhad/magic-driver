# magic-driver

A user-space driver for the Apple Magic Mouse 2 (Bluetooth) on Linux with KDE Plasma 6 on Wayland. It reads the mouse's raw multi-touch reports, filters out palms and resting fingers, and drives a virtual pointer and a virtual touchpad, so scrolling, momentum and gestures behave like a touchpad.

## Status

Early, personal project. It is tested on one machine with one mouse.

## Install

```sh
git clone https://github.com/RafikFarhad/magic-driver.git
cd magic-driver
```

On Arch Linux, `make install` builds and installs the package from your last local commit, and `make enable` turns the service on (`make help` lists every target). You can also build the package from [`packaging/arch/PKGBUILD`](https://github.com/RafikFarhad/magic-driver/blob/master/packaging/arch/PKGBUILD) (see [Arch Linux package](https://github.com/RafikFarhad/magic-driver/blob/master/docs/install.md#0-arch-linux-package) in the install guide). Anywhere else, use [`scripts/install.sh`](https://github.com/RafikFarhad/magic-driver/blob/master/scripts/install.sh) as described in [`docs/install.md`](https://github.com/RafikFarhad/magic-driver/blob/master/docs/install.md). The optional diagnostic window is described in [`docs/inspector.md`](https://github.com/RafikFarhad/magic-driver/blob/master/docs/inspector.md).

## Contributing

Read [`AGENTS.md`](https://github.com/RafikFarhad/magic-driver/blob/master/AGENTS.md) first, whether you are a person or an agent. It explains the layout, the commands, and the safety rules that keep your own mouse and desktop out of the test suite. Issues and pull requests go to [RafikFarhad/magic-driver](https://github.com/RafikFarhad/magic-driver).

## License

magic-driver is free software: you can redistribute it and/or modify it under the terms of the GNU General Public License as published by the Free Software Foundation, either version 3 of the License, or (at your option) any later version. See [`LICENSE`](https://github.com/RafikFarhad/magic-driver/blob/master/LICENSE).

The report format comes from the Linux kernel driver `drivers/hid/hid-magicmouse.c` (GPL-2.0-or-later), whose layout is described in [`docs/reference/magicmouse-report.md`](https://github.com/RafikFarhad/magic-driver/blob/master/docs/reference/magicmouse-report.md).

The inspector bundles no fonts. It uses the fonts already installed on the system through fontconfig.

## Trademarks

This project is not affiliated with, endorsed by or sponsored by Apple Inc. Apple and Magic Mouse are trademarks of Apple Inc., registered in the U.S. and other countries. The names are used only to say which device this software works with.

## Vibe check

This driver was vibe-coded from scratch. It works on exactly one mouse, on one desk, under one very specific mood, and reading the source is not recommended for anyone who wants to keep their weekend.

Contributions are welcome. Bring your friendly neighbourhood LLM, or write it bare-handed if you are feeling brave. The review process is rigorous: my LLM reads it, says it looks fine, and then I scroll for a bit. If my LLM approves and my mouse still scrolls, it gets merged. If it does not, we will blame the palm rejection.
