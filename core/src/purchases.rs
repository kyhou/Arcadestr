//! NIP-102 purchase receipt persistence and verification.

use lightning_invoice::Bolt11Invoice;
use nostr_sdk::{Event, EventId, PublicKey};
use sha2::{Digest, Sha256};
use sqlx::{Pool, Row, Sqlite};
use std::collections::{HashMap, HashSet};
use std::str::FromStr;
use thiserror::Error;

use crate::adp_protocol::coordinate_publisher;
use crate::authorization::{ResolvedAuthorization, CAPABILITY_ISSUE_RECEIPT};

const KIND_PURCHASE_RECEIPT: u16 = 1020;
const STATUS_PAID: &str = "paid";
const STATUS_FULFILLED: &str = "fulfilled";

/// Errors returned while parsing or storing purchase receipts.
#[derive(Debug, Error)]
pub enum PurchaseError {
    #[error("expected NIP-102 kind 1020 receipt, got kind {0}")]
    WrongKind(u16),
    #[error("missing required {0}")]
    MissingTag(&'static str),
    #[error("receipt buyer p tag does not match authenticated buyer")]
    BuyerMismatch,
    #[error("invalid payment proof: {0}")]
    ProofInvalid(String),
    #[error("missing payment proof: expected bolt11 + preimage or zap receipt e tag")]
    MissingPaymentProof,
    #[error("failed to serialize raw event: {0}")]
    EventSerialization(#[from] serde_json::Error),
    #[error("database error: {0}")]
    Database(#[from] sqlx::Error),
}

/// A verified NIP-102 receipt ready for persistence.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoredReceipt {
    pub event_id: String,
    pub order_id: String,
    pub listing_coordinate: String,
    pub buyer_pubkey: String,
    pub merchant_pubkey: String,
    pub payment_hash: Option<String>,
    pub status: String,
    pub created_at: u64,
    pub raw_event: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum ReceiptProof {
    Zap(EventId),
    Bolt11Preimage,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParsedReceipt {
    pub event: Event,
    pub order_id: String,
    pub buyer_pubkey: PublicKey,
    pub coordinate: String,
    pub authorization: Option<EventId>,
    pub payment_hash: Option<String>,
    pub amount_msat: Option<u64>,
    pub settled_at: Option<u64>,
    pub proofs: Vec<ReceiptProof>,
    pub status: String,
    pub predecessor: Option<EventId>,
}

#[derive(Debug, Clone, Copy, Default)]
pub struct ReceiptEvidence<'a> {
    pub bolt11: Option<&'a str>,
    pub preimage: Option<&'a str>,
    pub zap_receipts: &'a [Event],
    pub lsp_pubkey: Option<PublicKey>,
}

/// SQLite repository for purchase receipts.
pub struct PurchasesRepository {
    db: Pool<Sqlite>,
}

impl PurchasesRepository {
    /// Create a purchase repository from a SQLite pool.
    pub fn new(db: Pool<Sqlite>) -> Self {
        Self { db }
    }

    /// Insert or update a verified receipt by event id.
    ///
    /// # Errors
    /// Returns `PurchaseError::Database` when SQLite rejects the write.
    pub async fn upsert_receipt(&self, receipt: &StoredReceipt) -> Result<(), PurchaseError> {
        sqlx::query(
            r#"
            INSERT INTO purchases (
                event_id, order_id, listing_coordinate, buyer_pubkey,
                merchant_pubkey, payment_hash, status, created_at, raw_event
            )
            VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)
            ON CONFLICT(event_id) DO UPDATE SET
                order_id = excluded.order_id,
                listing_coordinate = excluded.listing_coordinate,
                buyer_pubkey = excluded.buyer_pubkey,
                merchant_pubkey = excluded.merchant_pubkey,
                payment_hash = excluded.payment_hash,
                status = excluded.status,
                created_at = excluded.created_at,
                raw_event = excluded.raw_event
            "#,
        )
        .bind(&receipt.event_id)
        .bind(&receipt.order_id)
        .bind(&receipt.listing_coordinate)
        .bind(&receipt.buyer_pubkey)
        .bind(&receipt.merchant_pubkey)
        .bind(&receipt.payment_hash)
        .bind(&receipt.status)
        .bind(receipt.created_at as i64)
        .bind(&receipt.raw_event)
        .execute(&self.db)
        .await?;

        Ok(())
    }

    /// Return whether the latest receipt grants ownership.
    ///
    /// # Errors
    /// Returns `PurchaseError::Database` when SQLite rejects the query.
    pub async fn is_owned(
        &self,
        buyer_pubkey: &str,
        listing_coordinate: &str,
    ) -> Result<bool, PurchaseError> {
        let rows = sqlx::query(
            r#"
            SELECT order_id, raw_event
            FROM purchases
            WHERE buyer_pubkey = ? AND listing_coordinate = ?
            ORDER BY created_at DESC, event_id DESC
            "#,
        )
        .bind(buyer_pubkey)
        .bind(listing_coordinate)
        .fetch_all(&self.db)
        .await?;

        let mut orders: HashMap<String, Vec<Event>> = HashMap::new();
        for row in rows {
            let order_id: String = row.get("order_id");
            let raw_event: String = row.get("raw_event");
            if let Ok(event) = serde_json::from_str(&raw_event) {
                orders.entry(order_id).or_default().push(event);
            }
        }
        Ok(orders.into_values().any(|events| {
            resolve_persisted_receipt_tip(&events)
                .and_then(|event| parse_receipt_event(event).ok())
                .is_some_and(|receipt| status_grants_ownership(&receipt.status))
        }))
    }

    /// Load persisted receipts for one authenticated buyer.
    pub async fn list_for_buyer(
        &self,
        buyer_pubkey: &str,
    ) -> Result<Vec<StoredReceipt>, PurchaseError> {
        let rows = sqlx::query(
            r#"
            SELECT event_id, order_id, listing_coordinate, buyer_pubkey,
                   merchant_pubkey, payment_hash, status, created_at, raw_event
            FROM purchases
            WHERE buyer_pubkey = ?
            ORDER BY created_at DESC, event_id DESC
            "#,
        )
        .bind(buyer_pubkey)
        .fetch_all(&self.db)
        .await?;

        Ok(rows
            .into_iter()
            .map(|row| StoredReceipt {
                event_id: row.get("event_id"),
                order_id: row.get("order_id"),
                listing_coordinate: row.get("listing_coordinate"),
                buyer_pubkey: row.get("buyer_pubkey"),
                merchant_pubkey: row.get("merchant_pubkey"),
                payment_hash: row.get("payment_hash"),
                status: row.get("status"),
                created_at: row.get::<i64, _>("created_at").max(0) as u64,
                raw_event: row.get("raw_event"),
            })
            .collect())
    }

    pub(crate) fn database(&self) -> &Pool<Sqlite> {
        &self.db
    }
}

pub(crate) fn stored_receipt_validation_error(receipt: &StoredReceipt) -> Option<&'static str> {
    if receipt.event_id.trim().is_empty()
        || receipt.order_id.trim().is_empty()
        || receipt.listing_coordinate.trim().is_empty()
        || receipt.buyer_pubkey.trim().is_empty()
        || receipt.merchant_pubkey.trim().is_empty()
    {
        return Some("Stored purchase receipt is incomplete.");
    }

    let Ok(event) = serde_json::from_str::<Event>(&receipt.raw_event) else {
        return Some("Stored purchase receipt could not be verified.");
    };
    if event.verify().is_err()
        || event.kind.as_u16() != KIND_PURCHASE_RECEIPT
        || event.id.to_string() != receipt.event_id
        || event.pubkey.to_hex() != receipt.merchant_pubkey
        || tag_value(&event, "p").as_deref() != Some(receipt.buyer_pubkey.as_str())
        || tag_value(&event, "a").as_deref() != Some(receipt.listing_coordinate.as_str())
        || tag_value(&event, "order").as_deref() != Some(receipt.order_id.as_str())
        || tag_value(&event, "status").unwrap_or_else(|| STATUS_PAID.to_string()) != receipt.status
        || validate_payment_proof(&event).is_err()
    {
        return Some("Stored purchase receipt could not be verified.");
    }

    if tag_value(&event, "e").is_some() && tag_value(&event, "bolt11").is_none() {
        return Some("Referenced payment proof is not available for local verification.");
    }

    None
}

pub(crate) fn stored_receipt_amount(receipt: &StoredReceipt) -> Option<(u64, &'static str)> {
    let event = serde_json::from_str::<Event>(&receipt.raw_event).ok()?;
    let invoice = Bolt11Invoice::from_str(&tag_value(&event, "bolt11")?).ok()?;
    let millisats = invoice.amount_milli_satoshis()?;
    if millisats % 1_000 == 0 {
        Some((millisats / 1_000, "SATS"))
    } else {
        Some((millisats, "MSATS"))
    }
}

/// Parse and validate a NIP-102 receipt event.
///
/// # Errors
/// Returns `PurchaseError` when required tags or payment proof are invalid.
pub fn parse_and_validate_receipt(
    event: &Event,
    buyer_pubkey_hex: &str,
) -> Result<StoredReceipt, PurchaseError> {
    let parsed = parse_receipt_event(event)?;
    validate_receipt_buyer(&parsed, buyer_pubkey_hex)?;
    let developer = coordinate_publisher(&parsed.coordinate)
        .ok_or_else(|| PurchaseError::ProofInvalid("malformed listing coordinate".into()))?;
    validate_adp_receipt_root(&parsed, developer, None)?;
    stored_receipt(&parsed)
}

/// Parse and validate a NIP-102 receipt against an ADP listing delegation snapshot.
///
/// # Errors
/// Returns `PurchaseError` when required tags, payment proof, or signer authorization is invalid.
pub fn parse_and_validate_receipt_with_listing(
    event: &Event,
    buyer_pubkey_hex: &str,
    listing_event: &Event,
) -> Result<StoredReceipt, PurchaseError> {
    listing_event.verify().map_err(|error| {
        PurchaseError::ProofInvalid(format!("invalid listing signature: {error}"))
    })?;
    let parsed = parse_receipt_event(event)?;
    validate_receipt_buyer(&parsed, buyer_pubkey_hex)?;
    let coordinate = listing_coordinate_from_listing(listing_event)?;
    if parsed.coordinate != coordinate {
        return Err(PurchaseError::ProofInvalid(
            "receipt listing coordinate does not match listing event".into(),
        ));
    }
    validate_adp_receipt_root(&parsed, listing_event.pubkey, None)?;
    stored_receipt(&parsed)
}

pub fn parse_and_validate_receipt_with_authorization(
    event: &Event,
    buyer_pubkey_hex: &str,
    authorization: Option<&ResolvedAuthorization>,
    evidence: ReceiptEvidence<'_>,
) -> Result<StoredReceipt, PurchaseError> {
    let parsed = parse_receipt_event(event)?;
    validate_receipt_buyer(&parsed, buyer_pubkey_hex)?;
    let developer = coordinate_publisher(&parsed.coordinate)
        .ok_or_else(|| PurchaseError::ProofInvalid("malformed listing coordinate".into()))?;
    validate_adp_receipt_root(&parsed, developer, authorization)?;
    validate_receipt_evidence(&parsed, evidence)?;
    stored_receipt(&parsed)
}

fn listing_coordinate_from_listing(listing: &Event) -> Result<String, PurchaseError> {
    if listing.kind.as_u16() != 30402 {
        return Err(PurchaseError::ProofInvalid(
            "listing event is not kind 30402".to_string(),
        ));
    }
    let d_tag = required_tag_value(listing, "d", "listing d tag")?;
    Ok(format!("30402:{}:{}", listing.pubkey.to_hex(), d_tag))
}

fn exact_tag(
    event: &Event,
    name: &'static str,
    required: bool,
) -> Result<Option<String>, PurchaseError> {
    let tags = event
        .tags
        .iter()
        .map(|tag| tag.clone().to_vec())
        .filter(|values| values.first().is_some_and(|value| value == name))
        .collect::<Vec<_>>();
    if tags.len() > 1 {
        return Err(PurchaseError::ProofInvalid(format!("duplicate {name} tag")));
    }
    let Some(values) = tags.first() else {
        return if required {
            Err(PurchaseError::MissingTag(name))
        } else {
            Ok(None)
        };
    };
    if values.len() != 2 || values[1].is_empty() {
        return Err(PurchaseError::ProofInvalid(format!("malformed {name} tag")));
    }
    Ok(Some(values[1].clone()))
}

fn parse_proofs(event: &Event) -> Result<Vec<ReceiptProof>, PurchaseError> {
    let mut proofs = Vec::new();
    let mut seen = HashSet::new();
    for values in event.tags.iter().map(|tag| tag.clone().to_vec()) {
        if values.first().is_none_or(|name| name != "proof") {
            continue;
        }
        let proof = match values.as_slice() {
            [_, kind] if kind == "bolt11-preimage" => ReceiptProof::Bolt11Preimage,
            [_, kind, id] if kind == "zap" => ReceiptProof::Zap(
                EventId::from_hex(id)
                    .map_err(|_| PurchaseError::ProofInvalid("malformed proof tag".into()))?,
            ),
            _ => return Err(PurchaseError::ProofInvalid("malformed proof tag".into())),
        };
        if !seen.insert(proof.clone()) {
            return Err(PurchaseError::ProofInvalid("duplicate proof tag".into()));
        }
        proofs.push(proof);
    }
    Ok(proofs)
}

pub fn parse_receipt_event(event: &Event) -> Result<ParsedReceipt, PurchaseError> {
    event.verify().map_err(|error| {
        PurchaseError::ProofInvalid(format!("invalid event signature: {error}"))
    })?;
    if event.kind.as_u16() != KIND_PURCHASE_RECEIPT {
        return Err(PurchaseError::WrongKind(event.kind.as_u16()));
    }
    let buyer_pubkey =
        PublicKey::from_hex(&exact_tag(event, "p", true)?.ok_or(PurchaseError::MissingTag("p"))?)
            .map_err(|_| PurchaseError::ProofInvalid("malformed p tag".into()))?;
    let coordinate = exact_tag(event, "a", true)?.ok_or(PurchaseError::MissingTag("a"))?;
    if coordinate_publisher(&coordinate).is_none() {
        return Err(PurchaseError::ProofInvalid(
            "malformed listing coordinate".into(),
        ));
    }
    let status = exact_tag(event, "status", true)?.ok_or(PurchaseError::MissingTag("status"))?;
    if !matches!(
        status.as_str(),
        "paid" | "fulfilled" | "refunded" | "disputed"
    ) {
        return Err(PurchaseError::ProofInvalid("malformed status tag".into()));
    }
    let authorization = exact_tag(event, "authorization", false)?
        .map(|value| {
            EventId::from_hex(&value)
                .map_err(|_| PurchaseError::ProofInvalid("malformed authorization tag".into()))
        })
        .transpose()?;
    let payment_hash = exact_tag(event, "payment_hash", false)?;
    if payment_hash
        .as_ref()
        .is_some_and(|hash| hash.len() != 64 || hex::decode(hash).is_err())
    {
        return Err(PurchaseError::ProofInvalid(
            "malformed payment_hash tag".into(),
        ));
    }
    let amount_msat = exact_tag(event, "amount_msat", false)?
        .map(|value| value.parse::<u64>())
        .transpose()
        .map_err(|_| PurchaseError::ProofInvalid("malformed amount_msat tag".into()))?;
    let settled_at = exact_tag(event, "settled_at", false)?
        .map(|value| value.parse::<u64>())
        .transpose()
        .map_err(|_| PurchaseError::ProofInvalid("malformed settled_at tag".into()))?;
    let predecessor = exact_tag(event, "e", false)?
        .map(|value| {
            EventId::from_hex(&value)
                .map_err(|_| PurchaseError::ProofInvalid("malformed e tag".into()))
        })
        .transpose()?;
    Ok(ParsedReceipt {
        event: event.clone(),
        order_id: exact_tag(event, "order", true)?.ok_or(PurchaseError::MissingTag("order"))?,
        buyer_pubkey,
        coordinate,
        authorization,
        payment_hash,
        amount_msat,
        settled_at,
        proofs: parse_proofs(event)?,
        status,
        predecessor,
    })
}

fn validate_receipt_buyer(parsed: &ParsedReceipt, buyer: &str) -> Result<(), PurchaseError> {
    if parsed.buyer_pubkey.to_hex() == buyer {
        Ok(())
    } else {
        Err(PurchaseError::BuyerMismatch)
    }
}

pub fn validate_adp_receipt_root(
    receipt: &ParsedReceipt,
    developer: PublicKey,
    authorization: Option<&ResolvedAuthorization>,
) -> Result<(), PurchaseError> {
    if receipt.predecessor.is_some()
        || receipt.status != STATUS_PAID
        || receipt.payment_hash.is_none()
        || receipt.amount_msat.is_none()
        || receipt.settled_at.is_none()
        || receipt.proofs.is_empty()
    {
        return Err(PurchaseError::ProofInvalid(
            "receipt root is incomplete".into(),
        ));
    }
    if receipt.event.pubkey == developer {
        if receipt.authorization.is_some() {
            return Err(PurchaseError::ProofInvalid(
                "direct receipt must omit authorization".into(),
            ));
        }
        return Ok(());
    }
    let anchor = receipt.authorization.ok_or_else(|| {
        PurchaseError::ProofInvalid("delegated receipt is missing authorization".into())
    })?;
    let authorization = authorization
        .ok_or_else(|| PurchaseError::ProofInvalid("authorization evidence unavailable".into()))?;
    if authorization.root_event_id != anchor
        || authorization.developer_pubkey != developer
        || !authorization.authorizes(
            &receipt.event.pubkey,
            &receipt.coordinate,
            CAPABILITY_ISSUE_RECEIPT,
            receipt.event.created_at.as_secs(),
        )
    {
        return Err(PurchaseError::ProofInvalid(
            "delegated receipt authorization denied".into(),
        ));
    }
    Ok(())
}

pub fn resolve_receipt_chain<'a>(
    events: &'a [Event],
    developer: PublicKey,
    authorization: Option<&ResolvedAuthorization>,
) -> Result<&'a Event, PurchaseError> {
    let parsed = events
        .iter()
        .filter_map(|event| {
            parse_receipt_event(event)
                .ok()
                .map(|parsed| (event, parsed))
        })
        .collect::<Vec<_>>();
    let roots = parsed
        .iter()
        .filter(|(_, receipt)| {
            receipt.predecessor.is_none()
                && receipt.status == STATUS_PAID
                && receipt.payment_hash.is_some()
                && receipt.amount_msat.is_some()
                && receipt.settled_at.is_some()
                && !receipt.proofs.is_empty()
        })
        .collect::<Vec<_>>();
    if roots.len() != 1 {
        return Err(PurchaseError::ProofInvalid(
            "receipt chain must have one valid paid root".into(),
        ));
    }
    let (root_event, root) = roots[0];
    validate_adp_receipt_root(root, developer, authorization)?;
    let delegated = root_event.pubkey != developer;
    let mut current_event = *root_event;
    let mut current = root.clone();
    let mut visited = HashSet::from([current_event.id]);
    loop {
        let mut valid = parsed
            .iter()
            .filter(|(_, candidate)| candidate.predecessor == Some(current_event.id))
            .filter(|(event, candidate)| {
                event.created_at > current_event.created_at
                    && candidate.order_id == root.order_id
                    && candidate.buyer_pubkey == root.buyer_pubkey
                    && candidate.coordinate == root.coordinate
                    && candidate.authorization == root.authorization
                    && candidate.payment_hash.is_none()
                    && candidate.amount_msat.is_none()
                    && candidate.settled_at.is_none()
                    && candidate.proofs.is_empty()
                    && (event.pubkey == developer
                        || (delegated && event.pubkey == root_event.pubkey))
                    && current.status != "refunded"
                    && candidate.status != STATUS_PAID
                    && candidate.predecessor != Some(event.id)
            })
            .collect::<Vec<_>>();
        if valid.len() > 1 {
            return Err(PurchaseError::ProofInvalid(format!(
                "receipt chain forks at {}",
                current_event.id
            )));
        }
        let Some((next_event, next)) = valid.pop() else {
            break;
        };
        if !visited.insert(next_event.id) {
            return Err(PurchaseError::ProofInvalid(
                "receipt chain contains a cycle".into(),
            ));
        }
        current_event = next_event;
        current = next.clone();
    }
    Ok(current_event)
}

