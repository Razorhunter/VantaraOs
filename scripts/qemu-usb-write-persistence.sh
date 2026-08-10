#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
CARGO_TARGET_DIR="${ROOT_DIR}/target/usb-write-test-cargo"
BOOTIMAGE="${CARGO_TARGET_DIR}/x86_64-vantara_os/debug/bootimage-kernel.bin"
USB_IMAGE="${ROOT_DIR}/target/qemu-usb-write-test.img"
QEMU_BIN="${QEMU:-qemu-system-x86_64}"

if ! command -v "${QEMU_BIN}" >/dev/null 2>&1; then
  echo "USB write persistence test skipped: ${QEMU_BIN} not found" >&2
  exit 127
fi

bash "${ROOT_DIR}/scripts/build-kernel-variant.sh" \
  "${CARGO_TARGET_DIR}" --features usb-write-test
rm -f "${USB_IMAGE}"
truncate -s 1M "${USB_IMAGE}"

boot_phase() {
  local phase="$1"
  local log_file="${ROOT_DIR}/target/qemu-usb-write-${phase}.log"
  local qemu_log="${ROOT_DIR}/target/qemu-usb-write-${phase}-qemu.log"
  local qemu_pid=""
  rm -f "${log_file}" "${qemu_log}"
  "${QEMU_BIN}" \
    -machine pc \
    -drive "format=raw,file=${BOOTIMAGE},index=0,media=disk" \
    -device "piix3-usb-uhci,id=uhci" \
    -drive "format=raw,file=${USB_IMAGE},if=none,id=usbdisk,cache=writeback" \
    -device "usb-storage,bus=uhci.0,drive=usbdisk" \
    -display none -serial "file:${log_file}" -monitor none \
    -no-reboot -no-shutdown >"${qemu_log}" 2>&1 &
  qemu_pid=$!
  for _ in $(seq 1 300); do
    if grep -Fq "[USB-WRITE-TEST] phase=${phase}" "${log_file}" 2>/dev/null; then break; fi
    if grep -Fq "[USB-WRITE-TEST] failed" "${log_file}" 2>/dev/null; then
      tail -n 160 "${log_file}" >&2 || true; kill "${qemu_pid}" || true; return 1
    fi
    if ! kill -0 "${qemu_pid}" >/dev/null 2>&1; then cat "${qemu_log}" >&2; return 1; fi
    sleep 0.1
  done
  grep -Fq "[USB-WRITE-TEST] phase=${phase}" "${log_file}"
  grep -Fq "persisted=true flush=true" "${log_file}"
  ! grep -Eiq "kernel panic|EXCEPTION:" "${log_file}"
  kill "${qemu_pid}" >/dev/null 2>&1 || true
  wait "${qemu_pid}" >/dev/null 2>&1 || true
}

boot_phase write
boot_phase verify
write_checksum="$(sed -n 's/.*phase=write .*checksum=\([0-9]*\).*/\1/p' "${ROOT_DIR}/target/qemu-usb-write-write.log" | tail -n 1)"
verify_checksum="$(sed -n 's/.*phase=verify .*checksum=\([0-9]*\).*/\1/p' "${ROOT_DIR}/target/qemu-usb-write-verify.log" | tail -n 1)"
[[ -n "${write_checksum}" && "${write_checksum}" == "${verify_checksum}" ]]
echo "USB write persistence test passed: WRITE(10) + SYNCHRONIZE CACHE survived reboot checksum=${write_checksum}"
