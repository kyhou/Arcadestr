//! One genuine local ADP split purchase using production clients and repositories.
//! Invoked by scripts/regtest; no identities, channels or wallets are generated.
use std::{
    collections::BTreeMap,
    fs::OpenOptions,
    io::Write,
    path::{Path, PathBuf},
    process::Command,
    str::FromStr,
    sync::Arc,
    time::Duration,
};

use anyhow::{anyhow, ensure, Context, Result};
use arcadestr_core::{
    adp_client::{
        AdpClient, AdpClientError, CreateSettlementQuoteRequest, DownloadAuth,
        SettlementProofRequest, SettlementProofResponse,
    },
    adp_publish::{
        build_adp_listing_event_builder, build_fulfillment_authorization_event_builder,
        AdpListingInput, FulfillmentAuthorizationInput,
    },
    adp_settlement::{
        build_terms_event_builder, parse_terms_event, verify_quote_event, CommercialTerms,
        FeePolicy, PaymentRole, QuoteState, RefundDestination, SignedQuote, TermsPayload,
    },
    adp_storage::{DownloadToken, DownloadTokensRepository},
    auth::{Account, AccountManager},
    authorization::{
        resolve_authorization_at, AuthorizationTerms, CAPABILITY_ISSUE_RECEIPT,
        CAPABILITY_UPLOAD_BUILD,
    },
    http_client::{HttpClient, ReqwestHttpClient},
    lnurlp::resolve_lud16_with_policy,
    nwc_client::{load_nwc_connection, NwcClient},
    purchases::{
        parse_and_validate_receipt_with_authorization, parse_receipt_event, PurchasesRepository,
        ReceiptEvidence,
    },
    settlement_repository::SettlementRepository,
    signers::{ActiveSigner, DirectKeySigner, NostrSigner},
    storage::Database,
};
use lightning_invoice::{Bolt11Invoice, Currency};
use nostr::{Event, Filter, Keys, PublicKey, Tag, Timestamp};
use nostr_sdk::Client;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};

const BASE: &str = "http://localhost:9099";
const RELAYS: [&str; 2] = ["ws://127.0.0.1:7777", "ws://127.0.0.1:7778"];
const NODES: [&str; 4] = ["alice", "bob", "carol", "dave"];
const GROSS: u64 = 1_000_000;
const FEE: u64 = 100_000;
const DTAG: &str = "arcadestr-regtest-e2e-single-operator-v1";

/// Test adapter observes dispatches and can terminate this process after the real
/// wallet settles, before the production durability module receives its proof.
struct AuditedWallet {
    inner: NwcClient,
    audit: Option<PathBuf>,
    crash_after_settlement: bool,
}

impl AuditedWallet {
    fn record(
        &self,
        method: &str,
        hash: &str,
    ) -> Result<(), arcadestr_core::nwc_client::NwcClientError> {
        if let Some(path) = &self.audit {
            let mut value: Value = if path.exists() {
                serde_json::from_slice(
                    &std::fs::read(path)
                        .map_err(|_| arcadestr_core::nwc_client::NwcClientError::InvalidLookup)?,
                )
                .map_err(|_| arcadestr_core::nwc_client::NwcClientError::InvalidLookup)?
            } else {
                json!({"pay_calls":{},"lookup_calls":{}})
            };
            value[method][hash] = json!(value[method][hash].as_u64().unwrap_or(0) + 1);
            write_private(path, &value)
                .map_err(|_| arcadestr_core::nwc_client::NwcClientError::InvalidLookup)?;
        }
        Ok(())
    }
}

#[async_trait::async_trait]
impl arcadestr_core::durable_payments::PaymentWallet for AuditedWallet {
    fn identity(&self) -> Result<String, arcadestr_core::nwc_client::NwcClientError> {
        self.inner.payment_identity()
    }
    async fn pay(
        &self,
        invoice: &str,
    ) -> Result<
        arcadestr_core::nwc_client::PayInvoiceResult,
        arcadestr_core::nwc_client::NwcClientError,
    > {
        let parsed = Bolt11Invoice::from_str(invoice)
            .map_err(|_| arcadestr_core::nwc_client::NwcClientError::InvalidLookup)?;
        self.record("pay_calls", &parsed.payment_hash().to_string())?;
        let result = self.inner.pay_invoice(invoice).await?;
        if self.crash_after_settlement {
            std::process::exit(86);
        }
        Ok(result)
    }
    async fn lookup(
        &self,
        hash: &str,
        amount: u64,
    ) -> Result<
        Option<arcadestr_core::nwc_client::PayInvoiceResult>,
        arcadestr_core::nwc_client::NwcClientError,
    > {
        self.record("lookup_calls", hash)?;
        self.inner.lookup_payment(hash, amount).await
    }
}

#[derive(Serialize, Deserialize)]
struct Manifest {
    listing: Event,
    terms: Event,
    authorization: Event,
    buyer: String,
    developer: String,
    operator: String,
    coordinate: String,
    file_hash: String,
    payload: PathBuf,
}

pub(crate) fn env_path(key: &str) -> Result<PathBuf> {
    Ok(PathBuf::from(
        std::env::var(key).map_err(|_| anyhow!("required private path missing"))?,
    ))
}

fn now() -> u64 {
    Timestamp::now().as_secs()
}

pub(crate) fn write_private(path: &Path, value: &impl Serialize) -> Result<()> {
    use std::os::unix::fs::OpenOptionsExt;
    let tmp = path.with_extension("tmp");
    let mut f = OpenOptions::new()
        .create(true)
        .truncate(true)
        .write(true)
        .mode(0o600)
        .open(&tmp)?;
    f.write_all(&serde_json::to_vec_pretty(value)?)?;
    f.sync_all()?;
    std::fs::rename(tmp, path)?;
    Ok(())
}

fn command_json(args: &[&str]) -> Result<Value> {
    let output = Command::new("docker")
        .args(args)
        .output()
        .context("Docker unavailable")?;
    ensure!(
        output.status.success(),
        "local Docker/LND command failed (raw output suppressed)"
    );
    serde_json::from_slice(&output.stdout).context("invalid local LND JSON")
}

