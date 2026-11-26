#!/usr/bin/env bash
# Copies the Tauri bundles for one platform out of target/release/bundle into
# <out-dir> under stable, versioned release names, and fails unless exactly
# one bundle of each expected kind exists.
#
# Usage: collect-bundles.sh <platform> <version> <out-dir>
#   platform: linux-x86_64 | windows-x86_64 | macos-x86_64 | macos-arm64
set -euo pipefail

platform="$1"
version="$2"
out="$3"
bundle_dir="target/release/bundle"
prefix="Arcadestr-$version-$platform"

case "$platform" in
  linux-x86_64)
    expected=("appimage:AppImage:$prefix.AppImage" "deb:deb:$prefix.deb" "rpm:rpm:$prefix.rpm")
    ;;
  windows-x86_64)
    expected=("msi:msi:$prefix.msi" "nsis:exe:$prefix-setup.exe")
    ;;
  macos-x86_64 | macos-arm64)
    expected=("dmg:dmg:$prefix.dmg")
    ;;
  *)
    echo "::error::unknown platform $platform" >&2
    exit 1
    ;;
esac

mkdir -p "$out"
for entry in "${expected[@]}"; do
  IFS=: read -r kind extension name <<<"$entry"
  shopt -s nullglob
  matches=("$bundle_dir/$kind"/*."$extension")
  shopt -u nullglob
  if ((${#matches[@]} != 1)); then
    echo "::error::expected exactly one $kind bundle (*.$extension), found ${#matches[@]}: ${matches[*]:-none}" >&2
    exit 1
  fi
  [[ "$(basename "${matches[0]}")" == *"$version"* ]] || {
    echo "::error::${matches[0]} is not for version $version" >&2
    exit 1
  }
  cp "${matches[0]}" "$out/$name"
  echo "$kind: ${matches[0]} -> $out/$name"
done
