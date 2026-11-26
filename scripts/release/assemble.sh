#!/usr/bin/env bash
# Verifies that <dir> holds exactly the expected release assets for
# <version>, writes and checks <dir>/SHA256SUMS, and writes release notes for
# <tag> at <sha> to <notes-file>. Run from a full-history checkout.
#
# Usage: assemble.sh <version> <tag> <sha> <dir> <notes-file>
set -euo pipefail

version="$1"
tag="$2"
sha="$3"
dir="$4"
notes="$5"

expected=(
  "Arcadestr-$version-linux-x86_64.AppImage"
  "Arcadestr-$version-linux-x86_64.deb"
  "Arcadestr-$version-linux-x86_64.rpm"
  "Arcadestr-$version-macos-arm64.dmg"
  "Arcadestr-$version-macos-x86_64.dmg"
  "Arcadestr-$version-windows-x86_64-setup.exe"
  "Arcadestr-$version-windows-x86_64.msi"
)

actual="$(cd "$dir" && find . -mindepth 1 -maxdepth 1 -printf '%P\n' | LC_ALL=C sort)"
wanted="$(printf '%s\n' "${expected[@]}" | LC_ALL=C sort)"
if [[ "$actual" != "$wanted" ]]; then
  echo "::error::release assets differ from the expected set" >&2
  diff <(echo "$wanted") <(echo "$actual") >&2 || true
  exit 1
fi
for asset in "${expected[@]}"; do
  [[ -s "$dir/$asset" ]] || { echo "::error::$asset is empty" >&2; exit 1; }
done

(cd "$dir" && LC_ALL=C sha256sum -- "${expected[@]}" >SHA256SUMS && sha256sum --check --strict SHA256SUMS)

previous="$(git describe --tags --abbrev=0 --match 'v[0-9]*.[0-9]*.[0-9]*' "$sha^" 2>/dev/null || true)"
{
  echo "Arcadestr $tag, built from \`$sha\`."
  echo
  echo "This is a pre-release. The installers are **not code-signed**; see"
  echo "[installing a release](https://github.com/kyhou/Arcadestr/blob/$sha/docs/releasing.md#installing-a-release)"
  echo "for the Windows and macOS warnings and how to verify \`SHA256SUMS\`."
  echo
  if [[ -n "$previous" ]]; then
    echo "## Changes since $previous"
    echo
    git log --no-merges --format='- %s (%h)' "$previous..$sha"
  else
    echo "## Changes"
    echo
    echo "First release; earlier history is not itemized."
  fi
} >"$notes"
echo "Assembled ${#expected[@]} assets for $tag; notes in $notes"