pub(crate) fn lnd(node: &str, method: &str, extra: &[&str]) -> Result<Value> {
    ensure!(NODES.contains(&node), "unknown LND participant");
    let container = format!("polar-n1-{node}");
    let mut args = vec![
        "exec",
        container.as_str(),
        "lncli",
        "--lnddir=/home/lnd/.lnd",
        "--network=regtest",
        method,
    ];
    args.extend(extra);
    command_json(&args)
}

pub(crate) fn num(value: &Value) -> Result<u64> {
    value
        .as_u64()
        .or_else(|| value.as_str().and_then(|s| s.parse().ok()))
        .ok_or_else(|| anyhow!("invalid numeric LND field"))
}

pub(crate) fn check_network() -> Result<BTreeMap<String, String>> {
    let bitcoin = command_json(&[
        "exec",
        "polar-n1-backend1",
        "bitcoin-cli",
        "-regtest",
        "-rpcuser=polaruser",
        "-rpcpassword=polarpass",
        "getblockchaininfo",
    ])?;
    ensure!(
        bitcoin["chain"] == "regtest" && bitcoin["initialblockdownload"] == false,
        "Bitcoin Core must be ready on regtest"
    );
    let mut keys = BTreeMap::new();
    for node in NODES {
        let info = lnd(node, "getinfo", &[])?;
        ensure!(
            info["chains"] == json!([{"chain":"bitcoin","network":"regtest"}])
                && info["synced_to_chain"] == true,
            "all participating nodes must be synced Bitcoin regtest"
        );
        keys.insert(
            node.into(),
            info["identity_pubkey"]
                .as_str()
                .context("LND identity missing")?
                .into(),
        );
    }
    Ok(keys)
}

pub(crate) fn balances() -> Result<BTreeMap<String, u64>> {
    let mut result = BTreeMap::new();
    for node in NODES {
        let channels = lnd(node, "listchannels", &[])?;
        let mut total = 0;
        for channel in channels["channels"]
            .as_array()
            .context("channels missing")?
        {
            // Milli-satoshi commitment values avoid rounding at the channel boundary.
            total += num(&channel["local_balance"])? * 1000;
        }
        result.insert(node.into(), total);
    }
    Ok(result)
}

pub(crate) async fn accounts() -> Result<(Keys, Keys)> {
    let directory = env_path("ARCADESTR_REGTEST_DATA_DIR")?;
    ensure!(
        directory.join(".master_key").is_file()
            && directory.join("accounts.db").is_file()
            && directory.join("arcadestr.db").is_file(),
        "existing Arcadestr storage required; refusing new identity/storage"
    );
    let manager = AccountManager::new(&directory)
        .await
        .map_err(|_| anyhow!("existing account store could not be opened"))?;
    let mut keys = Vec::new();
    for role in ["ALICE", "BOB"] {
        let public = std::env::var(format!("POLAR_{role}_NOSTR_PUBKEY"))?;
        let account = sqlx::query_as::<_, Account>("SELECT * FROM accounts WHERE pubkey=?")
            .bind(&public)
            .fetch_one(manager.db_pool())
            .await
            .context("established account not found")?;
        let secret = manager
            .get_nsec(&account)
            .map_err(|_| anyhow!("existing account decryption failed"))?;
        let key = Keys::parse(secret.as_str())
            .map_err(|_| anyhow!("existing signer could not be loaded"))?;
        ensure!(
            key.public_key().to_hex() == public,
            "stored signer identity mismatch"
        );
        keys.push(key);
    }
    Ok((keys.remove(0), keys.remove(0)))
}

pub(crate) fn signer(keys: &Keys) -> Result<ActiveSigner> {
    let secret = zeroize::Zeroizing::new(keys.secret_key().to_secret_hex());
    Ok(ActiveSigner::DirectKey(
        DirectKeySigner::from_key(secret.as_str())
            .map_err(|_| anyhow!("existing signer unavailable"))?,
    ))
}

pub(crate) fn safe_adp(error: AdpClientError) -> anyhow::Error {
    // Never print response bodies, signed Authorization headers, invoice proofs or token URLs.
    match error {
        AdpClientError::Http(arcadestr_core::http_client::HttpClientError::Status(code)) => {
            anyhow!("ADP HTTP {code}")
        }
        AdpClientError::Http(arcadestr_core::http_client::HttpClientError::StatusWithBody {
            status,
            ..
        }) => anyhow!("ADP HTTP {status} (body suppressed)"),
        AdpClientError::PurchaseProtocol(_) => {
            anyhow!("ADP purchase protocol rejected request (detail suppressed)")
        }
        _ => anyhow!("ADP operation failed (private detail suppressed)"),
    }
}

async fn relay_client(keys: &Keys) -> Result<Client> {
    let client = Client::new(keys.clone());
    for relay in RELAYS {
        client.add_relay(relay).await?;
    }
    client.connect().await;
    Ok(client)
}

async fn publish(client: &Client, event: &Event) -> Result<()> {
    event.verify().context("invalid event signature")?;
    for relay in RELAYS {
        let result = client
            .send_event_to([relay], event)
            .await
            .context("local relay publication failed")?;
        ensure!(
            !result.success.is_empty(),
            "local relay did not acknowledge event"
        );
    }
    Ok(())
}

