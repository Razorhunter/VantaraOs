#!/usr/bin/env bash
set -euo pipefail

TARGET_SPEC="${1:?target spec path required}"

ABI="$(sed -n 's/.*"rustc-abi": "\([^"]*\)".*/\1/p' "${TARGET_SPEC}" | head -n 1)"
if [[ "${ABI}" != "x86-softfloat" && "${ABI}" != "softfloat" ]]; then
  echo "unsupported generated rustc ABI for bootloader patch: ${ABI:-<missing>}" >&2
  exit 1
fi

LEGACY_WIDTH_FIELDS=0
if grep -Eq '"target-pointer-width"[[:space:]]*:[[:space:]]*"[0-9]+"' "${TARGET_SPEC}"; then
  LEGACY_WIDTH_FIELDS=1
fi

CARGO_HOME_DIR="${CARGO_HOME:-${HOME}/.cargo}"
if [[ ! -d "${CARGO_HOME_DIR}" && -d /usr/local/cargo ]]; then
  CARGO_HOME_DIR="/usr/local/cargo"
fi

patched=0
while IFS= read -r bootloader_target; do
  target_patched=0
  if ! grep -Fq "\"rustc-abi\": \"${ABI}\"" "${bootloader_target}"; then
    perl -pi -e "s/\"rustc-abi\": \"[^\"]+\"/\"rustc-abi\": \"${ABI}\"/" \
      "${bootloader_target}"
    target_patched=1
  fi
  if [[ "${LEGACY_WIDTH_FIELDS}" -eq 1 ]] && \
    grep -Eq '"target-(pointer-width|c-int-width)"[[:space:]]*:[[:space:]]*[0-9]+' "${bootloader_target}"; then
    perl -pi -e 's/("target-(?:pointer-width|c-int-width)"\s*:\s*)(\d+)/$1"$2"/g' \
      "${bootloader_target}"
    target_patched=1
  fi
  if [[ "${target_patched}" -eq 1 ]]; then
    echo "patched bootloader target: ${bootloader_target} (rustc-abi=${ABI}, legacy-width-fields=${LEGACY_WIDTH_FIELDS})"
    patched=1
  fi
done < <(find "${CARGO_HOME_DIR}" -path '*/bootloader-*/x86_64-bootloader.json' 2>/dev/null)

if [[ "${patched}" -eq 0 ]]; then
  echo "bootloader target ABI patch not needed or bootloader source not cached yet"
fi