pub fn validate_receipt_evidence(
    receipt: &ParsedReceipt,
    evidence: ReceiptEvidence<'_>,
) -> Result<(), PurchaseError> {
    match (evidence.bolt11, evidence.preimage) {
        (Some(bolt11), Some(preimage)) => {
            if !receipt.proofs.contains(&ReceiptProof::Bolt11Preimage) {
                return Err(PurchaseError::ProofInvalid(
                    "undeclared bolt11 proof".into(),
                ));
            }
            let invoice = Bolt11Invoice::from_str(bolt11).map_err(|error| {
                PurchaseError::ProofInvalid(format!("invalid bolt11 invoice: {error}"))
            })?;
            let bytes = hex::decode(preimage).map_err(|error| {
                PurchaseError::ProofInvalid(format!("invalid preimage hex: {error}"))
            })?;
            let digest = hex::encode(<[u8; 32]>::from(Sha256::digest(bytes)));
            if digest != invoice.payment_hash().to_string()
                || receipt.payment_hash.as_deref() != Some(digest.as_str())
                || invoice.amount_milli_satoshis() != receipt.amount_msat
            {
                return Err(PurchaseError::ProofInvalid(
                    "bolt11 proof contradicts receipt binding".into(),
                ));
            }
        }
        (None, None) => {}
        _ => {
            return Err(PurchaseError::ProofInvalid(
                "bolt11 and preimage must be supplied together".into(),
            ))
        }
    }
    let supplied_zaps = evidence
        .zap_receipts
        .iter()
        .map(|event| (event.id, event))
        .collect::<HashMap<_, _>>();
    for proof in &receipt.proofs {
        if let ReceiptProof::Zap(id) = proof {
            if !evidence.zap_receipts.is_empty() {
                let event = supplied_zaps.get(id).ok_or_else(|| {
                    PurchaseError::ProofInvalid("declared zap proof was not supplied".into())
                })?;
                event.verify().map_err(|_| {
                    PurchaseError::ProofInvalid("invalid zap proof signature".into())
                })?;
                if event.kind.as_u16() != 9735 {
                    return Err(PurchaseError::ProofInvalid(
                        "zap proof has wrong kind".into(),
                    ));
                }
                if evidence.lsp_pubkey.is_some_and(|lsp| event.pubkey != lsp) {
                    return Err(PurchaseError::ProofInvalid(
                        "zap proof signer is not the listing LSP".into(),
                    ));
                }
                let buyer = receipt.buyer_pubkey.to_hex();
                let has_buyer = event.tags.iter().any(|tag| matches!(tag.as_slice(), [name, value, ..] if name == "P" && value.as_str() == buyer));
                let has_coordinate = event.tags.iter().any(|tag| matches!(tag.as_slice(), [name, value, ..] if matches!(name.as_str(), "a" | "e") && value == &receipt.coordinate));
                if !has_buyer || !has_coordinate {
                    return Err(PurchaseError::ProofInvalid(
                        "zap proof binding mismatch".into(),
                    ));
                }
                let invoices = event
                    .tags
                    .iter()
                    .filter_map(|tag| match tag.as_slice() {
                        [name, value] if name == "bolt11" => Some(value.to_string()),
                        _ => None,
                    })
                    .collect::<Vec<_>>();
                if invoices.len() != 1 {
                    return Err(PurchaseError::ProofInvalid(
                        "zap proof must contain one bolt11 tag".into(),
                    ));
                }
                let invoice = Bolt11Invoice::from_str(&invoices[0]).map_err(|error| {
                    PurchaseError::ProofInvalid(format!("invalid zap invoice: {error}"))
                })?;
                if receipt.payment_hash.as_deref()
                    != Some(invoice.payment_hash().to_string().as_str())
                    || receipt.amount_msat != invoice.amount_milli_satoshis()
                    || receipt
                        .settled_at
                        .is_some_and(|settled| event.created_at.as_secs() > settled)
                {
                    return Err(PurchaseError::ProofInvalid(
                        "zap proof contradicts receipt binding".into(),
                    ));
                }
            }
        }
    }
    Ok(())
}