async fn prepare() -> Result<()> {
    let dtag = if std::env::var("ARCADESTR_REGTEST_SCENARIO").as_deref() == Ok("crash-durability") {
        "arcadestr-regtest-e2e-crash-durability-v1"
    } else {
        DTAG
    };
    check_network()?;
    let (buyer, developer) = accounts().await?;
    let operator = Keys::parse(&std::env::var("ADP_OPERATOR_NSEC")?)
        .map_err(|_| anyhow!("existing ADP operator invalid"))?;
    ensure!(
        operator.public_key().to_hex() == std::env::var("POLAR_CAROL_NOSTR_PUBKEY")?,
        "Carol identity differs from existing ADP operator"
    );
    let http: Arc<dyn HttpClient> = Arc::new(ReqwestHttpClient::new(Duration::from_secs(30))?);
    let adp = AdpClient::new(BASE, http);
    ensure!(
        adp.well_known().await.map_err(safe_adp)?.pubkey == operator.public_key().to_hex(),
        "wrong local ADP operator"
    );
    let publisher = relay_client(&developer).await?;
    let terms = build_terms_event_builder(&TermsPayload::Offer {
        terms: CommercialTerms {
            version: 1,
            operator_pubkey: operator.public_key().to_hex(),
            payment_destination: "carol@127.0.0.1:18080".into(),
            fee: FeePolicy::Fixed { amount_msat: FEE },
            limitations: vec!["isolated local regtest E2E fixture".into()],
        },
    })?
    .sign_with_keys(&operator)?;
    publish(&publisher, &terms).await?;
    parse_terms_event(&terms)?;
    let coordinate = format!("30402:{}:{dtag}", developer.public_key().to_hex());
    let provision = adp
        .provision(&signer(&developer)?, Some(&coordinate))
        .await
        .map_err(safe_adp)?;
    let fulfillment = PublicKey::from_hex(&provision.fulfillment_pubkey)?;
    let authorization = build_fulfillment_authorization_event_builder(&AuthorizationTerms {
        authorization_id: format!("{dtag}-{}", uuid::Uuid::new_v4()),
        coordinate: coordinate.clone(),
        operator_pubkey: operator.public_key(),
        fulfillment_pubkey: fulfillment,
        capabilities: [
            CAPABILITY_ISSUE_RECEIPT.to_string(),
            CAPABILITY_UPLOAD_BUILD.to_string(),
        ]
        .into_iter()
        .collect(),
        valid_from: now(),
    })?
    .sign_with_keys(&developer)?;
    publish(&publisher, &authorization).await?;
    let payload = env_path("ARCADESTR_REGTEST_PAYLOAD")?;
    let file_hash = hex::encode(Sha256::digest(std::fs::read(&payload)?));
    let input = AdpListingInput {
        d_tag: dtag.into(),
        title: "Regtest E2E — single operator (test fixture)".into(),
        description: "Isolated inert regtest download fixture; not a production game.".into(),
        price_sats: GROSS / 1000,
        lud16: Some("bob@127.0.0.1:18080".into()),
        tags: vec!["regtest".into()],
        images: vec![],
        servers: vec![BASE.into()],
        file_hash: Some(file_hash.clone()),
        version: Some("regtest-e2e-v1".into()),
        fulfillment_authorizations: vec![FulfillmentAuthorizationInput {
            root_event_id: authorization.id.to_hex(),
            fulfillment_pubkey: fulfillment.to_hex(),
            relay_hint: Some(RELAYS[0].into()),
        }],
        acquisition: arcadestr_core::marketplace::AcquisitionPolicy::Gated,
        platforms: vec![],
        campaigns: vec![],
        nip94_event_id: None,
        preserved_tags: vec![],
    };
    let listing = build_adp_listing_event_builder(&input)?
        .tags([Tag::parse([
            "adp_terms",
            &operator.public_key().to_hex(),
            &terms.id.to_hex(),
        ])?])
        .sign_with_keys(&developer)?;
    publish(&publisher, &listing).await?;
    let upload = adp
        .upload(&signer(&developer)?, &listing, &payload)
        .await
        .map_err(safe_adp)?;
    ensure!(
        upload.file_hash == file_hash && upload.game_coordinate == coordinate,
        "uploaded fixture mismatch"
    );
    write_private(
        &env_path("ARCADESTR_REGTEST_MANIFEST")?,
        &Manifest {
            listing,
            terms: terms.clone(),
            authorization,
            buyer: buyer.public_key().to_hex(),
            developer: developer.public_key().to_hex(),
            operator: operator.public_key().to_hex(),
            coordinate: coordinate.clone(),
            file_hash,
            payload,
        },
    )?;
    publisher.shutdown().await;
    println!(
        "{}",
        json!({"prepared":true,"terms_event_id":terms.id.to_hex(),"coordinate":coordinate,
        "fee_msat":FEE,"gross_msat":GROSS})
    );
    Ok(())
}

pub(crate) fn validate_invoice(
    bolt11: &str,
    expected_node: &str,
    expected_msat: u64,
) -> Result<Bolt11Invoice> {
    let invoice = Bolt11Invoice::from_str(bolt11).map_err(|_| anyhow!("invalid BOLT11"))?;
    ensure!(
        invoice.currency() == Currency::Regtest,
        "non-regtest invoice rejected"
    );
    ensure!(
        invoice.recover_payee_pub_key().to_string() == expected_node,
        "non-local or wrong invoice recipient rejected"
    );
    ensure!(
        invoice.amount_milli_satoshis() == Some(expected_msat)
            && invoice
                .duration_since_epoch()
                .as_secs()
                .checked_add(invoice.expiry_time().as_secs())
                .is_some_and(|end| end > now()),
        "wrong amount or expired invoice"
    );
    ensure!(
        expected_msat > 0 && expected_msat <= 10_000_000,
        "invoice outside test purchase cap"
    );
    Ok(invoice)
}

