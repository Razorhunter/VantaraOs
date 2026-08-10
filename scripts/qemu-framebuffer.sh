#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
CARGO_TARGET_DIR="${ROOT_DIR}/target/framebuffer-test-cargo"
BOOTIMAGE="${CARGO_TARGET_DIR}/x86_64-vantara_os/debug/bootimage-kernel.bin"
LOG_FILE="${ROOT_DIR}/target/qemu-framebuffer.log"
QEMU_LOG="${ROOT_DIR}/target/qemu-framebuffer-qemu.log"
QEMU_BIN="${QEMU:-qemu-system-x86_64}"
QEMU_PID=""
cleanup() { if [[ -n "${QEMU_PID}" ]]; then kill "${QEMU_PID}" >/dev/null 2>&1 || true; wait "${QEMU_PID}" >/dev/null 2>&1 || true; fi; }
trap cleanup EXIT
command -v "${QEMU_BIN}" >/dev/null 2>&1 || { echo "framebuffer test skipped: ${QEMU_BIN} not found" >&2; exit 127; }
bash "${ROOT_DIR}/scripts/build-kernel-variant.sh" "${CARGO_TARGET_DIR}" --features framebuffer-vga
rm -f "${LOG_FILE}" "${QEMU_LOG}"
"${QEMU_BIN}" -machine pc -drive "format=raw,file=${BOOTIMAGE},index=0,media=disk" -display none -serial "file:${LOG_FILE}" -monitor none -no-reboot -no-shutdown >"${QEMU_LOG}" 2>&1 &
QEMU_PID=$!
for _ in $(seq 1 900); do
  grep -Fq "[COMPOSITOR] ready" "${LOG_FILE}" 2>/dev/null && break
  kill -0 "${QEMU_PID}" >/dev/null 2>&1 || { cat "${QEMU_LOG}" >&2; exit 1; }
  sleep 0.1
done
grep -Fq "[FB] ready address=0xa0000 width=320 height=200 stride=320 bpp=8" "${LOG_FILE}"
grep -Fq "[DISPLAY] backend=LegacyVga mode=320x200 stride=320 bpp=8 refresh-millihertz=70000" "${LOG_FILE}"
grep -Fq "[COMPOSITOR] ready surfaces=2 frames=1 damaged=64000" "${LOG_FILE}"
! grep -Fq "checksum=00000000" "${LOG_FILE}"
! grep -Eiq "kernel panic|EXCEPTION:" "${LOG_FILE}"
echo "graphics test passed: generic display API, framebuffer, and damage-aware compositor rendered deterministically"
