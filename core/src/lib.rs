// Core business logic: NOSTR events, Lightning payments, signer integration.

// Signer is needed for both native and WASM targets
pub mod signers;

// Auth and storage are native-only (require sqlx, encryption, etc.)
#[cfg(feature = "native")]
pub mod achievements;

#[cfg(feature = "native")]
pub mod auth;

#[cfg(feature = "native")]
pub mod storage;

// NIP-46 remote signing module (native-only, uses OS keychain)
#[cfg(feature = "native")]
pub mod nip46;

pub mod saved_users;
pub mod version;

#[cfg(feature = "native")]
pub mod relay_cache;

#[cfg(feature = "native")]
pub mod relay_hints;

#[cfg(feature = "native")]
pub mod relay_pool;

#[cfg(feature = "native")]
pub mod relay_manager;

#[cfg(feature = "native")]
pub mod relay_events;

#[cfg(feature = "native")]
pub use relay_cache::{CachedRelayList, RelayCache, RelayCacheError, RelayHealth, RelayType};

#[cfg(feature = "native")]
pub use relay_pool::{RelayPool, RelaySource};

#[cfg(feature = "native")]
pub use relay_manager::{
    RelayManager, RelayManagerConfig, RelayManagerError, RelaySendResult, SendEventResult,
};

#[cfg(feature = "native")]
pub mod nostr;

#[cfg(feature = "native")]
pub mod subscriptions;

#[cfg(feature = "native")]
pub use subscriptions::{
    cleanup_view_subscriptions, close_subscriptions, dispatch_ephemeral_read,
    dispatch_ephemeral_reads_batch, dispatch_permanent_subscriptions, run_notification_loop,
    ConnectionKind, SerializableEvent, SubscriptionRegistry,
};

#[cfg(feature = "native")]
pub mod profile_fetcher;

#[cfg(feature = "native")]
pub mod nip05_validator;

#[cfg(feature = "native")]
pub mod http_client;

#[cfg(feature = "native")]
pub mod nip98_client;

#[cfg(feature = "native")]
pub mod adp_client;
#[cfg(feature = "native")]
pub mod adp_discovery;

#[cfg(feature = "native")]
pub mod lnurlp;

#[cfg(feature = "native")]
pub mod file_hash;

#[cfg(feature = "native")]
pub mod adp_storage;

#[cfg(feature = "native")]
pub mod adp_publish;

#[cfg(feature = "native")]
pub mod nwc_client;

#[cfg(test)]
mod nwc_client_contract_tests {
    use super::nwc_client::{
        build_pay_invoice_request_event, build_pay_invoice_request_json,
        parse_pay_invoice_response_event, parse_pay_invoice_response_json, NwcConnection,
    };
    use nostr::nips::nip44;
    use nostr::{EventBuilder, Keys, Kind, SecretKey, Tag, TagKind};

    #[test]
    fn nwc_uri_parses_wallet_pubkey_relays_secret_and_lud16() {
        let uri = concat!(
            "nostr+walletconnect://",
            "0000000000000000000000000000000000000000000000000000000000000001",
            "?relay=wss%3A%2F%2Frelay.example.com",
            "&relay=ws%3A%2F%2Flocalhost%3A10547",
            "&secret=0000000000000000000000000000000000000000000000000000000000000002",
            "&lud16=buyer%40example.com"
        );

        let connection = NwcConnection::parse(uri).expect("valid NWC URI parses");

        assert_eq!(
            connection.wallet_pubkey_hex(),
            "0000000000000000000000000000000000000000000000000000000000000001"
        );
        assert_eq!(
            connection.relay_urls(),
            &[
                "wss://relay.example.com".to_string(),
                "ws://localhost:10547".to_string()
            ]
        );
        assert_eq!(connection.lud16(), Some("buyer@example.com"));
    }

    #[test]
    fn pay_invoice_request_omits_amount_for_fixed_amount_invoice() {
        let json = build_pay_invoice_request_json("lnbc1fixedamountinvoice").unwrap();
        let value: serde_json::Value = serde_json::from_str(&json).unwrap();

        assert_eq!(value["method"], "pay_invoice");
        assert_eq!(value["params"]["invoice"], "lnbc1fixedamountinvoice");
        assert!(
            value["params"].get("amount").is_none(),
            "NIP-47 amount is an optional msat override and fixed LNURL invoices already encode the amount"
        );
    }