async fn purchase(resume: bool) -> Result<()> {
    let manifest: Manifest =
        serde_json::from_slice(&std::fs::read(env_path("ARCADESTR_REGTEST_MANIFEST")?)?)?;
    let (buyer_keys, developer_keys) = accounts().await?;
    let buyer = signer(&buyer_keys)?;
    ensure!(
        manifest.buyer == buyer_keys.public_key().to_hex()
            && manifest.developer == developer_keys.public_key().to_hex(),
        "scenario signer mismatch"
    );
    let http: Arc<dyn HttpClient> = Arc::new(ReqwestHttpClient::new(Duration::from_secs(30))?);
    let adp = AdpClient::new(BASE, http.clone());
    let info = adp.well_known().await.map_err(safe_adp)?;
    let config = info
        .settlement
        .context("missing settlement configuration")?;
    ensure!(
        info.pubkey == manifest.operator
            && config.split_enabled
            && config.terms_event_id.as_deref() == Some(&manifest.terms.id.to_hex())
            && config.fee == FeePolicy::Fixed { amount_msat: FEE },
        "ADP settlement configuration does not match published terms"
    );
    let terms = parse_terms_event(&manifest.terms)?;
    ensure!(
        matches!(terms, TermsPayload::Offer {terms} if terms.operator_pubkey == manifest.operator
        && terms.payment_destination == "carol@127.0.0.1:18080" && terms.fee == FeePolicy::Fixed{amount_msat:FEE}),
        "wrong terms"
    );
    let budget: u64 = std::env::var("ARCADESTR_REGTEST_PAYMENT_BUDGET_MSAT")?.parse()?;
    ensure!(
        (GROSS + 20_000..=50_000_000).contains(&budget),
        "insufficient/invalid shared automated payment budget"
    );
    let hub = rusqlite::Connection::open_with_flags(
        env_path("ARCADESTR_REGTEST_HUB_DB")?,
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
        "existing Hub must use local Alice LND"
    );
    let wallet_pubkey:String = hub.query_row("SELECT wallet_pubkey FROM apps WHERE name IN ('Arcadestr Regtest Buyer','Alice Arcadestr Regtest Buyer')", [], |r|r.get(0))?;
    let connection = load_nwc_connection(&buyer_keys.public_key())
        .map_err(|_| anyhow!("existing buyer NWC unavailable"))?;
    ensure!(
        connection.wallet_pubkey_hex() == wallet_pubkey,
        "NWC is not the established local Hub app"
    );
    ensure!(
        connection
            .relay_urls()
            .iter()
            .all(|s| matches!(s.as_str(), "ws://127.0.0.1:7778" | "ws://127.0.0.1:7778/")),
        "existing NWC must use the local regtest Hub relay"
    );
    let wallet = NwcClient::new(connection).with_timeout(Duration::from_secs(30));
    wallet
        .verify()
        .await
        .map_err(|_| anyhow!("existing NWC verification failed"))?;
    let wallet = AuditedWallet {
        inner: wallet,
        audit: std::env::var("ARCADESTR_REGTEST_WALLET_AUDIT")
            .ok()
            .map(PathBuf::from),
        crash_after_settlement: !resume
            && std::env::var("ARCADESTR_REGTEST_CRASH_STAGE").as_deref()
                == Ok("after_wallet_settlement"),
    };
    let nodes = check_network()?;
    let report_path = env_path("ARCADESTR_REGTEST_REPORT")?;
    let checkpoint: Option<Value> = if resume {
        Some(serde_json::from_slice(&std::fs::read(&report_path)?)?)
    } else {
        None
    };
    let before = if let Some(saved) = &checkpoint {
        serde_json::from_value(saved["balances_before_msat"].clone())?
    } else {
        balances()?
    };
    let refund = resolve_lud16_with_policy(http.as_ref(), "alice@127.0.0.1:18080", true).await?;
    let db = Database::new(&env_path("ARCADESTR_REGTEST_DATA_DIR")?.join("arcadestr.db")).await?;
    let settlements = SettlementRepository::new(db.pool().clone());
    let purchase_id = if let Some(saved) = &checkpoint {
        saved["purchase_id"]
            .as_str()
            .context("resume order missing")?
            .to_string()
    } else {
        format!("regtest-e2e-{}", uuid::Uuid::new_v4())
    };
    let response = if resume {
        let stored = settlements
            .load(&manifest.buyer, &purchase_id)
            .await?
            .context("resume quote missing")?;
        arcadestr_core::adp_client::SettlementQuoteResponse {
            quote: serde_json::from_str(&stored.quote_event_json)?,
            state: QuoteState::Active,
        }
    } else {
        adp.create_settlement_quote(
            &buyer,
            CreateSettlementQuoteRequest {
                purchase_id: purchase_id.clone(),
                listing_event: manifest.listing.clone(),
                terms_events: vec![manifest.terms.clone()],
                refund_destination: RefundDestination {
                    endpoint: "alice@127.0.0.1:18080".into(),
                    min_sendable_msat: refund.min_sendable_msat,
                    max_sendable_msat: refund.max_sendable_msat,
                },
                zap_requests: Default::default(),
            },
        )
        .await
        .map_err(safe_adp)
        .context("quote creation")?
    };
    let plaintext = buyer
        .nip44_decrypt(&response.quote.pubkey, &response.quote.content)
        .await?;
    let quote = verify_quote_event(&response.quote, &plaintext)?;
    ensure!(
        quote.quote_id == purchase_id
            && quote.buyer_pubkey == manifest.buyer
            && quote.processor_pubkey == manifest.operator
            && response.quote.pubkey.to_hex() == manifest.operator
            && quote.listing_event_id == manifest.listing.id.to_hex()
            && quote.listing_coordinate == manifest.coordinate
            && quote.state == QuoteState::Active
            && quote.gross_msat == GROSS
            && quote.legs.len() == 2,
        "quote binding/shape mismatch"
    );
    for leg in &quote.legs {
        ensure!(
            match leg.role {
                PaymentRole::Developer =>
                    leg.recipient_pubkey == manifest.developer
                        && leg.amount_msat == GROSS - FEE
                        && leg.destination == "bob@127.0.0.1:18080"
                        && leg.accepted_terms_id.is_none(),
                PaymentRole::Operator =>
                    leg.recipient_pubkey == manifest.operator
                        && leg.amount_msat == FEE
                        && leg.destination == "carol@127.0.0.1:18080"
                        && leg.accepted_terms_id.as_deref() == Some(&manifest.terms.id.to_hex()),
            },
            "quote fee allocation or recipient mismatch"
        );
    }
    if !resume {
        settlements
            .save_quote(
                &manifest.buyer,
                BASE,
                &serde_json::to_string(&response.quote)?,
                &quote,
                now(),
            )
            .await?;
    }
    let mut evidence = checkpoint.unwrap_or_else(||json!({"scenario":"single_operator_adp_purchase","purchase_id":purchase_id,
        "quote_event_id":response.quote.id.to_hex(),"listing_event_id":manifest.listing.id.to_hex(),
        "terms_event_id":manifest.terms.id.to_hex(),"coordinate":manifest.coordinate,"gross_msat":GROSS,
        "developer_msat":GROSS-FEE,"operator_msat":FEE,"state":"quote_verified","payments":[],"balances_before_msat":before}));
    write_private(&report_path, &evidence)?;
    let mut final_response = None;
    let mut fees = 0;
    let mut principal = 0;
    for leg in &quote.legs {
        let recorded = evidence["payments"]
            .as_array()
            .context("payments array")?
            .iter()
            .find(|p| p["leg_id"] == leg.leg_id)
            .cloned();
        if let Some(saved) = &recorded {
            if saved["state"] == "settled_and_proof_accepted" {
                principal += leg.amount_msat;
                fees += num(&saved["fee_msat"])?;
                continue;
            }
        }
        let invoice = if let Some(saved) = &recorded {
            let attempt_id = saved["attempt_id"]
                .as_str()
                .context("original attempt id missing")?;
            let original = settlements
                .invoice_attempt(&manifest.buyer, &purchase_id, attempt_id)
                .await?
                .context("original invoice missing")?;
            let plaintext = buyer
                .nip44_decrypt(
                    &buyer_keys.public_key(),
                    std::str::from_utf8(&original.encrypted_attempt)?,
                )
                .await?;
            serde_json::from_str::<arcadestr_core::adp_client::SettlementInvoiceResponse>(
                &plaintext,
            )?
        } else {
            ensure!(
                !settlements
                    .has_unreconciled_payment(&manifest.buyer, &purchase_id)
                    .await?,
                "reconcile prior payment before requesting another invoice"
            );
            adp.settlement_next_invoice(&buyer, &purchase_id)
                .await
                .map_err(safe_adp)?
        };
        ensure!(
            invoice.leg_id == leg.leg_id
                && invoice.quote_id == purchase_id
                && invoice.amount_msat == leg.amount_msat,
            "invoice not bound to expected quote leg"
        );
        let recipient = if leg.role == PaymentRole::Developer {
            "bob"
        } else {
            "carol"
        };
        ensure!(
            check_network()? == nodes,
            "participating nodes changed before payment"
        );
        let bolt11 = validate_invoice(&invoice.bolt11, &nodes[recipient], leg.amount_msat)?;
        ensure!(
            bolt11.payment_hash().to_string() == invoice.payment_hash,
            "invoice hash mismatch"
        );
        ensure!(
            principal + leg.amount_msat <= GROSS
                && principal + leg.amount_msat + fees + 10_000 <= budget,
            "shared payment budget exceeded"
        );
        let encrypted_invoice = buyer
            .nip44_encrypt(&buyer_keys.public_key(), &serde_json::to_string(&invoice)?)
            .await?;
        settlements
            .record_invoice_attempt(
                &manifest.buyer,
                &purchase_id,
                &invoice.leg_id,
                &invoice.attempt_id,
                encrypted_invoice.as_bytes(),
                &invoice.payment_hash,
                "disclosed",
                now(),
            )
            .await?;
        evidence["state"] = json!("payment_submitting");
        if recorded.is_none() {
            evidence["payments"].as_array_mut().context("payments array")?.push(json!({"leg_id":leg.leg_id,
            "recipient_node":recipient,"node_pubkey":nodes[recipient],"amount_msat":leg.amount_msat,"payment_hash":invoice.payment_hash,
            "attempt_id":invoice.attempt_id,"state":"reserved"}));
        }
        write_private(&report_path, &evidence)?; // Reserve/checkpoint before the irreversible NWC request.
        let paid = arcadestr_core::durable_payments::DurableNwcPayments::new(db.pool().clone())
            .pay_or_resume(&buyer, &wallet, &invoice.bolt11)
            .await
            .map_err(|_| anyhow!("NWC payment failed or ambiguous; never automatically retried"))?;
        if !resume
            && std::env::var("ARCADESTR_REGTEST_CRASH_STAGE").as_deref() == Ok("after_checkpoint")
        {
            std::process::exit(86);
        }
        principal += leg.amount_msat;
        ensure!(
            hex::encode(Sha256::digest(
                hex::decode(&paid.preimage).map_err(|_| anyhow!("invalid NWC proof"))?
            )) == invoice.payment_hash,
            "NWC preimage does not prove invoice payment"
        );
        let settled = lnd(recipient, "lookupinvoice", &[&invoice.payment_hash])?;
        ensure!(
            settled["settled"] == true && num(&settled["amt_paid_msat"])? == leg.amount_msat,
            "wrong receiving node or settled principal"
        );
        let payments = lnd("alice", "listpayments", &["--include_incomplete"])?;
        let payer_payment = payments["payments"]
            .as_array()
            .context("Alice payments missing")?
            .iter()
            .find(|p| p["payment_hash"].as_str() == Some(&invoice.payment_hash))
            .context("NWC payment not found in Alice")?;
        ensure!(
            payer_payment["status"] == "SUCCEEDED"
                && num(&payer_payment["value_msat"])? == leg.amount_msat,
            "Alice payment state/amount mismatch"
        );
        let fee = num(&payer_payment["fee_msat"])?;
        ensure!(
            fee <= 10_000 && paid.fees_paid_msat.is_none_or(|v| v == fee),
            "unexpected NWC fee"
        );
        fees += fee;
        let encrypted_proof = buyer
            .nip44_encrypt(&buyer_keys.public_key(), &paid.preimage)
            .await?;
        ensure!(
            settlements
                .checkpoint_proof(
                    &manifest.buyer,
                    &purchase_id,
                    &invoice.attempt_id,
                    encrypted_proof.as_bytes()
                )
                .await?,
            "proof checkpoint missing invoice"
        );
        // Private, account-encrypted test evidence only; no recovery/refund implementation.
        write_private(
            &report_path.with_file_name(format!("{}.proof.json", invoice.attempt_id)),
            &json!({"encrypted_preimage":encrypted_proof}),
        )?;
        let proof_response = adp
            .submit_settlement_proof(
                &buyer,
                &purchase_id,
                SettlementProofRequest {
                    attempt_id: invoice.attempt_id.clone(),
                    preimage: paid.preimage,
                },
            )
            .await
            .map_err(safe_adp)
            .context("payment proof submission")?;
        settlements
            .update_invoice_state(&manifest.buyer, &invoice.attempt_id, "settled", now())
            .await?;
        let last = evidence["payments"]
            .as_array_mut()
            .context("payments array")?
            .last_mut()
            .context("payment record")?;
        last["state"] = json!("settled_and_proof_accepted");
        last["fee_msat"] = json!(fee);
        write_private(&report_path, &evidence)?;
        final_response = Some(proof_response);
    }
    if final_response.is_none() && resume {
        return verify_paid().await;
    }
    let response = final_response.context("no final response")?;
    evidence["state"] = json!("proofs_accepted");
    write_private(&report_path, &evidence)?;
    finish_purchase(
        &manifest,
        &buyer_keys,
        &adp,
        &db,
        &settlements,
        &quote,
        &purchase_id,
        &mut evidence,
        response,
        &report_path,
    )
    .await
}

