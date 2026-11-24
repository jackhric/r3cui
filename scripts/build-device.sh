#!/bin/sh
# Cross-compile remote-ui for the Remote 3 (aarch64, glibc 2.33) in a container, against the
# manufacturer's SDK sysroot, then pack the installable archive.
#
# One-time setup: unpack ucr2-aarch64-toolchain-<ver>-noqt.tar.gz (unfoldedcircle/ucr2-toolchain
# releases) so that $UC_SDK/bin/aarch64-none-linux-gnu-gcc exists.
set -eu

here=$(cd "$(dirname "$0")" && pwd)
repo=$(cd "$here/.." && pwd)
UC_SDK=${UC_SDK:-$HOME/.cache/ucr3-toolchain/sdk/aarch64-buildroot-linux-gnu_sdk-buildroot}
# A Docker volume name, or a host path so CI can cache the downloaded crates between runs.
CARGO_REGISTRY=${CARGO_REGISTRY:-ucr3-ui-cargo-registry}
[ -x "$UC_SDK/bin/aarch64-none-linux-gnu-gcc" ] || { echo "SDK not found at $UC_SDK (set UC_SDK)" >&2; exit 1; }

docker run --rm \
    -v "$repo":/src -v "$UC_SDK":/sdk \
    -v "$CARGO_REGISTRY":/usr/local/cargo/registry \
    -w /src rust:1.100.0-bookworm sh -euc '
        apt-get update -qq && apt-get install -y -qq --no-install-recommends pkg-config clang libclang-dev >/dev/null
        rustup target add aarch64-unknown-linux-gnu >/dev/null
        /sdk/relocate-sdk.sh >/dev/null
        sysroot=/sdk/aarch64-buildroot-linux-gnu/sysroot
        export CARGO_TARGET_AARCH64_UNKNOWN_LINUX_GNU_LINKER=/sdk/bin/aarch64-none-linux-gnu-gcc
        export CC_aarch64_unknown_linux_gnu=/sdk/bin/aarch64-none-linux-gnu-gcc
        export CXX_aarch64_unknown_linux_gnu=/sdk/bin/aarch64-none-linux-gnu-g++
        export AR_aarch64_unknown_linux_gnu=/sdk/bin/aarch64-none-linux-gnu-ar
        export PKG_CONFIG_ALLOW_CROSS=1 PKG_CONFIG_SYSROOT_DIR=$sysroot
        export PKG_CONFIG_LIBDIR=$sysroot/usr/lib/pkgconfig:$sysroot/usr/share/pkgconfig
        export BINDGEN_EXTRA_CLANG_ARGS="--sysroot=$sysroot"
        cargo build --locked --release -p remote-ui --no-default-features --features device \
            --target aarch64-unknown-linux-gnu --target-dir /src/target-device
        chown -R '"$(id -u):$(id -g)"' /src/target-device /usr/local/cargo/registry
    '

"$repo/packaging/pack.sh" "$repo/target-device/aarch64-unknown-linux-gnu/release/remote-ui"
