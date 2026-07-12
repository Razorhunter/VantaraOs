#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
BOOTIMAGE="${ROOT_DIR}/target/kernel/x86_64-vantara_os/debug/bootimage-kernel.bin"
LOG_FILE="${ROOT_DIR}/target/qemu-network-rx.log"
QEMU_LOG="${ROOT_DIR}/target/qemu-network-rx-qemu.log"
QEMU_BIN="${QEMU:-qemu-system-x86_64}"
QEMU_PID=""

mkdir -p "${ROOT_DIR}/target"

cleanup() {
  if [[ -n "${QEMU_PID}" ]]; then
    kill "${QEMU_PID}" >/dev/null 2>&1 || true
    wait "${QEMU_PID}" >/dev/null 2>&1 || true
  fi
}
trap cleanup EXIT

if ! command -v "${QEMU_BIN}" >/dev/null 2>&1; then
  echo "network RX test skipped: ${QEMU_BIN} not found" >&2
  exit 127
fi

rm -f "${LOG_FILE}" "${QEMU_LOG}"
"${QEMU_BIN}" \
  -drive "format=raw,file=${BOOTIMAGE}" \
  -netdev "hubport,id=net0,hubid=0" \
  -device "e1000,netdev=net0,mac=52:54:00:12:34:56" \
  -netdev "hubport,id=net1,hubid=0" \
  -device "e1000,netdev=net1,mac=52:54:00:12:34:57" \
  -display none \
  -serial "file:${LOG_FILE}" \
  -monitor none \
  -no-reboot \
  -no-shutdown \
  >"${QEMU_LOG}" 2>&1 &
QEMU_PID=$!

wait_for_log() {
  local pattern="$1"
  for _ in $(seq 1 300); do
    if grep -Fq "${pattern}" "${LOG_FILE}" 2>/dev/null; then
      return 0
    fi
    if ! kill -0 "${QEMU_PID}" >/dev/null 2>&1; then
      echo "QEMU exited before network RX checkpoint: ${pattern}" >&2
      cat "${QEMU_LOG}" >&2 || true
      return 1
    fi
    sleep 0.1
  done
  echo "timeout waiting for network RX checkpoint: ${pattern}" >&2
  tail -n 120 "${LOG_FILE}" >&2 || true
  return 1
}

wait_for_log "[NET] detected 2 network device(s)"
wait_for_log "ip=10.0.2.15 arp_requests=0 arp_replies=1 arp_resolved=true ipv4_packets=0"
wait_for_log "udp_send_ok=true udp_socket_delivered=false"
wait_for_log "protocol=ipv4 ip=10.0.2.16 arp_requests=1 arp_replies=1 arp_resolved=true ipv4_packets=1 ipv4_checksum=true ipv4_source=10.0.2.15 ipv4_destination=10.0.2.16 ipv4_protocol=17 udp_packets=1 udp_checksum=true udp_source_port=40000 udp_destination_port=7777 udp_send_ok=false udp_socket_delivered=true"

if grep -Fq "KERNEL PANIC" "${LOG_FILE}"; then
  echo "network RX test failed: kernel panic detected" >&2
  exit 1
fi

kill "${QEMU_PID}" >/dev/null 2>&1 || true
wait "${QEMU_PID}" >/dev/null 2>&1 || true
QEMU_PID=""

echo "network UDP send/receive socket test passed: checksummed datagram delivered to bound port 7777"
echo "serial log: ${LOG_FILE}"
