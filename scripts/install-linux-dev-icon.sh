#!/usr/bin/env bash
set -euo pipefail

root=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)
data_home=${XDG_DATA_HOME:-"$HOME/.local/share"}

# The GTK Wayland app ID for the dev binary is arcadestr-desktop. Use a
# separate entry so the dev icon doesn't replace the installed app's launcher.
install -Dm644 "$root/desktop/icons/icon.png" \
  "$data_home/icons/hicolor/256x256/apps/arcadestr-desktop.png"
install -d "$data_home/applications"

binary="$root/target/debug/arcadestr-desktop"
# Escape the quoted Exec argument, then the desktop entry's string value.
binary=${binary//\\/\\\\}
binary=${binary//\"/\\\"}
binary=${binary//\`/\\\`}
binary=${binary//\$/\\\$}
binary=${binary//%/%%}
binary=${binary//\\/\\\\}
cat > "$data_home/applications/arcadestr-desktop.desktop" <<EOF
[Desktop Entry]
Type=Application
Name=Arcadestr (Development)
Exec="$binary"
Icon=arcadestr-desktop
NoDisplay=true
Terminal=false
EOF
