#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
BOOTIMAGE="${ROOT_DIR}/kernel/target/x86_64-vantara_os/debug/bootimage-kernel.bin"
LOG_FILE="${ROOT_DIR}/target/qemu-preemption.log"
MONITOR_LOG="${ROOT_DIR}/target/qemu-preemption-monitor.log"
MONITOR_FIFO="${ROOT_DIR}/target/qemu-preemption-monitor.fifo"
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
  echo "preemption integration test skipped: ${QEMU_BIN} not found" >&2
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

test_start="$(wc -c <"${LOG_FILE}")"
send_text "bg preemptdemo"
wait_for_log "preemptdemo: start" "${test_start}"
wait_for_log 'root:/$ ' "${test_start}"
wait_for_log "preemptdemo: done" "${test_start}"

test_output="${ROOT_DIR}/target/qemu-preemption.output"
tail -c "+$((test_start + 1))" "${LOG_FILE}" >"${test_output}"

start_offset="$(grep -aboF -m1 "preemptdemo: start" "${test_output}" | cut -d: -f1)"
prompt_offset="$(grep -aboF -m1 'root:/$ ' "${test_output}" | cut -d: -f1)"
done_offset="$(grep -aboF -m1 "preemptdemo: done" "${test_output}" | cut -d: -f1)"

if (( start_offset >= prompt_offset || prompt_offset >= done_offset )); then
  echo "preemption integration test failed: shell did not run during CPU-bound job" >&2
  cat "${test_output}" >&2
  exit 1
fi

procs_start="$(wc -c <"${LOG_FILE}")"
send_text "procs"
wait_for_log 'root:/$ ' "${procs_start}"
if ! tail -c "+$((procs_start + 1))" "${LOG_FILE}" | grep -Eq 'py=[1-9][0-9]*'; then
  echo "preemption integration test failed: timer preemption counter stayed zero" >&2
  tail -n 60 "${LOG_FILE}" >&2
  exit 1
fi

if grep -Fq "KERNEL PANIC" "${LOG_FILE}"; then
  echo "preemption integration test failed: kernel panic detected" >&2
  exit 1
fi

printf 'quit\n' >&3
wait "${QEMU_PID}" >/dev/null 2>&1 || true
QEMU_PID=""

echo "preemption integration test passed: CPU-bound process was involuntarily descheduled"
echo "serial log: ${LOG_FILE}"
echo "test output: ${test_output}"
