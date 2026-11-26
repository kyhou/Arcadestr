#!/usr/bin/env bash
# Build an exact committed source revision with makepkg in a clean Arch
# container. Never uses working-tree application files.
# Usage: build-arch.sh <commit> <version> <output-directory>
set -euo pipefail

sha="$(git rev-parse "$1^{commit}")"
version="$2"
out="$3"
[[ "$version" =~ ^[0-9]+\.[0-9]+\.[0-9]+$ ]] || exit 1
declared="$(git show "$sha:Cargo.toml" |
  python3 -c 'import sys,tomllib; print(tomllib.loads(sys.stdin.read())["workspace"]["package"]["version"])')"
[[ "$declared" == "$version" ]] || { echo "Source version differs from $version" >&2; exit 1; }

work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT
git archive --format=tar --prefix="arcadestr-$version/" "$sha" |
  gzip -n >"$work/arcadestr-$version.tar.gz"
checksum="$(sha256sum "$work/arcadestr-$version.tar.gz" | cut -d' ' -f1)"
sed -e "s/@VERSION@/$version/g" -e "s/@SHA256@/$checksum/g" \
  packaging/arch/PKGBUILD.in >"$work/PKGBUILD"
mkdir -p "$out"
out="$(realpath "$out")"

# Pin the base image; pacman -Syu supplies a consistent current Arch repository
# snapshot. makepkg must never run as root.
docker run --rm \
  -e HOST_UID="$(id -u)" -e HOST_GID="$(id -g)" \
  -e SOURCE_DATE_EPOCH="$(git show -s --format=%ct "$sha")" \
  -v "$work:/build" -v "$out:/output" \
  archlinux@sha256:949ecb1661609ab7e8f6854ba03efd62792e7a796313738e94b040c095c20d91 \
  bash -euo pipefail -c '
    pacman -Syu --noconfirm --needed cargo rust-wasm trunk clang pkgconf \
      webkit2gtk-4.1 gtk3 libayatana-appindicator xdotool openssl dbus \
      namcap desktop-file-utils python
    useradd --create-home builder
    chown -R builder:builder /build
    cd /build
    runuser -u builder -- makepkg --cleanbuild --noconfirm
    package=(arcadestr-*.pkg.tar.zst)
    [[ ${#package[@]} == 1 ]]
    # namcap may return success even when it reports packaging errors.
    namcap PKGBUILD "${package[0]}" | tee namcap.log
    desktop-file-validate src/arcadestr-*/packaging/arch/arcadestr.desktop
    mkdir verify
    bsdtar -xf "${package[0]}" -C verify
    grep -qx "pkgname = arcadestr" verify/.PKGINFO
    grep -qx "arch = x86_64" verify/.PKGINFO
    # Only the executable, desktop entry, icon, licenses and makepkg metadata belong
    # in the archive: no bundled graphics libraries, keys or user data.
    find verify -type f -printf "%P\n" | LC_ALL=C sort >actual-files
    printf "%s\n" .BUILDINFO .MTREE .PKGINFO \
      usr/bin/arcadestr usr/share/applications/arcadestr.desktop \
      usr/share/licenses/arcadestr/LICENSE-MIT \
      usr/share/licenses/arcadestr/LICENSE-APACHE \
      usr/share/icons/hicolor/256x256/apps/arcadestr.png |
      LC_ALL=C sort >expected-files
    diff -u expected-files actual-files
    python src/arcadestr-*/scripts/release/check-arch.py verify/usr/bin/arcadestr x86_64
    grep -q "^profile=release$" src/arcadestr-*/web/.arcadestr-frontend-build.txt
    if grep -a -q debug-mcp-bridge verify/usr/bin/arcadestr; then exit 1; fi
    readelf -d verify/usr/bin/arcadestr | grep NEEDED
    if grep -q " E: " namcap.log; then
      echo "Arch packaging errors must be resolved before uploading." >&2
      exit 1
    fi
    cp "${package[0]}" /output/
    chown "$HOST_UID:$HOST_GID" "/output/${package[0]}"
    # Restore mount ownership so the host can remove makepkg build products.
    chown -R "$HOST_UID:$HOST_GID" /build
  '