fn stored_receipt(parsed: &ParsedReceipt) -> Result<StoredReceipt, PurchaseError> {
    Ok(StoredReceipt {
        event_id: parsed.event.id.to_hex(),
        order_id: parsed.order_id.clone(),
        listing_coordinate: parsed.coordinate.clone(),
        buyer_pubkey: parsed.buyer_pubkey.to_hex(),
        merchant_pubkey: parsed.event.pubkey.to_hex(),
        payment_hash: parsed.payment_hash.clone(),
        status: parsed.status.clone(),
        created_at: parsed.event.created_at.as_secs(),
        raw_event: serde_json::to_string(&parsed.event)?,
    })
}

fn resolve_persisted_receipt_tip(events: &[Event]) -> Option<&Event> {
    let parsed = events
        .iter()
        .filter_map(|event| {
            parse_receipt_event(event)
                .ok()
                .map(|parsed| (event, parsed))
        })
        .collect::<Vec<_>>();
    let roots = parsed
        .iter()
        .filter(|(_, receipt)| {
            receipt.predecessor.is_none()
                && receipt.status == STATUS_PAID
                && receipt.payment_hash.is_some()
                && receipt.amount_msat.is_some()
                && receipt.settled_at.is_some()
                && !receipt.proofs.is_empty()
        })
        .collect::<Vec<_>>();
    if roots.len() != 1 {
        return None;
    }
    let (root_event, root) = roots[0];
    let developer = coordinate_publisher(&root.coordinate)?;
    let delegated = root_event.pubkey != developer;
    if (delegated && root.authorization.is_none()) || (!delegated && root.authorization.is_some()) {
        return None;
    }
    let mut current_event = *root_event;
    let mut current = root.clone();
    loop {
        let mut valid = parsed
            .iter()
            .filter(|(_, candidate)| candidate.predecessor == Some(current_event.id))
            .filter(|(event, candidate)| {
                event.created_at > current_event.created_at
                    && candidate.order_id == root.order_id
                    && candidate.buyer_pubkey == root.buyer_pubkey
                    && candidate.coordinate == root.coordinate
                    && candidate.authorization == root.authorization
                    && candidate.payment_hash.is_none()
                    && candidate.amount_msat.is_none()
                    && candidate.settled_at.is_none()
                    && candidate.proofs.is_empty()
                    && (event.pubkey == developer
                        || (delegated && event.pubkey == root_event.pubkey))
                    && current.status != "refunded"
                    && candidate.status != STATUS_PAID
            })
            .collect::<Vec<_>>();
        if valid.len() > 1 {
            return None;
        }
        let Some((next_event, next)) = valid.pop() else {
            break;
        };
        current_event = next_event;
        current = next.clone();
    }
    Some(current_event)
}

