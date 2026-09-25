#!/bin/bash
# clear_arcadestr_data.sh - Wipe all locally stored Arcadestr data.
#
# Removes databases, the encryption master key, settings, saved users,
# installed games, retained download packages, caches, and OS-keyring
# secrets so the app starts from a clean state.
#
# Quit Arcadestr before running this script.

set -u

DATA_DIR="${XDG_DATA_HOME:-$HOME/.local/share}/arcadestr"
TAURI_DIR="${XDG_DATA_HOME:-$HOME/.local/share}/com.arcadestr.app"
CACHE_DIR="${XDG_CACHE_HOME:-$HOME/.cache}/arcadestr"
CONFIG_DIR="${XDG_CONFIG_HOME:-$HOME/.config}/arcadestr"

KEYRING_SERVICES=("arcadestr-auth" "arcadestr-nip49" "arcadestr-nwc")

DRY_RUN=0
FORCE=0

usage() {
    cat <<'EOF'
Usage: clear_arcadestr_data.sh [--dry-run] [--force]

Clears all locally stored Arcadestr data. Quit the app first.

What is cleared:
  1. App data dir       ~/.local/share/arcadestr         (databases, master key, settings, caches)
  2. Tauri local data   ~/.local/share/com.arcadestr.app (installed games, retained packages, artifacts, webview cache)
  3. Cache dir          ~/.cache/arcadestr               (archive selections)
  4. Config dir         ~/.config/arcadestr              (legacy, if present)
  5. OS keyring         services arcadestr-auth / -nip49 / -nwc (signer secrets, NWC URI)

Raw paths honor XDG_DATA_HOME, XDG_CACHE_HOME and XDG_CONFIG_HOME.

Options:
  --dry-run   Only list what exists; delete nothing.
  --force     Skip the confirmation prompt.
  --help      Show this help.
EOF
}

for arg in "$@"; do
    case "$arg" in
        --dry-run) DRY_RUN=1 ;;
        --force|-f) FORCE=1 ;;
        --help|-h) usage; exit 0 ;;
        *) echo "Unknown option: $arg"; echo; usage; exit 1 ;;
    esac
done

show_size() {
    du -sh "$1" 2>/dev/null | awk '{print $1}' || echo "?"
}

dir_contents() {
    ls -A "$1" 2>/dev/null | head -n 30
}

found_any=0

check_dir() {
    local path="$1" label="$2"
    if [ -e "$path" ]; then
        found_any=1
        echo "[$label] $path"
        echo "  size: $(show_size "$path")"
        dir_contents "$path" | sed 's/^/  /'
    else
        echo "[$label] $path (absent)"
    fi
    echo
}

echo "=========================================="
echo "Arcadestr Data Cleanup"
echo "=========================================="
if [ "$DRY_RUN" -eq 1 ]; then
    echo "Dry run: nothing will be deleted."
fi
echo ""

echo "Step 1: App data dir (databases, master key, settings)"
echo "-------------------------------------------------------"
check_dir "$DATA_DIR" "app-data"

echo "Step 2: Tauri local data (installed games, packages, webview cache)"
echo "-------------------------------------------------------------------"
check_dir "$TAURI_DIR" "tauri-local-data"

echo "Step 3: Cache dir (archive selections)"
echo "---------------------------------------"
check_dir "$CACHE_DIR" "cache"

echo "Step 4: Config dir (legacy)"
echo "----------------------------"
check_dir "$CONFIG_DIR" "config"

echo "Step 5: OS keyring secrets"
echo "--------------------------"
if command -v secret-tool >/dev/null 2>&1; then
    for service in "${KEYRING_SERVICES[@]}"; do
        labels=$(secret-tool search --all service "$service" 2>/dev/null | grep '^label')
        if [ -n "$labels" ]; then
            found_any=1
            echo "[$service]"
            echo "$labels" | sed 's/^label = /  /'
        else
            echo "[$service] no entries"
        fi
    done
else
    echo "  ! 'secret-tool' not found (install libsecret-tools)."
    echo "    Signer secrets and the NWC URI will NOT be cleared."
fi
echo ""

if [ "$found_any" -eq 0 ]; then
    echo "No Arcadestr data found on this system. Nothing to do."
    exit 0
fi

if [ "$DRY_RUN" -eq 1 ]; then
    echo "Dry run complete. Re-run without --dry-run to delete the above."
    exit 0
fi

if [ "$FORCE" -eq 0 ]; then
    printf "This will permanently delete the data listed above. Continue? [y/N] "
    read -r answer
    case "$answer" in
        y|Y|yes|YES) ;;
        *) echo "Aborted."; exit 1 ;;
    esac
fi

delete_dir() {
    local path="$1" label="$2"
    if [ -e "$path" ]; then
        rm -rf "$path"
        echo "  ✓ Removed [$label]: $path"
    else
        echo "  - Skipped [$label] (absent): $path"
    fi
}

echo "Deleting..."
echo ""
delete_dir "$DATA_DIR" "app-data"
delete_dir "$TAURI_DIR" "tauri-local-data"
delete_dir "$CACHE_DIR" "cache"
delete_dir "$CONFIG_DIR" "config"

if command -v secret-tool >/dev/null 2>&1; then
    for service in "${KEYRING_SERVICES[@]}"; do
        count=$(secret-tool search --all service "$service" 2>/dev/null | grep -c '^\[/')
        if [ "$count" -gt 0 ]; then
            yes 2>/dev/null | secret-tool clear service "$service" >/dev/null 2>&1 \
                && echo "  ✓ Cleared keyring service: $service ($count entries)" \
                || echo "  ! Failed to clear keyring service: $service"
        fi
    done
else
    echo "  ! secret-tool unavailable; keyring entries were not cleared."
fi

echo ""
echo "=========================================="
echo "Cleanup complete. Arcadestr now starts from a clean state."
echo "=========================================="