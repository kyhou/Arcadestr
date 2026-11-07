//! Explicit opt-in real-Lightning partial/refund scenarios. Each case owns an
//! HTTP server, databases and deterministic deadline clock. Nostr is a controlled
//! test adapter, not a claim of live-relay timeout testing.
#[allow(dead_code)]
#[path = "regtest_split_purchase.rs"]
pub(crate) mod common;

use adp_server::{
    config::Config,
    keystore::KeyStore,
    relay::{Relay, RelayLookup, RelayResult},
    routes,
    settlement_clock::SettlementClock,
    state::AppState,
    storage::Storage,
};
use anyhow::{anyhow, ensure, Context, Result};
use arcadestr_core::{
    adp_client::{AdpClient, CreateSettlementQuoteRequest, DownloadAuth, SettlementProofRequest},
    adp_publish::{
        build_adp_listing_event_builder, build_fulfillment_authorization_event_builder,
        AdpListingInput, FulfillmentAuthorizationInput,
    },
    adp_settlement::{
        build_terms_event_builder, verify_quote_event, CommercialTerms, FeePolicy, PaymentRole,
        QuoteState, RefundDestination, TermsPayload,
    },
    authorization::{AuthorizationTerms, CAPABILITY_ISSUE_RECEIPT, CAPABILITY_UPLOAD_BUILD},
    durable_payments::{DurableNwcPayments, PaymentWallet},
    http_client::{HttpClient, ReqwestHttpClient},
    nwc_client::{load_nwc_connection, NwcClient, NwcClientError, PayInvoiceResult},
    signers::NostrSigner,
    storage::Database,
};
use async_trait::async_trait;
use nostr::{Event, EventId, Keys, PublicKey, Tag, Timestamp};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicU64, AtomicUsize, Ordering},
        Arc, Mutex,
    },
    time::Duration,
};

const LEG: u64 = 100_000;
const CASES: [&str; 6] = [
    "bob-paid-carol-fails",
    "bob-fails-before-carol",
    "refund-succeeds",
    "refund-outage-and-retry",
    "refund-process-crash",
    "repeated-recovery",
];

struct CaseRelay {
    base: String,
    events: Mutex<Vec<Event>>,
}
impl CaseRelay {
    fn add(&self, event: Event) {
        let mut events = self.events.lock().expect("test relay lock");
        if !events.iter().any(|e| e.id == event.id) {
            events.push(event);
        }
    }
    fn absent<T>() -> RelayLookup<T> {
        RelayLookup::ConfirmedAbsent {
            successful_relays: 2,
        }
    }
}
#[async_trait]
impl Relay for CaseRelay {
    fn server_url(&self) -> &str {
        &self.base
    }
    async fn publish(&self, event: &Event) -> RelayResult<()> {
        event.verify()?;
        self.add(event.clone());
        Ok(())
    }
    async fn lookup_event(&self, id: EventId) -> RelayLookup<Event> {
        self.events
            .lock()
            .expect("relay")
            .iter()
            .find(|e| e.id == id)
            .cloned()
            .map(RelayLookup::Found)
            .unwrap_or_else(Self::absent)
    }
    async fn lookup_listing(&self, author: PublicKey, d: &str) -> RelayLookup<Event> {
        self.events
            .lock()
            .expect("relay")
            .iter()
            .filter(|e| {
                e.kind.as_u16() == 30402
                    && e.pubkey == author
                    && e.tags.iter().any(|t| t.as_slice() == ["d", d])
            })
            .max_by_key(|e| (e.created_at, e.id))
            .cloned()
            .map(RelayLookup::Found)
            .unwrap_or_else(Self::absent)
    }
    async fn lookup_receipts(&self, _: PublicKey, _: &str) -> RelayLookup<Vec<Event>> {
        Self::absent()
    }
    async fn lookup_authorizations(&self, _: PublicKey, _: &str) -> RelayLookup<Vec<Event>> {
        Self::absent()
    }
    async fn lookup_authorization_successors(
        &self,
        _: PublicKey,
        _: EventId,
    ) -> RelayLookup<Vec<Event>> {
        Self::absent()
    }
}

struct BuyerWallet {
    inner: NwcClient,
    pays: AtomicUsize,
}
#[async_trait]
impl PaymentWallet for BuyerWallet {
    fn identity(&self) -> Result<String, NwcClientError> {
        self.inner.payment_identity()
    }
    async fn pay(&self, invoice: &str) -> Result<PayInvoiceResult, NwcClientError> {
        self.pays.fetch_add(1, Ordering::SeqCst);
        self.inner.pay_invoice(invoice).await
    }
    async fn lookup(
        &self,
        hash: &str,
        amount: u64,
    ) -> Result<Option<PayInvoiceResult>, NwcClientError> {
        self.inner.lookup_payment(hash, amount).await
    }
}

