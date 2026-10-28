#!/usr/bin/env bash
set -euo pipefail

cd "$(dirname "${BASH_SOURCE[0]}")/.."

forbidden_core_crates=(libc nix rustix evdev evdev-rs evdev-sys udev libudev libudev-sys mio tokio async-std chrono time)
forbidden_daemon_crates=(magic-driver-inspector eframe egui egui-winit egui_glow egui-wgpu epaint emath ecolor winit glow glutin glutin-winit wgpu wgpu-core wgpu-hal naga raw-window-handle wayland-client wayland-backend wayland-sys wayland-protocols wayland-cursor smithay-client-toolkit x11rb x11-dl xkbcommon-dl gtk gtk4 qt)

step() {
    printf '\n==> %s\n' "$*"
}

check_core_dependencies() {
    local tree found=()
    tree=$(cargo tree --locked -p md-core -e normal --prefix none --format '{p}' | awk '{print $1}' | sort -u)
    for crate in "${forbidden_core_crates[@]}"; do
        if grep -qx -- "$crate" <<<"$tree"; then
            found+=("$crate")
        fi
    done
    if ((${#found[@]} > 0)); then
        printf 'md-core depends on forbidden crates: %s\n' "${found[*]}" >&2
        cargo tree --locked -p md-core -e normal >&2
        return 1
    fi
    printf 'md-core has no Linux, I/O or clock dependencies\n'
}

check_daemon_dependencies() {
    local tree found=()
    tree=$(cargo tree --locked -p magic-driver -e normal,build --prefix none --format '{p}' | awk '{print $1}' | sort -u)
    for crate in "${forbidden_daemon_crates[@]}"; do
        if grep -qx -- "$crate" <<<"$tree"; then
            found+=("$crate")
        fi
    done
    if ((${#found[@]} > 0)); then
        printf 'the magic-driver daemon depends on GUI or graphics crates: %s\n' "${found[*]}" >&2
        cargo tree --locked -p magic-driver -e normal,build -i "${found[0]}" >&2
        return 1
    fi
    printf 'the magic-driver daemon has no GUI or graphics dependencies\n'
}

check_shell_scripts() {
    if ! command -v shellcheck >/dev/null 2>&1; then
        printf 'shellcheck is not installed, skipped (CI installs it)\n'
        return 0
    fi
    shellcheck scripts/*.sh
    printf 'shell scripts are shellcheck clean\n'
}

step "cargo fmt --check"
cargo fmt --all --check

step "cargo clippy"
cargo clippy --locked --workspace --all-targets -- -D warnings

step "cargo test"
cargo test --locked --workspace

step "md-core dependency check"
check_core_dependencies

step "magic-driver dependency check"
check_daemon_dependencies

step "shellcheck"
check_shell_scripts

printf '\nAll checks passed.\n'
