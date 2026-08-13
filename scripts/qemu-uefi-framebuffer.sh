#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
BOOTIMAGE="${ROOT_DIR}/target/modern-boot/vantara-uefi.img"
LOG_FILE="${ROOT_DIR}/target/qemu-uefi-framebuffer.log"
QEMU_LOG="${ROOT_DIR}/target/qemu-uefi-framebuffer-qemu.log"
MONITOR_FIFO="${ROOT_DIR}/target/qemu-uefi-framebuffer-monitor.fifo"
QEMU_BIN="${QEMU:-qemu-system-x86_64}"
OVMF_CODE="${OVMF_CODE:-}"
QEMU_PID=""

cleanup() {
  if [[ -n "${QEMU_PID}" ]]; then
    kill "${QEMU_PID}" >/dev/null 2>&1 || true
    wait "${QEMU_PID}" >/dev/null 2>&1 || true
  fi
  exec 3>&- || true
  rm -f "${MONITOR_FIFO}"
}
trap cleanup EXIT

wait_for_log() {
  local pattern="$1"
  local start_byte="${2:-0}"
  local attempts="${3:-1800}"
  for ((attempt = 0; attempt < attempts; attempt++)); do
    if grep -Fq "${pattern}" < <(tail -c "+$((start_byte + 1))" "${LOG_FILE}" 2>/dev/null); then
      return 0
    fi
    if ! kill -0 "${QEMU_PID}" >/dev/null 2>&1; then
      fail "QEMU exited before checkpoint: ${pattern}"
    fi
    sleep 0.1
  done
  fail "timeout waiting for checkpoint: ${pattern}"
}

if [[ -z "${OVMF_CODE}" ]]; then
  for candidate in \
    /usr/share/OVMF/OVMF_CODE.fd \
    /usr/share/OVMF/OVMF_CODE_4M.fd \
    /usr/share/edk2/x64/OVMF_CODE.fd; do
    if [[ -f "${candidate}" ]]; then
      OVMF_CODE="${candidate}"
      break
    fi
  done
fi

command -v "${QEMU_BIN}" >/dev/null 2>&1 || { echo "UEFI framebuffer test: QEMU missing" >&2; exit 127; }
[[ -f "${OVMF_CODE}" ]] || { echo "UEFI framebuffer test: OVMF firmware missing" >&2; exit 127; }

bash "${ROOT_DIR}/scripts/build-modern-boot.sh"
rm -f "${LOG_FILE}" "${QEMU_LOG}" "${MONITOR_FIFO}"
mkfifo "${MONITOR_FIFO}"
exec 3<>"${MONITOR_FIFO}"
"${QEMU_BIN}" \
  -machine q35 \
  -m 256M \
  -smp 1 \
  -bios "${OVMF_CODE}" \
  -drive "format=raw,file=${BOOTIMAGE},if=ide" \
  -device isa-debug-exit,iobase=0xf4,iosize=0x04 \
  -display none \
  -serial "file:${LOG_FILE}" \
  -monitor stdio \
  -no-reboot -no-shutdown \
  <"${MONITOR_FIFO}" >"${QEMU_LOG}" 2>&1 &
QEMU_PID=$!

fail() {
  echo "UEFI framebuffer test failed: $1" >&2
  echo "--- serial log tail ---" >&2
  tail -80 "${LOG_FILE}" >&2 || true
  exit 1
}

wait_for_log "[COMPOSITOR] ready"
grep -Eq '\[FB\] boot-handoff .*width=[1-9][0-9]* height=[1-9][0-9]* stride=[1-9][0-9]* bpp=32 .*checksum=[0-9a-f]*[1-9a-f][0-9a-f]*' "${LOG_FILE}" || fail "invalid GOP framebuffer metadata/checksum"
grep -Eq '\[DISPLAY\] backend=UefiGop .*bpp=32' "${LOG_FILE}" || fail "UEFI GOP display backend not active"
grep -Eq '\[COMPOSITOR\] ready surfaces=2 frames=2 damaged=1024064 checksum=[0-9a-f]*[1-9a-f][0-9a-f]* buffers=1 flips=2 rejected=0' "${LOG_FILE}" || fail "render, clipping, checksum, or page flip regression"
if grep -Eiq "kernel panic|EXCEPTION:" "${LOG_FILE}"; then
  fail "kernel panic detected"
fi

wait_for_log "[init-display] fb0 ready=1 address="
grep -Eq '\[init-display\] fb0 ready=1 address=0x[0-9a-f]+ width=1280 height=800 stride=5120 bpp=32 .*checksum=0x[0-9a-f]*[1-9a-f][0-9a-f]*' "${LOG_FILE}" || fail "PID 1 framebuffer snapshot mismatch"
wait_for_log "[init-display] display0 ready=1 backend=uefi-gop mode=1280x800 stride=5120 bpp=32 buffers=1 flips=2 rejected=0"
wait_for_log "login: "

printf 'quit\n' >&3
wait "${QEMU_PID}" >/dev/null 2>&1 || true
QEMU_PID=""
echo "UEFI framebuffer test passed: PID 1 verified GOP geometry, render, clipping, checksum, and page flips"
