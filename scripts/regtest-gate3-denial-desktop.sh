#!/usr/bin/env bash
# Private Secret Service prevents isolated GUI pairing from changing real keyring state.
set -euo pipefail
umask 077
root=${1:?private denial run directory required}
[[ -d "$root" && -f "$root/keyring-password" ]]
export HOME="$root/desktop-home"
export XDG_DATA_HOME="$root/desktop-data"
export XDG_CONFIG_HOME="$root/desktop-config"
export XDG_CACHE_HOME="$root/desktop-cache"
export ARCADESTR_REGTEST_ALLOW_LOOPBACK_HTTP=true
export ARCADESTR_RELAYS=ws://127.0.0.1:7778
export ARCADESTR_BLOCK_DISCOVERY=true
export ARCADESTR_REGTEST_NIP46_PROBE=true
unset ARCADESTR_REGTEST_NWC_RELAY_PROXY
export ARCADESTR_DENIAL_ROOT="$root"
exec dbus-run-session -- bash -c '
    set -euo pipefail
    gnome-keyring-daemon --unlock --components=secrets < "$ARCADESTR_DENIAL_ROOT/keyring-password" > "$ARCADESTR_DENIAL_ROOT/private-keyring.log" 2>&1
    exec tauri-driver --port 4446 --native-port 4447 --native-driver /usr/bin/WebKitWebDriver
'
