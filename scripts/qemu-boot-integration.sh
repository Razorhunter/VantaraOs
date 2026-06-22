#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
BOOTIMAGE="${ROOT_DIR}/kernel/target/x86_64-vantara_os/debug/bootimage-kernel.bin"
LOG_FILE="${ROOT_DIR}/target/qemu-boot-integration.log"
MONITOR_LOG="${ROOT_DIR}/target/qemu-boot-monitor.log"
MONITOR_FIFO="${ROOT_DIR}/target/qemu-boot-monitor.fifo"
QEMU_BIN="${QEMU:-qemu-system-x86_64}"
QEMU_PID=""

mkdir -p "${ROOT_DIR}/target"

cleanup() {
  if [[ -n "${QEMU_PID}" ]]; then
    kill "${QEMU_PID}" >/dev/null 2>&1 || true
    wait "${QEMU_PID}" >/dev/null 2>&1 || true
  fi
  exec 3>&- || true
  rm -f "${MONITOR_FIFO}"
}
trap cleanup EXIT

if ! command -v "${QEMU_BIN}" >/dev/null 2>&1; then
  echo "boot integration test skipped: ${QEMU_BIN} not found" >&2
  exit 127
fi

if [[ ! -f "${BOOTIMAGE}" ]]; then
  make -C "${ROOT_DIR}" kernel-build
fi

rm -f "${LOG_FILE}" "${MONITOR_LOG}" "${MONITOR_FIFO}"
mkfifo "${MONITOR_FIFO}"
exec 3<>"${MONITOR_FIFO}"

"${QEMU_BIN}" \
  -drive "format=raw,file=${BOOTIMAGE}" \
  -display none \
  -serial "file:${LOG_FILE}" \
  -monitor stdio \
  -no-reboot \
  -no-shutdown \
  <"${MONITOR_FIFO}" >"${MONITOR_LOG}" 2>&1 &
QEMU_PID=$!

wait_for_log() {
  local pattern="$1"
  local attempts="${2:-200}"

  for ((attempt = 0; attempt < attempts; attempt++)); do
    if grep -Fq "${pattern}" "${LOG_FILE}" 2>/dev/null; then
      return 0
    fi
    if ! kill -0 "${QEMU_PID}" >/dev/null 2>&1; then
      echo "QEMU exited before checkpoint: ${pattern}" >&2
      return 1
    fi
    sleep 0.1
  done

  echo "timeout waiting for checkpoint: ${pattern}" >&2
  echo "--- serial log tail ---" >&2
  tail -n 40 "${LOG_FILE}" >&2 || true
  return 1
}

wait_for_log "Vantara init started"
wait_for_log "[USER] boot policy queued /bin/init pid=1"
wait_for_log "[init] service manager running as pid 1"
wait_for_log "login: "

printf 'sendkey r\n' >&3
printf 'sendkey o\n' >&3
printf 'sendkey o\n' >&3
printf 'sendkey t\n' >&3
printf 'sendkey ret\n' >&3

wait_for_log "Welcome to Vantara OS, root"
wait_for_log "/bin/sh bytes="
wait_for_log 'root:/$ '

if grep -Fq "KERNEL PANIC" "${LOG_FILE}"; then
  echo "boot integration test failed: kernel panic detected" >&2
  exit 1
fi

printf 'quit\n' >&3
wait "${QEMU_PID}" >/dev/null 2>&1 || true
QEMU_PID=""

echo "boot integration test passed: init -> login -> sh"
echo "serial log: ${LOG_FILE}"
