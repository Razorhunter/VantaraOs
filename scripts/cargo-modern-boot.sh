#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
REAL_CARGO="${VANTARA_REAL_CARGO:-/usr/local/cargo/bin/cargo}"

if [[ "${1:-}" == "install" ]]; then
  # bootloader 0.11 builds its BIOS/UEFI firmware through nested `cargo
  # install` calls. Sharing CARGO_HOME with the parent Cargo process causes a
  # package-cache flock deadlock, so nested installs get an isolated cache.
  export CARGO_HOME="${ROOT_DIR}/target/modern-boot-cargo-home"
  export CARGO_TARGET_DIR="${ROOT_DIR}/target/modern-boot-firmware-target"
  mkdir -p "${CARGO_HOME}"
  mkdir -p "${CARGO_TARGET_DIR}"
fi

exec "${REAL_CARGO}" "$@"
