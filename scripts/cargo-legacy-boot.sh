#!/usr/bin/env bash
set -euo pipefail

REAL_CARGO="${VANTARA_REAL_CARGO:-/usr/local/cargo/bin/cargo}"
FILTERED_ARGS=()

while [[ "$#" -gt 0 ]]; do
  case "$1" in
    -Zjson-target-spec)
      shift
      ;;
    -Z)
      if [[ "${2:-}" == "json-target-spec" ]]; then
        shift 2
      else
        FILTERED_ARGS+=("$1")
        shift
      fi
      ;;
    *)
      FILTERED_ARGS+=("$1")
      shift
      ;;
  esac
done

exec "${REAL_CARGO}" "${FILTERED_ARGS[@]}"