struct RefundWallet {
    audit: PathBuf,
    crash: bool,
    unavailable: bool,
}
impl RefundWallet {
    fn count(&self, kind: &str) -> Result<(), NwcClientError> {
        let mut value: Value = if self.audit.exists() {
            serde_json::from_slice(
                &std::fs::read(&self.audit).map_err(|_| NwcClientError::InvalidLookup)?,
            )
            .map_err(|_| NwcClientError::InvalidLookup)?
        } else {
            json!({"pay_calls":0,"lookup_calls":0})
        };
        value[kind] = json!(value[kind].as_u64().unwrap_or(0) + 1);
        common::write_private(&self.audit, &value).map_err(|_| NwcClientError::InvalidLookup)
    }
}
#[async_trait]
impl PaymentWallet for RefundWallet {
    fn identity(&self) -> Result<String, NwcClientError> {
        if self.unavailable {
            return Err(NwcClientError::Relay(
                "injected pre-dispatch wallet outage".into(),
            ));
        }
        let nodes = common::check_network().map_err(|_| NwcClientError::InvalidLookup)?;
        Ok(format!("polar-lnd-regtest:{}", nodes["bob"]))
    }
    async fn pay(&self, invoice: &str) -> Result<PayInvoiceResult, NwcClientError> {
        let nodes = common::check_network().map_err(|_| NwcClientError::InvalidLookup)?;
        common::validate_invoice(invoice, &nodes["alice"], LEG)
            .map_err(|_| NwcClientError::InvalidLookup)?;
        self.count("pay_calls")?;
        let result = common::lnd(
            "bob",
            "payinvoice",
            &[
                "--force",
                "--json",
                "--fee_limit=10",
                "--timeout=20s",
                &format!("--pay_req={invoice}"),
            ],
        )
        .map_err(|_| NwcClientError::WalletTimeout)?;
        if result["status"] != "SUCCEEDED" {
            return Err(NwcClientError::WalletTimeout);
        }
        let preimage = result["payment_preimage"]
            .as_str()
            .ok_or(NwcClientError::InvalidLookup)?
            .to_string();
        if self.crash {
            std::process::exit(86);
        }
        Ok(PayInvoiceResult {
            preimage,
            fees_paid_msat: Some(
                common::num(&result["fee_msat"]).map_err(|_| NwcClientError::InvalidLookup)?,
            ),
        })
    }
    async fn lookup(
        &self,
        hash: &str,
        amount: u64,
    ) -> Result<Option<PayInvoiceResult>, NwcClientError> {
        self.count("lookup_calls")?;
        let result = common::lnd("bob", "listpayments", &["--include_incomplete"])
            .map_err(|_| NwcClientError::InvalidLookup)?;
        let record = result["payments"]
            .as_array()
            .ok_or(NwcClientError::InvalidLookup)?
            .iter()
            .find(|p| p["payment_hash"].as_str() == Some(hash));
        match record {
            Some(p)
                if p["status"] == "SUCCEEDED"
                    && common::num(&p["value_msat"]).ok() == Some(amount) =>
            {
                Ok(Some(PayInvoiceResult {
                    preimage: p["payment_preimage"]
                        .as_str()
                        .ok_or(NwcClientError::InvalidLookup)?
                        .into(),
                    fees_paid_msat: Some(
                        common::num(&p["fee_msat"]).map_err(|_| NwcClientError::InvalidLookup)?,
                    ),
                }))
            }
            _ => Ok(None),
        }
    }
}

#[derive(Serialize, Deserialize)]
struct RefundJob {
    base: String,
    quote_id: String,
    leg_id: String,
    attempt_id: String,
    bolt11: String,
    client_db: PathBuf,
    audit: PathBuf,
    result: PathBuf,
}

async fn refund_child(path: &Path, resume: bool) -> Result<()> {
    let job: RefundJob = serde_json::from_slice(&std::fs::read(path)?)?;
    ensure!(
        job.base.starts_with("http://127.0.0.1:"),
        "non-local test server"
    );
    let (_, bob) = common::accounts().await?;
    let signer = common::signer(&bob)?;
    let wallet = RefundWallet {
        audit: job.audit.clone(),
        crash: !resume,
        unavailable: false,
    };
    let db = Database::new(&job.client_db).await?;
    let payments = DurableNwcPayments::new(db.pool().clone());
    let paid = if resume {
        payments.reconcile(&signer, &wallet, &job.bolt11).await?
    } else {
        payments
            .pay_or_resume(&signer, &wallet, &job.bolt11)
            .await?
    };
    let http: Arc<dyn HttpClient> = Arc::new(ReqwestHttpClient::new(Duration::from_secs(30))?);
    let client = AdpClient::new(job.base, http);
    let confirmed = client
        .confirm_refund(
            &signer,
            &job.quote_id,
            &job.leg_id,
            &job.attempt_id,
            &paid.preimage,
        )
        .await
        .map_err(common::safe_adp)?;
    ensure!(confirmed["state"] == "verified", "refund not verified");
    common::write_private(
        &job.result,
        &json!({"state":"verified","attempt_id":job.attempt_id}),
    )?;
    Ok(())
}

pub(crate) struct Case {
    pub(crate) root: PathBuf,
    pub(crate) base: String,
    pub(crate) store: Storage,
    pub(crate) client: AdpClient,
    pub(crate) db: Database,
    relay: Arc<CaseRelay>,
    pub(crate) clock: Arc<AtomicU64>,
    pub(crate) buyer: Keys,
    pub(crate) bob: Keys,
    pub(crate) listing: Event,
    pub(crate) stop: Option<tokio::sync::oneshot::Sender<()>>,
    pub(crate) task: tokio::task::JoinHandle<()>,
}

async fn setup(
    root: PathBuf,
    name: &str,
    buyer: &Keys,
    bob: &Keys,
    operator: &Keys,
) -> Result<Case> {
    setup_case(
        root,
        name,
        buyer,
        bob,
        operator,
        None,
        Timestamp::now().as_secs(),
        "bob@127.0.0.1:18080",
        None,
        None,
    )
    .await
}

/// Additive live-relay fixture; the original six controlled cases are unchanged.
pub(crate) async fn setup_live(
    root: PathBuf,
    name: &str,
    buyer: &Keys,
    bob: &Keys,
    operator: &Keys,
    relays: &[String],
    issued_at: u64,
) -> Result<Case> {
    setup_case(
        root,
        name,
        buyer,
        bob,
        operator,
        Some(relays),
        issued_at,
        "bob@127.0.0.1:18080",
        None,
        None,
    )
    .await
}

