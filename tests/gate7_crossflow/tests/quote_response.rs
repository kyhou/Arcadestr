use super::fixtures::*;
use arcadestr_core::{
    adp_client::{AdpClient, CreateSettlementQuoteRequest},
    http_client::{HttpClient, HttpClientError, HttpDownloadOutcome},
    settlement_repository::SettlementRepository,
    storage::Database,
};
use nostr_sdk::{EventBuilder, Keys, Kind};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{
    path::Path,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex,
    },
};
use wiremock::{
    matchers::{method, path},
    Mock, MockServer, ResponseTemplate,
};

struct LostResponse {
    real: Arc<dyn HttpClient>,
    drop_once: AtomicBool,
}
#[async_trait::async_trait]
impl HttpClient for LostResponse {
    async fn get_json(&self, url: &str) -> Result<Value, HttpClientError> {
        self.real.get_json(url).await
    }
    async fn get_json_no_redirects(&self, url: &str) -> Result<Value, HttpClientError> {
        self.real.get_json_no_redirects(url).await
    }
    async fn post_json(
        &self,
        url: &str,
        body: Value,
        headers: Vec<(String, String)>,
    ) -> Result<Value, HttpClientError> {
        let response = self.real.post_json(url, body, headers).await?;
        if url.ends_with("/settlement/quote") && self.drop_once.swap(false, Ordering::SeqCst) {
            return Err(HttpClientError::Request(
                "fixture: response lost after actual server committed quote".into(),
            ));
        }
        Ok(response)
    }
    async fn download_to_path(
        &self,
        url: &str,
        headers: Vec<(String, String)>,
        dest: &Path,
        on_progress: &mut (dyn FnMut(u64, Option<u64>) + Send),
    ) -> Result<HttpDownloadOutcome, HttpClientError> {
        self.real
            .download_to_path(url, headers, dest, on_progress)
            .await
    }
}

