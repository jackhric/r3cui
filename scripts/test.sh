#!/bin/sh
# What CI checks, run locally: every crate's tests (the Kodi and Home Assistant clients against their in-process fakes,
# the drivers, the Integrations screens) and clippy with warnings as errors, for the desktop build and the device
# build (its evdev input and KMS display code; CI's Device package job cross-compiles it too). Extra arguments go
# to `cargo test`, e.g. `scripts/test.sh -p drivers`.
set -eu
cd "$(dirname "$0")/.."
cargo test --workspace --locked "$@"
# Nothing depends on androidtv's `testing` feature until its driver lands, so its fake-TV tests are run here.
cargo test -p androidtv --features testing --locked
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo clippy -p androidtv --all-targets --features testing --locked -- -D warnings
cargo clippy -p remote-ui --no-default-features --features device --locked -- -D warnings
