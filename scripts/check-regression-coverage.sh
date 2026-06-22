#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"

require_tests() {
  local category="$1"
  local file="$2"
  local minimum="$3"
  local count

  count="$(grep -c '#\[test_case\]' "${ROOT_DIR}/${file}" || true)"
  if (( count < minimum )); then
    echo "regression coverage missing: ${category} has ${count}, expected at least ${minimum}" >&2
    exit 1
  fi
  echo "${category} regression cases: ${count}"
}

require_file() {
  local file="$1"
  if [[ ! -f "${ROOT_DIR}/${file}" ]]; then
    echo "regression coverage missing: ${file}" >&2
    exit 1
  fi
}

require_tests "process" "kernel/src/user/process.rs" 10
require_tests "memory" "kernel/src/memory.rs" 3
require_tests "address-space" "kernel/src/user/address_space.rs" 5
require_tests "syscall" "kernel/src/user/syscall.rs" 10
require_tests "filesystem" "kernel/src/fs.rs" 4

require_file "scripts/qemu-smoke.sh"
require_file "scripts/qemu-boot-integration.sh"
require_file "scripts/qemu-service-manager.sh"
require_file "scripts/qemu-device-namespace.sh"
require_file "scripts/qemu-command-smoke.sh"
require_file "scripts/qemu-filesystem-write.sh"
require_file "scripts/qemu-filesystem-persistence.sh"
require_file "scripts/qemu-preemption.sh"
require_file "scripts/qemu-signal-job-control.sh"
require_file "scripts/qemu-terminal-signals.sh"
require_file "scripts/qemu-fault-isolation.sh"
require_file "scripts/build-initrd.sh"
require_file "scripts/build-dev-package.sh"
require_file "scripts/verify-dev-package.sh"

echo "regression coverage contract passed"
