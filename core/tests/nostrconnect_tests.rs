// Integration tests for NIP-46 nostrconnect:// flow

#[cfg(test)]
mod integration_tests {
    use arcadestr_core::signer::Nip46Signer;

    #[test]
    fn test_nostrconnect_uri_generation() {
        // Test the core URI generation
        let relay = "wss://relay.damus.io";
        let secret = "test_secret_12345";
        let perms = "sign_event:1,sign_event:30078";
        let name = "Arcadestr";

        let result = Nip46Signer::generate_nostrconnect_uri(relay, secret, Some(perms), Some(name));

        assert!(result.is_ok(), "URI generation should succeed");
        let (uri, client_keys) = result.unwrap();

        // Verify URI structure
        assert!(
            uri.starts_with("nostrconnect://"),
            "URI should start with nostrconnect://"
        );

        // Extract and verify components
        let parts: Vec<&str> = uri.split("?").collect();
        assert_eq!(parts.len(), 2, "URI should have query parameters");

        let pubkey_hex = &parts[0][15..]; // Remove "nostrconnect://"
        assert_eq!(pubkey_hex.len(), 64, "Pubkey should be 64 hex chars");

        // Verify query parameters
        let query = parts[1];
        assert!(query.contains("relay="), "Should contain relay parameter");
        assert!(
            query.contains("metadata="),
            "Should contain metadata parameter"
        );
        // Note: secret is handled internally by the library, not in the URI

        // Verify the returned pubkey matches the URI
        assert_eq!(
            client_keys.public_key().to_hex(),
            pubkey_hex,
            "Returned pubkey should match URI"
        );

        println!("Generated URI: {}", uri);
        println!("Client Pubkey: {}", client_keys.public_key().to_hex());
    }

    #[test]
    fn test_nostrconnect_uri_parses_correctly() {
        let relay = "wss://relay.example.com";
        let secret = "my_secret_key";

        let result = Nip46Signer::generate_nostrconnect_uri(relay, secret, None, None).unwrap();

        let (uri, _) = result;

        // Verify we can parse the URI
        // Format: nostrconnect://<pubkey>?relay=<encoded>&secret=<encoded>
        let without_scheme = uri.strip_prefix("nostrconnect://").unwrap();
        let parts: Vec<&str> = without_scheme.split("?").collect();

        assert_eq!(parts.len(), 2);

        let pubkey = parts[0];
        assert_eq!(pubkey.len(), 64);

        // Parse query parameters
        let query = parts[1];
        let params: std::collections::HashMap<_, _> = query
            .split("&")
            .filter_map(|p| {
                let mut kv = p.split("=");
                Some((kv.next()?, kv.next()?))
            })
            .collect();

        assert!(params.contains_key("relay"), "Should have relay parameter");
        assert!(
            params.contains_key("metadata"),
            "Should have metadata parameter"
        );
        // Note: secret is not in the URI - it's generated internally by the library

        // Verify relay is URL encoded
        let encoded_relay = params.get("relay").unwrap();
        let decoded_relay = urlencoding::decode(encoded_relay).unwrap();
        assert_eq!(decoded_relay, relay);
    }

    #[test]
    fn test_multiple_uris_are_unique() {
        let relay = "wss://relay.damus.io";

        let mut pubkeys = std::collections::HashSet::new();
        let mut uris = std::collections::HashSet::new();

        for i in 0..10 {
            let secret = format!("secret_{}", i);
            let result =
                Nip46Signer::generate_nostrconnect_uri(relay, &secret, None, None).unwrap();

            let (uri, client_keys) = result;

            // Each URI should be unique
            assert!(!uris.contains(&uri), "URI {} should be unique", i);
            uris.insert(uri);

            // Each pubkey should be unique
            let pubkey_hex = client_keys.public_key().to_hex();
            assert!(
                !pubkeys.contains(&pubkey_hex),
                "Pubkey {} should be unique",
                i
            );
            pubkeys.insert(pubkey_hex);
        }

        assert_eq!(uris.len(), 10, "Should have 10 unique URIs");
        assert_eq!(pubkeys.len(), 10, "Should have 10 unique pubkeys");
    }
}
