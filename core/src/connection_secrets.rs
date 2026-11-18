//! Keep NIP-46 connection secrets out of labels, metadata and diagnostics.
//!
//! `bunker://` and `nostrconnect://` URIs carry a `secret` query parameter that
//! authorizes a signer connection. The full URI belongs only in the OS
//! credential store (and, for a freshly generated pairing QR code, on screen).

use nostr::{PublicKey, ToBech32};

const REDACTED: &str = "<redacted>";

/// Replace the value of every `secret` query parameter with a marker.
/// Other parameters (relays, names) are preserved for diagnostics.
pub fn redact_connection_uri(value: &str) -> String {
    let Some((head, query)) = value.split_once('?') else {
        return value.to_string();
    };
    let (query, fragment) = match query.split_once('#') {
        Some((query, fragment)) => (query, Some(fragment)),
        None => (query, None),
    };
    let redacted = query
        .split('&')
        .map(|pair| {
            let key = pair.split_once('=').map_or(pair, |(key, _)| key);
            if key.eq_ignore_ascii_case("secret") {
                format!("{key}={REDACTED}")
            } else {
                pair.to_string()
            }
        })
        .collect::<Vec<_>>()
        .join("&");
    match fragment {
        Some(fragment) => format!("{head}?{redacted}#{fragment}"),
        None => format!("{head}?{redacted}"),
    }
}

/// Origin and path only, for URLs whose query may carry bearer material
/// (e.g. remote-signer approval links).
pub fn redact_url_query(value: &str) -> String {
    match value.split_once(['?', '#']) {
        Some((head, _)) => format!("{head}?{REDACTED}"),
        None => value.to_string(),
    }
}

/// Whether a stored account label could disclose connection material.
pub fn is_unsafe_account_label(label: &str) -> bool {
    let lower = label.to_ascii_lowercase();
    lower.contains("://") || lower.contains("secret=") || lower.contains("nsec1")
}

/// Safe default label for a remote-signer account.
pub fn default_remote_signer_name(user_pubkey: &PublicKey) -> String {
    let npub = user_pubkey
        .to_bech32()
        .unwrap_or_else(|_| user_pubkey.to_hex());
    let short = if npub.len() > 16 {
        format!("{}…{}", &npub[..12], &npub[npub.len() - 4..])
    } else {
        npub
    };
    format!("Remote signer {short}")
}

/// Replace an unsafe or empty label; keep safe user-chosen names unchanged.
pub fn sanitize_account_label(label: &str, user_pubkey_hex: &str) -> String {
    if !label.trim().is_empty() && !is_unsafe_account_label(label) {
        return label.to_string();
    }
    match PublicKey::from_hex(user_pubkey_hex) {
        Ok(pubkey) => default_remote_signer_name(&pubkey),
        Err(_) => "Remote signer".to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const BUNKER: &str = "bunker://8c9ffe47f0989a057ae81deb0fa24e7f07c19f78f9eeeb37f5e0e7b61858b96e?relay=ws%3A%2F%2F127.0.0.1%3A17801&secret=tuYFWYPuzagg";

    #[test]
    fn secret_values_are_removed_and_other_parameters_kept() {
        let redacted = redact_connection_uri(BUNKER);
        assert!(!redacted.contains("tuYFWYPuzagg"));
        assert!(redacted.contains("secret=<redacted>"));
        assert!(redacted.contains("relay=ws%3A%2F%2F127.0.0.1%3A17801"));
        let upper = redact_connection_uri("nostrconnect://abc?SECRET=x1&relay=wss://r#frag");
        assert!(!upper.contains("x1"));
        assert!(upper.ends_with("#frag"));
        assert_eq!(redact_connection_uri("bob@nsec.app"), "bob@nsec.app");
    }

    #[test]
    fn approval_url_queries_are_dropped() {
        assert_eq!(
            redact_url_query("https://nsec.app/approve?token=abc#x"),
            "https://nsec.app/approve?<redacted>"
        );
        assert_eq!(redact_url_query("https://a.example/"), "https://a.example/");
    }

    #[test]
    fn unsafe_labels_are_replaced_with_safe_default() {
        let keys = nostr::Keys::generate();
        let hex = keys.public_key().to_hex();
        let label = sanitize_account_label(BUNKER, &hex);
        assert!(label.starts_with("Remote signer npub1"));
        assert!(!label.contains("secret"));
        assert_eq!(sanitize_account_label("", &hex), label);
        assert_eq!(sanitize_account_label("Alice phone", &hex), "Alice phone");
        assert_eq!(sanitize_account_label(BUNKER, "not-hex"), "Remote signer");
        assert!(is_unsafe_account_label("nostrconnect://x?secret=y"));
    }
}
