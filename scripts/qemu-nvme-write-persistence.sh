#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
CARGO_TARGET_DIR="${ROOT_DIR}/target/nvme-write-test-cargo"
BOOTIMAGE="${CARGO_TARGET_DIR}/x86_64-vantara_os/debug/bootimage-kernel.bin"
NVME_IMAGE="${ROOT_DIR}/target/qemu-nvme-write-test.img"
QEMU_BIN="${QEMU:-qemu-system-x86_64}"

if ! command -v "${QEMU_BIN}" >/dev/null 2>&1; then
  echo "NVMe write persistence test skipped: ${QEMU_BIN} not found" >&2
  exit 127
fi

mkdir -p "${ROOT_DIR}/target"
(
  cd "${ROOT_DIR}/kernel"
  CARGO_TARGET_DIR="${CARGO_TARGET_DIR}" cargo bootimage --features nvme-write-test
)

rm -f "${NVME_IMAGE}"
truncate -s 1M "${NVME_IMAGE}"
printf 'VANTNVME' | dd of="${NVME_IMAGE}" conv=notrunc status=none

boot_phase() {
  local phase="$1"
  local log_file="${ROOT_DIR}/target/qemu-nvme-write-${phase}.log"
  local qemu_log="${ROOT_DIR}/target/qemu-nvme-write-${phase}-qemu.log"
  local qemu_pid=""

  rm -f "${log_file}" "${qemu_log}"
  "${QEMU_BIN}" \
    -machine q35 \
    -drive "format=raw,file=${BOOTIMAGE},index=0,media=disk" \
    -drive "format=raw,file=${NVME_IMAGE},if=none,id=nvme0,cache=writeback" \
    -device "nvme,drive=nvme0,serial=VANTARA0002" \
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

  for _ in $(seq 1 300); do
    if grep -Fq "[NVME-WRITE-TEST] phase=${phase}" "${log_file}" 2>/dev/null; then
      break
    fi
    if grep -Fq "[NVME-WRITE-TEST] failed" "${log_file}" 2>/dev/null; then
      echo "NVMe write ${phase} phase failed" >&2
      tail -n 120 "${log_file}" >&2 || true
      return 1
    fi
    if ! kill -0 "${qemu_pid}" >/dev/null 2>&1; then
      echo "QEMU exited before NVMe write ${phase} checkpoint" >&2
      cat "${qemu_log}" >&2 || true
      return 1
    fi
    sleep 0.1
  done

  grep -Fq "[NVME-WRITE-TEST] phase=${phase}" "${log_file}"
  grep -Fq "persisted=true flush=true" "${log_file}"
  if grep -Fq "KERNEL PANIC" "${log_file}"; then
    echo "NVMe write ${phase} phase failed: kernel panic detected" >&2
    return 1
  fi
  kill "${qemu_pid}" >/dev/null 2>&1 || true
  wait "${qemu_pid}" >/dev/null 2>&1 || true
  qemu_pid=""
  trap - RETURN
}

boot_phase write
boot_phase verify

write_checksum="$(sed -n 's/.*phase=write .*checksum=\([0-9]*\).*/\1/p' "${ROOT_DIR}/target/qemu-nvme-write-write.log" | tail -n 1)"
verify_checksum="$(sed -n 's/.*phase=verify .*checksum=\([0-9]*\).*/\1/p' "${ROOT_DIR}/target/qemu-nvme-write-verify.log" | tail -n 1)"
if [[ -z "${write_checksum}" || "${write_checksum}" != "${verify_checksum}" ]]; then
  echo "NVMe write persistence checksum mismatch" >&2
  exit 1
fi

echo "NVMe write persistence test passed: NVM write + flush survived reboot checksum=${write_checksum}"
echo "disk image: ${NVME_IMAGE}"