    #[test]
    fn pay_invoice_response_extracts_preimage_and_fees() {
        let response = r#"{
            "result_type": "pay_invoice",
            "error": null,
            "result": {
                "preimage": "0123456789abcdef",
                "fees_paid": 21
            }
        }"#;

        let result = parse_pay_invoice_response_json(response).expect("response parses");

        assert_eq!(result.preimage, "0123456789abcdef");
        assert_eq!(result.fees_paid_msat, Some(21));
    }

    #[test]
    fn pay_invoice_response_reports_wallet_error() {
        let response = r#"{
            "result_type": "pay_invoice",
            "error": {
                "code": "PAYMENT_FAILED",
                "message": "route failed"
            },
            "result": null
        }"#;

        let error = parse_pay_invoice_response_json(response).unwrap_err();

        assert_eq!(
            error.to_string(),
            "NWC wallet error PAYMENT_FAILED: route failed"
        );
    }

    #[test]
    fn pay_invoice_request_event_uses_nip47_kind_p_tag_and_encryption_tag() {
        let uri = concat!(
            "nostr+walletconnect://",
            "0000000000000000000000000000000000000000000000000000000000000001",
            "?relay=wss%3A%2F%2Frelay.example.com",
            "&secret=0000000000000000000000000000000000000000000000000000000000000002"
        );
        let connection = NwcConnection::parse(uri).unwrap();

        let event = build_pay_invoice_request_event(&connection, "lnbc1fixedamountinvoice")
            .expect("request event builds");

        assert_eq!(event.kind, Kind::WalletConnectRequest);
        assert!(event.tags.iter().any(|tag| tag.kind() == TagKind::p()));
        assert!(event
            .tags
            .iter()
            .any(|tag| { tag.as_slice() == ["encryption", "nip44_v2"] }));
        assert_ne!(
            event.content,
            build_pay_invoice_request_json("lnbc1fixedamountinvoice").unwrap()
        );
    }

    #[test]
    fn pay_invoice_response_event_validates_correlation_and_decrypts_preimage() {
        let wallet_keys = Keys::generate();
        let client_secret =
            SecretKey::from_hex("0000000000000000000000000000000000000000000000000000000000000002")
                .unwrap();
        let client_keys = Keys::new(client_secret);
        let uri = format!(
            "nostr+walletconnect://{}?relay=wss%3A%2F%2Frelay.example.com&secret={}",
            wallet_keys.public_key(),
            client_keys.secret_key().to_secret_hex()
        );
        let connection = NwcConnection::parse(&uri).unwrap();
        let request =
            build_pay_invoice_request_event(&connection, "lnbc1fixedamountinvoice").unwrap();
        let response_json = r#"{
            "result_type": "pay_invoice",
            "error": null,
            "result": { "preimage": "feedface", "fees_paid": 0 }
        }"#;
        let encrypted = nip44::encrypt(
            wallet_keys.secret_key(),
            &client_keys.public_key(),
            response_json,
            nip44::Version::V2,
        )
        .unwrap();
        let response = EventBuilder::new(Kind::WalletConnectResponse, encrypted)
            .tags([
                Tag::public_key(client_keys.public_key()),
                Tag::event(request.id),
            ])
            .sign_with_keys(&wallet_keys)
            .unwrap();

        let result = parse_pay_invoice_response_event(&connection, &response, request.id)
            .expect("response event validates and decrypts");

        assert_eq!(result.preimage, "feedface");
        assert_eq!(result.fees_paid_msat, Some(0));
    }
}

#[cfg(feature = "native")]
pub mod user_cache;

#[cfg(feature = "native")]
pub mod social_graph;

#[cfg(feature = "native")]
pub mod extended_network;

#[cfg(feature = "native")]
pub mod marketplace;

#[cfg(feature = "native")]
pub mod marketplace_cache;

#[cfg(feature = "native")]
pub mod purchases;

#[cfg(feature = "native")]
pub use profile_fetcher::{
    LruProfileCache, ProfileCache, ProfileFetcher, BATCH_SIZE, MAX_PROFILE_ATTEMPTS,
};

#[cfg(feature = "native")]
pub use user_cache::UserCache;

#[cfg(feature = "native")]
pub use nip05_validator::{Nip05Validator, ValidationCommand, ValidationResult};

#[cfg(feature = "native")]
pub mod lightning;

#[cfg(all(test, feature = "native"))]
pub mod test_helpers;

// WASM-compatible stubs
#[cfg(feature = "wasm")]
pub mod wasm_stub;
