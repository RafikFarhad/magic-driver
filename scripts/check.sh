#!/usr/bin/env bash
set -euo pipefail

cd "$(dirname "${BASH_SOURCE[0]}")/.."

forbidden_core_crates=(libc nix rustix evdev evdev-rs evdev-sys udev libudev libudev-sys mio tokio async-std chrono time)

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

step "cargo fmt --check"
cargo fmt --all --check

step "cargo clippy"
cargo clippy --locked --workspace --all-targets -- -D warnings

step "cargo test"
cargo test --locked --workspace

step "md-core dependency check"
check_core_dependencies

printf '\nAll checks passed.\n'
