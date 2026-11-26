#!/bin/sh
# Run the UI against the simulator with Slint live preview: edits to the .slint files under crates/ui/ui reload in
# the running window. With watchexec installed, Rust edits rebuild and restart it too.
set -eu

here=$(cd "$(dirname "$0")" && pwd)
cd "$here/.."
eval "$("$here/sim.sh" env)"
export SLINT_LIVE_PREVIEW=1
export RUST_LOG=${RUST_LOG:-info}

run="cargo run -p remote-ui --features slint/live-preview"
if command -v watchexec >/dev/null 2>&1; then
    exec watchexec --restart --watch crates --exts rs,toml -- $run
fi
exec $run
