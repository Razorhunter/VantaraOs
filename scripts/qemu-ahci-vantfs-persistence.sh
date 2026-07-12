#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
CARGO_TARGET_DIR="${ROOT_DIR}/target/ahci-vantfs-test-cargo"
BOOTIMAGE="${CARGO_TARGET_DIR}/x86_64-vantara_os/debug/bootimage-kernel.bin"
DISK_IMAGE="${ROOT_DIR}/target/qemu-ahci-vantfs-test.img"
QEMU_BIN="${QEMU:-qemu-system-x86_64}"

if ! command -v "${QEMU_BIN}" >/dev/null 2>&1; then
  echo "AHCI VANTFS persistence test skipped: ${QEMU_BIN} not found" >&2
  exit 127
fi

mkdir -p "${ROOT_DIR}/target"
bash "${ROOT_DIR}/scripts/build-kernel-variant.sh" \
  "${CARGO_TARGET_DIR}" \
  --features ahci-vantfs-test

cp "${BOOTIMAGE}" "${DISK_IMAGE}"
image_size="$(stat -c %s "${DISK_IMAGE}")"
aligned_size="$(( (image_size + 511) / 512 * 512 ))"
truncate -s "$((aligned_size + 1024 * 1024))" "${DISK_IMAGE}"

boot_phase() {
  local phase="$1"
  local log_file="${ROOT_DIR}/target/qemu-ahci-vantfs-${phase}.log"
  local qemu_log="${ROOT_DIR}/target/qemu-ahci-vantfs-${phase}-qemu.log"
  local qemu_pid=""

  rm -f "${log_file}" "${qemu_log}"
  "${QEMU_BIN}" \
    -machine q35 \
    -drive "format=raw,file=${DISK_IMAGE},index=0,media=disk,cache=writeback" \
    -display none \
    -serial "file:${log_file}" \
    -monitor none \
    -no-reboot \
    -no-shutdown \
    >"${qemu_log}" 2>&1 &
  qemu_pid=$!

  cleanup_phase() {
    if [[ -n "${qemu_pid}" ]]; then
      kill "${qemu_pid}" >/dev/null 2>&1 || true
      wait "${qemu_pid}" >/dev/null 2>&1 || true
    fi
  }
  trap cleanup_phase RETURN

  for _ in $(seq 1 350); do
    if grep -Fq "[AHCI-VANTFS-TEST] phase=${phase}" "${log_file}" 2>/dev/null; then
      break
    fi
    if grep -Fq "[AHCI-VANTFS-TEST] failed" "${log_file}" 2>/dev/null; then
      echo "AHCI VANTFS ${phase} phase failed" >&2
      tail -n 120 "${log_file}" >&2 || true
      return 1
    fi
    if ! kill -0 "${qemu_pid}" >/dev/null 2>&1; then
      echo "QEMU exited before AHCI VANTFS ${phase} checkpoint" >&2
      cat "${qemu_log}" >&2 || true
      return 1
    fi
    sleep 0.1
  done

  grep -Fq "[AHCI-VANTFS-TEST] phase=${phase}" "${log_file}"
  grep -Fq "type=0x7f" "${log_file}"
  grep -Fq "blocks=2048" "${log_file}"
  grep -Fq "payload_ok=true" "${log_file}"
  if grep -Fq "KERNEL PANIC" "${log_file}"; then
    echo "AHCI VANTFS ${phase} phase failed: kernel panic detected" >&2
    return 1
  fi
  kill "${qemu_pid}" >/dev/null 2>&1 || true
  wait "${qemu_pid}" >/dev/null 2>&1 || true
  qemu_pid=""
  trap - RETURN
}

boot_phase write
boot_phase verify

write_start="$(sed -n 's/.*phase=write .*start=\([0-9]*\).*/\1/p' "${ROOT_DIR}/target/qemu-ahci-vantfs-write.log" | tail -n 1)"
verify_start="$(sed -n 's/.*phase=verify .*start=\([0-9]*\).*/\1/p' "${ROOT_DIR}/target/qemu-ahci-vantfs-verify.log" | tail -n 1)"
if [[ -z "${write_start}" || "${write_start}" != "${verify_start}" ]]; then
  echo "AHCI VANTFS partition start mismatch across reboot" >&2
  exit 1
fi

echo "AHCI VANTFS persistence test passed: MBR + cache + VANTFS survived reboot start=${write_start}"
echo "disk image: ${DISK_IMAGE}"
