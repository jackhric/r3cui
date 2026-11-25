#!/bin/sh
# What CI checks, run locally: every crate's tests (the Kodi and Home Assistant clients against their in-process fakes,
# the drivers, the Integrations screens) and clippy with warnings as errors. Extra arguments go to `cargo test`,
# e.g. `scripts/test.sh -p drivers`.
set -eu
cd "$(dirname "$0")/.."
cargo test --workspace --locked "$@"
cargo clippy --workspace --all-targets --locked -- -D warnings