/// GUI-only fixture: reuse Alice's existing refund-capable NWC identity.
/// Controlled event adapter avoids replacing any live commercial terms.
pub(crate) async fn setup_recipient_gui(
    root: PathBuf,
    name: &str,
    buyer: &Keys,
    recipient: &Keys,
    operator: &Keys,
) -> Result<Case> {
    let gate = root.join("gui-pause-refund-confirmation");
    setup_case(
        root,
        name,
        buyer,
        recipient,
        operator,
        None,
        Timestamp::now().as_secs(),
        "alice@127.0.0.1:18080",
        Some(gate),
        None,
    )
    .await
}

/// Remote-buyer GUI setup reuses the already-published immutable operator offer.
/// The buyer is chosen by the authenticated desktop, not this server fixture.
pub(crate) async fn setup_remote_buyer_gui(
    root: PathBuf,
    fixture_owner: &Keys,
    developer: &Keys,
    operator: &Keys,
    terms: Event,
) -> Result<Case> {
    let gate = root.join("gui-pause-refund-confirmation");
    setup_case(
        root,
        "nip46-recovery",
        fixture_owner,
        developer,
        operator,
        None,
        Timestamp::now().as_secs(),
        "alice@127.0.0.1:18080",
        Some(gate),
        Some(terms),
    )
    .await
}

/// New live-relay, fully separate purchase fixture for a NIP-46/NWC buyer.
pub(crate) async fn setup_nip46_nwc_purchase(
    root: PathBuf,
    fixture_owner: &Keys,
    publisher: &Keys,
    operator: &Keys,
) -> Result<Case> {
    let relays = vec!["ws://127.0.0.1:7778".to_owned()];
    setup_case(
        root,
        "nip46-nwc-recovery",
        fixture_owner,
        publisher,
        operator,
        Some(&relays),
        Timestamp::now().as_secs(),
        "bob@127.0.0.1:18080",
        None,
        None,
    )
    .await
}

/// Reopen an existing isolated fixture; never provision or create a quote.
pub(crate) async fn restore_live(
    root: PathBuf,
    buyer: &Keys,
    bob: &Keys,
    operator: &Keys,
    urls: &[String],
    now: u64,
) -> Result<Case> {
    let store = Storage::connect(root.join("adp.db").to_str().context("DB path")?).await?;
    let listing_json: Option<String> =
        sqlx::query_scalar("SELECT listing_event_json FROM purchase_quotes LIMIT 1")
            .fetch_optional(&store.pool())
            .await?;
    let listing: Event = match listing_json {
        Some(json) => serde_json::from_str(&json)?,
        None => serde_json::from_slice(&std::fs::read(root.join("listing.json"))?)?,
    };
    listing.verify()?;
    ensure!(
        listing.pubkey == bob.public_key(),
        "existing fixture publisher mismatch"
    );
    let live_purchase_clock = listing.tags.iter().any(|tag| {
        let v = tag.as_slice();
        v.len() >= 2 && v[0] == "d" && v[1].starts_with("regtest-refunds-nip46-nwc-recovery-")
    });
    let base = listing
        .tags
        .iter()
        .find_map(|t| {
            let v = t.as_slice();
            if v.first().map(String::as_str) == Some("server") {
                v.get(1).cloned()
            } else {
                None
            }
        })
        .context("original server URL absent")?;
    let address = base
        .strip_prefix("http://127.0.0.1:")
        .context("nonlocal server")?
        .parse::<u16>()?;
    let listener = tokio::net::TcpListener::bind(("127.0.0.1", address)).await?;
    let master_key: [u8; 32] = hex::decode(std::env::var("ADP_MASTER_KEY")?)
        .map_err(|_| anyhow!("invalid private key"))?
        .try_into()
        .map_err(|_| anyhow!("invalid private key length"))?;
    let active = Arc::new(
        adp_server::relay::RelayClient::connect_with_timeout(
            operator.clone(),
            urls,
            base.clone(),
            Duration::from_secs(5),
        )
        .await?,
    );
    let state = AppState {
        config: Arc::new(Config {
            server_url: base.clone(),
            bind_addr: listener.local_addr()?.to_string(),
            db_path: root.join("adp.db").to_string_lossy().into(),
            files_dir: root.join("files").to_string_lossy().into(),
            max_upload_bytes: 1024 * 1024,
            relays: urls.to_vec(),
            operator_nsec: operator.secret_key().to_secret_hex(),
            master_key,
            allow_insecure_lnurl: true,
            download_token_ttl_secs: 900,
            file_reverify_interval_secs: 3600,
        }),
        storage: store.clone(),
        keystore: Arc::new(KeyStore::new(store.pool(), master_key)),
        relay: active,
        operator_keys: operator.clone(),
    };
    let clock = Arc::new(AtomicU64::new(now));
    let read = clock.clone();
    let app = routes::router_with_settlement_clock(
        state,
        if live_purchase_clock {
            SettlementClock::default()
        } else {
            SettlementClock::new(move || Some(read.load(Ordering::SeqCst)))
        },
    );
    let (stop, stopped) = tokio::sync::oneshot::channel();
    let task = tokio::spawn(async move {
        let _ = axum::serve(listener, app)
            .with_graceful_shutdown(async {
                let _ = stopped.await;
            })
            .await;
    });
    let client = AdpClient::new(
        base.clone(),
        Arc::new(ReqwestHttpClient::new(Duration::from_secs(30))?),
    );
    let db = Database::new(&root.join("client.db")).await?;
    Ok(Case {
        root,
        base: base.clone(),
        store,
        client,
        db,
        relay: Arc::new(CaseRelay {
            base,
            events: Mutex::new(vec![]),
        }),
        clock,
        buyer: buyer.clone(),
        bob: bob.clone(),
        listing,
        stop: Some(stop),
        task,
    })
}

