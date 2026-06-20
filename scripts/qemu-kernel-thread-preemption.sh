#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
CARGO_TARGET_DIR="${ROOT_DIR}/target/kernel-thread-preemption-cargo"
BOOTIMAGE="${CARGO_TARGET_DIR}/x86_64-vantara_os/debug/bootimage-kernel.bin"
LOG_FILE="${ROOT_DIR}/target/qemu-kernel-thread-preemption.log"
QEMU_BIN="${QEMU:-qemu-system-x86_64}"

mkdir -p "${ROOT_DIR}/target"

if ! command -v "${QEMU_BIN}" >/dev/null 2>&1; then
  echo "kernel-thread preemption test skipped: ${QEMU_BIN} not found" >&2
  exit 127
fi

(
  cd "${ROOT_DIR}/kernel"
  CARGO_TARGET_DIR="${CARGO_TARGET_DIR}" \
    cargo bootimage --features kernel-thread-preemption-test
)

rm -f "${LOG_FILE}"
timeout 20s "${QEMU_BIN}" \
  -drive "format=raw,file=${BOOTIMAGE}" \
  -display none \
  -serial "file:${LOG_FILE}" \
  -no-reboot \
  -no-shutdown &

QEMU_PID=$!
for _ in $(seq 1 150); do
  if grep -Fq "[KTHREAD] resume cycle passed" "${LOG_FILE}" 2>/dev/null; then
    break
  fi
  if grep -Eiq "kernel panic|EXCEPTION:" "${LOG_FILE}" 2>/dev/null; then
    break
  fi
  sleep 0.1
done
kill "${QEMU_PID}" >/dev/null 2>&1 || true
wait "${QEMU_PID}" >/dev/null 2>&1 || true

grep -Fq "[KTHREAD] starting timer-driven Ring-0 switch test" "${LOG_FILE}"
grep -Fq "[KTHREAD] A started" "${LOG_FILE}"
grep -Fq "[KTHREAD] preemption guard passed" "${LOG_FILE}"
grep -Fq "[KTHREAD] B started" "${LOG_FILE}"
grep -Fq "[KTHREAD] C started" "${LOG_FILE}"
grep -Fq "[KTHREAD] resume cycle passed" "${LOG_FILE}"

if grep -Eiq "kernel panic|EXCEPTION:" "${LOG_FILE}"; then
  echo "kernel-thread preemption test failed: kernel panic detected" >&2
  exit 1
fi

echo "kernel-thread preemption test passed: Ring-0 tasks were involuntarily switched"
echo "serial log: ${LOG_FILE}"
