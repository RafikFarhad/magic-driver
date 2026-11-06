PKG_DIR := target/arch-package
SOURCE ?= file://$(CURDIR)
REPO_URL ?= https://github.com/RafikFarhad/magic-driver.git
BRANCH ?= master
HASH := \#

.DEFAULT_GOAL := help

.PHONY: help build debug test check fmt lint run doctor status package install install-git enable disable logs clean

help:
	@echo "make build     release build of magic-driver and magic-driver-inspector"
	@echo "make debug     debug build"
	@echo "make test      cargo test for the whole workspace"
	@echo "make check     full gate: fmt, clippy, tests, dependency checks, shellcheck"
	@echo "make fmt       format the code"
	@echo "make lint      clippy with warnings as errors"
	@echo "make run       release build, then run the driver in this terminal (Ctrl+C restores the stock mouse)"
	@echo "make doctor    read-only environment check"
	@echo "make status    read-only device and service state"
	@echo "make package   build the Arch package from the last local commit into $(PKG_DIR)"
	@echo "make install   build and install the Arch package from the last local commit (asks for sudo)"
	@echo "make install-git  build and install from GitHub, BRANCH=<name> for another branch (default master)"
	@echo "make enable    enable and start the user service"
	@echo "make disable   stop and disable the user service"
	@echo "make logs      follow the service log"
	@echo "make clean     remove build output"
	@echo ""
	@echo "SOURCE=<git url> builds the package from another repository, default $(SOURCE)"

build:
	cargo build --release --locked -p magic-driver -p magic-driver-inspector

debug:
	cargo build --locked -p magic-driver -p magic-driver-inspector

test:
	cargo test --locked --workspace

check:
	scripts/check.sh

fmt:
	cargo fmt --all

lint:
	cargo clippy --locked --workspace --all-targets -- -D warnings

run: build
	target/release/magic-driver run

doctor: build
	target/release/magic-driver doctor

status: build
	target/release/magic-driver status

$(PKG_DIR)/PKGBUILD: packaging/arch/PKGBUILD packaging/arch/magic-driver.install
	mkdir -p $(PKG_DIR)
	cp packaging/arch/PKGBUILD packaging/arch/magic-driver.install $(PKG_DIR)/

package: $(PKG_DIR)/PKGBUILD
	cd $(PKG_DIR) && MAGIC_DRIVER_SOURCE="$(SOURCE)" makepkg -f --cleanbuild

install: $(PKG_DIR)/PKGBUILD
	cd $(PKG_DIR) && MAGIC_DRIVER_SOURCE="$(SOURCE)" makepkg -fsi --cleanbuild

install-git: $(PKG_DIR)/PKGBUILD
	cd $(PKG_DIR) && MAGIC_DRIVER_SOURCE="$(REPO_URL)$(HASH)branch=$(BRANCH)" makepkg -fsi --cleanbuild

enable:
	systemctl --user enable --now magic-driver

disable:
	systemctl --user disable --now magic-driver

logs:
	journalctl --user -u magic-driver -f

clean:
	cargo clean
