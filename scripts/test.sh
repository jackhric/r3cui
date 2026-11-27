#!/bin/sh
# What CI checks, run locally: every crate's tests (the Kodi and Home Assistant clients against their in-process fakes,
# the drivers, the Integrations screens) and clippy with warnings as errors, for the desktop build and the device
# build (its evdev input and KMS display code; CI's Device package job cross-compiles it too). Extra arguments go
# to `cargo test`, e.g. `scripts/test.sh -p kodi`.
set -eu
cd "$(dirname "$0")/.."
cargo test --workspace --locked "$@"
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo clippy -p r3cui --no-default-features --features device --locked -- -D warnings
