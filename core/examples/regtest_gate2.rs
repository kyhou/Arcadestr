//! Gate 2: one three-recipient purchase and an isolated real third-leg failure.
//! Production ADP/NWC/durable storage paths; independent persistent Dave process.
#[allow(dead_code)]
#[path = "regtest_split_purchase.rs"]
mod common;
#[allow(dead_code)]
#[path = "regtest_gate2_preflight.rs"]
mod protected;
use adp_server::{
    config::Config, keystore::KeyStore, relay::RelayClient, routes,
    settlement_clock::SettlementClock, state::AppState, storage::Storage,
};
use anyhow::{anyhow, ensure, Context, Result};
use arcadestr_core::{
    adp_client::{
        AdpClient, CreateSettlementQuoteRequest, DownloadAuth, SettlementInvoiceResponse,
        SettlementProofRequest, SettlementProofResponse,
    },
    adp_discovery::parse_adp_server_announcements,
    adp_publish::{
        apply_managed_adp_terms, build_adp_listing_event_builder,
        build_fulfillment_authorization_event_builder, AdpListingInput,
        FulfillmentAuthorizationInput,
    },
    adp_settlement::{
        build_terms_event_builder, parse_terms_event, verify_quote_event, AcceptedTermsRef,
        CommercialTerms, FeePolicy, QuoteState, RefundDestination, ServerOperator, SignedQuote,
        TermsPayload,
    },
    adp_storage::{DownloadToken, DownloadTokensRepository},
    authorization::{
        resolve_authorization_at, AuthorizationTerms, CAPABILITY_ISSUE_RECEIPT,
        CAPABILITY_UPLOAD_BUILD,
    },
    durable_payments::{DurableNwcPayments, PaymentWallet},
    http_client::{HttpClient, ReqwestHttpClient},
    nwc_client::{load_nwc_connection, NwcClient, NwcClientError, PayInvoiceResult},
    purchases::{
        parse_and_validate_receipt_with_authorization, parse_receipt_event, PurchasesRepository,
        ReceiptEvidence,
    },
    settlement_repository::SettlementRepository,
    signers::NostrSigner,
    storage::Database,
};
use async_trait::async_trait;
use nostr::{Event, EventId, Filter, Keys, Kind, PublicKey, Timestamp};
use nostr_sdk::Client;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc,
    },
    time::Duration,
};
const CAROL: &str = "http://localhost:9099";
const DAVE: &str = "http://127.0.0.1:19100";
const GROSS: u64 = 1_000_000;
const A_FEE: u64 = 100_000;
const B_FEE: u64 = 150_000;
const RELAYS: [&str; 4] = [
    "ws://127.0.0.1:7777",
    "ws://127.0.0.1:7778",
    "ws://127.0.0.1:10547",
    "ws://127.0.0.1:10548",
];
fn now() -> u64 {
    Timestamp::now().as_secs()
}
fn http() -> Result<Arc<dyn HttpClient>> {
    Ok(Arc::new(ReqwestHttpClient::new(Duration::from_secs(30))?))
}
fn client(url: &str) -> Result<AdpClient> {
    Ok(AdpClient::new(url, http()?))
}
fn read<T: serde::de::DeserializeOwned>(path: &Path) -> Result<T> {
    Ok(serde_json::from_slice(&std::fs::read(path)?)?)
}
async fn actors() -> Result<(Keys, Keys, Keys, Keys)> {
    let (buyer, bob) = common::accounts().await?;
    let carol = Keys::parse(&std::env::var("ADP_OPERATOR_NSEC")?)
        .map_err(|_| anyhow!("Carol signer invalid"))?;
    let secret = protected::private_file(&common::env_path("ARCADESTR_REGTEST_DAVE_SIGNER_FILE")?)?;
    let dave =
        Keys::parse(secret.as_str()).map_err(|_| anyhow!("protected Dave signer invalid"))?;
    ensure!(
        dave.public_key().to_hex() == std::env::var("POLAR_DAVE_NOSTR_PUBKEY")?
            && carol.public_key().to_hex() == std::env::var("POLAR_CAROL_NOSTR_PUBKEY")?
            && carol.public_key() != dave.public_key(),
        "independent operator identities mismatch"
    );
    Ok((buyer, bob, carol, dave))
}
async fn relay(keys: &Keys) -> Result<Client> {
    let c = Client::new(keys.clone());
    for url in RELAYS {
        c.add_relay(url).await?;
    }
    c.connect().await;
    Ok(c)
}
async fn publish(c: &Client, event: &Event) -> Result<()> {
    event.verify()?;
    for url in RELAYS {
        ensure!(
            !c.send_event_to([url], event).await?.success.is_empty(),
            "relay did not acknowledge signed fixture"
        );
    }
    Ok(())
}
#[derive(Clone, Serialize, Deserialize)]
struct Operators {
    carol_terms: Event,
    dave_terms: Event,
    carol: String,
    dave: String,
}
async fn prepare() -> Result<()> {
    let (_, _, carol, dave) = actors().await?;
    let root = common::env_path("ARCADESTR_REGTEST_OPERATOR_DIR")?;
    std::fs::create_dir_all(&root)?;
    let baseline: Value = read(&common::env_path("ARCADESTR_REGTEST_CAROL_MANIFEST")?)?;
    let retained: Event = serde_json::from_value(baseline["terms"].clone())?;
    let active_id = EventId::from_hex(&std::env::var("ADP_TERMS_EVENT_ID")?)?;
    let c = relay(&dave).await?;
    let carol_terms: Event = if retained.id == active_id {
        retained
    } else {
        c.fetch_events(Filter::new().id(active_id), Duration::from_secs(10))
            .await?
            .into_iter()
            .find(|event| event.id == active_id)
            .context("Carol currently advertised immutable terms unavailable")?
    };
    ensure!(
        matches!(parse_terms_event(&carol_terms)?,TermsPayload::Offer{terms} if terms.operator_pubkey==carol.public_key().to_hex() && terms.fee==FeePolicy::Fixed{amount_msat:A_FEE}),
        "Carol baseline terms mismatch"
    );
    let path = root.join(format!("{}.terms.json", dave.public_key()));
    let dave_terms: Event = if path.exists() {
        read(&path)?
    } else {
        let event = build_terms_event_builder(&TermsPayload::Offer {
            terms: CommercialTerms {
                version: 1,
                operator_pubkey: dave.public_key().to_hex(),
                payment_destination: "dave@127.0.0.1:18080".into(),
                fee: FeePolicy::Fixed { amount_msat: B_FEE },
                limitations: vec!["local regtest operator B".into()],
            },
        })?
        .sign_with_keys(&dave)?;
        common::write_private(&path, &event)?;
        event
    };
    ensure!(
        matches!(parse_terms_event(&dave_terms)?,TermsPayload::Offer{terms} if terms.operator_pubkey==dave.public_key().to_hex() && terms.fee==FeePolicy::Fixed{amount_msat:B_FEE} && terms.payment_destination=="dave@127.0.0.1:18080"),
        "Dave persisted terms mismatch"
    );
    publish(&c, &carol_terms).await?;
    publish(&c, &dave_terms).await?;
    let announcement_path = root.join("carol-gate2-announcement.json");
    let announcement: Event = if announcement_path.exists() {
        read(&announcement_path)?
    } else {
        let event = adp_server::announcement::build_server_announcement_event(
            &carol,
            "gate2-carol-regtest",
            "Carol regtest operator A",
            CAROL,
            "",
        )
        .map_err(|_| anyhow!("Carol announcement signing failed"))?;
        common::write_private(&announcement_path, &event)?;
        event
    };
    publish(&c, &announcement).await?;
    c.shutdown().await;
    let operators = Operators {
        carol_terms,
        dave_terms: dave_terms.clone(),
        carol: carol.public_key().to_hex(),
        dave: dave.public_key().to_hex(),
    };
    let metadata = root.join("operators.json");
    if metadata.exists() {
        let previous: Operators = read(&metadata)?;
        if previous.carol_terms.id != operators.carol_terms.id {
            common::write_private(
                &root.join(format!("operators-before-{}.json", previous.carol_terms.id)),
                &previous,
            )?;
        }
    }
    common::write_private(&metadata, &operators)?;
    println!(
        "{}",
        json!({"dave_pubkey":dave.public_key().to_hex(),"dave_terms_event_id":dave_terms.id.to_hex(),"operator_metadata":root.join("operators.json")})
    );
    Ok(())
}
#[derive(Clone, Serialize, Deserialize)]
struct Manifest {
    listing: Event,
    authorizations: Vec<Event>,
    operators: Operators,
    buyer: String,
    developer: String,
    coordinate: String,
    base: String,
    file_hash: String,
}
async fn announced(c: &Client, expected: &[(&str, &str, &Event)]) -> Result<Vec<Event>> {
    let events = c
        .fetch_events(
            Filter::new().kind(Kind::Custom(30403)),
            Duration::from_secs(10),
        )
        .await?;
    let events: Vec<_> = events.into_iter().collect();
    let parsed = parse_adp_server_announcements(events.clone());
    let mut found = vec![];
    for (url, key, terms) in expected {
        ensure!(
            parsed.iter().any(|a| a.url == *url
                && a.pubkey == *key
                && a.split_enabled
                && a.terms_event_id.as_deref() == Some(&terms.id.to_hex())),
            "missing or conflicting signed operator announcement"
        );
        let event = events
            .iter()
            .find(|e| {
                e.pubkey.to_hex() == *key
                    && e.tags.iter().any(|t| t.as_slice() == ["url", *url])
                    && e.tags
                        .iter()
                        .any(|t| t.as_slice() == ["terms", terms.id.to_hex().as_str()])
            })
            .context("announcement missing")?;
        event.verify()?;
        found.push(event.clone());
    }
    Ok(found)
}
async fn manifest(root: &Path, base: &str, issued: u64) -> Result<Manifest> {
    let (buyer, bob, carol, dave) = actors().await?;
    let operators: Operators =
        read(&common::env_path("ARCADESTR_REGTEST_OPERATOR_DIR")?.join("operators.json"))?;
    let a = client(base)?;
    let b = client(DAVE)?;
    for (adp, key, term, fee) in [
        (&a, &carol, &operators.carol_terms, A_FEE),
        (&b, &dave, &operators.dave_terms, B_FEE),
    ] {
        let info = adp.well_known().await.map_err(common::safe_adp)?;
        let config = info.settlement.context("split metadata absent")?;
        ensure!(
            info.pubkey == key.public_key().to_hex()
                && config.split_enabled
                && config.terms_event_id.as_deref() == Some(&term.id.to_hex())
                && config.fee == FeePolicy::Fixed { amount_msat: fee },
            "independent HTTP operator terms mismatch"
        );
    }
    let c = relay(&bob).await?;
    let announcements = announced(
        &c,
        &[
            (base, &operators.carol, &operators.carol_terms),
            (DAVE, &operators.dave, &operators.dave_terms),
        ],
    )
    .await?;
    common::write_private(&root.join("announcements.json"), &announcements)?;
    let d = format!("regtest-gate2-{}", uuid::Uuid::new_v4());
    let coordinate = format!("30402:{}:{d}", bob.public_key());
    let mut refs = vec![];
    let mut authorizations = vec![];
    for (adp, key) in [(&a, &carol), (&b, &dave)] {
        let provision = adp
            .provision(&common::signer(&bob)?, Some(&coordinate))
            .await
            .map_err(common::safe_adp)?;
        let fulfillment = PublicKey::from_hex(&provision.fulfillment_pubkey)?;
        let event = build_fulfillment_authorization_event_builder(&AuthorizationTerms {
            authorization_id: format!("{d}-{}", key.public_key()),
            coordinate: coordinate.clone(),
            operator_pubkey: key.public_key(),
            fulfillment_pubkey: fulfillment,
            capabilities: [
                CAPABILITY_ISSUE_RECEIPT.to_owned(),
                CAPABILITY_UPLOAD_BUILD.to_owned(),
            ]
            .into_iter()
            .collect(),
            valid_from: issued,
        })?
        .sign_with_keys(&bob)?;
        refs.push(FulfillmentAuthorizationInput {
            root_event_id: event.id.to_hex(),
            fulfillment_pubkey: fulfillment.to_hex(),
            relay_hint: None,
        });
        publish(&c, &event).await?;
        authorizations.push(event);
    }
    let payload = common::env_path("ARCADESTR_REGTEST_PAYLOAD")?;
    let file_hash = hex::encode(Sha256::digest(std::fs::read(&payload)?));
    let mut input = AdpListingInput {
        d_tag: d,
        title: "Gate 2 isolated multi-operator purchase".into(),
        description: "local regtest".into(),
        price_sats: 1000,
        lud16: Some("bob@127.0.0.1:18080".into()),
        tags: vec![],
        images: vec![],
        servers: vec![base.into(), DAVE.into()],
        file_hash: Some(file_hash.clone()),
        version: Some("gate2-v1".into()),
        fulfillment_authorizations: refs,
        acquisition: arcadestr_core::marketplace::AcquisitionPolicy::Gated,
        platforms: vec![],
        campaigns: vec![],
        nip94_event_id: None,
        preserved_tags: vec![],
    };
    apply_managed_adp_terms(
        &mut input,
        &[
            ServerOperator {
                url: base.into(),
                operator_pubkey: operators.carol.clone(),
            },
            ServerOperator {
                url: DAVE.into(),
                operator_pubkey: operators.dave.clone(),
            },
        ],
        &[
            AcceptedTermsRef {
                operator_pubkey: operators.carol.clone(),
                terms_event_id: operators.carol_terms.id.to_hex(),
            },
            AcceptedTermsRef {
                operator_pubkey: operators.dave.clone(),
                terms_event_id: operators.dave_terms.id.to_hex(),
            },
        ],
    )?;
    let listing = build_adp_listing_event_builder(&input)?.sign_with_keys(&bob)?;
    publish(&c, &listing).await?;
    for adp in [&a, &b] {
        adp.upload(&common::signer(&bob)?, &listing, &payload)
            .await
            .map_err(common::safe_adp)?;
    }
    c.shutdown().await;
    let result = Manifest {
        listing,
        authorizations,
        operators,
        buyer: buyer.public_key().to_hex(),
        developer: bob.public_key().to_hex(),
        coordinate,
        base: base.into(),
        file_hash,
    };
    common::write_private(&root.join("manifest.json"), &result)?;
    Ok(result)
}
struct Wallet {
    nwc: Option<NwcClient>,
    payer: String,
    audit: PathBuf,
}
impl Wallet {
    fn count(&self, method: &str, hash: &str) -> Result<(), NwcClientError> {
        let mut v: Value = if self.audit.exists() {
            read(&self.audit).map_err(|_| NwcClientError::InvalidLookup)?
        } else {
            json!({"pay_calls":{},"lookup_calls":{}})
        };
        v[method][hash] = json!(v[method][hash].as_u64().unwrap_or(0) + 1);
        common::write_private(&self.audit, &v).map_err(|_| NwcClientError::InvalidLookup)
    }
}
#[async_trait]
impl PaymentWallet for Wallet {
    fn identity(&self) -> Result<String, NwcClientError> {
        match &self.nwc {
            Some(nwc) => nwc.payment_identity(),
            None => Ok(format!(
                "polar-lnd-regtest:{}",
                common::check_network().map_err(|_| NwcClientError::InvalidLookup)?[&self.payer]
            )),
        }
    }
    async fn pay(&self, invoice: &str) -> Result<PayInvoiceResult, NwcClientError> {
        let hash = invoice
            .parse::<lightning_invoice::Bolt11Invoice>()
            .map_err(|_| NwcClientError::InvalidLookup)?
            .payment_hash()
            .to_string();
        self.count("pay_calls", &hash)?;
        if let Some(nwc) = &self.nwc {
            return nwc.pay_invoice(invoice).await;
        }
        let nodes = common::check_network().map_err(|_| NwcClientError::InvalidLookup)?;
        let parsed = invoice
            .parse::<lightning_invoice::Bolt11Invoice>()
            .map_err(|_| NwcClientError::InvalidLookup)?;
        common::validate_invoice(
            invoice,
            &nodes["alice"],
            parsed
                .amount_milli_satoshis()
                .ok_or(NwcClientError::InvalidLookup)?,
        )
        .map_err(|_| NwcClientError::InvalidLookup)?;
        let value = common::lnd(
            &self.payer,
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
        if value["status"] != "SUCCEEDED" {
            return Err(NwcClientError::WalletTimeout);
        }
        Ok(PayInvoiceResult {
            preimage: value["payment_preimage"]
                .as_str()
                .ok_or(NwcClientError::InvalidLookup)?
                .into(),
            fees_paid_msat: Some(
                common::num(&value["fee_msat"]).map_err(|_| NwcClientError::InvalidLookup)?,
            ),
        })
    }
    async fn lookup(
        &self,
        hash: &str,
        amount: u64,
    ) -> Result<Option<PayInvoiceResult>, NwcClientError> {
        self.count("lookup_calls", hash)?;
        if let Some(nwc) = &self.nwc {
            return nwc.lookup_payment(hash, amount).await;
        }
        let values = common::lnd(&self.payer, "listpayments", &["--include_incomplete"])
            .map_err(|_| NwcClientError::InvalidLookup)?;
        let record = values["payments"]
            .as_array()
            .ok_or(NwcClientError::InvalidLookup)?
            .iter()
            .find(|v| v["payment_hash"].as_str() == Some(hash));
        Ok(match record {
            Some(v)
                if v["status"] == "SUCCEEDED"
                    && common::num(&v["value_msat"]).ok() == Some(amount) =>
            {
                Some(PayInvoiceResult {
                    preimage: v["payment_preimage"]
                        .as_str()
                        .ok_or(NwcClientError::InvalidLookup)?
                        .into(),
                    fees_paid_msat: Some(
                        common::num(&v["fee_msat"]).map_err(|_| NwcClientError::InvalidLookup)?,
                    ),
                })
            }
            _ => None,
        })
    }
}
async fn buyer_wallet(buyer: &Keys, audit: PathBuf) -> Result<Wallet> {
    let connection = load_nwc_connection(&buyer.public_key())
        .map_err(|_| anyhow!("existing NWC unavailable"))?;
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
    let key:String=hub.query_row("SELECT wallet_pubkey FROM apps WHERE name IN ('Arcadestr Regtest Buyer','Alice Arcadestr Regtest Buyer')",[],|r|r.get(0))?;
    ensure!(
        backend == "LND"
            && address == "127.0.0.1:10001"
            && connection.wallet_pubkey_hex() == key
            && !connection.relay_urls().is_empty()
            && connection
                .relay_urls()
                .iter()
                .all(|u| matches!(u.as_str(), "ws://127.0.0.1:7778" | "ws://127.0.0.1:7778/")),
        "buyer NWC binding changed"
    );
    drop(hub);
    let nwc = NwcClient::new(connection).with_timeout(Duration::from_secs(30));
    nwc.verify()
        .await
        .map_err(|_| anyhow!("buyer NWC offline"))?;
    Ok(Wallet {
        nwc: Some(nwc),
        payer: "alice".into(),
        audit,
    })
}
fn request(m: &Manifest, id: &str) -> CreateSettlementQuoteRequest {
    CreateSettlementQuoteRequest {
        purchase_id: id.into(),
        listing_event: m.listing.clone(),
        terms_events: vec![
            m.operators.carol_terms.clone(),
            m.operators.dave_terms.clone(),
        ],
        refund_destination: RefundDestination {
            endpoint: "alice@127.0.0.1:18080".into(),
            min_sendable_msat: 1000,
            max_sendable_msat: 10_000_000,
        },
        zap_requests: Default::default(),
    }
}
fn validate_quote(m: &Manifest, q: &SignedQuote) -> Result<()> {
    ensure!(
        q.gross_msat == GROSS
            && q.legs.len() == 3
            && q.listing_event_id == m.listing.id.to_hex()
            && q.buyer_pubkey == m.buyer
            && q.processor_pubkey == m.operators.carol,
        "quote identity/shape mismatch"
    );
    for (index, (amount, key, destination, terms)) in [
        (
            A_FEE,
            &m.operators.carol,
            "carol@127.0.0.1:18080",
            Some(m.operators.carol_terms.id.to_hex()),
        ),
        (
            B_FEE,
            &m.operators.dave,
            "dave@127.0.0.1:18080",
            Some(m.operators.dave_terms.id.to_hex()),
        ),
        (
            GROSS - A_FEE - B_FEE,
            &m.developer,
            "bob@127.0.0.1:18080",
            None,
        ),
    ]
    .into_iter()
    .enumerate()
    {
        let l = &q.legs[index];
        ensure!(
            l.amount_msat == amount
                && l.recipient_pubkey == *key
                && l.destination == destination
                && l.role
                    == if index < 2 {
                        arcadestr_core::adp_settlement::PaymentRole::Operator
                    } else {
                        arcadestr_core::adp_settlement::PaymentRole::Developer
                    }
                && l.accepted_terms_id == terms,
            "quote allocation/order mismatch"
        );
    }
    Ok(())
}
async fn success(root: &Path, resume: bool) -> Result<()> {
    let (buyer, _, _, _) = actors().await?;
    let signer = common::signer(&buyer)?;
    let m: Manifest = if resume {
        read(&root.join("manifest.json"))?
    } else {
        manifest(root, CAROL, now()).await?
    };
    let adp = client(&m.base)?;
    let db = Database::new(&root.join("client.db")).await?;
    let repository = SettlementRepository::new(db.pool().clone());
    let wallet = buyer_wallet(&buyer, root.join("wallet-audit.json")).await?;
    let payments = DurableNwcPayments::new(db.pool().clone());
    let mut evidence: Value = if resume {
        read(&root.join("purchase.json"))?
    } else {
        json!({"purchase_id":format!("regtest-gate2-{}",uuid::Uuid::new_v4()),"state":"prepared","payments":[],"balances_before_msat":common::balances()?})
    };
    let id = evidence["purchase_id"]
        .as_str()
        .context("purchase ID")?
        .to_owned();
    let quote_event: Event = if resume {
        read(&root.join("quote.json"))?
    } else {
        let event = adp
            .create_settlement_quote(&signer, request(&m, &id))
            .await
            .map_err(common::safe_adp)?
            .quote;
        common::write_private(&root.join("quote.json"), &event)?;
        event
    };
    let q = verify_quote_event(
        &quote_event,
        &signer
            .nip44_decrypt(&quote_event.pubkey, &quote_event.content)
            .await?,
    )?;
    validate_quote(&m, &q)?;
    evidence["quote_event_id"] = json!(quote_event.id.to_hex());
    common::write_private(&root.join("purchase.json"), &evidence)?;
    if !resume {
        repository
            .save_quote(
                &m.buyer,
                &m.base,
                &serde_json::to_string(&quote_event)?,
                &q,
                now(),
            )
            .await?;
    }
    let nodes = common::check_network()?;
    let mut final_response = None;
    for (index, leg) in q.legs.iter().enumerate() {
        let saved = evidence["payments"]
            .as_array()
            .context("payments")?
            .iter()
            .find(|p| p["leg_id"] == leg.leg_id)
            .cloned();
        if saved
            .as_ref()
            .is_some_and(|p| p["state"] == "proof_accepted")
        {
            continue;
        }
        let invoice: SettlementInvoiceResponse = if let Some(p) = &saved {
            read(&root.join(format!(
                "{}.invoice.json",
                p["attempt_id"].as_str().context("attempt")?
            )))?
        } else {
            ensure!(
                !repository.has_unreconciled_payment(&m.buyer, &id).await?,
                "earlier payment unresolved"
            );
            adp.settlement_next_invoice(&signer, &id)
                .await
                .map_err(common::safe_adp)?
        };
        let node = ["carol", "dave", "bob"][index];
        ensure!(
            invoice.leg_id == leg.leg_id
                && invoice.amount_msat == leg.amount_msat
                && common::check_network()? == nodes,
            "invoice binding/network mismatch"
        );
        common::validate_invoice(&invoice.bolt11, &nodes[node], leg.amount_msat)?;
        if saved.is_none() {
            let encrypted = signer
                .nip44_encrypt(&buyer.public_key(), &serde_json::to_string(&invoice)?)
                .await?;
            repository
                .record_invoice_attempt(
                    &m.buyer,
                    &id,
                    &leg.leg_id,
                    &invoice.attempt_id,
                    encrypted.as_bytes(),
                    &invoice.payment_hash,
                    "disclosed",
                    now(),
                )
                .await?;
            common::write_private(
                &root.join(format!("{}.invoice.json", invoice.attempt_id)),
                &invoice,
            )?;
            evidence["payments"].as_array_mut().context("payments")?.push(json!({"leg_id":leg.leg_id,"attempt_id":invoice.attempt_id,"payment_hash":invoice.payment_hash,"recipient_node":node,"amount_msat":leg.amount_msat,"state":"reserved"}));
            common::write_private(&root.join("purchase.json"), &evidence)?;
        }
        let paid = payments
            .pay_or_resume(&signer, &wallet, &invoice.bolt11)
            .await
            .map_err(|_| {
                anyhow!("payment failed/uncertain; original intent retained, never redispatch")
            })?;
        let encrypted = signer
            .nip44_encrypt(&buyer.public_key(), &paid.preimage)
            .await?;
        ensure!(
            repository
                .checkpoint_proof(&m.buyer, &id, &invoice.attempt_id, encrypted.as_bytes())
                .await?,
            "proof checkpoint failed"
        );
        if index == 0 && !resume {
            evidence["state"] = json!("crash_after_first_proof_checkpoint");
            common::write_private(&root.join("purchase.json"), &evidence)?;
            std::process::exit(86);
        }
        let response = adp
            .submit_settlement_proof(
                &signer,
                &id,
                SettlementProofRequest {
                    attempt_id: invoice.attempt_id.clone(),
                    preimage: paid.preimage.clone(),
                },
            )
            .await
            .map_err(common::safe_adp)?;
        let retry = adp
            .submit_settlement_proof(
                &signer,
                &id,
                SettlementProofRequest {
                    attempt_id: invoice.attempt_id.clone(),
                    preimage: paid.preimage,
                },
            )
            .await
            .map_err(common::safe_adp)?;
        ensure!(
            response.receipt.as_ref().map(|e| e.id) == retry.receipt.as_ref().map(|e| e.id),
            "proof retry changed receipt identity"
        );
        if index < 2 {
            ensure!(
                response.receipt.is_none() && response.download_token.is_none(),
                "partial purchase granted ownership"
            );
        }
        repository
            .update_invoice_state(&m.buyer, &invoice.attempt_id, "settled", now())
            .await?;
        let entry = evidence["payments"]
            .as_array_mut()
            .context("payments")?
            .iter_mut()
            .find(|p| p["leg_id"] == leg.leg_id)
            .context("payment")?;
        entry["state"] = json!("proof_accepted");
        entry["fee_msat"] = json!(paid.fees_paid_msat.unwrap_or(0));
        common::write_private(&root.join("purchase.json"), &evidence)?;
        final_response = Some(retry);
    }
    finish(
        root,
        &m,
        &buyer,
        &db,
        &q,
        &id,
        &mut evidence,
        final_response.context("final proof response absent; use verification")?,
    )
    .await
}
async fn finish(
    root: &Path,
    m: &Manifest,
    buyer: &Keys,
    db: &Database,
    q: &SignedQuote,
    id: &str,
    evidence: &mut Value,
    response: SettlementProofResponse,
) -> Result<()> {
    ensure!(response.state == "settled", "purchase not settled");
    let receipt = response.receipt.context("receipt absent")?;
    receipt.verify()?;
    let parsed = parse_receipt_event(&receipt)?;
    let aggregate = parsed.aggregate.as_ref().context("not aggregate")?;
    ensure!(
        receipt.kind.as_u16() == 1020
            && parsed.order_id == id
            && aggregate.quote_event_id.to_hex() == evidence["quote_event_id"]
            && aggregate.listing_event_id == m.listing.id
            && aggregate.processor_pubkey.to_hex() == m.operators.carol
            && aggregate.gross_msat == GROSS
            && aggregate.legs.len() == 3,
        "aggregate receipt binding mismatch"
    );
    for leg in &q.legs {
        ensure!(
            aggregate.legs.iter().any(|p| p.leg_id == leg.leg_id
                && p.recipient_pubkey == leg.recipient_pubkey
                && p.amount_msat == leg.amount_msat
                && p.role == leg.role
                && p.accepted_terms_id == leg.accepted_terms_id
                && p.proof_type == "bolt11-preimage"
                && p.proof_reference == p.payment_hash
                && evidence["payments"]
                    .as_array()
                    .is_some_and(|ps| ps.iter().any(|v| v["payment_hash"] == p.payment_hash))),
            "receipt leg mismatch"
        );
    }
    let auth = resolve_authorization_at(
        m.authorizations[0].id,
        &m.authorizations,
        parsed.settled_at.context("settled timestamp")?,
    )?;
    let stored = parse_and_validate_receipt_with_authorization(
        &receipt,
        &m.buyer,
        Some(&auth),
        ReceiptEvidence::default(),
    )?;
    let purchases = PurchasesRepository::new(db.pool().clone());
    purchases.upsert_receipt(&stored).await?;
    SettlementRepository::new(db.pool().clone())
        .mark_receipt(&m.buyer, id, &serde_json::to_string(&receipt)?, now())
        .await?;
    ensure!(
        purchases.is_owned(&m.buyer, &m.coordinate).await?,
        "buyer ownership not persisted"
    );
    let token = response.download_token.context("token missing")?;
    let expires = response.token_expires_at.context("token expiry")?;
    ensure!(expires > now() as i64, "expired token");
    DownloadTokensRepository::new(db.pool().clone())
        .upsert(&DownloadToken {
            buyer_pubkey: m.buyer.clone(),
            game_coordinate: m.coordinate.clone(),
            server_url: m.base.clone(),
            token: token.clone(),
            expires_at: expires,
        })
        .await?;
    let download = client(&m.base)?
        .download(
            &m.coordinate,
            DownloadAuth::Token(token.clone()),
            &root.join("payload.download"),
            |_, _| {},
        )
        .await
        .map_err(common::safe_adp)?;
    ensure!(
        hex::encode(Sha256::digest(std::fs::read(
            root.join("payload.download")
        )?)) == m.file_hash,
        "download hash mismatch"
    );
    let c = relay(buyer).await?;
    let events = c
        .fetch_events(Filter::new().id(receipt.id), Duration::from_secs(10))
        .await?;
    ensure!(
        events.iter().any(|e| e == &receipt),
        "receipt not retrieved"
    );
    c.shutdown().await;
    let before: BTreeMap<String, u64> =
        serde_json::from_value(evidence["balances_before_msat"].clone())?;
    let after: BTreeMap<String, u64> = if evidence["state"] == "complete" {
        serde_json::from_value(evidence["balances_after_msat"].clone())?
    } else {
        common::balances()?
    };
    let mut fees = 0;
    let audit: Value = read(&root.join("wallet-audit.json"))?;
    let outgoing = common::lnd("alice", "listpayments", &["--include_incomplete"])?;
    for payment in evidence["payments"].as_array().context("payments")? {
        let hash = payment["payment_hash"].as_str().context("hash")?;
        let node = payment["recipient_node"].as_str().context("node")?;
        ensure!(audit["pay_calls"][hash] == 1, "duplicate NWC dispatch");
        let invoice = common::lnd(node, "lookupinvoice", &[hash])?;
        ensure!(
            invoice["settled"] == true
                && common::num(&invoice["amt_paid_msat"])? == common::num(&payment["amount_msat"])?,
            "receiving invoice amount mismatch"
        );
        let matches: Vec<_> = outgoing["payments"]
            .as_array()
            .context("payer history")?
            .iter()
            .filter(|p| p["payment_hash"].as_str() == Some(hash))
            .collect();
        ensure!(
            matches.len() == 1
                && matches[0]["status"] == "SUCCEEDED"
                && common::num(&matches[0]["value_msat"])? == common::num(&payment["amount_msat"])?,
            "duplicate/wrong payer record"
        );
        let fee = common::num(&matches[0]["fee_msat"])?;
        ensure!(fee <= 10_000, "excess routing fee");
        fees += fee;
    }
    ensure!(
        before["alice"] == after["alice"] + GROSS + fees
            && after["bob"] == before["bob"] + 750_000
            && after["carol"] == before["carol"] + A_FEE
            && after["dave"] == before["dave"] + B_FEE,
        "channel balance mismatch"
    );
    let server = rusqlite::Connection::open_with_flags(
        common::env_path("ARCADESTR_REGTEST_ADP_DB")?,
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
    )?;
    let count: i64 = server.query_row(
        "SELECT COUNT(*) FROM payment_legs WHERE quote_id=? AND settled_at IS NOT NULL",
        [id],
        |r| r.get(0),
    )?;
    ensure!(count == 3, "three server proofs absent");
    let proofs: i64 = server.query_row(
        "SELECT COUNT(*) FROM payment_proofs WHERE order_id=?",
        [id],
        |r| r.get(0),
    )?;
    ensure!(proofs == 3, "proof consumption mismatch");
    let ownership: i64 = server.query_row(
        "SELECT COUNT(*) FROM purchases WHERE order_id=? AND buyer_pubkey=? AND receipt_event_id=?",
        rusqlite::params![id, m.buyer, receipt.id.to_hex()],
        |r| r.get(0),
    )?;
    let token_binding:i64=server.query_row("SELECT COUNT(*) FROM download_tokens WHERE token=? AND buyer_pubkey=? AND game_coordinate=? AND expires_at>?",rusqlite::params![token,m.buyer,m.coordinate,now() as i64],|r|r.get(0))?;
    ensure!(
        ownership == 1 && token_binding == 1,
        "server receipt ownership/token buyer binding mismatch"
    );
    let dave = rusqlite::Connection::open_with_flags(
        common::env_path("ARCADESTR_REGTEST_DAVE_DB")?,
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
    )?;
    ensure!(
        dave.query_row(
            "SELECT COUNT(*) FROM purchase_quotes WHERE quote_id=?",
            [id],
            |r| r.get::<_, i64>(0)
        )? == 0,
        "second operator created an extra sale"
    );
    evidence["state"] = json!("complete");
    evidence["receipt_event_id"] = json!(receipt.id.to_hex());
    evidence["receipt_kind"] = json!(1020);
    evidence["ownership_persisted"] = json!(true);
    evidence["download_token_valid"] = json!(true);
    evidence["server_token_buyer_bound"] = json!(true);
    evidence["download_sha256"] = json!(m.file_hash);
    evidence["download_bytes"] = json!(download.bytes_written);
    evidence["balances_after_msat"] = json!(after);
    evidence["routing_fees_msat"] = json!(fees);
    evidence["wallet_dispatch_audit"] = audit;
    common::write_private(&root.join("receipt.json"), &receipt)?;
    common::write_private(&root.join("purchase.json"), evidence)?;
    Ok(())
}
async fn verify_success(root: &Path) -> Result<()> {
    let m: Manifest = read(&root.join("manifest.json"))?;
    let mut evidence: Value = read(&root.join("purchase.json"))?;
    ensure!(
        evidence["state"] == "complete",
        "verification cannot dispatch unfinished purchase"
    );
    let (buyer, _, _, _) = actors().await?;
    let signer = common::signer(&buyer)?;
    let wallet = buyer_wallet(&buyer, root.join("wallet-audit.json")).await?;
    let db = Database::new(&root.join("client.db")).await?;
    let quote_event: Event = read(&root.join("quote.json"))?;
    let q = verify_quote_event(
        &quote_event,
        &signer
            .nip44_decrypt(&quote_event.pubkey, &quote_event.content)
            .await?,
    )?;
    validate_quote(&m, &q)?;
    let last = evidence["payments"]
        .as_array()
        .context("payments")?
        .last()
        .context("last proof")?;
    let attempt = last["attempt_id"].as_str().context("attempt")?;
    let invoice: SettlementInvoiceResponse = read(&root.join(format!("{attempt}.invoice.json")))?;
    let paid = DurableNwcPayments::new(db.pool().clone())
        .reconcile(&signer, &wallet, &invoice.bolt11)
        .await?;
    let id = evidence["purchase_id"]
        .as_str()
        .context("purchase")?
        .to_owned();
    let response = client(&m.base)?
        .submit_settlement_proof(
            &signer,
            &id,
            SettlementProofRequest {
                attempt_id: attempt.into(),
                preimage: paid.preimage,
            },
        )
        .await
        .map_err(common::safe_adp)?;
    finish(root, &m, &buyer, &db, &q, &id, &mut evidence, response).await
}

async fn failure(root: &Path) -> Result<Value> {
    let (buyer, _, carol, dave) = actors().await?;
    let before = common::balances()?;
    let nodes = common::check_network()?;
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
    let base = format!("http://{}", listener.local_addr()?);
    let store = Storage::connect(root.join("adp.db").to_str().context("DB path")?).await?;
    let master: [u8; 32] = hex::decode(std::env::var("ADP_MASTER_KEY")?)?
        .try_into()
        .map_err(|_| anyhow!("private master key length"))?;
    let cfg = Config {
        server_url: base.clone(),
        bind_addr: listener.local_addr()?.to_string(),
        db_path: root.join("adp.db").to_string_lossy().into(),
        files_dir: root.join("files").to_string_lossy().into(),
        max_upload_bytes: 1024 * 1024,
        relays: RELAYS.iter().map(|s| s.to_string()).collect(),
        operator_nsec: carol.secret_key().to_secret_hex(),
        master_key: master,
        allow_insecure_lnurl: true,
        download_token_ttl_secs: 900,
        file_reverify_interval_secs: 3600,
    };
    let relay_server =
        Arc::new(RelayClient::connect(carol.clone(), &cfg.relays, base.clone()).await?);
    let clock = Arc::new(AtomicU64::new(now() - 86400 + 750));
    let read_clock = clock.clone();
    let app = routes::router_with_settlement_clock(
        AppState {
            config: Arc::new(cfg),
            storage: store.clone(),
            keystore: Arc::new(KeyStore::new(store.pool(), master)),
            relay: relay_server,
            operator_keys: carol.clone(),
        },
        SettlementClock::new(move || Some(read_clock.load(Ordering::SeqCst))),
    );
    let (stop, stopped) = tokio::sync::oneshot::channel();
    let task = tokio::spawn(async move {
        let _ = axum::serve(listener, app)
            .with_graceful_shutdown(async {
                let _ = stopped.await;
            })
            .await;
    });
    let c = relay(&carol).await?;
    let announcement = adp_server::announcement::build_server_announcement_event(
        &carol,
        &format!("gate2-failure-{}", uuid::Uuid::new_v4()),
        "Gate2 isolated failure router",
        &base,
        "",
    )
    .map_err(|_| anyhow!("announcement"))?;
    publish(&c, &announcement).await?;
    c.shutdown().await;
    let m = manifest(root, &base, clock.load(Ordering::SeqCst)).await?;
    let adp = client(&base)?;
    let signer = common::signer(&buyer)?;
    let wallet = buyer_wallet(&buyer, root.join("wallet-audit.json")).await?;
    let db = Database::new(&root.join("client.db")).await?;
    let payments = DurableNwcPayments::new(db.pool().clone());
    let id = format!("gate2-failure-{}", uuid::Uuid::new_v4());
    let response = adp
        .create_settlement_quote(&signer, request(&m, &id))
        .await
        .map_err(common::safe_adp)?;
    let q = verify_quote_event(
        &response.quote,
        &signer
            .nip44_decrypt(&response.quote.pubkey, &response.quote.content)
            .await?,
    )?;
    validate_quote(&m, &q)?;
    let mut invoices = vec![];
    for (index, node) in ["carol", "dave", "bob"].into_iter().enumerate() {
        let invoice = adp
            .settlement_next_invoice(&signer, &id)
            .await
            .map_err(common::safe_adp)?;
        common::validate_invoice(&invoice.bolt11, &nodes[node], q.legs[index].amount_msat)?;
        ensure!(
            common::check_network()? == nodes,
            "regtest identities changed"
        );
        if index == 2 {
            common::lnd("bob", "cancelinvoice", &[&invoice.payment_hash])?;
            ensure!(
                payments
                    .pay_or_resume(&signer, &wallet, &invoice.bolt11)
                    .await
                    .is_err(),
                "canceled third invoice unexpectedly paid"
            );
        } else {
            let paid = payments
                .pay_or_resume(&signer, &wallet, &invoice.bolt11)
                .await?;
            let proof = adp
                .submit_settlement_proof(
                    &signer,
                    &id,
                    SettlementProofRequest {
                        attempt_id: invoice.attempt_id.clone(),
                        preimage: paid.preimage,
                    },
                )
                .await
                .map_err(common::safe_adp)?;
            ensure!(
                proof.receipt.is_none() && proof.download_token.is_none(),
                "partial purchase gained ownership"
            );
        }
        invoices.push(invoice);
    }
    clock.store(q.confirmation_expires_at + 1, Ordering::SeqCst);
    let status = adp
        .settlement_status(&signer, &id)
        .await
        .map_err(common::safe_adp)?;
    ensure!(
        status.state == QuoteState::Failed
            && status.settled_legs == 2
            && status.receipt_event_id.is_none(),
        "failed three-leg quote mismatch"
    );
    let recovery = adp
        .prepare_recovery(&signer, &id)
        .await
        .map_err(common::safe_adp)?;
    let event = recovery.event.context("recovery evidence absent")?;
    event.verify()?;
    ensure!(
        recovery.evidence.paid_leg_ids.len() == 2,
        "wrong refund evidence"
    );
    for (index, (key, node, amount)) in [(&carol, "carol", A_FEE), (&dave, "dave", B_FEE)]
        .into_iter()
        .enumerate()
    {
        let recipient = common::signer(key)?;
        let prepared = adp
            .prepare_refund(&recipient, &id, &q.legs[index].leg_id)
            .await
            .map_err(common::safe_adp)?;
        common::validate_invoice(&prepared.bolt11, &nodes["alice"], amount)?;
        let payer = Wallet {
            nwc: None,
            payer: node.into(),
            audit: root.join(format!("refund-{node}.json")),
        };
        let paid = payments
            .pay_or_resume(&recipient, &payer, &prepared.bolt11)
            .await?;
        for _ in 0..3 {
            ensure!(
                payments
                    .pay_or_resume(&recipient, &payer, &prepared.bolt11)
                    .await?
                    == paid,
                "refund retry changed proof"
            );
            ensure!(
                adp.confirm_refund(
                    &recipient,
                    &id,
                    &q.legs[index].leg_id,
                    &prepared.attempt_id,
                    &paid.preimage
                )
                .await
                .map_err(common::safe_adp)?["state"]
                    == "verified",
                "refund not verified"
            );
        }
        let audit: Value = read(&payer.audit)?;
        ensure!(
            audit["pay_calls"]
                .as_object()
                .is_some_and(|v| v.len() == 1 && v.values().all(|v| v == 1)),
            "duplicate recipient refund"
        );
    }
    for table in ["purchases", "download_tokens"] {
        let count: i64 = sqlx::query_scalar(&format!("SELECT COUNT(*) FROM {table}"))
            .fetch_one(&store.pool())
            .await?;
        ensure!(count == 0, "failed purchase gained authorization");
    }
    ensure!(
        adp.download(
            &m.coordinate,
            DownloadAuth::Nip98 { signer: &signer },
            &root.join("forbidden.download"),
            |_, _| {}
        )
        .await
        .is_err(),
        "failed purchase downloaded payload"
    );
    ensure!(
        common::lnd("bob", "lookupinvoice", &[&invoices[2].payment_hash])?["settled"] == false,
        "Bob received canceled third payment"
    );
    let verified: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM refund_obligations WHERE state='verified'")
            .fetch_one(&store.pool())
            .await?;
    ensure!(verified == 2, "recipient obligations not discharged");
    let after = common::balances()?;
    ensure!(
        after == before,
        "partial purchase refunds did not restore balances"
    );
    let result = json!({"status":"pass","quote_id":id,"third_payment":"Bob canceled 750-sat invoice, failed NWC attempt, intent retained and never redispatched","settled_legs":2,"refunds_verified":2,"purchase_receipts":0,"ownership":false,"download_rejected":true,"recovery_event_id":event.id.to_hex(),"balances_before_msat":before,"balances_after_msat":after,"invoice_hashes":invoices.iter().map(|i|i.payment_hash.clone()).collect::<Vec<_>>(),"wallet_audit":read::<Value>(&root.join("wallet-audit.json"))?,"deadline_time":"test-owned injected confirmation deadline; invoices/Lightning use wall time"});
    common::write_private(&root.join("result.json"), &result)?;
    let _ = stop.send(());
    task.await?;
    Ok(result)
}
fn verify_failure(root: &Path) -> Result<Value> {
    let mut result: Value = read(&root.join("result.json"))?;
    ensure!(
        result["status"] == "pass"
            && result["balances_before_msat"] == result["balances_after_msat"],
        "failure evidence incomplete"
    );
    let manifest: Manifest = read(&root.join("manifest.json"))?;
    let id = result["quote_id"].as_str().context("failed quote ID")?;
    let server = rusqlite::Connection::open_with_flags(
        root.join("adp.db"),
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
    )?;
    let client = rusqlite::Connection::open_with_flags(
        root.join("client.db"),
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
    )?;
    let (state, failure): (String, u64) = server.query_row(
        "SELECT state,failure_at FROM purchase_quotes WHERE quote_id=?",
        [id],
        |r| Ok((r.get(0)?, r.get(1)?)),
    )?;
    ensure!(state == "failed", "failed quote resurrected");
    for table in ["purchases", "download_tokens"] {
        ensure!(
            server.query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |r| r
                .get::<_, i64>(0))?
                == 0,
            "failed order gained ownership/token"
        );
    }
    let consumed:i64=server.query_row("SELECT COUNT(*) FROM invoice_attempts WHERE quote_id=? AND settled_at IS NOT NULL AND proof_encrypted IS NOT NULL",[id],|r|r.get(0))?;
    ensure!(consumed == 2, "wrong failed-order proof count");
    let hashes = result["invoice_hashes"]
        .as_array()
        .context("failure invoice hashes")?;
    let audit: Value = read(&root.join("wallet-audit.json"))?;
    for (index, node) in ["carol", "dave", "bob"].into_iter().enumerate() {
        let hash = hashes[index].as_str().context("hash")?;
        let invoice = common::lnd(node, "lookupinvoice", &[hash])?;
        ensure!(
            invoice["settled"] == (index < 2) && audit["pay_calls"][hash] == 1,
            "failed leg received funds or was redispatched"
        );
        if index == 2 {
            ensure!(invoice["state"] == "CANCELED", "third invoice not canceled");
            ensure!(client.query_row("SELECT COUNT(*) FROM nwc_payment_journal WHERE account_pubkey=? AND payment_hash=? AND encrypted_payment IS NULL",[manifest.buyer.clone(),hash.to_owned()],|r|r.get::<_,i64>(0))?==1,"original blocked third-leg intent lost");
        }
    }
    let mut query=server.prepare("SELECT a.payment_hash,l.recipient_pubkey,a.amount_msat,a.proof_encrypted,o.due_at FROM refund_attempts a JOIN payment_legs l ON l.quote_id=a.quote_id AND l.leg_id=a.leg_id JOIN refund_obligations o ON o.quote_id=a.quote_id AND o.leg_id=a.leg_id WHERE a.quote_id=? AND a.verified_at IS NOT NULL AND o.state='verified'")?;
    let refunds: Vec<(String, String, u64, Vec<u8>, u64)> = query
        .query_map([id], |r| {
            Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?))
        })?
        .collect::<std::result::Result<_, _>>()?;
    ensure!(
        refunds.len() == 2,
        "refund obligations not independently verified"
    );
    let cipher =
        adp_server::crypto::MasterCipher::new(&hex::decode(std::env::var("ADP_MASTER_KEY")?)?)?;
    for (hash, key, amount, encrypted, due) in refunds {
        let node = if key == manifest.operators.carol {
            "carol"
        } else {
            ensure!(key == manifest.operators.dave, "wrong refund payer");
            "dave"
        };
        ensure!(
            amount == if node == "carol" { A_FEE } else { B_FEE } && due == failure + 604800,
            "refund amount/deadline drift"
        );
        let outgoing = common::lnd(node, "listpayments", &["--include_incomplete"])?;
        let records: Vec<_> = outgoing["payments"]
            .as_array()
            .context("outgoing missing")?
            .iter()
            .filter(|v| v["payment_hash"].as_str() == Some(&hash))
            .collect();
        ensure!(
            records.len() == 1
                && records[0]["status"] == "SUCCEEDED"
                && common::num(&records[0]["value_msat"])? == amount,
            "duplicate/wrong refund payment"
        );
        let incoming = common::lnd("alice", "lookupinvoice", &[&hash])?;
        ensure!(
            incoming["state"] == "SETTLED" && common::num(&incoming["amt_paid_msat"])? == amount,
            "Alice refund principal mismatch"
        );
        let preimage = String::from_utf8(cipher.decrypt(&serde_json::from_slice::<
            adp_server::crypto::EncryptedSecret,
        >(&encrypted)?)?)?;
        arcadestr_core::durable_payments::validate_payment_proof(
            incoming["payment_request"]
                .as_str()
                .context("invoice missing")?,
            &PayInvoiceResult {
                preimage,
                fees_paid_msat: None,
            },
        )?;
        ensure!(client.query_row("SELECT COUNT(*) FROM nwc_payment_journal WHERE account_pubkey=? AND payment_hash=? AND encrypted_payment IS NOT NULL",[key,hash.clone()],|r|r.get::<_,i64>(0))?==1,"refund proof checkpoint missing");
        let audit: Value = read(&root.join(format!("refund-{node}.json")))?;
        ensure!(
            audit["pay_calls"][&hash] == 1,
            "refund dispatcher repeated payment"
        );
    }
    result["persistence_reverified"] = json!(true);
    Ok(result)
}

