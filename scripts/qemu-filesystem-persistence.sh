#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
BOOTIMAGE="${ROOT_DIR}/kernel/target/x86_64-vantara_os/debug/bootimage-kernel.bin"
QEMU_BIN="${QEMU:-qemu-system-x86_64}"
DISK_IMAGE="${ROOT_DIR}/target/vantara-persist-test.img"

if ! command -v "${QEMU_BIN}" >/dev/null 2>&1; then
  echo "filesystem persistence test skipped: ${QEMU_BIN} not found" >&2
  exit 127
fi

mkdir -p "${ROOT_DIR}/target"
rm -f "${DISK_IMAGE}"
truncate -s 1M "${DISK_IMAGE}"

send_text() {
  local fd="$1"
  local text="$2"
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
    printf 'sendkey %s\n' "${key}" >&"${fd}"
  done
  printf 'sendkey ret\n' >&"${fd}"
}

wait_for_log() {
  local log_file="$1"
  local qemu_pid="$2"
  local pattern="$3"
  local start_byte="${4:-0}"
  for _ in $(seq 1 250); do
    if tail -c "+$((start_byte + 1))" "${log_file}" 2>/dev/null | grep -Fq "${pattern}"; then
      return 0
    fi
    kill -0 "${qemu_pid}" >/dev/null 2>&1 || return 1
    sleep 0.1
  done
  echo "timeout waiting for persistence checkpoint: ${pattern}" >&2
  tail -n 100 "${log_file}" >&2 || true
  return 1
}

boot_and_run() (
  local phase="$1"
  local command="$2"
  local expected="$3"
  local mount_marker="$4"
  local second_command="${5:-}"
  local second_expected="${6:-}"
  local log_file="${ROOT_DIR}/target/qemu-filesystem-persistence-${phase}.log"
  local monitor_log="${ROOT_DIR}/target/qemu-filesystem-persistence-${phase}-monitor.log"
  local monitor_fifo="${ROOT_DIR}/target/qemu-filesystem-persistence-${phase}-monitor.fifo"
  local qemu_pid=""
  local command_start

  rm -f "${log_file}" "${monitor_log}" "${monitor_fifo}"
  mkfifo "${monitor_fifo}"
  exec 3<>"${monitor_fifo}"

  cleanup() {
    if [[ -n "${qemu_pid}" ]]; then
      kill "${qemu_pid}" >/dev/null 2>&1 || true
      wait "${qemu_pid}" >/dev/null 2>&1 || true
    fi
    exec 3>&- || true
    rm -f "${monitor_fifo}"
  }
  trap cleanup EXIT

  "${QEMU_BIN}" \
    -drive "format=raw,file=${BOOTIMAGE},index=0,media=disk" \
    -drive "format=raw,file=${DISK_IMAGE},index=1,media=disk" \
    -display none \
    -serial "file:${log_file}" \
    -monitor stdio \
    -no-reboot \
    -no-shutdown \
    <"${monitor_fifo}" >"${monitor_log}" 2>&1 &
  qemu_pid=$!

  wait_for_log "${log_file}" "${qemu_pid}" "${mount_marker}"
  wait_for_log "${log_file}" "${qemu_pid}" "login: "
  send_text 3 "root"
  wait_for_log "${log_file}" "${qemu_pid}" 'root:/$ '

  command_start="$(wc -c <"${log_file}")"
  send_text 3 "${command}"
  wait_for_log "${log_file}" "${qemu_pid}" "${expected}" "${command_start}"
  wait_for_log "${log_file}" "${qemu_pid}" 'root:/$ ' "${command_start}"

  if [[ -n "${second_command}" ]]; then
    command_start="$(wc -c <"${log_file}")"
    send_text 3 "${second_command}"
    wait_for_log "${log_file}" "${qemu_pid}" "${second_expected}" "${command_start}"
    wait_for_log "${log_file}" "${qemu_pid}" 'root:/$ ' "${command_start}"
  fi

  if grep -Fq "KERNEL PANIC" "${log_file}"; then
    echo "filesystem persistence ${phase} failed: kernel panic detected" >&2
    return 1
  fi

  printf 'quit\n' >&3
  wait "${qemu_pid}" >/dev/null 2>&1 || true
  qemu_pid=""
)

boot_and_run first \
  "mkdir /persist/docs" 'root:/$ ' \
  "formatted new VANTFS01 volume" \
  "write /persist/docs/hello forever" "[FD] pid="
boot_and_run second "cat /persist/docs/hello" "forever" "mounted existing VANTFS01 volume"

echo "filesystem persistence test passed: data survived two QEMU boots"
echo "disk image: ${DISK_IMAGE}"