fn validate_payment_proof(event: &Event) -> Result<Option<String>, PurchaseError> {
    let parsed = parse_receipt_event(event)?;
    if parsed.proofs.is_empty() {
        return Err(PurchaseError::MissingPaymentProof);
    }
    Ok(parsed.payment_hash)
}

fn required_tag_value(
    event: &Event,
    tag_name: &'static str,
    display_name: &'static str,
) -> Result<String, PurchaseError> {
    tag_value(event, tag_name).ok_or(PurchaseError::MissingTag(display_name))
}

fn tag_value(event: &Event, tag_name: &str) -> Option<String> {
    event.tags.iter().find_map(|tag| match tag.as_slice() {
        [name, value, ..] if name == tag_name && !value.is_empty() => Some(value.to_string()),
        _ => None,
    })
}

fn status_grants_ownership(status: &str) -> bool {
    matches!(status, STATUS_PAID | STATUS_FULFILLED)
}

#[cfg(any())]
mod tests {
    use super::{
        parse_and_validate_receipt, parse_and_validate_receipt_with_listing, stored_receipt_amount,
        stored_receipt_validation_error, PurchaseError, PurchasesRepository,
    };
    use crate::storage::Database;
    use bitcoin::hashes::{sha256, Hash as _};
    use bitcoin::secp256k1::{Secp256k1, SecretKey};
    use lightning_invoice::{Currency, InvoiceBuilder};
    use lightning_types::payment::PaymentSecret;
    use nostr_sdk::{Event, EventBuilder, Keys, Kind, Tag, TagKind, Timestamp};
    use std::time::Duration;
    use tempfile::TempDir;

