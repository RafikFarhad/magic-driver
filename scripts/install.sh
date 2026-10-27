#!/usr/bin/env bash
set -euo pipefail

repo_root=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
destdir=${DESTDIR:-}
dry_run=0
binary=""

usage() {
    cat <<EOF
Usage: scripts/install.sh [--dry-run] [--binary PATH]

Installs magic-driver for the current user:
  /usr/local/bin/magic-driver                    (sudo)
  /etc/udev/rules.d/70-magic-driver.rules        (sudo)
  /etc/modules-load.d/magic-driver.conf          (sudo)
  \$XDG_CONFIG_HOME/systemd/user/magic-driver.service
  \$XDG_CONFIG_HOME/magic-driver/config.toml      (only if missing)
then reloads udev, re-triggers only the Magic Mouse 2 and uinput nodes,
and enables and starts the user service. Every created file is written to
the manifest \$XDG_DATA_HOME/magic-driver/install-manifest.

Options:
  --dry-run      print every action without performing it
  --binary PATH  install a prebuilt binary instead of running cargo build
  -h, --help     show this help

DESTDIR, when set, is prefixed to every file path and skips the udev and
systemd commands, for staged installs and tests.
EOF
}

die() {
    printf 'install.sh: %s\n' "$*" >&2
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
        --binary)
            (($# >= 2)) || die "--binary needs a path"
            binary=$2
            shift
            ;;
        --binary=*) binary=${1#--binary=} ;;
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
rule_path=/etc/udev/rules.d/70-magic-driver.rules
modules_path=/etc/modules-load.d/magic-driver.conf
unit_path=$config_home/systemd/user/magic-driver.service
config_path=$config_home/magic-driver/config.toml
manifest_path=$data_home/magic-driver/install-manifest

manifest=()

install_system_file() {
    local mode=$1 source=$2 target=$3
    note "install: $target"
    run sudo install -D -m "$mode" -o root -g root "$source" "$destdir$target"
    manifest+=("system $target")
}

install_user_file() {
    local mode=$1 source=$2 target=$3
    note "install: $target"
    run install -D -m "$mode" "$source" "$destdir$target"
    manifest+=("user $target")
}

if [[ -z $binary ]]; then
    note "build: cargo build --release --locked"
    run cargo build --release --locked --manifest-path "$repo_root/Cargo.toml"
    binary=$repo_root/target/release/magic-driver
fi
if ((!dry_run)) && [[ ! -x $binary ]]; then
    die "binary $binary does not exist or is not executable"
fi

install_system_file 0755 "$binary" "$bin_path"
install_system_file 0644 "$repo_root/dist/70-magic-driver.rules" "$rule_path"
install_system_file 0644 "$repo_root/dist/modules-load.d/magic-driver.conf" "$modules_path"

system_command sudo udevadm control --reload
found_mouse=0
for hid in /sys/bus/hid/devices/0005:004C:0269.*; do
    [[ -e $hid ]] || continue
    found_mouse=1
    system_command sudo udevadm trigger --action=change "--parent-match=$(readlink -f "$hid")"
done
if ((!found_mouse)); then
    note "note: no Magic Mouse 2 connected; the rule applies when it connects"
fi
if [[ -e /sys/devices/virtual/misc/uinput ]]; then
    system_command sudo udevadm trigger --action=change /sys/devices/virtual/misc/uinput
fi

install_user_file 0644 "$repo_root/dist/magic-driver.service" "$unit_path"

if [[ -e $destdir$config_path ]]; then
    note "keep: $config_path already exists"
else
    note "config: $config_path"
    run install -D -m 0644 "$repo_root/dist/config.example.toml" "$destdir$config_path"
fi

note "manifest: $manifest_path"
for entry in "${manifest[@]}"; do
    note "manifest entry: $entry"
done
if ((!dry_run)); then
    mkdir -p "$(dirname "$destdir$manifest_path")"
    printf '%s\n' "${manifest[@]}" >"$destdir$manifest_path"
fi

system_command systemctl --user daemon-reload
system_command systemctl --user enable --now magic-driver.service

if ((dry_run)); then
    note "dry run finished, nothing was changed"
else
    note "magic-driver installed; check it with: magic-driver status && magic-driver doctor"
fi