#[allow(clippy::too_many_arguments)]
async fn finish_purchase(
    manifest: &Manifest,
    buyer_keys: &Keys,
    adp: &AdpClient,
    db: &Database,
    settlements: &SettlementRepository,
    quote: &SignedQuote,
    purchase_id: &str,
    evidence: &mut Value,
    response: SettlementProofResponse,
    report_path: &Path,
) -> Result<()> {
    let before: BTreeMap<String, u64> =
        serde_json::from_value(evidence["balances_before_msat"].clone())?;
    let fees = evidence["payments"]
        .as_array()
        .context("payments evidence missing")?
        .iter()
        .map(|p| num(&p["fee_msat"]))
        .collect::<Result<Vec<_>>>()?
        .iter()
        .sum::<u64>();
    ensure!(response.state == "settled", "purchase did not settle");
    let receipt = response.receipt.context("aggregate receipt missing")?;
    receipt.verify()?;
    ensure!(receipt.kind.as_u16() == 1020, "wrong receipt kind");
    let parsed = parse_receipt_event(&receipt)?;
    let aggregate = parsed
        .aggregate
        .as_ref()
        .context("not an aggregate receipt")?;
    ensure!(
        parsed.order_id == purchase_id
            && aggregate.quote_event_id == response_quote_id(&evidence)?
            && aggregate.listing_event_id == manifest.listing.id
            && aggregate.processor_pubkey.to_hex() == manifest.operator
            && aggregate.gross_msat == GROSS
            && aggregate.legs.len() == quote.legs.len(),
        "aggregate receipt quote/listing binding mismatch"
    );
    for leg in &quote.legs {
        ensure!(
            aggregate.legs.iter().any(|proof| proof.leg_id == leg.leg_id
                && proof.amount_msat == leg.amount_msat
                && proof.role == leg.role
                && proof.proof_type == "bolt11-preimage"
                && proof.proof_reference == proof.payment_hash
                && proof.recipient_pubkey == leg.recipient_pubkey
                && proof.accepted_terms_id == leg.accepted_terms_id
                && evidence["payments"]
                    .as_array()
                    .is_some_and(|payments| payments
                        .iter()
                        .any(|p| p["payment_hash"] == proof.payment_hash))),
            "receipt proof leg mismatch"
        );
    }
    let authorization = resolve_authorization_at(
        manifest.authorization.id,
        &[manifest.authorization.clone()],
        parsed.settled_at.context("settled_at missing")?,
    )?;
    let stored = parse_and_validate_receipt_with_authorization(
        &receipt,
        &manifest.buyer,
        Some(&authorization),
        ReceiptEvidence::default(),
    )?;
    let purchases = PurchasesRepository::new(db.pool().clone());
    purchases.upsert_receipt(&stored).await?;
    settlements
        .mark_receipt(
            &manifest.buyer,
            &purchase_id,
            &serde_json::to_string(&receipt)?,
            now(),
        )
        .await?;
    ensure!(
        purchases
            .is_owned(&manifest.buyer, &manifest.coordinate)
            .await?,
        "ownership not persisted"
    );
    let client_state = settlements
        .load(&manifest.buyer, purchase_id)
        .await?
        .context("client settlement missing")?;
    ensure!(
        client_state.state == "settled"
            && client_state
                .receipt_event_json
                .as_deref()
                .and_then(|s| serde_json::from_str::<Event>(s).ok())
                .is_some_and(|e| e.id == receipt.id),
        "client settlement/receipt state mismatch"
    );
    let token = response.download_token.context("download token missing")?;
    let expires_at = response.token_expires_at.context("token expiry missing")?;
    ensure!(
        expires_at > now() as i64 && !token.is_empty(),
        "invalid download token"
    );
    DownloadTokensRepository::new(db.pool().clone())
        .upsert(&DownloadToken {
            buyer_pubkey: manifest.buyer.clone(),
            game_coordinate: manifest.coordinate.clone(),
            server_url: BASE.into(),
            token: token.clone(),
            expires_at,
        })
        .await?;
    let destination = report_path.with_file_name(format!("{purchase_id}.download"));
    let download = adp
        .download(
            &manifest.coordinate,
            DownloadAuth::Token(token.clone()),
            &destination,
            |_, _| {},
        )
        .await
        .map_err(safe_adp)?;
    ensure!(
        download.bytes_written > 0
            && hex::encode(Sha256::digest(std::fs::read(&destination)?)) == manifest.file_hash,
        "download content/hash mismatch"
    );
    let client = relay_client(buyer_keys).await?;
    let published = client
        .fetch_events(Filter::new().id(receipt.id), Duration::from_secs(10))
        .await?;
    ensure!(
        published.iter().any(|e| e == &receipt),
        "aggregate receipt not retrievable from local relays"
    );
    client.shutdown().await;
    // A later read-only recheck may follow other legitimate regtest purchases.
    // Keep the original transaction's before/after reconciliation, rather than
    // attributing the entire wallet's subsequent activity to this old order.
    let after = if evidence["state"] == "complete" {
        evidence["balances_current_msat"] = json!(balances()?);
        serde_json::from_value(evidence["balances_after_msat"].clone())?
    } else {
        balances()?
    };
    for payment in evidence["payments"]
        .as_array()
        .context("payment evidence")?
    {
        let node = payment["recipient_node"]
            .as_str()
            .context("recipient node missing")?;
        let hash = payment["payment_hash"]
            .as_str()
            .context("payment hash missing")?;
        let settled = lnd(node, "lookupinvoice", &[hash])?;
        ensure!(
            settled["settled"] == true
                && num(&settled["amt_paid_msat"])? == num(&payment["amount_msat"])?,
            "historical receiving invoice/amount mismatch"
        );
    }
    ensure!(
        after["bob"] == before["bob"] + GROSS - FEE
            && after["carol"] == before["carol"] + FEE
            && after["dave"] == before["dave"]
            && before["alice"] == after["alice"] + GROSS + fees,
        "recipient/payer channel balance deltas mismatch"
    );
    let server_db = rusqlite::Connection::open_with_flags(
        env_path("ARCADESTR_REGTEST_ADP_DB")?,
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
    )?;
    let server_state: String = server_db.query_row(
        "SELECT state FROM purchase_quotes WHERE quote_id=?",
        [&purchase_id],
        |r| r.get(0),
    )?;
    let (legs, settled): (u64, u64) = server_db.query_row(
        "SELECT COUNT(*), COUNT(settled_at) FROM payment_legs WHERE quote_id=?",
        [&purchase_id],
        |r| Ok((r.get(0)?, r.get(1)?)),
    )?;
    let ownership: u64 = server_db.query_row(
        "SELECT COUNT(*) FROM purchases WHERE order_id=? AND buyer_pubkey=? AND receipt_event_id=?",
        rusqlite::params![purchase_id, manifest.buyer, receipt.id.to_hex()],
        |r| r.get(0),
    )?;
    ensure!(
        server_state == "settled" && legs == 2 && settled == 2 && ownership == 1,
        "ADP database state mismatch"
    );
    let token_binding:u64 = server_db.query_row("SELECT COUNT(*) FROM download_tokens WHERE token=? AND buyer_pubkey=? AND game_coordinate=? AND expires_at>?",
        rusqlite::params![token,manifest.buyer,manifest.coordinate,now() as i64],|r|r.get(0))?;
    let proofs: u64 = server_db.query_row(
        "SELECT COUNT(*) FROM payment_proofs WHERE order_id=?",
        [purchase_id],
        |r| r.get(0),
    )?;
    ensure!(
        token_binding == 1 && proofs == 2,
        "server token/proof bindings mismatch"
    );
    evidence["state"] = json!("complete");
    evidence["receipt_event_id"] = json!(receipt.id.to_hex());
    evidence["receipt_kind"] = json!(1020);
    evidence["ownership_persisted"] = json!(true);
    evidence["download_token_valid"] = json!(true);
    evidence["token_expires_at"] = json!(expires_at);
    evidence["download_sha256"] = json!(manifest.file_hash);
    evidence["download_bytes"] = json!(download.bytes_written);
    evidence["balances_after_msat"] = json!(after);
    evidence["routing_fees_msat"] = json!(fees);
    evidence["server_state"] = json!(server_state);
    evidence["settled_legs"] = json!(settled);
    evidence["client_settlement_state"] = json!(client_state.state);
    evidence["receipt_signature_verified"] = json!(true);
    evidence["receipt_retrieved"] = json!(true);
    evidence["server_payment_proofs"] = json!(proofs);
    evidence["server_token_buyer_bound"] = json!(true);
    if let Ok(path) = std::env::var("ARCADESTR_REGTEST_WALLET_AUDIT") {
        let audit: Value = serde_json::from_slice(&std::fs::read(path)?)?;
        for payment in evidence["payments"]
            .as_array()
            .context("payments evidence")?
        {
            let hash = payment["payment_hash"].as_str().context("payment hash")?;
            ensure!(
                audit["pay_calls"][hash] == 1,
                "duplicate NWC pay dispatch detected"
            );
        }
        evidence["wallet_dispatch_audit"] = audit;
        evidence["crash_stage"] =
            json!(std::env::var("ARCADESTR_REGTEST_CRASH_STAGE").unwrap_or_default());
    }
    write_private(report_path, &evidence)?;
    println!("{}", evidence);
    Ok(())
}

