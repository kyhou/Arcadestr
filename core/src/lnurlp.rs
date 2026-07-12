//! LNURL-pay helpers for ADP purchase flows.

use serde::Deserialize;
use thiserror::Error;

use crate::http_client::{HttpClient, HttpClientError};

/// LNURL-pay endpoint metadata resolved from a LUD-16 address.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LnurlPayEndpoint {
    pub callback: String,
    pub min_sendable_msat: u64,
    pub max_sendable_msat: u64,
    pub nostr_pubkey: Option<String>,
}

/// Errors returned while resolving LNURL-pay metadata or invoices.
#[derive(Debug, Error)]
pub enum LnurlError {
    /// The LUD-16 address is not `name@domain`.
    #[error("malformed lud16 address")]
    MalformedLud16,

    /// The requested amount is outside the endpoint's sendable range.
    #[error("amount {amount_msat} msat outside range {min_msat}..={max_msat}")]
    AmountOutOfRange {
        amount_msat: u64,
        min_msat: u64,
        max_msat: u64,
    },

    /// The HTTP layer failed.
    #[error("LNURL HTTP request failed: {0}")]
    Http(#[from] HttpClientError),

    /// The response did not match the expected LNURL-pay shape.
    #[error("LNURL response decode failed: {0}")]
    Decode(#[from] serde_json::Error),
}

#[derive(Debug, Deserialize)]
struct LnurlPayResponse {
    callback: String,
    #[serde(rename = "minSendable")]
    min_sendable_msat: u64,
    #[serde(rename = "maxSendable")]
    max_sendable_msat: u64,
    #[serde(rename = "nostrPubkey")]
    nostr_pubkey: Option<String>,
}

#[derive(Debug, Deserialize)]
struct InvoiceResponse {
    pr: String,
}

/// Resolves a LUD-16 address to LNURL-pay endpoint metadata.
///
/// # Errors
/// Returns [`LnurlError`] if the address is malformed, unreachable, or invalid JSON.
pub async fn resolve_lud16(
    http: &dyn HttpClient,
    lud16: &str,
) -> Result<LnurlPayEndpoint, LnurlError> {
    let (name, domain) = split_lud16(lud16)?;
    let url = format!("https://{domain}/.well-known/lnurlp/{name}");
    let value = http.get_json(&url).await?;
    let response: LnurlPayResponse = serde_json::from_value(value)?;

    Ok(LnurlPayEndpoint {
        callback: response.callback,
        min_sendable_msat: response.min_sendable_msat,
        max_sendable_msat: response.max_sendable_msat,
        nostr_pubkey: response.nostr_pubkey,
    })
}

/// Requests a bolt11 invoice from an LNURL-pay endpoint.
///
/// # Errors
/// Returns [`LnurlError`] when the amount is outside endpoint bounds or the callback fails.
pub async fn request_invoice(
    http: &dyn HttpClient,
    endpoint: &LnurlPayEndpoint,
    amount_msat: u64,
) -> Result<String, LnurlError> {
    if amount_msat < endpoint.min_sendable_msat || amount_msat > endpoint.max_sendable_msat {
        return Err(LnurlError::AmountOutOfRange {
            amount_msat,
            min_msat: endpoint.min_sendable_msat,
            max_msat: endpoint.max_sendable_msat,
        });
    }

    let separator = if endpoint.callback.contains('?') {
        '&'
    } else {
        '?'
    };
    let url = format!("{}{separator}amount={amount_msat}", endpoint.callback);
    let response: InvoiceResponse = serde_json::from_value(http.get_json(&url).await?)?;
    Ok(response.pr)
}

fn split_lud16(lud16: &str) -> Result<(&str, &str), LnurlError> {
    let (name, domain) = lud16.split_once('@').ok_or(LnurlError::MalformedLud16)?;
    if name.is_empty() || domain.is_empty() || domain.contains('@') {
        return Err(LnurlError::MalformedLud16);
    }
    Ok((name, domain))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    use crate::test_helpers::http_mocks::MockHttpClient;

    #[tokio::test]
    async fn resolve_lud16_fetches_well_known_lnurlp_endpoint() {
        let pubkey = "d94a3f0b5b907fda6c1d2716af34e4d533ddf8f6f6f0f8f1f4a3f605f6c9a3b4";
        let mock = MockHttpClient::new().with_json_response(
            "https://example.com/.well-known/lnurlp/studio",
            json!({
                "callback": "https://example.com/lnurl/callback",
                "minSendable": 1000,
                "maxSendable": 1000000,
                "nostrPubkey": pubkey
            }),
        );

        let endpoint = resolve_lud16(&mock, "studio@example.com")
            .await
            .expect("lud16 should resolve");

        assert_eq!(endpoint.callback, "https://example.com/lnurl/callback");
        assert_eq!(endpoint.min_sendable_msat, 1000);
        assert_eq!(endpoint.max_sendable_msat, 1000000);
        assert_eq!(endpoint.nostr_pubkey.as_deref(), Some(pubkey));
        assert_eq!(
            mock.call_count("https://example.com/.well-known/lnurlp/studio"),
            1
        );
    }

    #[tokio::test]
    async fn resolve_lud16_rejects_malformed_addresses() {
        let mock = MockHttpClient::new();

        for lud16 in ["not-an-address", "@example.com", "studio@", "a@b@c"] {
            let err = resolve_lud16(&mock, lud16)
                .await
                .expect_err("malformed lud16 should fail");
            assert!(matches!(err, LnurlError::MalformedLud16));
        }
    }

    #[tokio::test]
    async fn request_invoice_calls_callback_with_amount() {
        let mock = MockHttpClient::new().with_prefix_json_response(
            "https://example.com/lnurl/callback",
            json!({ "pr": "lnbc1testinvoice" }),
        );
        let endpoint = LnurlPayEndpoint {
            callback: "https://example.com/lnurl/callback".to_string(),
            min_sendable_msat: 1000,
            max_sendable_msat: 1000000,
            nostr_pubkey: None,
        };

        let bolt11 = request_invoice(&mock, &endpoint, 21000)
            .await
            .expect("invoice request should succeed");

        assert_eq!(bolt11, "lnbc1testinvoice");
        assert_eq!(
            mock.last_requested_url().as_deref(),
            Some("https://example.com/lnurl/callback?amount=21000")
        );
    }

    #[tokio::test]
    async fn request_invoice_rejects_amount_outside_endpoint_bounds() {
        let mock = MockHttpClient::new();
        let endpoint = LnurlPayEndpoint {
            callback: "https://example.com/lnurl/callback".to_string(),
            min_sendable_msat: 1000,
            max_sendable_msat: 1000000,
            nostr_pubkey: None,
        };

        let low = request_invoice(&mock, &endpoint, 999)
            .await
            .expect_err("amount below min should fail");
        let high = request_invoice(&mock, &endpoint, 1000001)
            .await
            .expect_err("amount above max should fail");

        assert!(matches!(low, LnurlError::AmountOutOfRange { .. }));
        assert!(matches!(high, LnurlError::AmountOutOfRange { .. }));
    }
}
