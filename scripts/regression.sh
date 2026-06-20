#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"

make -C "${ROOT_DIR}" fmt
make -C "${ROOT_DIR}" check
bash "${ROOT_DIR}/scripts/check-regression-coverage.sh"
bash "${ROOT_DIR}/scripts/check-unsafe-audit.sh"
make -C "${ROOT_DIR}" kernel-build
bash "${ROOT_DIR}/scripts/qemu-smoke.sh"
bash "${ROOT_DIR}/scripts/qemu-boot-integration.sh"
bash "${ROOT_DIR}/scripts/qemu-command-smoke.sh"
bash "${ROOT_DIR}/scripts/qemu-preemption.sh"
bash "${ROOT_DIR}/scripts/qemu-signal-job-control.sh"
bash "${ROOT_DIR}/scripts/qemu-terminal-signals.sh"
bash "${ROOT_DIR}/scripts/qemu-fault-isolation.sh"
bash "${ROOT_DIR}/scripts/qemu-kernel-thread-preemption.sh"

echo "Vantara regression suite passed: process + memory + syscall + filesystem + boot"
