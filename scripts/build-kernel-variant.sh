#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
CARGO_TARGET_DIR_ARG="${1:?cargo target dir required}"
shift

TARGET_SPEC="${ROOT_DIR}/target/generated/x86_64-vantara_os.json"
CARGO_PATH="/usr/local/cargo/bin:${PATH}"

bash "${ROOT_DIR}/scripts/generate-rust-target.sh" "${TARGET_SPEC}"
(
  cd "${ROOT_DIR}/kernel"
  PATH="${CARGO_PATH}" cargo fetch --target "${TARGET_SPEC}"
)
bash "${ROOT_DIR}/scripts/patch-bootloader-target.sh" "${TARGET_SPEC}"
(
  cd "${ROOT_DIR}/kernel"
  PATH="${CARGO_PATH}" CARGO_TARGET_DIR="${CARGO_TARGET_DIR_ARG}" \
    cargo bootimage --target "${TARGET_SPEC}" "$@"
)