async fn complete(root: &Path, verification: bool) -> Result<()> {
    verify_success(&root.join("success")).await?;
    let negative: Value = if verification {
        verify_failure(&root.join("failure"))?
    } else {
        std::fs::create_dir_all(root.join("failure"))?;
        failure(&root.join("failure")).await?
    };
    ensure!(negative["status"] == "pass", "negative case incomplete");
    let purchase: Value = read(&root.join("success/purchase.json"))?;
    let operators: Operators =
        read(&common::env_path("ARCADESTR_REGTEST_OPERATOR_DIR")?.join("operators.json"))?;
    let c = relay(&actors().await?.0).await?;
    announced(
        &c,
        &[
            (CAROL, &operators.carol, &operators.carol_terms),
            (DAVE, &operators.dave, &operators.dave_terms),
        ],
    )
    .await?;
    c.shutdown().await;
    let report = json!({"gate":"settlement-verification-2","status":"pass","allocation_msat":{"bob":750000,"carol":A_FEE,"dave":B_FEE},"gross_msat":GROSS,"independent_operators":operators,"purchase":purchase,"third_leg_failure":negative,"purchase_successful_dispatches":3,"negative_successful_dispatches":2,"negative_failed_dispatches":1,"refund_dispatches":2,"verification_only":verification});
    common::write_private(&root.join("report.json"), &report)?;
    println!("{}", report);
    Ok(())
}
#[tokio::main]
async fn main() {
    let action = std::env::args().nth(1).unwrap_or_default();
    let result = async {
        common::check_network()?;
        match action.as_str() {
            "prepare" => prepare().await,
            "run" | "resume" => {
                let budget: u64 =
                    std::env::var("ARCADESTR_REGTEST_PAYMENT_BUDGET_MSAT")?.parse()?;
                ensure!(
                    (2_400_000..=50_000_000).contains(&budget),
                    "Gate2 shared budget invalid"
                );
                let root = common::env_path("ARCADESTR_REGTEST_GATE2_DIR")?;
                std::fs::create_dir_all(root.join("success"))?;
                success(&root.join("success"), action == "resume").await?;
                complete(&root, false).await
            }
            "verify" => complete(&common::env_path("ARCADESTR_REGTEST_GATE2_DIR")?, true).await,
            _ => Err(anyhow!("use scripts/regtest test --gate2")),
        }
    }
    .await;
    if let Err(error) = result {
        eprintln!("Gate 2 failed: {error:#}");
        std::process::exit(1);
    }
}
