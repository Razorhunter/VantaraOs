#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
BOOTIMAGE="${ROOT_DIR}/target/kernel/x86_64-vantara_os/debug/bootimage-kernel.bin"
LOG_FILE="${ROOT_DIR}/target/qemu-device-namespace.log"
MONITOR_LOG="${ROOT_DIR}/target/qemu-device-namespace-monitor.log"
MONITOR_FIFO="${ROOT_DIR}/target/qemu-device-namespace-monitor.fifo"
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
  echo "device namespace test skipped: ${QEMU_BIN} not found" >&2
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

wait_for_log "[VFS] mounted backend=devfs path=/dev mount_id=3"
wait_for_log "[VFS] mounted backend=devfs path=/Devices mount_id=5"
wait_for_log "driver=IntelE1000"
wait_for_log "mmio_ready=true"
wait_for_log "mac_valid=true"
wait_for_log "rxq=true"
wait_for_log "txq=true"
wait_for_log "depth=16"
wait_for_log "tx_test=true"
wait_for_log "tx_packets=1"
wait_for_log "login: "
send_text "root"
wait_for_log 'root:/$ '

run_command "ls /" "dev"
run_command "ls /" "System"
run_command "ls /" "Apps"
run_command "ls /" "Users"
run_command "ls /" "Config"
run_command "ls /" "Data"
run_command "ls /" "Cache"
run_command "ls /" "Logs"
run_command "ls /" "Runtime"
run_command "ls /" "Devices"
run_command "ls /" "Temp"
run_command "ls /" "Packages"
run_command "ls /" "Boot"
run_command "ls /" "Volumes"
run_command "cat /Devices/drivers" "devfs"
run_command "cat /Devices/net" "e1000"
run_command "stat /System" "type: dir"
run_command "stat /Apps/ls" "type: file"
run_command "mkdir /Temp/layout"
run_command "stat /Temp/layout" "type: dir"
run_command "ls /dev" "drivers"
run_command "cat /dev/drivers" "devfs"
run_command "cat /dev/pci" "PCI devices:"
run_command "cat /dev/net" "Network devices:"
run_command "cat /dev/block-cache" "VANTFS block cache"
run_command "cat /dev/partitions" "VANTFS storage view"
run_command "cat /dev/ahci" "AHCI controllers:"
run_command "cat /dev/nvme" "NVMe controllers:"
run_command "stat /dev/null" "readonly: 0"
run_command "write /dev/null discarded"
run_command "cat /dev/null"
run_command "stat /dev/zero" "readonly: 0"
run_command "write /dev/zero discarded"

if grep -Fq "KERNEL PANIC" "${LOG_FILE}"; then
  echo "device namespace test failed: kernel panic detected" >&2
  exit 1
fi

printf 'quit\n' >&3
wait "${QEMU_PID}" >/dev/null 2>&1 || true
QEMU_PID=""

echo "device namespace test passed: devfs nodes and live registries"
echo "serial log: ${LOG_FILE}"
