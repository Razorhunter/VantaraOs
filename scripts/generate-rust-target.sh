#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
SOURCE_TARGET="${ROOT_DIR}/kernel/x86_64-vantara_os.json"
OUTPUT_TARGET="${1:-${ROOT_DIR}/target/generated/x86_64-vantara_os.json}"
TMP_TARGET="${OUTPUT_TARGET%.json}.tmp.json"
RUSTC_BIN="${RUSTC:-rustc}"

if ! command -v "${RUSTC_BIN}" >/dev/null 2>&1 && [[ -x /usr/local/cargo/bin/rustc ]]; then
  RUSTC_BIN="/usr/local/cargo/bin/rustc"
fi

mkdir -p "$(dirname "${OUTPUT_TARGET}")"

cleanup() {
  rm -f "${TMP_TARGET}"
}
trap cleanup EXIT

for abi in x86-softfloat softfloat; do
  sed "s/\"rustc-abi\": \".*\"/\"rustc-abi\": \"${abi}\"/" \
    "${SOURCE_TARGET}" >"${TMP_TARGET}"
  if printf '' | "${RUSTC_BIN}" - \
    --crate-name ___ \
    -Zunstable-options \
    --print=file-names \
    --target "${TMP_TARGET}" \
    --crate-type bin >/dev/null 2>&1; then
    mv "${TMP_TARGET}" "${OUTPUT_TARGET}"
    echo "generated Rust target: ${OUTPUT_TARGET} (rustc-abi=${abi})"
    exit 0
  fi
done

echo "failed to generate Rust target: rustc accepts neither x86-softfloat nor softfloat ABI" >&2
exit 1