// Re-run assertions for an already settled paid order after fixing a test blocker.
// This path cannot request invoices or invoke NWC payments; it is not a recovery/refund scenario.
async fn verify_paid() -> Result<()> {
    check_network()?;
    let manifest: Manifest =
        serde_json::from_slice(&std::fs::read(env_path("ARCADESTR_REGTEST_MANIFEST")?)?)?;
    let report_path = env_path("ARCADESTR_REGTEST_REPORT")?;
    let mut evidence: Value = serde_json::from_slice(&std::fs::read(&report_path)?)?;
    let purchase_id = evidence["purchase_id"]
        .as_str()
        .context("paid purchase id missing")?
        .to_string();
    let payments = evidence["payments"]
        .as_array()
        .context("paid evidence missing")?;
    ensure!(
        payments.len() == 2
            && payments
                .iter()
                .all(|p| p["state"] == "settled_and_proof_accepted"),
        "not a completed paid order; no automatic payment retry"
    );
    let (keys, _) = accounts().await?;
    ensure!(
        keys.public_key().to_hex() == manifest.buyer,
        "buyer identity mismatch"
    );
    let buyer = signer(&keys)?;
    let http: Arc<dyn HttpClient> = Arc::new(ReqwestHttpClient::new(Duration::from_secs(30))?);
    let adp = AdpClient::new(BASE, http);
    let status = adp
        .settlement_status(&buyer, &purchase_id)
        .await
        .map_err(safe_adp)?;
    ensure!(
        status.state == QuoteState::Settled && status.settled_legs == 2,
        "server order is not already settled"
    );
    let db = Database::new(&env_path("ARCADESTR_REGTEST_DATA_DIR")?.join("arcadestr.db")).await?;
    let settlements = SettlementRepository::new(db.pool().clone());
    let stored = settlements
        .load(&manifest.buyer, &purchase_id)
        .await?
        .context("stored quote missing")?;
    let event: Event = serde_json::from_str(&stored.quote_event_json)?;
    let plaintext = buyer.nip44_decrypt(&event.pubkey, &event.content).await?;
    let quote = verify_quote_event(&event, &plaintext)?;
    ensure!(
        quote.quote_id == purchase_id && quote.listing_event_id == manifest.listing.id.to_hex(),
        "paid order/manifest mismatch"
    );
    let payment_hash = payments.last().context("last payment missing")?["payment_hash"]
        .as_str()
        .context("hash missing")?;
    let attempt_id: String = sqlx::query_scalar("SELECT attempt_id FROM settlement_invoice_attempts WHERE account_pubkey=? AND quote_id=? AND payment_hash=? AND state='settled'")
        .bind(&manifest.buyer).bind(&purchase_id).bind(payment_hash).fetch_one(db.pool()).await?;
    let attempt = settlements
        .invoice_attempt(&manifest.buyer, &purchase_id, &attempt_id)
        .await?
        .context("stored attempt missing")?;
    let ciphertext = if let Some(proof) = attempt.encrypted_proof {
        String::from_utf8(proof)?
    } else {
        // Compatibility for the original successful baseline, recorded before
        // the additive durability migration. New scenarios use production storage.
        let encrypted: Value = serde_json::from_slice(&std::fs::read(
            report_path.with_file_name(format!("{attempt_id}.proof.json")),
        )?)?;
        encrypted["encrypted_preimage"]
            .as_str()
            .context("legacy baseline proof missing")?
            .to_string()
    };
    let preimage = buyer.nip44_decrypt(&keys.public_key(), &ciphertext).await?;
    ensure!(
        hex::encode(Sha256::digest(hex::decode(&preimage)?)) == payment_hash,
        "stored paid proof mismatch"
    );
    let response = adp
        .submit_settlement_proof(
            &buyer,
            &purchase_id,
            SettlementProofRequest {
                attempt_id,
                preimage,
            },
        )
        .await
        .map_err(safe_adp)?;
    finish_purchase(
        &manifest,
        &keys,
        &adp,
        &db,
        &settlements,
        &quote,
        &purchase_id,
        &mut evidence,
        response,
        &report_path,
    )
    .await
}

