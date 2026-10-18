#!/usr/bin/env bash
# Build, then flash the board while it is powered from the PPK2.
#
# Light sleep drops the USB PHY, so probe-rs can only reach the board in the
# 20 s no-sleep window after reset. The PPK2 power-cycles the board to open that
# window (no GPIO2 press needed), probe-rs flashes and exits, and a second power
# cycle boots the new firmware with nothing attached.
#
#   tools/ppk2/flash.sh                          # features: use-env,log-server
#   FETCH_GAP_S=30 tools/ppk2/flash.sh
#   FEATURES=use-env tools/ppk2/flash.sh
#
# FETCH_GAP_S is read at build time (see src/tasks/fetch.rs). The USB cable must be connected for flashing, but the
# board must be powered by the PPK2 only.
set -euo pipefail

root="$(cd "$(dirname "$0")/../.." && pwd)"
cd "$root"
# shellcheck source=/dev/null
. "$HOME/export-esp.sh"

features="${FEATURES:-use-env,log-server}"
elf="target/xtensa-esp32s3-none-elf/release/bay-wheels-controller"
ppk2() { uv run --quiet tools/ppk2/ppk2.py "$@"; }

cargo build --release --features "$features"

version="$(git describe --always --dirty --tags 2>/dev/null || echo unknown)"
mkdir -p firmware-archive
cp "$elf" "firmware-archive/$version.elf"
echo "Archived ELF as firmware-archive/$version.elf" >&2

ppk2 cycle
for _ in $(seq 40); do
  probe-rs list 2>/dev/null | grep -qi "esp" && break
  sleep 0.25
done
probe-rs download --chip esp32s3 --preverify "$elf"
ppk2 cycle
echo "Flashed $version (FETCH_GAP_S=${FETCH_GAP_S:-120})" >&2
