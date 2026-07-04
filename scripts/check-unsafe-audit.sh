#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
BASELINE="${ROOT_DIR}/scripts/unsafe-baseline.tsv"
CURRENT="$(mktemp)"

cleanup() {
  rm -f "${CURRENT}"
}
trap cleanup EXIT

while IFS= read -r file; do
  count="$(
    {
      grep -Eo 'unsafe[[:space:]]*(\{|fn|impl|extern)|#\[unsafe' "${ROOT_DIR}/${file}" ||
        true
    } | wc -l
  )"
  if (( count > 0 )); then
    printf '%s\t%s\n' "${file}" "${count}"
  fi
done < <(find "${ROOT_DIR}/kernel/src" -type f -name '*.rs' -printf '%P\n' |
  sed 's#^#kernel/src/#' | LC_ALL=C sort) >"${CURRENT}"

if ! cmp -s "${BASELINE}" "${CURRENT}"; then
  echo "unsafe audit baseline changed; review every delta and update safety notes" >&2
  diff -u "${BASELINE}" "${CURRENT}" >&2 || true
  exit 1
fi

for file in \
  kernel/src/allocator.rs \
  kernel/src/drivers/ahci.rs \
  kernel/src/drivers/nvme.rs \
  kernel/src/memory.rs \
  kernel/src/user/address_space.rs \
  kernel/src/user/ring3.rs \
  kernel/src/user/syscall.rs; do
  if ! grep -Fq 'SAFETY:' "${ROOT_DIR}/${file}"; then
    echo "unsafe audit missing SAFETY notes in ${file}" >&2
    exit 1
  fi
done

grep -Fq '#![deny(unsafe_op_in_unsafe_fn)]' "${ROOT_DIR}/kernel/src/lib.rs"
grep -Fq '#![deny(unsafe_op_in_unsafe_fn)]' "${ROOT_DIR}/kernel/src/main.rs"

echo "unsafe audit passed: $(awk '{sum += $2} END {print sum}' "${CURRENT}") boundaries tracked"
