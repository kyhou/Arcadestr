# Task Context: NIP-46 Handshake & State Machine Implementation

Session ID: 2025-04-01-nip46-signer
Created: 2025-04-01
Status: completed

## Current Request
Update NIP-46 authentication to use Nip46Signer from nostr-sdk with explicit state machine and 60-second timeout.

## Context Files (Standards to Follow)
- .opencode/context/core/standards/code-quality.md
- .opencode/nips/46-remote-signing.md
- NOSTRCONNECT_IMPLEMENTATION.md

## Reference Files (Source Material)
- core/src/nip46/auth.rs
- core/src/nip46/session.rs
- core/src/nip46/types.rs
- core/src/nip46/storage.rs (already updated)

## Components
1. ✅ types.rs - Updated AppSignerState.active_client to Option<nostr_sdk::Client>
2. ✅ auth.rs - Updated init_signer_session to use NostrConnect with 60s timeout and return Client
3. ✅ session.rs - Updated activate_profile to use NostrConnect with 60s timeout and build Client

## Constraints
- Use NIP-44 only (already handled by nostr-connect crate)
- Wrap NostrConnect in Arc before passing to Client
- Timeout must be exactly 60 seconds
- No fallback relay URLs
- Do not change function signatures unless necessary

## Exit Criteria
- [x] types.rs updated with correct Client type
- [x] auth.rs uses NostrConnect with 60s timeout and builds Client
- [x] session.rs builds Client with Arc<NostrConnect>
- [x] Code compiles without errors
