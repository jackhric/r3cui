#!/usr/bin/env bash
# Build the installable custom-UI archive: release.json at root + the binary at bin/remote-ui.
# Usage: pack.sh <path-to-aarch64-remote-ui-binary>
# Output: packaging/out/remote-ui.tar.gz
set -euo pipefail

BIN="${1:?usage: pack.sh <path-to-remote-ui-binary>}"
HERE="$(cd "$(dirname "$0")" && pwd)"
OUT="$HERE/out"
STAGE="$(mktemp -d)"
trap 'rm -rf "$STAGE"' EXIT

mkdir -p "$STAGE/bin" "$OUT"
cp "$HERE/release.json" "$STAGE/release.json"
cp "$BIN" "$STAGE/bin/remote-ui"
chmod +x "$STAGE/bin/remote-ui"

# No symlinks (the installer strips them); keep it a plain tar.
tar -C "$STAGE" -czf "$OUT/remote-ui.tar.gz" release.json bin/remote-ui
echo "wrote $OUT/remote-ui.tar.gz"
echo
echo "Install (⚠ writes permanent void_warranty flag):"
echo "  curl -u 'web-configurator:<PIN>' \\"
echo "    'http://<IP>/api/system/install/ui?void_warranty=yes' \\"
echo "    --form 'file=@\"$OUT/remote-ui.tar.gz\"'"
