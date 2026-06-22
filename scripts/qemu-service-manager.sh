#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
BOOTIMAGE="${ROOT_DIR}/kernel/target/x86_64-vantara_os/debug/bootimage-kernel.bin"
LOG_FILE="${ROOT_DIR}/target/qemu-service-manager.log"
MONITOR_LOG="${ROOT_DIR}/target/qemu-service-manager-monitor.log"
MONITOR_FIFO="${ROOT_DIR}/target/qemu-service-manager-monitor.fifo"
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
  echo "service manager test skipped: ${QEMU_BIN} not found" >&2
  exit 127
fi

if [[ ! -f "${BOOTIMAGE}" ]]; then
  make -C "${ROOT_DIR}" kernel-build
fi

wait_for_log() {
  local pattern="$1"
  local start_byte="${2:-0}"
  local attempts="${3:-200}"

  for ((attempt = 0; attempt < attempts; attempt++)); do
    if grep -Fq "${pattern}" < <(
      tail -c "+$((start_byte + 1))" "${LOG_FILE}" 2>/dev/null
    ); then
      return 0
    fi
    if ! kill -0 "${QEMU_PID}" >/dev/null 2>&1; then
      echo "QEMU exited before checkpoint: ${pattern}" >&2
      return 1
    fi
    sleep 0.1
  done

  echo "timeout waiting for checkpoint: ${pattern}" >&2
  tail -n 60 "${LOG_FILE}" >&2 || true
  return 1
}

send_text() {
  local text="$1"
  local char key

  for ((index = 0; index < ${#text}; index++)); do
    char="${text:index:1}"
    case "${char}" in
      " ") key="spc" ;;
      [a-z0-9]) key="${char}" ;;
      *)
        echo "unsupported QEMU sendkey character: ${char}" >&2
        return 2
        ;;
    esac
    printf 'sendkey %s\n' "${key}" >&3
  done
  printf 'sendkey ret\n' >&3
}

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

wait_for_log "[PROCESS] Created kernel process pid=0"
wait_for_log "[USER] boot policy queued /bin/init pid=1"
wait_for_log "[init] service manager running as pid 1"
wait_for_log "login: "

send_text "root"
wait_for_log 'root:/$ '

restart_start="$(wc -c <"${LOG_FILE}")"
send_text "kill 2"
wait_for_log "[init] login exited; restarting" "${restart_start}"
wait_for_log "[init] starting login" "${restart_start}"
wait_for_log "login: " "${restart_start}"

if grep -Fq "KERNEL PANIC" "${LOG_FILE}"; then
  echo "service manager test failed: kernel panic detected" >&2
  exit 1
fi

printf 'quit\n' >&3
wait "${QEMU_PID}" >/dev/null 2>&1 || true
QEMU_PID=""

echo "service manager test passed: PID 1 supervised and restarted login"
echo "serial log: ${LOG_FILE}"