    const LISTING_D_TAG: &str = "game";
    const ZAP_EVENT_ID: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";

    fn listing_coordinate(merchant: &Keys) -> String {
        format!("30402:{}:{LISTING_D_TAG}", merchant.public_key().to_hex())
    }

    fn receipt_event(kind: Kind, merchant: &Keys, extra_tags: Vec<Tag>) -> Event {
        EventBuilder::new(kind, "")
            .tags(extra_tags)
            .sign_with_keys(merchant)
            .expect("receipt event signs")
    }

    fn receipt_event_at(kind: Kind, signer: &Keys, extra_tags: Vec<Tag>, created_at: u64) -> Event {
        EventBuilder::new(kind, "")
            .tags(extra_tags)
            .custom_created_at(Timestamp::from(created_at))
            .sign_with_keys(signer)
            .expect("receipt event signs")
    }

    fn listing_event_at(
        merchant: &Keys,
        fulfillment: &Keys,
        valid_from: u64,
        revoked_at: Option<u64>,
        created_at: u64,
    ) -> Event {
        let valid_from = valid_from.to_string();
        let revoked_at = revoked_at
            .map(|value| value.to_string())
            .unwrap_or_default();
        EventBuilder::new(Kind::Custom(30402), "")
            .tags([
                Tag::custom(TagKind::d(), [LISTING_D_TAG]),
                Tag::custom(
                    TagKind::custom("fulfillment_pubkey"),
                    [fulfillment.public_key().to_hex(), valid_from, revoked_at],
                ),
            ])
            .custom_created_at(Timestamp::from(created_at))
            .sign_with_keys(merchant)
            .expect("listing event signs")
    }

    fn valid_tags(buyer_hex: &str, merchant: &Keys) -> Vec<Tag> {
        vec![
            Tag::custom(TagKind::p(), [buyer_hex]),
            Tag::custom(TagKind::a(), [listing_coordinate(merchant)]),
            Tag::custom(TagKind::custom("order"), ["order-1"]),
            Tag::custom(TagKind::e(), [ZAP_EVENT_ID]),
        ]
    }

    fn valid_receipt(status: &str, buyer: &Keys, merchant: &Keys) -> Event {
        let buyer_hex = buyer.public_key().to_hex();
        let mut tags = valid_tags(&buyer_hex, merchant);
        tags.push(Tag::custom(TagKind::custom("status"), [status]));
        receipt_event(Kind::Custom(1020), merchant, tags)
    }

