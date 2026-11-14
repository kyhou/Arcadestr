//! Stock relay roles matching the local Amethyst checkout.
//! General defaults: commons/.../defaults/{AmethystDefaults,Constants}.kt.
//! QR login defaults: commonsUI/.../account/ui/login/LoginViewModel.kt.
//! Keep signer rendezvous relays separate from feed/profile discovery.

/// Amethyst's DefaultNIP65RelaySet, used when an account has no known relay list.
pub const DEFAULT_RELAYS: &[&str] = &[
    "wss://nostr.mom",
    "wss://nos.lol",
    "wss://nostr.bitcoiner.social",
];

/// Amethyst's Constants.bootstrapInbox.
pub const DISCOVERY_RELAYS: &[&str] = &[
    "wss://relay.primal.net",
    "wss://nostr.mom",
    "wss://nos.lol",
    "wss://nostr.bitcoiner.social",
    "wss://nostr.oxtr.dev",
    "wss://directory.yabu.me",
];

/// Amethyst's DefaultIndexerRelayList.
pub const INDEXER_RELAYS: &[&str] = &[
    "wss://purplepag.es",
    "wss://indexer.coracle.social",
    "wss://user.kindpag.es",
    "wss://directory.yabu.me",
    "wss://profiles.nostr1.com",
    "wss://search.brainstorm.world",
];

/// Amethyst's DEFAULT_NOSTR_CONNECT_RELAYS (also used by Amber and Opal).
pub const PRIMARY_NIP46_RELAY: &str = "wss://relay.nip46.com";

pub const DEFAULT_NIP46_RELAYS: &[&str] = &[
    PRIMARY_NIP46_RELAY,
    "wss://nrs.primal.net",
    "wss://bucket.coracle.social",
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stock_relay_roles_are_valid_unique_secure_websocket_urls() {
        for relays in [
            DEFAULT_RELAYS,
            DISCOVERY_RELAYS,
            INDEXER_RELAYS,
            DEFAULT_NIP46_RELAYS,
        ] {
            assert!(!relays.is_empty());
            let mut seen = std::collections::HashSet::new();
            for relay in relays {
                let parsed = url::Url::parse(relay).expect("valid stock relay URL");
                assert_eq!(parsed.scheme(), "wss");
                assert!(parsed.host_str().is_some());
                assert!(parsed.username().is_empty() && parsed.password().is_none());
                assert!(parsed.query().is_none() && parsed.fragment().is_none());
                assert!(seen.insert(*relay), "duplicate relay: {relay}");
            }
        }
    }

    #[test]
    fn signer_defaults_match_amethyst_login_defaults() {
        assert_eq!(
            DEFAULT_NIP46_RELAYS,
            [
                "wss://relay.nip46.com",
                "wss://nrs.primal.net",
                "wss://bucket.coracle.social",
            ]
        );
    }
}
