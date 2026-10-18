#!/usr/bin/env bash
# Builds one ELF per Sleep-Floor-Plan.md step 1 measurement into
# target/step1/<name>.elf, so each PPK2 run is just a flash + measure.
# Usage: tools/ppk2/build-step1.sh [name...]   (default: all)
# Needs ~/export-esp.sh (sourced here) and .env with SSID, PASSWORD,
# LOG_SERVER and GBFS_MIRROR.
set -euo pipefail
cd "$(dirname "$0")/../.."
# shellcheck disable=SC1090
source ~/export-esp.sh

FEATURES=use-env,log-server,gbfs-mirror,sleep-trace
ELF=target/xtensa-esp32s3-none-elf/release/bay-wheels-controller
OUT=target/step1

# name: build env vars (empty = today's firmware)
VARIANTS=(
  "baseline:"
  "deep-al5887-standby:DEEP_SLEEP_TEST=1"
  "deep-al5887-shutdown:DEEP_SLEEP_TEST=2"
  "led-idle-1:LED_IDLE=1"
  "led-idle-2:LED_IDLE=2"
  "vddsdio-pd:LED_IDLE=1 SLEEP_VDDSDIO_PD=1"
  "rtc-peri-pd:LED_IDLE=1 SLEEP_RTC_PERI_PD=1"
)

mkdir -p "$OUT"
for v in "${VARIANTS[@]}"; do
  name=${v%%:*}
  vars=${v#*:}
  if [ $# -gt 0 ] && [[ ! " $* " =~ " $name " ]]; then
    continue
  fi
  echo "== $name ${vars:-(defaults)}"
  # option_env! changes rebuild only the crate that reads them.
  env $vars cargo build --release --features "$FEATURES"
  cp "$ELF" "$OUT/$name.elf"
done
ls -l "$OUT"
