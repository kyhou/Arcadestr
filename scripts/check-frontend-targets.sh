#!/usr/bin/env bash
# Compile both WASM modes and the supported native desktop. No app is launched.
set -euo pipefail
cd "$(dirname "${BASH_SOURCE[0]}")/.."

cargo check -j 1 -p arcadestr-app --target wasm32-unknown-unknown --features web --locked
cargo check -j 1 -p arcadestr-web --target wasm32-unknown-unknown --features web --locked
cargo check -j 1 -p arcadestr-app -p arcadestr-web --target wasm32-unknown-unknown --locked
cargo check -j 1 -p arcadestr-desktop --locked
