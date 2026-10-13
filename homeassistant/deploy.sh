#!/usr/bin/env bash
# Copies the bay_wheels_log add-on to Home Assistant's local add-ons folder over
# SSH (HA's SSH add-on has tar but no rsync), then installs or rebuilds it.
#
#   homeassistant/deploy.sh [ssh-target]     (default: root@homeassistant.local)
#
# Bump `version` in bay_wheels_log/config.yaml when changing the add-on so
# Home Assistant picks up the rebuild.
set -euo pipefail

TARGET="${1:-root@homeassistant.local}"
SLUG=bay_wheels_log
APP="local_${SLUG}"
HERE="$(cd "$(dirname "$0")" && pwd)"

# HA OS 16 mounts local add-ons ("apps") at /local_apps; older ones at /addons.
REMOTE_DIR=$(ssh "$TARGET" 'for d in /local_apps /addons; do [ -d "$d" ] && echo "$d" && break; done')
[ -n "$REMOTE_DIR" ] || { echo "no local add-ons folder found on $TARGET" >&2; exit 1; }

echo "Copying $SLUG to $TARGET:$REMOTE_DIR/$SLUG"
COPYFILE_DISABLE=1 tar -C "$HERE" -cf - "$SLUG" \
  | ssh "$TARGET" "rm -rf '$REMOTE_DIR/$SLUG' && tar -C '$REMOTE_DIR' -xf -"

ssh "$TARGET" "ha store reload >/dev/null
# 'info' also succeeds for apps that are only available; installed ones have a version.
if ha apps info $APP 2>/dev/null | grep -qE '^version: [^n]'; then
  echo 'Rebuilding $APP'; ha apps rebuild $APP || ha apps update $APP
else
  echo 'Installing $APP (builds the image on the device; takes a minute)'; ha apps install $APP
fi
ha apps start $APP || true
ha apps info $APP | grep -E '^(state|version):'"

HOST="${TARGET#*@}"
echo "Status page: http://${HOST}:8099/"