    fn bolt11_with_preimage() -> (String, String) {
        let preimage = [7_u8; 32];
        let payment_hash = sha256::Hash::hash(&preimage);
        let private_key = SecretKey::from_slice(&[42_u8; 32]).expect("private key is valid");
        let invoice = InvoiceBuilder::new(Currency::Bitcoin)
            .description("Arcadestr purchase receipt test".to_string())
            .payment_hash(payment_hash)
            .payment_secret(PaymentSecret([11_u8; 32]))
            .amount_milli_satoshis(21_000)
            .duration_since_epoch(Duration::from_secs(1_700_000_000))
            .min_final_cltv_expiry_delta(144)
            .build_signed(|hash| Secp256k1::new().sign_ecdsa_recoverable(hash, &private_key))
            .expect("test invoice builds");

        (invoice.to_string(), hex::encode(preimage))
    }

    fn bolt11_receipt(
        buyer_hex: &str,
        merchant: &Keys,
        bolt11: &str,
        preimage_hex: &str,
    ) -> Vec<Tag> {
        valid_tags(buyer_hex, merchant)
            .into_iter()
            .filter(|tag| tag.kind() != TagKind::e())
            .chain([
                Tag::custom(TagKind::custom("bolt11"), [bolt11]),
                Tag::custom(TagKind::custom("preimage"), [preimage_hex]),
            ])
            .collect()
    }

    #[test]
    fn validates_direct_developer_receipt_with_listing() {
        let buyer = Keys::generate();
        let merchant = Keys::generate();
        let fulfillment = Keys::generate();
        let buyer_hex = buyer.public_key().to_hex();
        let listing = listing_event_at(&merchant, &fulfillment, 1_700_000_000, None, 1_700_000_000);
        let receipt = receipt_event_at(
            Kind::Custom(1020),
            &merchant,
            valid_tags(&buyer_hex, &merchant),
            1_700_000_100,
        );

        let stored = parse_and_validate_receipt_with_listing(&receipt, &buyer_hex, &listing)
            .expect("direct developer receipt should validate");

        assert_eq!(stored.listing_coordinate, listing_coordinate(&merchant));
    }

    #[test]
    fn validates_active_delegated_fulfillment_receipt_with_listing() {
        let buyer = Keys::generate();
        let merchant = Keys::generate();
        let fulfillment = Keys::generate();
        let buyer_hex = buyer.public_key().to_hex();
        let listing = listing_event_at(&merchant, &fulfillment, 1_700_000_000, None, 1_700_000_000);
        let receipt = receipt_event_at(
            Kind::Custom(1020),
            &fulfillment,
            valid_tags(&buyer_hex, &merchant),
            1_700_000_100,
        );

        let stored = parse_and_validate_receipt_with_listing(&receipt, &buyer_hex, &listing)
            .expect("active delegated receipt should validate");

        assert_eq!(stored.merchant_pubkey, fulfillment.public_key().to_hex());
    }

    #[test]
    fn rejects_delegated_receipt_before_valid_from() {
        let buyer = Keys::generate();
        let merchant = Keys::generate();
        let fulfillment = Keys::generate();
        let buyer_hex = buyer.public_key().to_hex();
        let listing = listing_event_at(&merchant, &fulfillment, 1_700_000_000, None, 1_700_000_000);
        let receipt = receipt_event_at(
            Kind::Custom(1020),
            &fulfillment,
            valid_tags(&buyer_hex, &merchant),
            1_699_999_999,
        );

        assert!(parse_and_validate_receipt_with_listing(&receipt, &buyer_hex, &listing).is_err());
    }

    #[test]
    fn rejects_delegated_receipt_at_or_after_revoked_at() {
        let buyer = Keys::generate();
        let merchant = Keys::generate();
        let fulfillment = Keys::generate();
        let buyer_hex = buyer.public_key().to_hex();
        let listing = listing_event_at(
            &merchant,
            &fulfillment,
            1_700_000_000,
            Some(1_735_000_000),
            1_700_000_000,
        );
        let receipt = receipt_event_at(
            Kind::Custom(1020),
            &fulfillment,
            valid_tags(&buyer_hex, &merchant),
            1_735_000_000,
        );

        assert!(parse_and_validate_receipt_with_listing(&receipt, &buyer_hex, &listing).is_err());
    }

    #[test]
    fn validates_delegated_receipt_before_revoked_at() {
        let buyer = Keys::generate();
        let merchant = Keys::generate();
        let fulfillment = Keys::generate();
        let buyer_hex = buyer.public_key().to_hex();
        let listing = listing_event_at(
            &merchant,
            &fulfillment,
            1_700_000_000,
            Some(1_735_000_000),
            1_700_000_000,
        );
        let receipt = receipt_event_at(
            Kind::Custom(1020),
            &fulfillment,
            valid_tags(&buyer_hex, &merchant),
            1_734_999_999,
        );

        assert!(parse_and_validate_receipt_with_listing(&receipt, &buyer_hex, &listing).is_ok());
    }

    #[test]
    fn clamps_revoked_at_older_than_listing_created_at() {
        let buyer = Keys::generate();
        let merchant = Keys::generate();
        let fulfillment = Keys::generate();
        let buyer_hex = buyer.public_key().to_hex();
        let listing = listing_event_at(
            &merchant,
            &fulfillment,
            1_500_000_000,
            Some(1_600_000_000),
            1_700_000_000,
        );
        let receipt = receipt_event_at(
            Kind::Custom(1020),
            &fulfillment,
            valid_tags(&buyer_hex, &merchant),
            1_650_000_000,
        );

        assert!(parse_and_validate_receipt_with_listing(&receipt, &buyer_hex, &listing).is_ok());
    }

    #[test]
    fn rejects_receipt_signer_matching_no_listing_delegation() {
        let buyer = Keys::generate();
        let merchant = Keys::generate();
        let fulfillment = Keys::generate();
        let stranger = Keys::generate();
        let buyer_hex = buyer.public_key().to_hex();
        let listing = listing_event_at(&merchant, &fulfillment, 1_700_000_000, None, 1_700_000_000);
        let receipt = receipt_event_at(
            Kind::Custom(1020),
            &stranger,
            valid_tags(&buyer_hex, &merchant),
            1_700_000_100,
        );

        assert!(parse_and_validate_receipt_with_listing(&receipt, &buyer_hex, &listing).is_err());
    }