fn response_quote_id(evidence: &Value) -> Result<nostr::EventId> {
    Ok(nostr::EventId::from_hex(
        evidence["quote_event_id"]
            .as_str()
            .context("quote id missing")?,
    )?)
}

#[tokio::main]
async fn main() {
    let action = std::env::args().nth(1).unwrap_or_default();
    let result = match action.as_str() {
        "prepare" => prepare().await,
        "purchase" => purchase(false).await,
        "resume-purchase" => purchase(true).await,
        "verify-paid" => verify_paid().await,
        _ => Err(anyhow!(
            "use prepare, purchase or verify-paid via scripts/regtest"
        )),
    };
    if let Err(error) = result {
        // All application/network errors are sanitized at their boundaries above.
        eprintln!("regtest split {action} failed: {error:#}");
        std::process::exit(1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bitcoin::{
        hashes::{sha256, Hash},
        secp256k1::{Secp256k1, SecretKey},
    };
    use lightning_invoice::InvoiceBuilder;
    use lightning_types::payment::PaymentSecret;

    fn invoice(currency: Currency, timestamp: u64, amount: u64) -> (String, String) {
        let secret = SecretKey::from_slice(&[42; 32]).expect("synthetic secret");
        let node =
            bitcoin::secp256k1::PublicKey::from_secret_key(&Secp256k1::new(), &secret).to_string();
        let value = InvoiceBuilder::new(currency)
            .description("synthetic regtest guard".into())
            .payment_hash(sha256::Hash::hash(&[7; 32]))
            .payment_secret(PaymentSecret([11; 32]))
            .amount_milli_satoshis(amount)
            .duration_since_epoch(Duration::from_secs(timestamp))
            .expiry_time(Duration::from_secs(600))
            .min_final_cltv_expiry_delta(144)
            .build_signed(|hash| Secp256k1::new().sign_ecdsa_recoverable(hash, &secret))
            .expect("invoice");
        (value.to_string(), node)
    }

    #[test]
    fn payment_guard_accepts_only_exact_local_unexpired_regtest_invoice() {
        let (valid, node) = invoice(Currency::Regtest, now(), 1000);
        assert!(validate_invoice(&valid, &node, 1000).is_ok());
        assert!(validate_invoice(&valid, "wrong-local-node", 1000).is_err());
        assert!(validate_invoice(&valid, &node, 1001).is_err());
        assert!(validate_invoice("malformed", &node, 1000).is_err());
        for currency in [Currency::Bitcoin, Currency::BitcoinTestnet] {
            let (other, _) = invoice(currency, now(), 1000);
            assert!(validate_invoice(&other, &node, 1000).is_err());
        }
        let (expired, _) = invoice(Currency::Regtest, 1, 1000);
        assert!(validate_invoice(&expired, &node, 1000).is_err());
        let (oversized, _) = invoice(Currency::Regtest, now(), 10000001);
        assert!(validate_invoice(&oversized, &node, 10000001).is_err());
    }
}