async fn setup_case(
    root: PathBuf,
    name: &str,
    buyer: &Keys,
    bob: &Keys,
    operator: &Keys,
    live_relays: Option<&[String]>,
    issued_at: u64,
    developer_destination: &str,
    confirmation_gate: Option<PathBuf>,
    original_terms: Option<Event>,
) -> Result<Case> {
    std::fs::create_dir_all(&root)?;
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
    let base = format!("http://{}", listener.local_addr()?);
    let store = Storage::connect(root.join("adp.db").to_str().context("case DB path")?).await?;
    let bytes = hex::decode(std::env::var("ADP_MASTER_KEY")?)
        .map_err(|_| anyhow!("private master key invalid"))?;
    let master_key: [u8; 32] = bytes
        .try_into()
        .map_err(|_| anyhow!("private master key length invalid"))?;
    let relay = Arc::new(CaseRelay {
        base: base.clone(),
        events: Mutex::new(vec![]),
    });
    let active_relay: Arc<dyn Relay> = match live_relays {
        Some(urls) => Arc::new(
            adp_server::relay::RelayClient::connect_with_timeout(
                operator.clone(),
                urls,
                base.clone(),
                Duration::from_secs(5),
            )
            .await?,
        ),
        None => relay.clone(),
    };
    let state = AppState {
        config: Arc::new(Config {
            server_url: base.clone(),
            bind_addr: listener.local_addr()?.to_string(),
            db_path: root.join("adp.db").to_string_lossy().into(),
            files_dir: root.join("files").to_string_lossy().into(),
            max_upload_bytes: 1024 * 1024,
            relays: vec!["ws://127.0.0.1:10547".into(), "ws://127.0.0.1:10548".into()],
            operator_nsec: operator.secret_key().to_secret_hex(),
            master_key,
            allow_insecure_lnurl: true,
            download_token_ttl_secs: 900,
            file_reverify_interval_secs: 3600,
        }),
        storage: store.clone(),
        keystore: Arc::new(KeyStore::new(store.pool(), master_key)),
        relay: active_relay.clone(),
        operator_keys: operator.clone(),
    };
    let clock = Arc::new(AtomicU64::new(issued_at));
    let clock_read = clock.clone();
    let app = routes::router_with_settlement_clock(
        state,
        if name == "nip46-nwc-recovery" {
            SettlementClock::default()
        } else {
            SettlementClock::new(move || Some(clock_read.load(Ordering::SeqCst)))
        },
    );
    let app = if let Some(gate) = confirmation_gate {
        app.layer(axum::middleware::from_fn(
            move |request: axum::extract::Request, next: axum::middleware::Next| {
                let gate = gate.clone();
                async move {
                    if gate.exists()
                        && request.uri().path().contains("/refund/")
                        && request.uri().path().ends_with("/confirm")
                    {
                        return axum::response::IntoResponse::into_response(
                            axum::http::StatusCode::SERVICE_UNAVAILABLE,
                        );
                    }
                    if gate.with_file_name("gui-pause-purchase-proof").exists()
                        && request.uri().path().starts_with("/settlement/")
                        && request.uri().path().ends_with("/proof")
                    {
                        return axum::response::IntoResponse::into_response(
                            axum::http::StatusCode::SERVICE_UNAVAILABLE,
                        );
                    }
                    if gate
                        .with_file_name("gui-pause-invoice-preparation")
                        .exists()
                        && request.uri().path().starts_with("/settlement/")
                        && request.uri().path().ends_with("/next")
                    {
                        return axum::response::IntoResponse::into_response(
                            axum::http::StatusCode::SERVICE_UNAVAILABLE,
                        );
                    }
                    next.run(request).await
                }
            },
        ))
    } else {
        app
    };
    let (stop, stopped) = tokio::sync::oneshot::channel();
    let task = tokio::spawn(async move {
        let _ = axum::serve(listener, app)
            .with_graceful_shutdown(async {
                let _ = stopped.await;
            })
            .await;
    });
    let http: Arc<dyn HttpClient> = Arc::new(ReqwestHttpClient::new(Duration::from_secs(30))?);
    let client = AdpClient::new(base.clone(), http);
    let terms = if let Some(event) = original_terms {
        ensure!(
            matches!(arcadestr_core::adp_settlement::parse_terms_event(&event)?,TermsPayload::Offer{terms} if terms.operator_pubkey==operator.public_key().to_hex() && terms.fee==FeePolicy::Fixed{amount_msat:LEG}),
            "original operator offer differs from capped fixture"
        );
        event
    } else {
        build_terms_event_builder(&TermsPayload::Offer {
            terms: CommercialTerms {
                version: 1,
                operator_pubkey: operator.public_key().to_hex(),
                payment_destination: "carol@127.0.0.1:18080".into(),
                fee: FeePolicy::Fixed { amount_msat: LEG },
                limitations: vec!["isolated failure/refund fixture".into()],
            },
        })?
        .sign_with_keys(operator)?
    };
    active_relay
        .publish(&terms)
        .await
        .map_err(|_| anyhow!("fixture terms publication failed"))?;
    let d = format!("regtest-refunds-{name}-{}", uuid::Uuid::new_v4());
    let coordinate = format!("30402:{}:{d}", bob.public_key());
    let provision = client
        .provision(&common::signer(bob)?, Some(&coordinate))
        .await
        .map_err(common::safe_adp)?;
    let fulfillment = PublicKey::from_hex(&provision.fulfillment_pubkey)?;
    let authorization = build_fulfillment_authorization_event_builder(&AuthorizationTerms {
        authorization_id: d.clone(),
        coordinate: coordinate.clone(),
        operator_pubkey: operator.public_key(),
        fulfillment_pubkey: fulfillment,
        capabilities: [
            CAPABILITY_ISSUE_RECEIPT.to_string(),
            CAPABILITY_UPLOAD_BUILD.to_string(),
        ]
        .into_iter()
        .collect(),
        valid_from: issued_at,
    })?
    .sign_with_keys(bob)?;
    active_relay
        .publish(&authorization)
        .await
        .map_err(|_| anyhow!("fixture authorization publication failed"))?;
    let payload = common::env_path("ARCADESTR_REGTEST_PAYLOAD")?;
    let hash = hex::encode(Sha256::digest(std::fs::read(&payload)?));
    let listing = build_adp_listing_event_builder(&AdpListingInput {
        d_tag: d,
        title: format!("Isolated refunds: {name}"),
        description: "regtest only".into(),
        price_sats: 200,
        lud16: Some(developer_destination.into()),
        tags: vec![],
        images: vec![],
        servers: vec![base.clone()],
        file_hash: Some(hash),
        version: Some("refund-tests-v1".into()),
        fulfillment_authorizations: vec![FulfillmentAuthorizationInput {
            root_event_id: authorization.id.to_hex(),
            fulfillment_pubkey: fulfillment.to_hex(),
            relay_hint: None,
        }],
        acquisition: arcadestr_core::marketplace::AcquisitionPolicy::Gated,
        platforms: if name == "nip46-nwc-recovery" {
            vec!["linux-x86_64".into()]
        } else {
            vec![]
        },
        campaigns: vec![],
        nip94_event_id: None,
        preserved_tags: vec![],
    })?
    .tags([Tag::parse([
        "adp_terms",
        &operator.public_key().to_hex(),
        &terms.id.to_hex(),
    ])?])
    .sign_with_keys(bob)?;
    active_relay
        .publish(&listing)
        .await
        .map_err(|_| anyhow!("fixture listing publication failed"))?;
    client
        .upload(&common::signer(bob)?, &listing, &payload)
        .await
        .map_err(common::safe_adp)?;
    common::write_private(&root.join("terms.json"), &terms)?;
    common::write_private(&root.join("authorization.json"), &authorization)?;
    common::write_private(&root.join("listing.json"), &listing)?;
    let db = Database::new(&root.join("client.db")).await?;
    Ok(Case {
        root,
        base,
        store,
        client,
        db,
        relay,
        clock,
        buyer: buyer.clone(),
        bob: bob.clone(),
        listing,
        stop: Some(stop),
        task,
    })
}

