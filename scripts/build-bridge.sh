#!/bin/sh
# Build the r3cui bridge (a custom integration) as a fully static aarch64 binary in a container, then pack it.
# The integration sandbox may lack a compatible libc, so this is musl, not the SDK's glibc used by build-device.sh.
set -eu

here=$(cd "$(dirname "$0")" && pwd)
repo=$(cd "$here/.." && pwd)
# A Docker volume name, or a host path so CI can cache the downloaded crates between runs.
CARGO_REGISTRY=${CARGO_REGISTRY:-ucr3-ui-cargo-registry}

docker run --rm \
    -v "$repo":/src \
    -v "$CARGO_REGISTRY":/usr/local/cargo/registry \
    -w /src rust:1.100.0-bookworm sh -euc '
        rustup target add aarch64-unknown-linux-musl >/dev/null
        # The bridge is pure Rust and musl ships its own crt, so the toolchain'\''s lld links it without a cross gcc.
        lld=$(find "$(rustc --print sysroot)" -name rust-lld -type f | head -n1)
        export CARGO_TARGET_AARCH64_UNKNOWN_LINUX_MUSL_LINKER=$lld
        cargo build --locked --release -p r3cui-bridge --bin driver \
            --target aarch64-unknown-linux-musl --target-dir /src/target-device
        chown -R '"$(id -u):$(id -g)"' /src/target-device /usr/local/cargo/registry
    '

"$repo/packaging/bridge.sh" "$repo/target-device/aarch64-unknown-linux-musl/release/driver"
