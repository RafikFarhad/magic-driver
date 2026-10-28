#!/usr/bin/env bash
set -euo pipefail

destdir=${DESTDIR:-}
dry_run=0
purge=0

usage() {
    cat <<EOF
Usage: scripts/uninstall.sh [--dry-run] [--purge]

Stops and disables the magic-driver user service, removes every file listed
in \$XDG_DATA_HOME/magic-driver/install-manifest (with sudo for system
files, including magic-driver-inspector when it was installed with
--with-inspector), reloads udev and re-triggers the Magic Mouse 2 and uinput nodes.
The user config in \$XDG_CONFIG_HOME/magic-driver/ is kept unless --purge
is given.

Options:
  --dry-run   print every action without performing it
  --purge     also remove the user config
  -h, --help  show this help

DESTDIR, when set, is prefixed to every file path and skips the udev and
systemd commands, for staged installs and tests.
EOF
}

die() {
    printf 'uninstall.sh: %s\n' "$*" >&2
    exit 1
}

note() {
    printf '%s\n' "$*"
}

run() {
    if ((dry_run)); then
        printf 'dry-run: %s\n' "$*"
    else
        "$@"
    fi
}

system_command() {
    if [[ -n $destdir ]]; then
        printf 'skip (DESTDIR is set): %s\n' "$*"
    else
        run "$@"
    fi
}

while (($# > 0)); do
    case $1 in
        --dry-run) dry_run=1 ;;
        --purge) purge=1 ;;
        -h | --help)
            usage
            exit 0
            ;;
        *) die "unknown argument: $1 (see --help)" ;;
    esac
    shift
done

if ((EUID == 0)); then
    die "run this as your normal user, not as root; it calls sudo only where needed"
fi
[[ -n ${HOME:-} ]] || die "HOME is not set"

config_home=${XDG_CONFIG_HOME:-$HOME/.config}
data_home=${XDG_DATA_HOME:-$HOME/.local/share}

bin_path=/usr/local/bin/magic-driver
inspector_path=/usr/local/bin/magic-driver-inspector
rule_path=/etc/udev/rules.d/70-magic-driver.rules
modules_path=/etc/modules-load.d/magic-driver.conf
unit_path=$config_home/systemd/user/magic-driver.service
config_dir=$config_home/magic-driver
config_path=$config_dir/config.toml
manifest_path=$data_home/magic-driver/install-manifest

[[ -f $destdir$manifest_path ]] || die "no install manifest at $destdir$manifest_path, nothing to uninstall"

entries=()
while IFS= read -r line || [[ -n $line ]]; do
    [[ -n $line ]] || continue
    kind=${line%% *}
    path=${line#* }
    case "$kind:$path" in
        "system:$bin_path" | "system:$inspector_path" | "system:$rule_path" | "system:$modules_path" | "user:$unit_path") ;;
        *) die "refusing to act on unexpected manifest entry: $line" ;;
    esac
    entries+=("$kind $path")
done <"$destdir$manifest_path"

if [[ -n $destdir ]]; then
    printf 'skip (DESTDIR is set): systemctl --user disable --now magic-driver.service\n'
elif ((dry_run)); then
    printf 'dry-run: systemctl --user disable --now magic-driver.service\n'
else
    systemctl --user disable --now magic-driver.service || note "note: the service was not enabled or not loaded"
fi

for entry in "${entries[@]}"; do
    kind=${entry%% *}
    path=${entry#* }
    note "remove: $path"
    if [[ $kind == system ]]; then
        run sudo rm -f -- "$destdir$path"
    else
        run rm -f -- "$destdir$path"
    fi
done

system_command systemctl --user daemon-reload
system_command sudo udevadm control --reload
for hid in /sys/bus/hid/devices/0005:004C:0269.*; do
    [[ -e $hid ]] || continue
    system_command sudo udevadm trigger --action=change "--parent-match=$(readlink -f "$hid")"
done
if [[ -e /sys/devices/virtual/misc/uinput ]]; then
    system_command sudo udevadm trigger --action=change /sys/devices/virtual/misc/uinput
fi

if ((purge)); then
    note "remove: $config_path"
    run rm -f -- "$destdir$config_path"
    if [[ -d $destdir$config_dir ]]; then
        run rmdir --ignore-fail-on-non-empty -- "$destdir$config_dir"
    fi
else
    note "keep: $config_path (use --purge to remove it)"
fi

note "remove: $manifest_path"
run rm -f -- "$destdir$manifest_path"
run rmdir --ignore-fail-on-non-empty -- "$(dirname "$destdir$manifest_path")"

if ((dry_run)); then
    note "dry run finished, nothing was changed"
else
    note "magic-driver uninstalled; the mouse uses the stock driver"
fi