async fn run_case(mut case: Case, name: &str) -> Result<Value> {
    let before = common::balances()?;
    let nodes = common::check_network()?;
    let buyer = common::signer(&case.buyer)?;
    let bob = common::signer(&case.bob)?;
    let connection = load_nwc_connection(&case.buyer.public_key())
        .map_err(|_| anyhow!("existing buyer NWC missing"))?;
    let hub = rusqlite::Connection::open_with_flags(
        common::env_path("ARCADESTR_REGTEST_HUB_DB")?,
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
    )?;
    let backend: String = hub.query_row(
        "SELECT value FROM user_configs WHERE key='LNBackendType' AND encrypted=0",
        [],
        |r| r.get(0),
    )?;
    let address: String = hub.query_row(
        "SELECT value FROM user_configs WHERE key='LNDAddress' AND encrypted=0",
        [],
        |r| r.get(0),
    )?;
    ensure!(
        backend == "LND" && address == "127.0.0.1:10001",
        "existing Hub must use Alice LND"
    );
    let pubkey:String=hub.query_row("SELECT wallet_pubkey FROM apps WHERE name IN ('Arcadestr Regtest Buyer','Alice Arcadestr Regtest Buyer')",[],|r|r.get(0))?;
    ensure!(
        connection.wallet_pubkey_hex() == pubkey
            && !connection.relay_urls().is_empty()
            && connection
                .relay_urls()
                .iter()
                .all(|s| matches!(s.as_str(), "ws://127.0.0.1:7778" | "ws://127.0.0.1:7778/")),
        "existing local buyer NWC binding mismatch"
    );
    drop(hub);
    let wallet = BuyerWallet {
        inner: NwcClient::new(connection).with_timeout(Duration::from_secs(30)),
        pays: AtomicUsize::new(0),
    };
    wallet
        .inner
        .verify()
        .await
        .map_err(|_| anyhow!("NWC unavailable"))?;
    let terms: Event = serde_json::from_slice(&std::fs::read(case.root.join("terms.json"))?)?;
    let id = format!("regtest-refund-{}", uuid::Uuid::new_v4());
    let response = case
        .client
        .create_settlement_quote(
            &buyer,
            CreateSettlementQuoteRequest {
                purchase_id: id.clone(),
                listing_event: case.listing.clone(),
                terms_events: vec![terms],
                refund_destination: RefundDestination {
                    endpoint: "alice@127.0.0.1:18080".into(),
                    min_sendable_msat: 1000,
                    max_sendable_msat: 10000000,
                },
                zap_requests: Default::default(),
            },
        )
        .await
        .map_err(common::safe_adp)?;
    let quote = verify_quote_event(
        &response.quote,
        &buyer
            .nip44_decrypt(&response.quote.pubkey, &response.quote.content)
            .await?,
    )?;
    ensure!(
        quote.legs.len() == 2
            && quote.gross_msat == 2 * LEG
            && quote.legs[0].role == PaymentRole::Developer
            && quote.legs[1].role == PaymentRole::Operator
            && quote.legs.iter().all(|l| l.amount_msat == LEG),
        "Bob-first canonical test ordering/amount mismatch"
    );
    let payments = DurableNwcPayments::new(case.db.pool().clone());
    let first = case
        .client
        .settlement_next_invoice(&buyer, &id)
        .await
        .map_err(common::safe_adp)?;
    common::validate_invoice(&first.bolt11, &nodes["bob"], LEG)?;
    let no_paid = name == "bob-fails-before-carol";
    if no_paid {
        common::lnd("bob", "cancelinvoice", &[&first.payment_hash])?;
        ensure!(
            payments
                .pay_or_resume(&buyer, &wallet, &first.bolt11)
                .await
                .is_err(),
            "canceled Bob invoice must fail"
        );
        let repeated = case
            .client
            .settlement_next_invoice(&buyer, &id)
            .await
            .map_err(common::safe_adp)?;
        ensure!(
            repeated.attempt_id == first.attempt_id,
            "Carol must remain undisclosed"
        );
        case.clock
            .store(quote.payment_expires_at + 1, Ordering::SeqCst);
    } else {
        ensure!(
            common::check_network()? == nodes,
            "regtest participants changed"
        );
        let paid = payments
            .pay_or_resume(&buyer, &wallet, &first.bolt11)
            .await?;
        let proof = case
            .client
            .submit_settlement_proof(
                &buyer,
                &id,
                SettlementProofRequest {
                    attempt_id: first.attempt_id.clone(),
                    preimage: paid.preimage,
                },
            )
            .await
            .map_err(common::safe_adp)?;
        ensure!(
            proof.state == "active" && proof.receipt.is_none() && proof.download_token.is_none(),
            "partial purchase granted ownership"
        );
        let second = case
            .client
            .settlement_next_invoice(&buyer, &id)
            .await
            .map_err(common::safe_adp)?;
        common::validate_invoice(&second.bolt11, &nodes["carol"], LEG)?;
        common::lnd("carol", "cancelinvoice", &[&second.payment_hash])?;
        ensure!(
            common::check_network()? == nodes,
            "regtest participants changed"
        );
        ensure!(
            payments
                .pay_or_resume(&buyer, &wallet, &second.bolt11)
                .await
                .is_err(),
            "canceled Carol invoice must fail"
        );
        ensure!(
            common::lnd("carol", "lookupinvoice", &[&second.payment_hash])?["settled"] == false,
            "Carol unexpectedly received funds"
        );
        case.clock
            .store(quote.confirmation_expires_at + 1, Ordering::SeqCst);
    }
    let status = case
        .client
        .settlement_status(&buyer, &id)
        .await
        .map_err(common::safe_adp)?;
    ensure!(
        status.state == QuoteState::Failed
            && status.settled_legs == if no_paid { 0 } else { 1 }
            && status.receipt_event_id.is_none(),
        "failed settlement state mismatch"
    );
    let expected_failure = if no_paid {
        quote.payment_expires_at
    } else {
        quote.confirmation_expires_at
    };
    ensure!(
        status.failure_at == Some(expected_failure)
            && status.refund_due_at == Some(expected_failure + 604800),
        "deadline drift"
    );
    let partial = common::balances()?;
    for node in ["alice", "bob", "carol", "dave"] {
        let delta = if no_paid {
            0
        } else if node == "alice" {
            -100000
        } else if node == "bob" {
            100000
        } else {
            0
        };
        ensure!(
            partial[node] as i64 - before[node] as i64 == delta,
            "partial-state balance mismatch"
        );
    }
    let recovery = case
        .client
        .prepare_recovery(&buyer, &id)
        .await
        .map_err(common::safe_adp)?;
    let event = recovery.event.context("processor recovery event missing")?;
    event.verify()?;
    ensure!(
        event.kind.as_u16() == 1034
            && recovery.evidence.failure_at == expected_failure
            && recovery.evidence.paid_leg_ids.len() == if no_paid { 0 } else { 1 },
        "recovery evidence mismatch"
    );
    let repeats = if name == "repeated-recovery" { 10 } else { 3 };
    let nwc_calls = wallet.pays.load(Ordering::SeqCst);
    for _ in 0..repeats {
        let next = case
            .client
            .prepare_recovery(&buyer, &id)
            .await
            .map_err(common::safe_adp)?;
        ensure!(
            next.event.as_ref().is_some_and(|e| e.id == event.id),
            "recovery retry changed immutable evidence identity"
        );
    }
    ensure!(
        wallet.pays.load(Ordering::SeqCst) == nwc_calls,
        "recovery sent a new payment"
    );
    let mut result = json!({"case":name,"quote_id":id,"state":"failed","failure_at":expected_failure,
        "refund_due_at":expected_failure+604800,"recovery_event_id":event.id.to_hex(),"purchase_receipts":0,
        "ownership":false,"settled_purchase_legs":if no_paid{0}else{1},"balances_before_msat":before,"partial_balances_msat":partial,
        "deadline_clock":"test-owned injected; Lightning invoice clock remains real","nostr_relay":"controlled test adapter"});
    if !no_paid {
        let prepared = case
            .client
            .prepare_refund(&bob, &id, &first.leg_id)
            .await
            .map_err(common::safe_adp)?;
        common::validate_invoice(&prepared.bolt11, &nodes["alice"], LEG)?;
        for _ in 0..repeats {
            let again = case
                .client
                .prepare_refund(&bob, &id, &first.leg_id)
                .await
                .map_err(common::safe_adp)?;
            ensure!(
                again.attempt_id == prepared.attempt_id && again.bolt11 == prepared.bolt11,
                "refund preparation created a second payable invoice"
            );
        }
        let audit = case.root.join("refund-wallet-audit.json");
        if name == "refund-process-crash" {
            let path = case.root.join("refund-job.json");
            let job = RefundJob {
                base: case.base.clone(),
                quote_id: id.clone(),
                leg_id: first.leg_id.clone(),
                attempt_id: prepared.attempt_id.clone(),
                bolt11: prepared.bolt11.clone(),
                client_db: case.root.join("client.db"),
                audit: audit.clone(),
                result: case.root.join("refund-child-result.json"),
            };
            common::write_private(&path, &job)?;
            let executable = std::env::current_exe()?;
            let initial = tokio::process::Command::new(&executable)
                .arg("refund-crash-child")
                .arg(&path)
                .output()
                .await?;
            ensure!(
                initial.status.code() == Some(86),
                "refund child did not crash at expected boundary (output suppressed)"
            );
            let refund_hash = prepared
                .bolt11
                .parse::<lightning_invoice::Bolt11Invoice>()
                .map_err(|_| anyhow!("invalid refund invoice"))?
                .payment_hash()
                .to_string();
            let count:i64=sqlx::query_scalar("SELECT COUNT(*) FROM nwc_payment_journal WHERE account_pubkey=? AND payment_hash=? AND encrypted_payment IS NULL")
                .bind(case.bob.public_key().to_hex()).bind(&refund_hash).fetch_one(case.db.pool()).await?;
            ensure!(count == 1, "missing exact Bob refund dispatch checkpoint");
            let pending:i64=sqlx::query_scalar("SELECT COUNT(*) FROM refund_attempts WHERE quote_id=? AND attempt_id=? AND payment_hash=? AND verified_at IS NULL")
                .bind(&id).bind(&prepared.attempt_id).bind(&refund_hash).fetch_one(&case.store.pool()).await?;
            ensure!(
                pending == 1,
                "refund proof unexpectedly persisted before crash recovery"
            );
            ensure!(
                common::lnd("alice", "lookupinvoice", &[&refund_hash])?["settled"] == true,
                "refund not settled before crash"
            );
            for _ in 0..2 {
                let resumed = tokio::process::Command::new(&executable)
                    .arg("refund-resume-child")
                    .arg(&path)
                    .output()
                    .await?;
                ensure!(
                    resumed.status.success(),
                    "refund resume failed (private output suppressed)"
                );
            }
            result["refund_crash_exit"] = json!(86);
        } else {
            if name == "refund-outage-and-retry" {
                let offline = RefundWallet {
                    audit: audit.clone(),
                    crash: false,
                    unavailable: true,
                };
                ensure!(
                    payments
                        .pay_or_resume(&bob, &offline, &prepared.bolt11)
                        .await
                        .is_err(),
                    "expected pre-dispatch outage"
                );
                ensure!(!audit.exists(), "outage unexpectedly dispatched a refund");
                result["refund_failure_injection"] =
                    json!("wallet unavailable before dispatch; no payment intent/payment sent");
            }
            let refund_wallet = RefundWallet {
                audit: audit.clone(),
                crash: false,
                unavailable: false,
            };
            let paid = payments
                .pay_or_resume(&bob, &refund_wallet, &prepared.bolt11)
                .await?;
            for _ in 0..repeats {
                let cached = payments
                    .pay_or_resume(&bob, &refund_wallet, &prepared.bolt11)
                    .await?;
                ensure!(cached == paid, "retry returned another refund proof");
                let confirmed = case
                    .client
                    .confirm_refund(
                        &bob,
                        &id,
                        &first.leg_id,
                        &prepared.attempt_id,
                        &paid.preimage,
                    )
                    .await
                    .map_err(common::safe_adp)?;
                ensure!(
                    confirmed["state"] == "verified",
                    "refund confirmation failed"
                );
            }
        }
        let audit_value: Value = serde_json::from_slice(&std::fs::read(&audit)?)?;
        ensure!(audit_value["pay_calls"] == 1, "duplicate refund dispatch");
        let refund_hash = prepared
            .bolt11
            .parse::<lightning_invoice::Bolt11Invoice>()
            .map_err(|_| anyhow!("invalid refund invoice"))?
            .payment_hash()
            .to_string();
        let outgoing = common::lnd("bob", "listpayments", &["--include_incomplete"])?;
        let matching: Vec<_> = outgoing["payments"]
            .as_array()
            .context("Bob outgoing payments missing")?
            .iter()
            .filter(|p| p["payment_hash"].as_str() == Some(&refund_hash))
            .collect();
        ensure!(
            matching.len() == 1
                && matching[0]["status"] == "SUCCEEDED"
                && common::num(&matching[0]["value_msat"])? == LEG,
            "Bob refund payment mismatch"
        );
        ensure!(
            common::lnd("alice", "lookupinvoice", &[&refund_hash])?["settled"] == true,
            "Alice refund invoice not settled"
        );
        let checkpoint:i64=sqlx::query_scalar("SELECT COUNT(*) FROM nwc_payment_journal WHERE account_pubkey=? AND payment_hash=? AND encrypted_payment IS NOT NULL")
            .bind(case.bob.public_key().to_hex()).bind(&refund_hash).fetch_one(case.db.pool()).await?;
        ensure!(checkpoint == 1, "Bob refund proof not durably checkpointed");
        let state: String =
            sqlx::query_scalar("SELECT state FROM refund_obligations WHERE quote_id=?")
                .bind(&id)
                .fetch_one(&case.store.pool())
                .await?;
        let count: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM refund_attempts WHERE quote_id=? AND verified_at IS NOT NULL AND proof_encrypted IS NOT NULL",
        )
        .bind(&id)
        .fetch_one(&case.store.pool())
        .await?;
        ensure!(
            state == "verified" && count == 1,
            "refund persisted state mismatch"
        );
        ensure!(
            case.client
                .prepare_refund(&bob, &id, &first.leg_id)
                .await
                .is_err(),
            "verified obligation yielded another invoice"
        );
        result["refund_state"] = json!(state);
        result["refund_attempt_id"] = json!(prepared.attempt_id);
        result["refund_principal_msat"] = json!(LEG);
        result["refund_dispatch_audit"] = audit_value;
    } else {
        let count: i64 =
            sqlx::query_scalar("SELECT COUNT(*) FROM refund_obligations WHERE quote_id=?")
                .bind(&id)
                .fetch_one(&case.store.pool())
                .await?;
        ensure!(count == 0, "unpaid purchase created a refund obligation");
        result["refund_state"] = json!("no_obligation");
    }
    let after = common::balances()?;
    let client_proofs:i64=sqlx::query_scalar("SELECT COUNT(*) FROM nwc_payment_journal WHERE account_pubkey=? AND encrypted_payment IS NOT NULL")
        .bind(case.buyer.public_key().to_hex()).fetch_one(case.db.pool()).await?;
    let server_proofs:i64=sqlx::query_scalar("SELECT COUNT(*) FROM invoice_attempts WHERE quote_id=? AND settled_at IS NOT NULL AND proof_encrypted IS NOT NULL")
        .bind(&id).fetch_one(&case.store.pool()).await?;
    ensure!(
        client_proofs == if no_paid { 0 } else { 1 } && server_proofs == client_proofs,
        "purchase proof persistence mismatch"
    );
    ensure!(
        wallet.pays.load(Ordering::SeqCst) == if no_paid { 1 } else { 2 },
        "unexpected purchase dispatch count"
    );
    result["persisted_purchase_proofs"] = json!(server_proofs);
    ensure!(
        after == before,
        "final balances did not reconcile after principal refund"
    );
    let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM purchases")
        .fetch_one(&case.store.pool())
        .await?;
    let tokens: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM download_tokens")
        .fetch_one(&case.store.pool())
        .await?;
    ensure!(
        count == 0
            && tokens == 0
            && !case
                .relay
                .events
                .lock()
                .expect("relay")
                .iter()
                .any(|e| e.kind.as_u16() == 1020),
        "failed purchase minted ownership/receipt/token"
    );
    ensure!(
        case.client
            .download(
                &quote.listing_coordinate,
                DownloadAuth::Nip98 { signer: &buyer },
                &case.root.join("must-not-download.bin"),
                |_, _| {}
            )
            .await
            .is_err(),
        "failed purchase downloaded game"
    );
    result["balances_after_msat"] = json!(after);
    result["nwc_pay_dispatches"] = json!(wallet.pays.load(Ordering::SeqCst));
    result["status"] = json!("pass");
    common::write_private(&case.root.join("result.json"), &result)?;
    if let Some(stop) = case.stop.take() {
        let _ = stop.send(());
    }
    let _ = case.task.await;
    Ok(result)
}

