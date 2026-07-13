#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
BOOTIMAGE="${ROOT_DIR}/target/kernel/x86_64-vantara_os/debug/bootimage-kernel.bin"
GOLDEN_DIR="${ROOT_DIR}/scripts/golden"
QEMU_BIN="${QEMU:-qemu-system-x86_64}"
CASE="${1:-all}"

CASES=(ls cat procs rusthello threaddemo pipedemo eventdemo msgdemo jobdemo udpdemo tcpdemo)

if ! command -v "${QEMU_BIN}" >/dev/null 2>&1; then
  echo "command smoke test skipped: ${QEMU_BIN} not found" >&2
  exit 127
fi

if [[ ! -f "${BOOTIMAGE}" ]]; then
  make -C "${ROOT_DIR}" kernel-build
fi

case_command() {
  case "$1" in
    ls) echo "ls" ;;
    cat) echo "cat /MOTD" ;;
    procs) echo "procs" ;;
    rusthello) echo "rusthello" ;;
    threaddemo) echo "threaddemo" ;;
    pipedemo) echo "pipedemo" ;;
    eventdemo) echo "eventdemo" ;;
    msgdemo) echo "msgdemo" ;;
    jobdemo) echo "jobdemo" ;;
    udpdemo) echo "udpdemo" ;;
    tcpdemo) echo "tcpdemo" ;;
    *)
      echo "unknown command smoke case: $1" >&2
      return 2
      ;;
  esac
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

wait_for_log() {
  local log_file="$1"
  local qemu_pid="$2"
  local pattern="$3"
  local start_byte="${4:-0}"
  local attempts="${5:-150}"

  for ((attempt = 0; attempt < attempts; attempt++)); do
    if grep -Fq "${pattern}" < <(
      tail -c "+$((start_byte + 1))" "${log_file}" 2>/dev/null
    ); then
      return 0
    fi
    if ! kill -0 "${qemu_pid}" >/dev/null 2>&1; then
      echo "QEMU exited before checkpoint: ${pattern}" >&2
      return 1
    fi
    sleep 0.1
  done

  echo "timeout waiting for checkpoint: ${pattern}" >&2
  echo "--- serial log tail ---" >&2
  tail -n 50 "${log_file}" >&2 || true
  return 1
}

check_golden() {
  local output_file="$1"
  local golden_file="$2"
  local remaining_file
  local pattern

  remaining_file="$(mktemp "${ROOT_DIR}/target/qemu-golden.XXXXXX")"
  cp "${output_file}" "${remaining_file}"

  while IFS= read -r pattern || [[ -n "${pattern}" ]]; do
    [[ -z "${pattern}" || "${pattern}" == \#* ]] && continue

    local line
    line="$(grep -nF -m1 "${pattern}" "${remaining_file}" || true)"
    if [[ -z "${line}" ]]; then
      echo "golden output mismatch: missing '${pattern}'" >&2
      echo "golden file: ${golden_file}" >&2
      echo "--- command output ---" >&2
      cat "${output_file}" >&2
      rm -f "${remaining_file}"
      return 1
    fi

    local line_number="${line%%:*}"
    tail -n "+$((line_number + 1))" "${remaining_file}" >"${remaining_file}.next"
    mv "${remaining_file}.next" "${remaining_file}"
  done <"${golden_file}"

  rm -f "${remaining_file}"
}

run_case() (
  local name="$1"
  local command
  local log_file="${ROOT_DIR}/target/qemu-command-${name}.log"
  local output_file="${ROOT_DIR}/target/qemu-command-${name}.output"
  local monitor_log="${ROOT_DIR}/target/qemu-command-${name}-monitor.log"
  local monitor_fifo="${ROOT_DIR}/target/qemu-command-${name}-monitor.fifo"
  local screen_file="${ROOT_DIR}/target/qemu-command-${name}.ppm"
  local golden_file="${GOLDEN_DIR}/${name}.golden"
  local qemu_pid=""
  local command_start
  local -a network_args=()

  if [[ "${name}" == "udpdemo" || "${name}" == "tcpdemo" ]]; then
    network_args=(
      -netdev "hubport,id=net0,hubid=0"
      -device "e1000,netdev=net0,mac=52:54:00:12:34:56"
      -netdev "hubport,id=net1,hubid=0"
      -device "e1000,netdev=net1,mac=52:54:00:12:34:57"
    )
  fi

  command="$(case_command "${name}")"
  mkdir -p "${ROOT_DIR}/target"
  rm -f "${log_file}" "${output_file}" "${screen_file}" "${monitor_log}" "${monitor_fifo}"
  mkfifo "${monitor_fifo}"
  exec 3<>"${monitor_fifo}"

  cleanup_case() {
    if [[ -n "${qemu_pid}" ]]; then
      kill "${qemu_pid}" >/dev/null 2>&1 || true
      wait "${qemu_pid}" >/dev/null 2>&1 || true
    fi
    exec 3>&- || true
    rm -f "${monitor_fifo}"
  }
  trap cleanup_case EXIT

  "${QEMU_BIN}" \
    -drive "format=raw,file=${BOOTIMAGE}" \
    "${network_args[@]}" \
    -display none \
    -serial "file:${log_file}" \
    -monitor stdio \
    -no-reboot \
    -no-shutdown \
    <"${monitor_fifo}" >"${monitor_log}" 2>&1 &
  qemu_pid=$!

  wait_for_log "${log_file}" "${qemu_pid}" "login: "
  send_text "root"
  wait_for_log "${log_file}" "${qemu_pid}" "Welcome to Vantara OS, root"
  wait_for_log "${log_file}" "${qemu_pid}" 'root:/$ '

  command_start="$(wc -c <"${log_file}")"
  send_text "${command}"
  wait_for_log "${log_file}" "${qemu_pid}" 'root:/$ ' "${command_start}"

  tail -c "+$((command_start + 1))" "${log_file}" >"${output_file}"

  if grep -Fq "KERNEL PANIC" "${log_file}"; then
    echo "${name} command smoke test failed: kernel panic detected" >&2
    return 1
  fi

  check_golden "${output_file}" "${golden_file}"

  printf 'screendump %s\n' "${screen_file}" >&3
  for ((attempt = 0; attempt < 50; attempt++)); do
    [[ -s "${screen_file}" ]] && break
    sleep 0.1
  done
  if [[ ! -s "${screen_file}" ]]; then
    echo "${name} command smoke test failed: VGA screendump was not created" >&2
    return 1
  fi

  printf 'quit\n' >&3
  wait "${qemu_pid}" >/dev/null 2>&1 || true
  qemu_pid=""

  echo "${name} command smoke test passed"
  echo "serial log: ${log_file}"
  echo "command output: ${output_file}"
  echo "VGA screenshot: ${screen_file}"
)

if [[ "${CASE}" == "all" ]]; then
  for name in "${CASES[@]}"; do
    run_case "${name}"
  done
else
  run_case "${CASE}"
fi
