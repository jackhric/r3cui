#!/usr/bin/env bash
# Build the installable custom-integration archive: driver.json at root, the binary at bin/driver.
# Usage: bridge.sh <path-to-static-aarch64-driver-binary>
# Output: packaging/out/bridge/r3cui-bridge.tar.gz, which pack.sh carries inside r3cui's own archive.
set -euo pipefail

BIN="${1:?usage: bridge.sh <path-to-driver-binary>}"
HERE="$(cd "$(dirname "$0")" && pwd)"
OUT="$HERE/out/bridge"
STAGE="$(mktemp -d)"
trap 'rm -rf "$STAGE"' EXIT

mkdir -p "$STAGE/bin" "$OUT"
cp "$HERE/../crates/bridge/driver.json" "$STAGE/driver.json"
cp "$BIN" "$STAGE/bin/driver"
chmod +x "$STAGE/bin/driver"

tar -C "$STAGE" -czf "$OUT/r3cui-bridge.tar.gz" driver.json bin/driver
echo "wrote $OUT/r3cui-bridge.tar.gz"
echo
echo "Install (custom integrations do not set the warranty flag):"
echo "  curl -u 'web-configurator:<PIN>' \\"
echo "    'http://<IP>/api/intg/install' \\"
echo "    --form 'file=@\"$OUT/r3cui-bridge.tar.gz\"'"
