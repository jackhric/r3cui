#!/bin/sh
# Cross-compile remote-ui for the Remote 3 (aarch64, glibc 2.33) in a container, against the
# manufacturer's SDK sysroot, then pack the installable archive.
#
# One-time setup: unpack ucr2-aarch64-toolchain-<ver>-noqt.tar.gz (unfoldedcircle/ucr2-toolchain
# releases) so that $UC_SDK/bin/aarch64-none-linux-gnu-gcc exists.
set -eu
# R3CUI_FEATURES adds cargo features to the device build, e.g. R3CUI_FEATURES=demo for the made-up remote and devices.

here=$(cd "$(dirname "$0")" && pwd)
repo=$(cd "$here/.." && pwd)
UC_SDK=${UC_SDK:-$HOME/.cache/ucr3-toolchain/sdk/aarch64-buildroot-linux-gnu_sdk-buildroot}
# A Docker volume name, or a host path so CI can cache the downloaded crates between runs.
CARGO_REGISTRY=${CARGO_REGISTRY:-ucr3-ui-cargo-registry}
# Worked out here, where git is: the container builds as this version and pack.sh names the archive for it. A build
# with extra features says so after the commit, e.g. 0.2.0+g1eb1474.demo.
version=$("$repo/scripts/version.sh")
[ -n "${R3CUI_FEATURES:-}" ] && version="$version$(case $version in *+*) echo . ;; *) echo + ;; esac)$(echo "$R3CUI_FEATURES" | tr ',' '.')"
export R3CUI_VERSION="$version"
R3CUI_COMMIT=$("$repo/scripts/version.sh" --commit)
export R3CUI_COMMIT
[ -x "$UC_SDK/bin/aarch64-none-linux-gnu-gcc" ] || { echo "SDK not found at $UC_SDK (set UC_SDK)" >&2; exit 1; }

docker run --rm \
    -e R3CUI_VERSION -e R3CUI_COMMIT \
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
        cargo build --locked --release -p r3cui --no-default-features --features device'"${R3CUI_FEATURES:+,$R3CUI_FEATURES}"' \
            --target aarch64-unknown-linux-gnu --target-dir /src/target-device
        chown -R '"$(id -u):$(id -g)"' /src/target-device /usr/local/cargo/registry
    '

"$repo/scripts/build-bridge.sh"
"$repo/packaging/pack.sh" "$repo/target-device/aarch64-unknown-linux-gnu/release/remote-ui"
