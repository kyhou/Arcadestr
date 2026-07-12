//! NIP-102 purchase receipt persistence and verification.

use lightning_invoice::Bolt11Invoice;
use nostr_sdk::{Event, PublicKey};
use sha2::{Digest, Sha256};
use sqlx::{Pool, Row, Sqlite};
use std::collections::HashSet;
use std::str::FromStr;
use thiserror::Error;

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
            SELECT order_id, status
            FROM purchases
            WHERE buyer_pubkey = ? AND listing_coordinate = ?
            ORDER BY created_at DESC, event_id DESC
            "#,
        )
        .bind(buyer_pubkey)
        .bind(listing_coordinate)
        .fetch_all(&self.db)
        .await?;

        let mut seen_orders = HashSet::new();
        for row in rows {
            let order_id: String = row.get("order_id");
            if seen_orders.insert(order_id) {
                let status: String = row.get("status");
                if status_grants_ownership(&status) {
                    return Ok(true);
                }
            }
        }

        Ok(false)
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
    parse_and_validate_receipt_inner(event, buyer_pubkey_hex, None)
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
    parse_and_validate_receipt_inner(event, buyer_pubkey_hex, Some(listing_event))
}

fn parse_and_validate_receipt_inner(
    event: &Event,
    buyer_pubkey_hex: &str,
    listing_event: Option<&Event>,
) -> Result<StoredReceipt, PurchaseError> {
    event.verify().map_err(|error| {
        PurchaseError::ProofInvalid(format!("invalid event signature: {error}"))
    })?;

    let kind = event.kind.as_u16();
    if kind != KIND_PURCHASE_RECEIPT {
        return Err(PurchaseError::WrongKind(kind));
    }

    let buyer_pubkey = required_tag_value(event, "p", "p tag")?;
    if buyer_pubkey != buyer_pubkey_hex {
        return Err(PurchaseError::BuyerMismatch);
    }

    let listing_coordinate = required_tag_value(event, "a", "a tag")?;
    match listing_event {
        Some(listing) => validate_adp_receipt_signer(&listing_coordinate, event, listing)?,
        None => validate_listing_coordinate_merchant(&listing_coordinate, &event.pubkey.to_hex())?,
    }

    let order_id = required_tag_value(event, "order", "order tag")?;
    let payment_hash = validate_payment_proof(event)?;
    let status = tag_value(event, "status").unwrap_or_else(|| STATUS_PAID.to_string());
    let raw_event = serde_json::to_string(event)?;

    Ok(StoredReceipt {
        event_id: event.id.to_string(),
        order_id,
        listing_coordinate,
        buyer_pubkey,
        merchant_pubkey: event.pubkey.to_hex(),
        payment_hash,
        status,
        created_at: event.created_at.as_secs(),
        raw_event,
    })
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct FulfillmentDelegation {
    pubkey: PublicKey,
    valid_from: u64,
    revoked_at: Option<u64>,
}

impl FulfillmentDelegation {
    fn authorizes_at(&self, at: u64, listing_created_at: u64) -> bool {
        if self.valid_from > at {
            return false;
        }
        match self.revoked_at {
            Some(revoked_at) => revoked_at.max(listing_created_at) > at,
            None => true,
        }
    }
}

fn validate_adp_receipt_signer(
    listing_coordinate: &str,
    receipt: &Event,
    listing: &Event,
) -> Result<(), PurchaseError> {
    let expected_coordinate = listing_coordinate_from_listing(listing)?;
    if expected_coordinate != listing_coordinate {
        return Err(PurchaseError::ProofInvalid(
            "receipt listing coordinate does not match listing event".to_string(),
        ));
    }

    if receipt.pubkey == listing.pubkey {
        return Ok(());
    }

    let receipt_created_at = receipt.created_at.as_secs();
    let listing_created_at = listing.created_at.as_secs();
    let mut revocation_hit: Option<(u64, u64)> = None;

    for delegation in fulfillment_delegations(listing)? {
        if delegation.pubkey != receipt.pubkey {
            continue;
        }
        if delegation.authorizes_at(receipt_created_at, listing_created_at) {
            return Ok(());
        }
        if delegation.valid_from > receipt_created_at {
            continue;
        }
        if let Some(revoked_at) = delegation.revoked_at {
            let effective_revoked_at = revoked_at.max(listing_created_at);
            if effective_revoked_at <= receipt_created_at {
                revocation_hit = Some((effective_revoked_at, receipt_created_at));
            }
        }
    }

    if let Some((revoked_at, created_at)) = revocation_hit {
        return Err(PurchaseError::ProofInvalid(format!(
            "fulfillment key was revoked at {revoked_at}, receipt created_at {created_at} is at or after revocation"
        )));
    }

    Err(PurchaseError::ProofInvalid(
        "receipt signer is not authorized by listing fulfillment_pubkey tags".to_string(),
    ))
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

fn fulfillment_delegations(listing: &Event) -> Result<Vec<FulfillmentDelegation>, PurchaseError> {
    listing
        .tags
        .iter()
        .filter_map(|tag| {
            let values = tag.clone().to_vec();
            (values.first().map(String::as_str) == Some("fulfillment_pubkey")).then_some(values)
        })
        .map(|values| {
            let pubkey_hex = values.get(1).ok_or(PurchaseError::MissingTag(
                "fulfillment_pubkey.pubkey",
            ))?;
            let valid_from = values
                .get(2)
                .ok_or(PurchaseError::MissingTag("fulfillment_pubkey.valid_from"))?
                .parse::<u64>()
                .map_err(|_| PurchaseError::MissingTag("fulfillment_pubkey.valid_from"))?;
            let pubkey = PublicKey::from_hex(pubkey_hex)
                .map_err(|_| PurchaseError::MissingTag("fulfillment_pubkey.pubkey"))?;
            let revoked_at = values
                .get(3)
                .filter(|value| !value.is_empty())
                .map(|value| {
                    value
                        .parse::<u64>()
                        .map_err(|_| PurchaseError::MissingTag("fulfillment_pubkey.revoked_at"))
                })
                .transpose()?;
            Ok(FulfillmentDelegation {
                pubkey,
                valid_from,
                revoked_at,
            })
        })
        .collect()
}

fn validate_listing_coordinate_merchant(
    listing_coordinate: &str,
    merchant_pubkey_hex: &str,
) -> Result<(), PurchaseError> {
    let mut parts = listing_coordinate.split(':');
    let Some(kind) = parts.next() else {
        return Err(PurchaseError::ProofInvalid(
            "malformed listing coordinate: missing kind".to_string(),
        ));
    };
    let Some(expected_merchant) = parts.next() else {
        return Err(PurchaseError::ProofInvalid(
            "malformed listing coordinate: missing merchant pubkey".to_string(),
        ));
    };
    let Some(d_tag) = parts.next() else {
        return Err(PurchaseError::ProofInvalid(
            "malformed listing coordinate: missing d tag".to_string(),
        ));
    };

    if kind != "30402" || expected_merchant.is_empty() || d_tag.is_empty() || parts.next().is_some()
    {
        return Err(PurchaseError::ProofInvalid(
            "malformed listing coordinate".to_string(),
        ));
    }

    if expected_merchant != merchant_pubkey_hex {
        return Err(PurchaseError::ProofInvalid(
            "listing coordinate merchant does not match receipt signer".to_string(),
        ));
    }

    Ok(())
}

fn validate_payment_proof(event: &Event) -> Result<Option<String>, PurchaseError> {
    if tag_value(event, "e").is_some() {
        // Optimistically accept zap receipt references for now; relay-level
        // verification of the referenced zap receipt is deferred.
        return Ok(None);
    }

    let Some(bolt11) = tag_value(event, "bolt11") else {
        return Err(PurchaseError::MissingPaymentProof);
    };
    let Some(preimage_hex) = tag_value(event, "preimage") else {
        return Err(PurchaseError::MissingPaymentProof);
    };

    let preimage = hex::decode(preimage_hex)
        .map_err(|error| PurchaseError::ProofInvalid(format!("invalid preimage hex: {error}")))?;
    let invoice = Bolt11Invoice::from_str(&bolt11)
        .map_err(|error| PurchaseError::ProofInvalid(format!("invalid bolt11 invoice: {error}")))?;
    let digest: [u8; 32] = Sha256::digest(&preimage).into();
    let digest_hex = hex::encode(digest);
    let payment_hash_hex = invoice.payment_hash().to_string();

    if digest_hex != payment_hash_hex {
        return Err(PurchaseError::ProofInvalid(
            "preimage hash does not match bolt11 payment hash".to_string(),
        ));
    }

    Ok(Some(payment_hash_hex))
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

#[cfg(test)]
mod tests {
    use super::{
        parse_and_validate_receipt, parse_and_validate_receipt_with_listing, PurchaseError,
        PurchasesRepository,
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
        let revoked_at = revoked_at.map(|value| value.to_string()).unwrap_or_default();
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
