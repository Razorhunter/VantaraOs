#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
BOOTIMAGE="${ROOT_DIR}/target/kernel/x86_64-vantara_os/debug/bootimage-kernel.bin"
LOG_FILE="${ROOT_DIR}/target/qemu-fault-isolation.log"
MONITOR_LOG="${ROOT_DIR}/target/qemu-fault-isolation-monitor.log"
MONITOR_FIFO="${ROOT_DIR}/target/qemu-fault-isolation-monitor.fifo"
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
  echo "fault isolation test skipped: ${QEMU_BIN} not found" >&2
  exit 127
fi

if [[ ! -f "${BOOTIMAGE}" ]]; then
  make -C "${ROOT_DIR}" kernel-build
fi

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
      [A-Z]) key="shift-${char,,}" ;;
      *)
        echo "unsupported QEMU sendkey character: ${char}" >&2
        return 2
        ;;
    esac
    printf 'sendkey %s\n' "${key}" >&3
  done
  printf 'sendkey ret\n' >&3
}

wait_for_log() {
  local pattern="$1"
  local start_byte="${2:-0}"
  local attempts="${3:-150}"

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
  echo "--- serial log tail ---" >&2
  tail -n 60 "${LOG_FILE}" >&2 || true
  return 1
}

assert_kernel_alive() {
  if grep -Fq "KERNEL PANIC" "${LOG_FILE}"; then
    echo "fault isolation test failed: kernel panic detected" >&2
    return 1
  fi
  if grep -Fq "EXCEPTION: PAGE FAULT" "${LOG_FILE}"; then
    echo "fault isolation test failed: user fault reached kernel panic path" >&2
    return 1
  fi
}

run_fault_case() {
  local command="$1"
  local kind="$2"
  local start_byte

  start_byte="$(wc -c <"${LOG_FILE}")"
  send_text "${command}"
  wait_for_log "name=fault fault=page kind=${kind}" "${start_byte}"
  wait_for_log "status=0xbadf00d" "${start_byte}"
  wait_for_log "waitpid reaped pid=" "${start_byte}"
  wait_for_log 'root:/$ ' "${start_byte}"
  assert_kernel_alive
  echo "${kind} user page-fault containment passed"
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

wait_for_log "login: "
send_text "root"
wait_for_log "Welcome to Vantara OS, root"
wait_for_log 'root:/$ '

run_fault_case "fault" "Null"
run_fault_case "fault stackguard" "StackGuard"

EXCEPTION_START="$(wc -c <"${LOG_FILE}")"
send_text "fault invalidopcode"
wait_for_log "name=fault fault=invalid-opcode" "${EXCEPTION_START}"
wait_for_log "status=0xbad0006" "${EXCEPTION_START}"
wait_for_log 'root:/$ ' "${EXCEPTION_START}"
assert_kernel_alive

EXCEPTION_START="$(wc -c <"${LOG_FILE}")"
send_text "fault generalprotection"
wait_for_log "name=fault fault=general-protection" "${EXCEPTION_START}"
wait_for_log "status=0xbad000d" "${EXCEPTION_START}"
wait_for_log 'root:/$ ' "${EXCEPTION_START}"
assert_kernel_alive

EXCEPTION_START="$(wc -c <"${LOG_FILE}")"
send_text "fault dividezero"
wait_for_log "name=fault fault=divide-error" "${EXCEPTION_START}"
wait_for_log "status=0xbad0000" "${EXCEPTION_START}"
wait_for_log 'root:/$ ' "${EXCEPTION_START}"
assert_kernel_alive

POINTER_START="$(wc -c <"${LOG_FILE}")"
send_text "fault invalidread"
wait_for_log "fault: invalid read rejected" "${POINTER_START}"
wait_for_log 'root:/$ ' "${POINTER_START}"

POINTER_START="$(wc -c <"${LOG_FILE}")"
send_text "fault invalidwrite"
wait_for_log "fault: invalid write rejected" "${POINTER_START}"
wait_for_log 'root:/$ ' "${POINTER_START}"
assert_kernel_alive

RECOVERY_START="$(wc -c <"${LOG_FILE}")"
send_text "rusthello"
wait_for_log "hello from Rust ELF" "${RECOVERY_START}"
wait_for_log 'root:/$ ' "${RECOVERY_START}"
assert_kernel_alive

printf 'quit\n' >&3
wait "${QEMU_PID}" >/dev/null 2>&1 || true
QEMU_PID=""

echo "fault isolation regression test passed: page faults + divide/invalid-opcode/general-protection + pointer rejection + shell recovery"
echo "serial log: ${LOG_FILE}"