    #[test]
    fn rejects_wrong_kind() {
        // Arrange
        let buyer = Keys::generate();
        let merchant = Keys::generate();
        let buyer_hex = buyer.public_key().to_hex();
        let event = receipt_event(Kind::TextNote, &merchant, valid_tags(&buyer_hex, &merchant));

        // Act
        let error = parse_and_validate_receipt(&event, &buyer_hex)
            .expect_err("wrong kind should be rejected");

        // Assert
        assert!(error.to_string().contains("kind 1020"));
    }

    #[test]
    fn rejects_missing_p_tag() {
        // Arrange
        let buyer = Keys::generate();
        let merchant = Keys::generate();
        let buyer_hex = buyer.public_key().to_hex();
        let event = receipt_event(
            Kind::Custom(1020),
            &merchant,
            vec![
                Tag::custom(TagKind::a(), [listing_coordinate(&merchant)]),
                Tag::custom(TagKind::custom("order"), ["order-1"]),
                Tag::custom(TagKind::e(), [ZAP_EVENT_ID]),
            ],
        );

        // Act
        let error = parse_and_validate_receipt(&event, &buyer_hex)
            .expect_err("missing p tag should be rejected");

        // Assert
        assert!(error.to_string().contains("p tag"));
    }

    #[test]
    fn rejects_buyer_mismatch() {
        // Arrange
        let buyer = Keys::generate();
        let other_buyer = Keys::generate();
        let merchant = Keys::generate();
        let buyer_hex = buyer.public_key().to_hex();
        let other_buyer_hex = other_buyer.public_key().to_hex();
        let event = receipt_event(
            Kind::Custom(1020),
            &merchant,
            valid_tags(&other_buyer_hex, &merchant),
        );

        // Act
        let error = parse_and_validate_receipt(&event, &buyer_hex)
            .expect_err("buyer mismatch should be rejected");

        // Assert
        assert!(error.to_string().contains("buyer"));
    }

    #[test]
    fn rejects_invalid_signature_before_kind_validation() {
        // Arrange
        let buyer = Keys::generate();
        let merchant = Keys::generate();
        let buyer_hex = buyer.public_key().to_hex();
        let mut event = valid_receipt("paid", &buyer, &merchant);
        event.kind = Kind::TextNote;

        // Act
        let error = parse_and_validate_receipt(&event, &buyer_hex)
            .expect_err("invalid signature should be rejected first");

        // Assert
        assert!(matches!(error, PurchaseError::ProofInvalid(_)));
    }

    #[test]
    fn rejects_malformed_listing_coordinate() {
        // Arrange
        let buyer = Keys::generate();
        let merchant = Keys::generate();
        let buyer_hex = buyer.public_key().to_hex();
        let mut tags = valid_tags(&buyer_hex, &merchant);
        tags.retain(|tag| tag.kind() != TagKind::a());
        tags.push(Tag::custom(TagKind::a(), ["30402:missing-d-tag"]));
        let event = receipt_event(Kind::Custom(1020), &merchant, tags);

        // Act
        let error = parse_and_validate_receipt(&event, &buyer_hex)
            .expect_err("malformed coordinate should be rejected");

        // Assert
        assert!(matches!(error, PurchaseError::ProofInvalid(_)));
    }

    #[test]
    fn rejects_listing_coordinate_merchant_mismatch() {
        // Arrange
        let buyer = Keys::generate();
        let merchant = Keys::generate();
        let other_merchant = Keys::generate();
        let buyer_hex = buyer.public_key().to_hex();
        let mut tags = valid_tags(&buyer_hex, &merchant);
        tags.retain(|tag| tag.kind() != TagKind::a());
        tags.push(Tag::custom(
            TagKind::a(),
            [listing_coordinate(&other_merchant)],
        ));
        let event = receipt_event(Kind::Custom(1020), &merchant, tags);

        // Act
        let error = parse_and_validate_receipt(&event, &buyer_hex)
            .expect_err("merchant mismatch should be rejected");

        // Assert
        assert!(matches!(error, PurchaseError::ProofInvalid(_)));
    }

    #[test]
    fn rejects_missing_order_and_a_tags() {
        // Arrange
        let buyer = Keys::generate();
        let merchant = Keys::generate();
        let buyer_hex = buyer.public_key().to_hex();
        let missing_a = receipt_event(
            Kind::Custom(1020),
            &merchant,
            vec![
                Tag::custom(TagKind::p(), [buyer_hex.as_str()]),
                Tag::custom(TagKind::custom("order"), ["order-1"]),
                Tag::custom(TagKind::e(), [ZAP_EVENT_ID]),
            ],
        );
        let missing_order = receipt_event(
            Kind::Custom(1020),
            &merchant,
            vec![
                Tag::custom(TagKind::p(), [buyer_hex.as_str()]),
                Tag::custom(TagKind::a(), [listing_coordinate(&merchant)]),
                Tag::custom(TagKind::e(), [ZAP_EVENT_ID]),
            ],
        );

        // Act
        let missing_a_error = parse_and_validate_receipt(&missing_a, &buyer_hex)
            .expect_err("missing a tag should be rejected");
        let missing_order_error = parse_and_validate_receipt(&missing_order, &buyer_hex)
            .expect_err("missing order tag should be rejected");

        // Assert
        assert!(missing_a_error.to_string().contains("a tag"));
        assert!(missing_order_error.to_string().contains("order tag"));
    }

    #[test]
    fn rejects_invalid_preimage_hex() {
        // Arrange
        let buyer = Keys::generate();
        let merchant = Keys::generate();
        let buyer_hex = buyer.public_key().to_hex();
        let (bolt11, _) = bolt11_with_preimage();
        let tags = bolt11_receipt(&buyer_hex, &merchant, &bolt11, "not-hex");
        let event = receipt_event(Kind::Custom(1020), &merchant, tags);

        // Act
        let error = parse_and_validate_receipt(&event, &buyer_hex)
            .expect_err("invalid preimage hex should be rejected");

        // Assert
        assert!(matches!(error, PurchaseError::ProofInvalid(_)));
    }

    #[test]
    fn accepts_valid_bolt11_with_matching_preimage() {
        // Arrange
        let buyer = Keys::generate();
        let merchant = Keys::generate();
        let buyer_hex = buyer.public_key().to_hex();
        let (bolt11, preimage_hex) = bolt11_with_preimage();
        let event = receipt_event(
            Kind::Custom(1020),
            &merchant,
            bolt11_receipt(&buyer_hex, &merchant, &bolt11, &preimage_hex),
        );

        // Act
        let receipt = parse_and_validate_receipt(&event, &buyer_hex)
            .expect("matching bolt11 and preimage should validate");

        // Assert
        assert!(receipt.payment_hash.is_some());
        assert_eq!(receipt.order_id, "order-1");
        assert_eq!(stored_receipt_amount(&receipt), Some((21, "SATS")));
    }

