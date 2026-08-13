#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
BOOTIMAGE="${ROOT_DIR}/target/kernel/x86_64-vantara_os/debug/bootimage-kernel.bin"
LOG_FILE="${ROOT_DIR}/target/qemu-virtio-gpu-discovery.log"
QEMU_LOG="${ROOT_DIR}/target/qemu-virtio-gpu-discovery-qemu.log"
QEMU_BIN="${QEMU:-qemu-system-x86_64}"
QEMU_PID=""

cleanup() {
  if [[ -n "${QEMU_PID}" ]]; then
    kill "${QEMU_PID}" >/dev/null 2>&1 || true
    wait "${QEMU_PID}" >/dev/null 2>&1 || true
  fi
}
trap cleanup EXIT

command -v "${QEMU_BIN}" >/dev/null 2>&1 || {
  echo "VirtIO-GPU discovery test: QEMU missing" >&2
  exit 127
}

rm -f "${LOG_FILE}" "${QEMU_LOG}"
"${QEMU_BIN}" \
  -machine q35 \
  -m 256M \
  -smp 1 \
  -drive "format=raw,file=${BOOTIMAGE},index=0,media=disk" \
  -vga none \
  -device virtio-vga \
  -display none \
  -serial "file:${LOG_FILE}" \
  -monitor none \
  -no-reboot -no-shutdown >"${QEMU_LOG}" 2>&1 &
QEMU_PID=$!

wait_for_log() {
  local pattern="$1"
  for _ in $(seq 1 600); do
    if grep -Fq "${pattern}" "${LOG_FILE}" 2>/dev/null; then
      return 0
    fi
    if ! kill -0 "${QEMU_PID}" >/dev/null 2>&1; then
      echo "QEMU exited before VirtIO-GPU checkpoint: ${pattern}" >&2
      cat "${QEMU_LOG}" >&2 || true
      exit 1
    fi
    sleep 0.1
  done
  echo "timeout waiting for VirtIO-GPU checkpoint: ${pattern}" >&2
  tail -100 "${LOG_FILE}" >&2 || true
  exit 1
}

wait_for_log "[VIRTIO-GPU] "
wait_for_log "vendor=1af4 device=1050"
wait_for_log "capabilities=5"
wait_for_log "common=true"
wait_for_log "notify=true"
wait_for_log "isr=true"
wait_for_log "device_cfg=true"
wait_for_log "transport_ready=true"
wait_for_log "version1=true"
wait_for_log "features_ok=true"
wait_for_log "queue_size=64"
wait_for_log "queue_enabled=true"
wait_for_log "driver_ok=true"
wait_for_log "init=Ok(())"
wait_for_log "[init-display] virtio-gpu detected=true transport-ready=true capabilities=5 common=true notify=true isr=true device-config=true version1=true features-ok=true queue-size=64 queue-enabled=true driver-ok=true"

if grep -Eiq "kernel panic|EXCEPTION:" "${LOG_FILE}"; then
  echo "VirtIO-GPU discovery test failed: kernel panic detected" >&2
  exit 1
fi

echo "VirtIO-GPU discovery test passed: modern PCI transport capabilities found"
echo "serial log: ${LOG_FILE}"
