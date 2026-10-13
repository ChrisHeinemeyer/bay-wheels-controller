#!/usr/bin/env bash
# Cargo runner (see .cargo/config.toml): archives the ELF under the same
# version string the firmware embeds and reports (`git describe`), so a
# panic backtrace from the field can be symbolized later with
# scripts/symbolize.py, then flashes and attaches with probe-rs.
set -euo pipefail

elf="$1"
root="$(cd "$(dirname "$0")/.." && pwd)"
version="$(git -C "$root" describe --always --dirty --tags 2>/dev/null || echo unknown)"
mkdir -p "$root/firmware-archive"
cp "$elf" "$root/firmware-archive/$version.elf"
echo "Archived ELF as firmware-archive/$version.elf" >&2

exec probe-rs run --chip=esp32s3 --preverify --always-print-stacktrace --no-location \
  --catch-hardfault "$@"
