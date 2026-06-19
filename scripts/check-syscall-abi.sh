#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
KERNEL_ABI="${ROOT_DIR}/kernel/src/user/syscall.rs"
RUST_ABI="${ROOT_DIR}/userland/native/src/abi.rs"
ASM_ABI="${ROOT_DIR}/userland/native/asm/abi.inc"
KERNEL_VALUES="$(mktemp)"
RUST_VALUES="$(mktemp)"

cleanup() {
  rm -f "${KERNEL_VALUES}" "${RUST_VALUES}"
}
trap cleanup EXIT

extract_rust_syscalls() {
  sed -n \
    's/^pub const \(SYS_[A-Z_]*\): u64 = \([0-9][0-9]*\);$/\1 \2/p' \
    "$1" | LC_ALL=C sort
}

extract_rust_syscalls "${KERNEL_ABI}" >"${KERNEL_VALUES}"
extract_rust_syscalls "${RUST_ABI}" >"${RUST_VALUES}"

if ! cmp -s "${KERNEL_VALUES}" "${RUST_VALUES}"; then
  echo "syscall ABI mismatch between kernel and Rust userland" >&2
  diff -u "${KERNEL_VALUES}" "${RUST_VALUES}" >&2 || true
  exit 1
fi

while read -r name value; do
  asm_value="$(sed -n "s/^%define[[:space:]]\\+${name}[[:space:]]\\+\\([0-9][0-9]*\\)$/\\1/p" "${ASM_ABI}")"
  if [[ -n "${asm_value}" && "${asm_value}" != "${value}" ]]; then
    echo "syscall ABI mismatch for ${name}: kernel=${value} asm=${asm_value}" >&2
    exit 1
  fi
done <"${KERNEL_VALUES}"

echo "syscall ABI v1 constants are synchronized"
