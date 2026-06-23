#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
BOOTIMAGE="${ROOT_DIR}/kernel/target/x86_64-vantara_os/debug/bootimage-kernel.bin"
DISK_IMAGE="${ROOT_DIR}/target/vantara-block-cache-test.img"
LOG_FILE="${ROOT_DIR}/target/qemu-block-cache.log"
MONITOR_LOG="${ROOT_DIR}/target/qemu-block-cache-monitor.log"
MONITOR_FIFO="${ROOT_DIR}/target/qemu-block-cache-monitor.fifo"
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
  echo "block cache test skipped: ${QEMU_BIN} not found" >&2
  exit 127
fi

rm -f "${DISK_IMAGE}" "${LOG_FILE}" "${MONITOR_LOG}" "${MONITOR_FIFO}"
truncate -s 1M "${DISK_IMAGE}"
mkfifo "${MONITOR_FIFO}"
exec 3<>"${MONITOR_FIFO}"

wait_for_log() {
  local pattern="$1"
  local start_byte="${2:-0}"
  for _ in $(seq 1 250); do
    if tail -c "+$((start_byte + 1))" "${LOG_FILE}" 2>/dev/null | grep -Fq "${pattern}"; then
      return 0
    fi
    kill -0 "${QEMU_PID}" >/dev/null 2>&1 || return 1
    sleep 0.1
  done
  echo "timeout waiting for block-cache checkpoint: ${pattern}" >&2
  tail -n 100 "${LOG_FILE}" >&2 || true
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
  -drive "format=raw,file=${BOOTIMAGE},index=0,media=disk" \
  -drive "format=raw,file=${DISK_IMAGE},index=1,media=disk" \
  -display none \
  -serial "file:${LOG_FILE}" \
  -monitor stdio \
  -no-reboot \
  -no-shutdown \
  <"${MONITOR_FIFO}" >"${MONITOR_LOG}" 2>&1 &
QEMU_PID=$!

wait_for_log "[BLOCKCACHE] ready capacity=16 policy=write-through"
wait_for_log "login: "
send_text "root"
wait_for_log 'root:/$ '

run_command "write /persist/cache cached"
run_command "cat /persist/cache" "cached"
run_command "cat /persist/cache" "cached"

stats_start="$(wc -c <"${LOG_FILE}")"
send_text "cat /dev/block-cache"
wait_for_log "VANTFS block cache" "${stats_start}"
wait_for_log 'root:/$ ' "${stats_start}"

stats_output="$(
  tail -c "+$((stats_start + 1))" "${LOG_FILE}" \
    | sed -n '/VANTFS block cache/,/root:\/\$ /p'
)"
hits="$(sed -nE 's/^read_hits ([0-9]+).*/\1/p' <<<"${stats_output}" | tail -n1)"
misses="$(sed -nE 's/^read_misses ([0-9]+).*/\1/p' <<<"${stats_output}" | tail -n1)"
writes="$(sed -nE 's/^writes ([0-9]+).*/\1/p' <<<"${stats_output}" | tail -n1)"

if [[ -z "${hits}" || -z "${misses}" || -z "${writes}" ]]; then
  echo "block cache test failed: cache counters missing" >&2
  echo "${stats_output}" >&2
  exit 1
fi
if (( hits == 0 || misses == 0 || writes == 0 )); then
  echo "block cache test failed: expected non-zero hits, misses, and writes" >&2
  echo "${stats_output}" >&2
  exit 1
fi
if grep -Fq "KERNEL PANIC" "${LOG_FILE}"; then
  echo "block cache test failed: kernel panic detected" >&2
  exit 1
fi

printf 'quit\n' >&3
wait "${QEMU_PID}" >/dev/null 2>&1 || true
QEMU_PID=""

echo "block cache test passed: hits=${hits} misses=${misses} writes=${writes}"
echo "serial log: ${LOG_FILE}"
