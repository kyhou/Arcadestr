#!/bin/bash
# clear_arcadestr_data.sh - One-time cleanup script for Arcadestr
# 
# This script clears all saved profile data from the system keyring and local cache.
# Use this when:
# - You want to start fresh with a clean state
# - Profile data is corrupted or inconsistent
# - The app is restoring the wrong profile on startup
# - You want to switch to a different Nostr account

echo "=========================================="
echo "Arcadestr Data Cleanup Script"
echo "=========================================="
echo ""

# Function to clear a secret-tool entry
clear_keyring_entry() {
    local key="$1"
    if secret-tool clear arcadestr "$key" 2>/dev/null; then
        echo "  ✓ Cleared: arcadestr/$key"
        return 0
    else
        echo "  ✗ Not found or failed: arcadestr/$key"
        return 1
    fi
}

echo "Step 1: Clearing keyring entries..."
echo "----------------------------------------"

# Clear the last active profile ID
clear_keyring_entry "last_active_profile"

# Clear the profile index
clear_keyring_entry "profile_index"

# Find and clear all arcadestr entries
if command -v secret-tool &> /dev/null; then
    echo ""
    echo "Searching for all arcadestr entries..."
    
    # Get all unique arcadestr keys
    keys=$(secret-tool search --all arcadestr 2>/dev/null | grep -o 'arcadestr_[^=]*' | sort -u)
    
    if [ -n "$keys" ]; then
        echo "Found entries to clear:"
        for key in $keys; do
            # Extract just the key name after "arcadestr_"
            key_name="${key#arcadestr_}"
            clear_keyring_entry "$key_name"
        done
    else
        echo "No additional arcadestr entries found in keyring."
    fi
else
    echo "WARNING: secret-tool not found. Keyring entries may not be cleared."
    echo "Install libsecret-tools package if you want to clear keyring entries."
fi

echo ""
echo "Step 2: Clearing local cache..."
echo "----------------------------------------"

# Clear local cache directory
CACHE_DIR="$HOME/.local/share/arcadestr"
if [ -d "$CACHE_DIR" ]; then
    echo "Removing cache directory: $CACHE_DIR"
    rm -rf "$CACHE_DIR"
    echo "  ✓ Cache directory removed"
else
    echo "  ✗ Cache directory not found: $CACHE_DIR"
fi

# Also check for the old cache location (if any)
OLD_CACHE_DIR="$HOME/.cache/arcadestr"
if [ -d "$OLD_CACHE_DIR" ]; then
    echo "Removing old cache directory: $OLD_CACHE_DIR"
    rm -rf "$OLD_CACHE_DIR"
    echo "  ✓ Old cache directory removed"
fi

echo ""
echo "Step 3: Clearing config files..."
echo "----------------------------------------"

# Clear config directory if it exists
CONFIG_DIR="$HOME/.config/arcadestr"
if [ -d "$CONFIG_DIR" ]; then
    echo "Removing config directory: $CONFIG_DIR"
    rm -rf "$CONFIG_DIR"
    echo "  ✓ Config directory removed"
else
    echo "  ✗ Config directory not found: $CONFIG_DIR"
fi

echo ""
echo "=========================================="
echo "Cleanup Complete!"
echo "=========================================="
echo ""
echo "All Arcadestr data has been cleared."
echo ""
echo "Next steps:"
echo "1. Restart Arcadestr"
echo "2. Login with your bunker URI or NIP-05 identifier"
echo "3. The app will create a fresh profile"
echo ""
echo "Your npub: npub1vcq8nv3lctr8ctk2dp7h3e0su4f7gklgx4dlm2375l6u69hvuh6syj3d9l"
echo "Expected hex: 660079b23fc2c67c2eca687d78e5f0e553e45be8355bfdaa3ea7f5cd16ece5f5"
echo ""
