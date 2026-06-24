//! NIP-102 purchase receipt persistence and verification.

use lightning_invoice::Bolt11Invoice;
use nostr_sdk::Event;
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
    let kind = event.kind.as_u16();
    if kind != KIND_PURCHASE_RECEIPT {
        return Err(PurchaseError::WrongKind(kind));
    }

    let buyer_pubkey = required_tag_value(event, "p", "p tag")?;
    if buyer_pubkey != buyer_pubkey_hex {
        return Err(PurchaseError::BuyerMismatch);
    }

    let listing_coordinate = required_tag_value(event, "a", "a tag")?;
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

fn validate_payment_proof(event: &Event) -> Result<Option<String>, PurchaseError> {
    if tag_value(event, "e").is_some() {
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
    use super::{parse_and_validate_receipt, PurchaseError, PurchasesRepository};
    use crate::storage::Database;
    use bitcoin::hashes::{sha256, Hash as _};
    use bitcoin::secp256k1::{Secp256k1, SecretKey};
    use lightning_invoice::{Currency, InvoiceBuilder};
    use lightning_types::payment::PaymentSecret;
    use nostr_sdk::{Event, EventBuilder, Keys, Kind, Tag, TagKind};
    use std::time::Duration;
    use tempfile::TempDir;

    const LISTING_COORDINATE: &str =
        "30402:0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef:game";
    const ZAP_EVENT_ID: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";

    fn receipt_event(kind: Kind, merchant: &Keys, extra_tags: Vec<Tag>) -> Event {
        EventBuilder::new(kind, "")
            .tags(extra_tags)
            .sign_with_keys(merchant)
            .expect("receipt event signs")
    }

    fn valid_tags(buyer_hex: &str) -> Vec<Tag> {
        vec![
            Tag::custom(TagKind::p(), [buyer_hex]),
            Tag::custom(TagKind::a(), [LISTING_COORDINATE]),
            Tag::custom(TagKind::custom("order"), ["order-1"]),
            Tag::custom(TagKind::e(), [ZAP_EVENT_ID]),
        ]
    }

    fn valid_receipt(status: &str, buyer: &Keys, merchant: &Keys) -> Event {
        let buyer_hex = buyer.public_key().to_hex();
        let mut tags = valid_tags(&buyer_hex);
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

    fn bolt11_receipt(buyer_hex: &str, bolt11: &str, preimage_hex: &str) -> Vec<Tag> {
        valid_tags(buyer_hex)
            .into_iter()
            .filter(|tag| tag.kind() != TagKind::e())
            .chain([
                Tag::custom(TagKind::custom("bolt11"), [bolt11]),
                Tag::custom(TagKind::custom("preimage"), [preimage_hex]),
            ])
            .collect()
    }

    #[test]
    fn rejects_wrong_kind() {
        // Arrange
        let buyer = Keys::generate();
        let merchant = Keys::generate();
        let buyer_hex = buyer.public_key().to_hex();
        let event = receipt_event(Kind::TextNote, &merchant, valid_tags(&buyer_hex));

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
                Tag::custom(TagKind::a(), [LISTING_COORDINATE]),
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
        let event = receipt_event(Kind::Custom(1020), &merchant, valid_tags(&other_buyer_hex));

        // Act
        let error = parse_and_validate_receipt(&event, &buyer_hex)
            .expect_err("buyer mismatch should be rejected");

        // Assert
        assert!(error.to_string().contains("buyer"));
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
                Tag::custom(TagKind::a(), [LISTING_COORDINATE]),
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
        let tags = bolt11_receipt(&buyer_hex, &bolt11, "not-hex");
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
            bolt11_receipt(&buyer_hex, &bolt11, &preimage_hex),
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
            bolt11_receipt(&buyer_hex, &bolt11, &mismatched_preimage),
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
        let tags = valid_tags(&buyer_hex)
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
        assert_eq!(receipt.listing_coordinate, LISTING_COORDINATE);
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
            .is_owned(&buyer_hex, LISTING_COORDINATE)
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
            .is_owned(&buyer_hex, LISTING_COORDINATE)
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
            .is_owned(&buyer_hex, LISTING_COORDINATE)
            .await
            .expect("ownership query succeeds");

        // Assert
        assert!(is_owned);
    }
}
