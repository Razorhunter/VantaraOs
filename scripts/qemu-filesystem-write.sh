#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
BOOTIMAGE="${ROOT_DIR}/target/kernel/x86_64-vantara_os/debug/bootimage-kernel.bin"
QEMU_BIN="${QEMU:-qemu-system-x86_64}"
LOG_FILE="${ROOT_DIR}/target/qemu-filesystem-write.log"
MONITOR_LOG="${ROOT_DIR}/target/qemu-filesystem-write-monitor.log"
MONITOR_FIFO="${ROOT_DIR}/target/qemu-filesystem-write-monitor.fifo"

if ! command -v "${QEMU_BIN}" >/dev/null 2>&1; then
  echo "filesystem write test skipped: ${QEMU_BIN} not found" >&2
  exit 127
fi

mkdir -p "${ROOT_DIR}/target"
rm -f "${LOG_FILE}" "${MONITOR_LOG}" "${MONITOR_FIFO}"
mkfifo "${MONITOR_FIFO}"
exec 3<>"${MONITOR_FIFO}"
qemu_pid=""

cleanup() {
  if [[ -n "${qemu_pid}" ]]; then
    kill "${qemu_pid}" >/dev/null 2>&1 || true
    wait "${qemu_pid}" >/dev/null 2>&1 || true
  fi
  exec 3>&- || true
  rm -f "${MONITOR_FIFO}"
}
trap cleanup EXIT

wait_for_log() {
  local pattern="$1"
  local start_byte="${2:-0}"
  for _ in $(seq 1 200); do
    if tail -c "+$((start_byte + 1))" "${LOG_FILE}" 2>/dev/null | grep -Fq "${pattern}"; then
      return 0
    fi
    kill -0 "${qemu_pid}" >/dev/null 2>&1 || return 1
    sleep 0.1
  done
  echo "timeout waiting for filesystem checkpoint: ${pattern}" >&2
  tail -n 80 "${LOG_FILE}" >&2 || true
  return 1
}

send_text() {
  local text="$1"
  local char key
  for ((index = 0; index < ${#text}; index++)); do
    char="${text:index:1}"
    case "${char}" in
      " ") key="spc" ;;
      "/") key="slash" ;;
      "-") key="minus" ;;
      ".") key="dot" ;;
      [a-z0-9]) key="${char}" ;;
      *) echo "unsupported sendkey character: ${char}" >&2; return 2 ;;
    esac
    printf 'sendkey %s\n' "${key}" >&3
  done
  printf 'sendkey ret\n' >&3
}

run_command() {
  local command="$1"
  local expected="${2:-}"
  local start
  start="$(wc -c <"${LOG_FILE}")"
  send_text "${command}"
  if [[ -n "${expected}" ]]; then
    wait_for_log "${expected}" "${start}"
  fi
  wait_for_log 'root:/$ ' "${start}"
}

"${QEMU_BIN}" \
  -drive "format=raw,file=${BOOTIMAGE}" \
  -display none \
  -serial "file:${LOG_FILE}" \
  -monitor stdio \
  -no-reboot \
  -no-shutdown \
  <"${MONITOR_FIFO}" >"${MONITOR_LOG}" 2>&1 &
qemu_pid=$!

wait_for_log "login: "
send_text "root"
wait_for_log 'root:/$ '

run_command "mkdir /tmp/docs"
run_command "touch /tmp/docs/note"
run_command "write /tmp/docs/note hello"
run_command "cat /tmp/docs/note" "hello"
run_command "cat /tmp/docs" "cat: not a file"
run_command "stat /tmp/docs" "type: dir"
run_command "mv /tmp/docs/note /tmp/docs/done"
run_command "ls /tmp/docs" "done"
run_command "rm /tmp/docs/done"
run_command "rmdir /tmp/docs"

start="$(wc -c <"${LOG_FILE}")"
send_text "cat /tmp/docs/done"
wait_for_log "cat: not found" "${start}"
wait_for_log 'root:/$ ' "${start}"

if grep -Fq "KERNEL PANIC" "${LOG_FILE}"; then
  echo "filesystem write test failed: kernel panic detected" >&2
  exit 1
fi

printf 'quit\n' >&3
wait "${qemu_pid}" >/dev/null 2>&1 || true
qemu_pid=""

echo "filesystem write lifecycle test passed"
echo "serial log: ${LOG_FILE}"