    #[test]
    fn rejects_valid_bolt11_with_mismatched_preimage() {
        // Arrange
        let buyer = Keys::generate();
        let merchant = Keys::generate();
        let buyer_hex = buyer.public_key().to_hex();
        let (bolt11, _) = bolt11_with_preimage();
        let mismatched_preimage = hex::encode([8_u8; 32]);
        let event = receipt_event(
            Kind::Custom(1020),
            &merchant,
            bolt11_receipt(&buyer_hex, &merchant, &bolt11, &mismatched_preimage),
        );

        // Act
        let error = parse_and_validate_receipt(&event, &buyer_hex)
            .expect_err("mismatched preimage should be rejected");

        // Assert
        assert!(matches!(error, PurchaseError::ProofInvalid(_)));
    }

    #[test]
    fn rejects_receipt_without_payment_proof() {
        // Arrange
        let buyer = Keys::generate();
        let merchant = Keys::generate();
        let buyer_hex = buyer.public_key().to_hex();
        let tags = valid_tags(&buyer_hex, &merchant)
            .into_iter()
            .filter(|tag| tag.kind() != TagKind::e())
            .collect::<Vec<_>>();
        let event = receipt_event(Kind::Custom(1020), &merchant, tags);

        // Act
        let error = parse_and_validate_receipt(&event, &buyer_hex)
            .expect_err("receipt without proof should be rejected");

        // Assert
        assert!(error.to_string().contains("payment proof"));
    }

    #[test]
    fn accepts_zap_e_tag_as_payment_proof() {
        // Arrange
        let buyer = Keys::generate();
        let merchant = Keys::generate();
        let buyer_hex = buyer.public_key().to_hex();
        let event = valid_receipt("paid", &buyer, &merchant);

        // Act
        let receipt = parse_and_validate_receipt(&event, &buyer_hex)
            .expect("zap e tag should be accepted as proof");

        // Assert
        assert_eq!(receipt.order_id, "order-1");
        assert_eq!(receipt.listing_coordinate, listing_coordinate(&merchant));
        assert_eq!(receipt.buyer_pubkey, buyer_hex);
        assert_eq!(receipt.merchant_pubkey, merchant.public_key().to_hex());
        assert_eq!(receipt.status, "paid");
        assert_eq!(
            stored_receipt_validation_error(&receipt),
            Some("Referenced payment proof is not available for local verification.")
        );
    }

    #[tokio::test]
    async fn reports_paid_receipt_as_owned() {
        // Arrange
        let temp_dir = TempDir::new().expect("temp dir is created");
        let db_path = temp_dir.path().join("purchases.sqlite");
        let db = Database::new(&db_path).await.expect("database opens");
        let repository = PurchasesRepository::new(db.pool().clone());
        let buyer = Keys::generate();
        let merchant = Keys::generate();
        let buyer_hex = buyer.public_key().to_hex();
        let receipt =
            parse_and_validate_receipt(&valid_receipt("paid", &buyer, &merchant), &buyer_hex)
                .expect("valid receipt parses");

        // Act
        repository
            .upsert_receipt(&receipt)
            .await
            .expect("receipt is stored");
        let is_owned = repository
            .is_owned(&buyer_hex, &listing_coordinate(&merchant))
            .await
            .expect("ownership query succeeds");

        // Assert
        assert!(is_owned);
    }

    #[tokio::test]
    async fn latest_refunded_receipt_removes_ownership() {
        // Arrange
        let temp_dir = TempDir::new().expect("temp dir is created");
        let db_path = temp_dir.path().join("purchases.sqlite");
        let db = Database::new(&db_path).await.expect("database opens");
        let repository = PurchasesRepository::new(db.pool().clone());
        let buyer = Keys::generate();
        let merchant = Keys::generate();
        let buyer_hex = buyer.public_key().to_hex();
        let paid =
            parse_and_validate_receipt(&valid_receipt("paid", &buyer, &merchant), &buyer_hex)
                .expect("paid receipt parses");
        let mut refunded =
            parse_and_validate_receipt(&valid_receipt("refunded", &buyer, &merchant), &buyer_hex)
                .expect("refunded receipt parses");
        refunded.created_at = paid.created_at + 1;

        // Act
        repository
            .upsert_receipt(&paid)
            .await
            .expect("paid receipt is stored");
        repository
            .upsert_receipt(&refunded)
            .await
            .expect("refunded receipt is stored");
        let is_owned = repository
            .is_owned(&buyer_hex, &listing_coordinate(&merchant))
            .await
            .expect("ownership query succeeds");

        // Assert
        assert!(!is_owned);
    }

    #[tokio::test]
    async fn refunded_order_does_not_revoke_separate_paid_order() {
        // Arrange
        let temp_dir = TempDir::new().expect("temp dir is created");
        let db_path = temp_dir.path().join("purchases.sqlite");
        let db = Database::new(&db_path).await.expect("database opens");
        let repository = PurchasesRepository::new(db.pool().clone());
        let buyer = Keys::generate();
        let merchant = Keys::generate();
        let buyer_hex = buyer.public_key().to_hex();
        let mut paid_order =
            parse_and_validate_receipt(&valid_receipt("paid", &buyer, &merchant), &buyer_hex)
                .expect("paid receipt parses");
        paid_order.order_id = "paid-order".to_string();
        let mut refunded_order =
            parse_and_validate_receipt(&valid_receipt("refunded", &buyer, &merchant), &buyer_hex)
                .expect("refunded receipt parses");
        refunded_order.order_id = "refunded-order".to_string();
        refunded_order.created_at = paid_order.created_at + 1;

        // Act
        repository
            .upsert_receipt(&paid_order)
            .await
            .expect("paid receipt is stored");
        repository
            .upsert_receipt(&refunded_order)
            .await
            .expect("refunded receipt is stored");
        let is_owned = repository
            .is_owned(&buyer_hex, &listing_coordinate(&merchant))
            .await
            .expect("ownership query succeeds");

        // Assert
        assert!(is_owned);
    }
}
