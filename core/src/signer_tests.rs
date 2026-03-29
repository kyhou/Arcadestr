// Tests for NIP-46 nostrconnect:// URI generation

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_generate_nostrconnect_uri_basic() {
        let relay = "wss://relay.damus.io";
        let secret = "test_secret_123";

        let result = Nip46Signer::generate_nostrconnect_uri(relay, secret, None, None);

        assert!(result.is_ok());
        let (uri, pubkey) = result.unwrap();

        // Check URI starts with nostrconnect://
        assert!(uri.starts_with("nostrconnect://"));

        // Check URI contains the relay
        assert!(uri.contains("relay=wss%3A%2F%2Frelay.damus.io"));

        // Check URI contains the secret
        assert!(uri.contains("secret=test_secret_123"));

        // Check pubkey is valid
        assert!(!pubkey.to_hex().is_empty());
    }

    #[test]
    fn test_generate_nostrconnect_uri_with_perms() {
        let relay = "wss://relay.nostr.band";
        let secret = "secret456";
        let perms = "sign_event:1,sign_event:30078";

        let result = Nip46Signer::generate_nostrconnect_uri(relay, secret, Some(perms), None);

        assert!(result.is_ok());
        let (uri, _) = result.unwrap();

        // Check URI contains permissions
        assert!(uri.contains("perms="));
        assert!(uri.contains("sign_event"));
    }

    #[test]
    fn test_generate_nostrconnect_uri_with_name() {
        let relay = "wss://relay.example.com";
        let secret = "secret789";
        let name = "TestApp";

        let result = Nip46Signer::generate_nostrconnect_uri(relay, secret, None, Some(name));

        assert!(result.is_ok());
        let (uri, _) = result.unwrap();

        // Check URI contains app name
        assert!(uri.contains("name=TestApp"));
    }

    #[test]
    fn test_generate_nostrconnect_uri_url_encoding() {
        let relay = "wss://relay with spaces.com";
        let secret = "secret with spaces";

        let result = Nip46Signer::generate_nostrconnect_uri(relay, secret, None, None);

        assert!(result.is_ok());
        let (uri, _) = result.unwrap();

        // Check that spaces are URL encoded
        assert!(!uri.contains(" "));
        assert!(uri.contains("%20") || uri.contains("+"));
    }

    #[test]
    fn test_generate_nostrconnect_uri_unique_secrets() {
        let relay = "wss://relay.damus.io";

        // Generate multiple URIs and ensure they're different
        let mut uris = Vec::new();
        for i in 0..5 {
            let secret = format!("secret_{}", i);
            let result =
                Nip46Signer::generate_nostrconnect_uri(relay, &secret, None, None).unwrap();
            uris.push(result.0);
        }

        // Check all URIs are unique
        let unique_uris: std::collections::HashSet<_> = uris.iter().collect();
        assert_eq!(uris.len(), unique_uris.len());
    }

    #[test]
    fn test_nostrconnect_uri_format() {
        let relay = "wss://relay.damus.io";
        let secret = "test123";

        let result = Nip46Signer::generate_nostrconnect_uri(
            relay,
            secret,
            Some("sign_event:1"),
            Some("Arcadestr"),
        );

        assert!(result.is_ok());
        let (uri, _) = result.unwrap();

        // Parse the URI to verify format
        // nostrconnect://<pubkey>?relay=<encoded>&secret=<encoded>&perms=<encoded>&name=<encoded>
        let parts: Vec<&str> = uri.split("?").collect();
        assert_eq!(parts.len(), 2);

        let scheme_and_pubkey = parts[0];
        assert!(scheme_and_pubkey.starts_with("nostrconnect://"));

        let pubkey_hex = &scheme_and_pubkey[15..]; // Remove "nostrconnect://"
        assert_eq!(pubkey_hex.len(), 64); // Hex pubkey is 64 chars

        // Verify all query parameters are present
        let query = parts[1];
        assert!(query.contains("relay="));
        assert!(query.contains("secret="));
        assert!(query.contains("perms="));
        assert!(query.contains("name="));
    }
}
