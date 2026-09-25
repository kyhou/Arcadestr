#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
WORK="$ROOT/work"
OUT="$ROOT/generated"

rm -rf "$WORK" "$OUT"
mkdir -p "$WORK" "$OUT"

normalize_mtimes() {
  find "$1" -exec touch -h -t 202601010000.00 {} +
}

# Nested Windows-style package with an executable candidate and realistic data.
mkdir -p "$WORK/windows/bin" "$WORK/windows/data/maps" "$WORK/windows/data/audio"
printf 'MZ\x90\x00Arcadestr A3.6 fixture\r\n' > "$WORK/windows/bin/Game.exe"
printf '{"renderer":"vulkan","language":"en"}\n' > "$WORK/windows/data/config.json"
printf 'training-map\n' > "$WORK/windows/data/maps/tutorial.dat"
printf 'fixture-audio\n' > "$WORK/windows/data/audio/theme.dat"
dd if=/dev/zero of="$WORK/windows/data/assets.pack" bs=1M count=8 status=none
normalize_mtimes "$WORK/windows"
(cd "$WORK/windows" && zip -0 -q -X -r "$OUT/windows-nested.zip" .)

# Linux-style tar.zst; preserve a genuine executable bit and nested resources.
mkdir -p "$WORK/linux/bin" "$WORK/linux/share/game/levels" "$WORK/linux/share/game/config"
printf '#!/bin/sh\nexit 0\n' > "$WORK/linux/bin/game"
chmod 0755 "$WORK/linux/bin/game"
printf 'level-one\n' > "$WORK/linux/share/game/levels/one.dat"
printf 'fullscreen=true\n' > "$WORK/linux/share/game/config/default.ini"
tar --sort=name --mtime='2026-01-01 UTC' --owner=0 --group=0 --numeric-owner --zstd \
  -cf "$OUT/linux-layout.tar.zst" -C "$WORK/linux" bin share

# Blocking path-rule fixture: Windows reserved device name.
mkdir -p "$WORK/hazard/bin" "$WORK/hazard/data"
printf 'MZ hazardous fixture\r\n' > "$WORK/hazard/bin/Game.exe"
printf 'reserved device entry\n' > "$WORK/hazard/data/CON"
normalize_mtimes "$WORK/hazard"
(cd "$WORK/hazard" && zip -q -X -r "$OUT/reserved-device.zip" .)

# Case-fold collision fixture.
mkdir -p "$WORK/collision/bin"
printf 'MZ upper fixture\r\n' > "$WORK/collision/bin/Game.exe"
printf 'MZ lower fixture\r\n' > "$WORK/collision/bin/game.exe"
normalize_mtimes "$WORK/collision"
(cd "$WORK/collision" && zip -q -X -r "$OUT/case-collision.zip" .)

# Buyer-side extraction hazards that cannot be represented by normal host paths.
OUT="$OUT" python3 - <<'PY'
import os
import zipfile

out = os.environ["OUT"]

def info(name, mode=0o100644):
    entry = zipfile.ZipInfo(name, date_time=(2026, 1, 1, 0, 0, 0))
    entry.create_system = 3
    entry.external_attr = mode << 16
    entry.compress_type = zipfile.ZIP_DEFLATED
    return entry

def archive(name, entries):
    with zipfile.ZipFile(os.path.join(out, name), "w", strict_timestamps=True) as writer:
        for entry_name, payload, mode in entries:
            writer.writestr(info(entry_name, mode), payload, compresslevel=9)

archive("escaping-symlink.zip", [
    ("escape-link", b"../../outside", 0o120777),
])
archive("traversal.zip", [("../outside", b"escape", 0o100644)])
archive("absolute-path.zip", [("/outside", b"escape", 0o100644)])
archive("backslash-name.zip", [("dir\\outside", b"escape", 0o100644)])
archive("ratio-bomb.zip", [
    ("ratio.bin", b"\0" * (64 * 1024 * 1024 + 1024 * 1024), 0o100644),
])
PY

# GNU tar records the second inode reference as a hardlink entry.
mkdir -p "$WORK/hardlink"
printf 'hardlink target\n' > "$WORK/hardlink/target"
ln "$WORK/hardlink/target" "$WORK/hardlink/game-hardlink"
normalize_mtimes "$WORK/hardlink"
tar --sort=name --mtime='2026-01-01 UTC' --owner=0 --group=0 --numeric-owner --zstd \
  -cf "$OUT/hardlink.tar.zst" -C "$WORK/hardlink" game-hardlink target

# Large searchable index with several realistic top-level paths and 5,000 assets.
mkdir -p "$WORK/large/bin" "$WORK/large/assets/maps" "$WORK/large/assets/shaders"
printf 'MZ large-index fixture\r\n' > "$WORK/large/bin/Game.exe"
printf 'shader-main\n' > "$WORK/large/assets/shaders/main.spv"
for index in $(seq -w 1 5000); do
  printf 'asset-%s\n' "$index" > "$WORK/large/assets/maps/map-${index}.dat"
done
normalize_mtimes "$WORK/large"
(cd "$WORK/large" && zip -q -X -r "$OUT/large-index-5000.zip" .)

(cd "$OUT" && sha256sum ./*.zip ./*.tar.zst > "$ROOT/SHA256SUMS")
printf 'Generated fixtures in %s\n' "$OUT"
