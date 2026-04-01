## Rust Guidelines Enforcement

Follow the Microsoft Rust Guidelines:
- https://microsoft.github.io/rust-guidelines/guidelines/index.html

Core rules (must comply):
- Naming conventions
- Error handling patterns (Result, no unwrap in production)
- Ownership and borrowing correctness
- Unsafe usage restrictions
- API design consistency

If a change violates these, reject or rewrite it.

### Protocol Layer ('nostr/')

Each NIP is a standalone 'object':
- Condensed NIP reference docs at '.claude/nips/*.md' with index at '.claude/nips/README.md'

### Relay Layer ('relay/')

- 'Relay' — single WebSocket connection via OkHttp
- 'RelayPool' — connection pooling with persistent/ephemeral split
- 'OutboxRouter' — outbox/inbox routing per NIP-65
- 'RelayScoreBoard' — tracks relay reliability and author coverage
- 'SubscriptionManager' — REQ subscription lifecycle

### Repository Layer ('repo/')

- 'EventRepository' — LRU cache (5,000 events), profile parsing, reaction/repost/zap tracking
- 'ContactRepository' — follow list with SharedPreferences persistence
- 'KeyRepository' — EncryptedSharedPreferences for private keys
- 'DmRepository' — conversation caching with ECDH key cache

## Code Conventions

- Default relays: 'wss://relay.damus.io', 'wss://relay.primal.net'

## Crypto Stack

- **Signing**: secp256k1 (Schnorr) using native bindings (secp256k1 crate or libsecp256k1)
- **NIP-44 encryption**: ECDH + HKDF + XChaCha20-Poly1305 (xchacha20poly1305, hkdf, hmac, sha2 or libsodium)
- **Key storage**: libsecret (Secret Service API) or AES-256-GCM with Argon2-derived key
