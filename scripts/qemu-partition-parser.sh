#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
BOOTIMAGE="${ROOT_DIR}/target/kernel/x86_64-vantara_os/debug/bootimage-kernel.bin"
DISK_IMAGE="${ROOT_DIR}/target/vantara-partition-test.img"
QEMU_BIN="${QEMU:-qemu-system-x86_64}"

if ! command -v "${QEMU_BIN}" >/dev/null 2>&1; then
  echo "partition parser test skipped: ${QEMU_BIN} not found" >&2
  exit 127
fi

mkdir -p "${ROOT_DIR}/target"
rm -f "${DISK_IMAGE}"
truncate -s 2M "${DISK_IMAGE}"

# MBR entry 0: type 0x7f, LBA start 64, length 2048 sectors.
printf '\x00\x00\x00\x00\x7f\x00\x00\x00\x40\x00\x00\x00\x00\x08\x00\x00' \
  | dd of="${DISK_IMAGE}" bs=1 seek=446 conv=notrunc status=none
printf '\x55\xaa' \
  | dd of="${DISK_IMAGE}" bs=1 seek=510 conv=notrunc status=none

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
  echo "timeout waiting for partition checkpoint: ${pattern}" >&2
  tail -n 100 "${log_file}" >&2 || true
  return 1
}

boot_and_run() (
  local phase="$1"
  local command="$2"
  local expected="$3"
  local log_file="${ROOT_DIR}/target/qemu-partition-${phase}.log"
  local monitor_log="${ROOT_DIR}/target/qemu-partition-${phase}-monitor.log"
  local monitor_fifo="${ROOT_DIR}/target/qemu-partition-${phase}-monitor.fifo"
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

  wait_for_log "${log_file}" "${qemu_pid}" "[PARTITION] MBR type=0x7f start=64 blocks=2048"
  wait_for_log "${log_file}" "${qemu_pid}" "login: "
  send_text 3 "root"
  wait_for_log "${log_file}" "${qemu_pid}" 'root:/$ '

  command_start="$(wc -c <"${log_file}")"
  send_text 3 "cat /dev/partitions"
  wait_for_log "${log_file}" "${qemu_pid}" "mode mbr" "${command_start}"
  wait_for_log "${log_file}" "${qemu_pid}" "start_block 64" "${command_start}"
  wait_for_log "${log_file}" "${qemu_pid}" 'root:/$ ' "${command_start}"

  command_start="$(wc -c <"${log_file}")"
  send_text 3 "${command}"
  wait_for_log "${log_file}" "${qemu_pid}" "${expected}" "${command_start}"
  wait_for_log "${log_file}" "${qemu_pid}" 'root:/$ ' "${command_start}"

  if grep -Fq "KERNEL PANIC" "${log_file}"; then
    echo "partition parser ${phase} failed: kernel panic detected" >&2
    return 1
  fi

  printf 'quit\n' >&3
  wait "${qemu_pid}" >/dev/null 2>&1 || true
  qemu_pid=""
)

boot_and_run first "write /persist/partition inside" "[FD] pid="
boot_and_run second "cat /persist/partition" "inside"

echo "partition parser test passed: MBR range persisted across reboot"
echo "disk image: ${DISK_IMAGE}"
