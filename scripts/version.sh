#!/bin/sh
# The version this checkout builds as, in semver: the workspace version for its release (the commit tagged
# v<version>), otherwise the commit added as build metadata, e.g. 0.2.0+g1eb1474. R3CUI_VERSION, when set, wins: the
# release workflow names its build so. Used by the app's build script (what it shows and reports) and by pack.sh (the
# archive's name and its release.json).
set -eu
[ -n "${R3CUI_VERSION:-}" ] && { echo "$R3CUI_VERSION"; exit 0; }
repo=$(cd "$(dirname "$0")/.." && pwd)
version=$(sed -n '/^\[workspace\.package\]/,/^\[/s/^version = "\(.*\)"$/\1/p' "$repo/Cargo.toml")
[ -n "$version" ] || { echo "no workspace version in $repo/Cargo.toml" >&2; exit 1; }
if git -C "$repo" describe --exact-match --tags --match "v$version" HEAD >/dev/null 2>&1; then
  echo "$version"
elif sha=$(git -C "$repo" rev-parse --short=7 HEAD 2>/dev/null); then
  echo "$version+g$sha"
else
  echo "$version+unknown"
fi