async fn run() -> Result<()> {
    common::check_network()?;
    let budget: u64 = std::env::var("ARCADESTR_REGTEST_PAYMENT_BUDGET_MSAT")?.parse()?;
    ensure!(
        (1_980_000..=50_000_000).contains(&budget),
        "refund suite budget invalid"
    );
    let root = common::env_path("ARCADESTR_REGTEST_REFUND_RUN_DIR")?;
    std::fs::create_dir_all(&root)?;
    let (buyer, bob) = common::accounts().await?;
    let operator = Keys::parse(&std::env::var("ADP_OPERATOR_NSEC")?)
        .map_err(|_| anyhow!("operator signer invalid"))?;
    ensure!(
        operator.public_key().to_hex() == std::env::var("POLAR_CAROL_NOSTR_PUBKEY")?,
        "existing operator mismatch"
    );
    let mut results = vec![];
    for name in CASES {
        let case = setup(root.join(name), name, &buyer, &bob, &operator).await?;
        results.push(
            run_case(case, name)
                .await
                .with_context(|| format!("case {name}"))?,
        );
    }
    let report = json!({"suite":"failed-partial-settlements-and-refunds","cases":results,"status":"pass",
        "max_reserved_msat":1980000,"purchase_price_sats":200,"developer_sats":100,"operator_sats":100,
        "invoice_order":"amount, role, recipient; developer first for equal amounts","real_lightning":true,"isolated_http_and_databases":true,
        "nostr":"controlled test adapter for deterministic deadlines","refund_retry_failure":"pre-dispatch outage; ambiguous outcomes still blocked"});
    common::write_private(&root.join("report.json"), &report)?;
    println!("{}", report);
    Ok(())
}

#[tokio::main]
async fn main() {
    let action = std::env::args().nth(1).unwrap_or_default();
    let result = match action.as_str() {
        "run" => run().await,
        "refund-crash-child" | "refund-resume-child" => match std::env::args().nth(2) {
            Some(path) => refund_child(Path::new(&path), action == "refund-resume-child").await,
            None => Err(anyhow!("private refund job missing")),
        },
        _ => Err(anyhow!("invoke through scripts/regtest test --refunds")),
    };
    if let Err(error) = result {
        eprintln!("refund regtest failed: {error:#}");
        std::process::exit(1);
    }
}
