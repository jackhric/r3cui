#!/usr/bin/env bash
# Build the installable custom-UI archive: release.json at root, the binary at bin/remote-ui, settings in config/, and the
# the r3cui bridge's own archive at bin/r3cui-bridge.tar.gz for System → Voice to install.
# Usage: pack.sh <path-to-aarch64-remote-ui-binary>
# Output: packaging/out/r3cui-<version>.tar.gz, the version from scripts/version.sh (R3CUI_VERSION wins), also written
# into the archive's release.json.
set -euo pipefail

BIN="${1:?usage: pack.sh <path-to-remote-ui-binary>}"
HERE="$(cd "$(dirname "$0")" && pwd)"
OUT="$HERE/out"
VERSION="$("$HERE/../scripts/version.sh")"
ARCHIVE="$OUT/r3cui-$VERSION.tar.gz"
STAGE="$(mktemp -d)"
trap 'rm -rf "$STAGE"' EXIT

mkdir -p "$STAGE/bin" "$OUT"
sed -e "s/\"version\": \"[^\"]*\"/\"version\": \"$VERSION\"/" -e "s/\"release_date\": \"[^\"]*\"/\"release_date\": \"$(date -u +%F)\"/" "$HERE/release.json" > "$STAGE/release.json"
grep -q "\"version\": \"$VERSION\"" "$STAGE/release.json" || { echo "release.json has no version to set" >&2; exit 1; }
cp "$BIN" "$STAGE/bin/remote-ui"
chmod +x "$STAGE/bin/remote-ui"
BRIDGE="$OUT/bridge/r3cui-bridge.tar.gz"
[ -f "$BRIDGE" ] || { echo "no $BRIDGE: run scripts/build-bridge.sh first" >&2; exit 1; }
cp "$BRIDGE" "$STAGE/bin/r3cui-bridge.tar.gz"

# The remote exposes this directory to the UI as UC_CONFIG_HOME: settings.json, and modules.json when there is one
# (the drivers' setup; a Home Assistant token is pasted into it, so the archive is then private). Copied without
# keeping times, so a newer archive's modules.json takes over from the one the remote saved.
cp -r "$HERE/../config" "$STAGE/config"
[ -f "$STAGE/config/modules.json" ] && echo "including config/modules.json: it may hold tokens, keep the archive private"

# No symlinks (the installer strips them); keep it a plain tar.
# One archive in out/ at a time, so the newest is the only one to pick up.
rm -f "$OUT"/r3cui-*.tar.gz
tar -C "$STAGE" -czf "$ARCHIVE" release.json bin/remote-ui bin/r3cui-bridge.tar.gz config
echo "wrote $ARCHIVE"
echo
echo "Install (⚠ writes permanent void_warranty flag):"
echo "  curl -u 'web-configurator:<PIN>' \\"
echo "    'http://<IP>/api/system/install/ui?void_warranty=yes' \\"
echo "    --form 'file=@\"$ARCHIVE\"'"