#[tokio::test]
async fn dropped_quote_response_reuses_durable_identity_after_restart_without_live_invoice_or_payment(
) {
    let events: Events = Arc::new(Mutex::new(vec![]));
    let (one, _one) = relay(events.clone()).await;
    let (two, _two) = relay(events.clone()).await;
    let server = Server::start(&[one, two]).await;
    let metadata = MockServer::start().await;
    for (name, amount, seed) in [("dev", 900000, 40u8), ("operator", 100000, 41u8)] {
        Mock::given(method("GET")).and(path(format!("/.well-known/lnurlp/{name}")))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({"callback":format!("{}/{name}/offline-invoice",metadata.uri()),"minSendable":1,"maxSendable":1000000000,"nostrPubkey":server.operator.public_key().to_hex(),"allowsNostr":false}))).mount(&metadata).await;
        let requests = Arc::new(std::sync::atomic::AtomicU8::new(0));
        Mock::given(method("GET")).and(path(format!("/{name}/offline-invoice")))
            .respond_with(move |_:&wiremock::Request|ResponseTemplate::new(200).set_body_json(json!({"pr":synthetic_invoice(amount,seed+2*requests.fetch_add(1,Ordering::SeqCst)),"routes":[]})))
            .mount(&metadata).await;
    }
    let developer = Keys::generate();
    let buyer = Keys::generate();
    let developer_address = format!("dev@{}", metadata.address());
    let operator_address = format!("operator@{}", metadata.address());
    let terms = adp_core::build_terms_event_builder(&adp_core::TermsPayload::Offer {
        terms: adp_core::CommercialTerms {
            version: 1,
            operator_pubkey: server.operator.public_key().to_hex(),
            payment_destination: operator_address,
            fee: adp_core::FeePolicy::Fixed {
                amount_msat: 100000,
            },
            limitations: vec![],
        },
    })
    .expect("terms")
    .sign_with_keys(&server.operator)
    .expect("signed offer");
    let coordinate = format!("30402:{}:paid", developer.public_key());
    let sale = EventBuilder::new(Kind::Custom(30402), "")
        .tags([
            tag(&["d", "paid"]),
            tag(&["title", "Synthetic quote fixture"]),
            tag(&["price", "1000", "SATS"]),
            tag(&["lud16", &developer_address]),
            tag(&["server", &server.url]),
            tag(&[
                "adp_terms",
                &server.operator.public_key().to_hex(),
                &terms.id.to_hex(),
            ]),
        ])
        .sign_with_keys(&developer)
        .expect("signed listing");
    events
        .lock()
        .expect("local relay store")
        .extend([sale.clone(), terms.clone()]);
    let dir = tempfile::tempdir_in("/tmp/opencode").expect("buyer database");
    let db_path = dir.path().join("buyer.db");
    let db = Database::new(&db_path).await.expect("fresh DB");
    let repo = SettlementRepository::new(db.pool().clone());
    let commitment = hex::encode(Sha256::digest(b"immutable synthetic quote request"));
    let first = repo
        .reserve_quote_initiation(
            &buyer.public_key().to_hex(),
            &server.url,
            &coordinate,
            &sale.id.to_hex(),
            &commitment,
            None,
        )
        .await
        .expect("initial identity");
    let client = AdpClient::new(
        &server.url,
        Arc::new(LostResponse {
            real: http(),
            drop_once: AtomicBool::new(true),
        }),
    );
    let request = |id: String| CreateSettlementQuoteRequest {
        purchase_id: id,
        listing_event: sale.clone(),
        terms_events: vec![terms.clone()],
        refund_destination: arcadestr_core::adp_settlement::RefundDestination {
            endpoint: "buyer@refund.example.test".into(),
            min_sendable_msat: 1,
            max_sendable_msat: 1000000000,
        },
        zap_requests: Default::default(),
    };
    let lost = client
        .create_settlement_quote(&signer(&buyer), request(first.clone()))
        .await;
    assert!(
        matches!(lost,Err(arcadestr_core::adp_client::AdpClientError::Http(HttpClientError::Request(ref message))) if message.contains("response lost")),
        "drop specifically after the actual server committed, not an unrelated server rejection"
    );
    assert!(server
        .storage
        .settlement_quote(&first)
        .await
        .expect("remote checkpoint")
        .is_some());
    assert!(repo
        .list_for_account(&buyer.public_key().to_hex())
        .await
        .expect("no delivered quote")
        .is_empty());
    db.close().await;
    let reopened = Database::new(&db_path).await.expect("restart");
    let repo = SettlementRepository::new(reopened.pool().clone());
    let retry = repo
        .reserve_quote_initiation(
            &buyer.public_key().to_hex(),
            &server.url,
            &coordinate,
            &sale.id.to_hex(),
            &commitment,
            None,
        )
        .await
        .expect("restart retry identity");
    let response = client
        .create_settlement_quote(&signer(&buyer), request(retry.clone()))
        .await
        .expect("actual server replay");
    response.quote.verify().expect("signed response");
    let count: i64 = sqlx_count_quotes(&server).await;
    assert_eq!(
        count, 1,
        "one remote purchase identity after lost response and restart"
    );
    assert_eq!(
        retry, first,
        "same initiating identity must survive ambiguous transport outcomes"
    );
    assert!(
        arcadestr_core::purchase_contract::require_live_purchase_authorization().is_err(),
        "production live gate remains closed"
    );
}

pub(super) fn synthetic_invoice(amount: u64, seed: u8) -> String {
    use bitcoin::{
        hashes::{sha256, Hash},
        secp256k1::{Secp256k1, SecretKey},
    };
    lightning_invoice::InvoiceBuilder::new(lightning_invoice::Currency::Regtest)
        .description("Gate 7 offline fixture, no Lightning node or funds".into())
        .amount_milli_satoshis(amount)
        .payment_hash(sha256::Hash::hash(&[seed; 32]))
        .payment_secret(lightning_invoice::PaymentSecret([7; 32]))
        .duration_since_epoch(std::time::Duration::from_secs(
            nostr_sdk::Timestamp::now().as_secs(),
        ))
        .expiry_time(std::time::Duration::from_secs(600))
        .min_final_cltv_expiry_delta(40)
        .build_signed(|hash| {
            Secp256k1::new().sign_ecdsa_recoverable(
                hash,
                &SecretKey::from_slice(&[42; 32]).expect("fixture key"),
            )
        })
        .expect("offline BOLT11 bytes")
        .to_string()
}

async fn sqlx_count_quotes(server: &Server) -> i64 {
    sqlx::query_scalar("SELECT COUNT(*) FROM purchase_quotes")
        .fetch_one(&server.storage.pool())
        .await
        .expect("remote quote count")
}
