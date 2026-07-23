use thiserror::Error;

use serde::{Deserialize, Serialize};
use sqlx::Row;

use crate::entitlements::GrantStatus;
use crate::entitlements_repository::{EntitlementsRepository, EntitlementsRepositoryError};
use crate::purchases::{
    stored_receipt_amount, stored_receipt_validation_error, PurchaseError, PurchasesRepository,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OwnershipSource {
    None,
    PurchaseReceipt,
    EntitlementGrant,
}

#[derive(Debug, Error)]
pub enum OwnershipError {
    #[error(transparent)]
    Purchase(#[from] PurchaseError),
    #[error(transparent)]
    Entitlement(#[from] EntitlementsRepositoryError),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DurableAcquisitionKind {
    Purchase,
    PromotionClaim,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DurableCredentialStatus {
    Active,
    Disputed,
    Refunded,
    Revoked,
    Unverified,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DurableAcquisitionRecord {
    pub record_type: DurableAcquisitionKind,
    pub game_coordinate: String,
    pub listing_title: Option<String>,
    pub amount: Option<u64>,
    pub currency: Option<String>,
    pub acquired_at: u64,
    pub status: DurableCredentialStatus,
    pub record_id: String,
    pub validation_error: Option<String>,
    pub campaign_id: Option<String>,
}

pub struct OwnershipService {
    purchases: PurchasesRepository,
    entitlements: EntitlementsRepository,
}

impl OwnershipService {
    pub fn new(purchases: PurchasesRepository, entitlements: EntitlementsRepository) -> Self {
        Self {
            purchases,
            entitlements,
        }
    }

    pub async fn source_for(
        &self,
        buyer_pubkey: &str,
        game_coordinate: &str,
    ) -> Result<OwnershipSource, OwnershipError> {
        if self
            .entitlements
            .is_owned(buyer_pubkey, game_coordinate)
            .await?
        {
            return Ok(OwnershipSource::EntitlementGrant);
        }
        if self
            .purchases
            .is_owned(buyer_pubkey, game_coordinate)
            .await?
        {
            return Ok(OwnershipSource::PurchaseReceipt);
        }
        Ok(OwnershipSource::None)
    }

    pub async fn is_owned(
        &self,
        buyer_pubkey: &str,
        game_coordinate: &str,
    ) -> Result<bool, OwnershipError> {
        Ok(self.source_for(buyer_pubkey, game_coordinate).await? != OwnershipSource::None)
    }

    pub async fn durable_records_for(
        &self,
        buyer_pubkey: &str,
    ) -> Result<Vec<DurableAcquisitionRecord>, OwnershipError> {
        let receipts = self.purchases.list_for_buyer(buyer_pubkey).await?;
        let mut purchase_chains = std::collections::HashMap::new();
        for receipt in receipts {
            purchase_chains
                .entry((receipt.listing_coordinate.clone(), receipt.order_id.clone()))
                .or_insert_with(Vec::new)
                .push(receipt);
        }
        let mut records = Vec::new();

        for mut chain in purchase_chains.into_values() {
            chain.sort_by(|left, right| {
                right
                    .created_at
                    .cmp(&left.created_at)
                    .then_with(|| right.event_id.cmp(&left.event_id))
            });
            let Some(receipt) = chain.first() else {
                continue;
            };
            let mut validation_error = chain
                .iter()
                .find_map(stored_receipt_validation_error)
                .map(str::to_string);
            if matches!(receipt.status.as_str(), "refunded" | "refund" | "disputed")
                && !chain
                    .iter()
                    .skip(1)
                    .any(|prior| matches!(prior.status.as_str(), "paid" | "fulfilled"))
            {
                validation_error = Some("Stored purchase status chain is incomplete.".to_string());
            }
            let status = if validation_error.is_some() {
                DurableCredentialStatus::Unverified
            } else {
                purchase_status(&receipt.status)
            };
            let (amount, currency) = chain
                .iter()
                .find_map(stored_receipt_amount)
                .map(|(amount, currency)| (Some(amount), Some(currency.to_string())))
                .unwrap_or((None, None));
            let acquired_at = chain
                .iter()
                .filter(|event| matches!(event.status.as_str(), "paid" | "fulfilled"))
                .map(|event| event.created_at)
                .min()
                .unwrap_or(receipt.created_at);
            records.push(DurableAcquisitionRecord {
                record_type: DurableAcquisitionKind::Purchase,
                game_coordinate: receipt.listing_coordinate.clone(),
                listing_title: None,
                amount,
                currency,
                acquired_at,
                status,
                record_id: receipt.event_id.clone(),
                validation_error,
                campaign_id: None,
            });
        }

        for grant in self.entitlements.history_for_buyer(buyer_pubkey).await? {
            let status = match grant.status {
                Some(GrantStatus::Granted) if grant.validation_error.is_none() => {
                    DurableCredentialStatus::Active
                }
                Some(GrantStatus::Revoked) if grant.validation_error.is_none() => {
                    DurableCredentialStatus::Revoked
                }
                _ => DurableCredentialStatus::Unverified,
            };
            records.push(DurableAcquisitionRecord {
                record_type: DurableAcquisitionKind::PromotionClaim,
                game_coordinate: grant.game_coordinate,
                listing_title: None,
                amount: None,
                currency: None,
                acquired_at: grant.acquired_at,
                status,
                record_id: grant.grant_id,
                validation_error: grant.validation_error,
                campaign_id: grant.campaign_id,
            });
        }

        for record in &mut records {
            record.listing_title =
                listing_title(self.purchases.database(), &record.game_coordinate).await?;
        }
        records.sort_by(|left, right| {
            right
                .acquired_at
                .cmp(&left.acquired_at)
                .then_with(|| {
                    acquisition_kind_order(left.record_type)
                        .cmp(&acquisition_kind_order(right.record_type))
                })
                .then_with(|| left.record_id.cmp(&right.record_id))
        });
        Ok(records)
    }
}

fn purchase_status(status: &str) -> DurableCredentialStatus {
    match status {
        "paid" | "fulfilled" => DurableCredentialStatus::Active,
        "disputed" => DurableCredentialStatus::Disputed,
        "refunded" | "refund" => DurableCredentialStatus::Refunded,
        _ => DurableCredentialStatus::Unverified,
    }
}

fn acquisition_kind_order(kind: DurableAcquisitionKind) -> u8 {
    match kind {
        DurableAcquisitionKind::Purchase => 0,
        DurableAcquisitionKind::PromotionClaim => 1,
    }
}

async fn listing_title(
    db: &sqlx::SqlitePool,
    coordinate: &str,
) -> Result<Option<String>, PurchaseError> {
    let mut parts = coordinate.split(':');
    let (Some("30402"), Some(publisher), Some(product_id), None) =
        (parts.next(), parts.next(), parts.next(), parts.next())
    else {
        return Ok(None);
    };
    let rows = sqlx::query(
        "SELECT publisher_npub, title FROM marketplace_listings WHERE product_id = ? ORDER BY updated_at DESC",
    )
    .bind(product_id)
    .fetch_all(db)
    .await?;

    Ok(rows.into_iter().find_map(|row| {
        let stored_publisher: String = row.get("publisher_npub");
        let matches = stored_publisher == publisher
            || nostr::PublicKey::parse(&stored_publisher)
                .map(|key| key.to_hex() == publisher)
                .unwrap_or(false);
        matches.then(|| row.get("title"))
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::adp_protocol::ENTITLEMENT_GRANT_KIND;
    use crate::purchases::{parse_and_validate_receipt, StoredReceipt};
    use crate::storage::Database;
    use bitcoin::hashes::{sha256, Hash as _};
    use bitcoin::secp256k1::{Secp256k1, SecretKey};
    use lightning_invoice::{Currency, InvoiceBuilder};
    use lightning_types::payment::PaymentSecret;
    use nostr::{Event, EventBuilder, EventId, Keys, Kind, Tag, TagKind, Timestamp};
    use std::time::Duration;
    use tempfile::TempDir;

    const SOURCE_ID: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";

    async fn setup(name: &str) -> (TempDir, Database, OwnershipService, PurchasesRepository) {
        let temp = TempDir::new().expect("temp directory should be created");
        let database = Database::new(&temp.path().join(format!("{name}.sqlite")))
            .await
            .expect("test database should open");
        let service = OwnershipService::new(
            PurchasesRepository::new(database.pool().clone()),
            EntitlementsRepository::new(database.pool().clone()),
        );
        let purchases = PurchasesRepository::new(database.pool().clone());
        (temp, database, service, purchases)
    }

    fn coordinate(merchant: &Keys, id: &str) -> String {
        format!("30402:{}:{id}", merchant.public_key().to_hex())
    }

    fn receipt(
        buyer: &Keys,
        merchant: &Keys,
        id: &str,
        order: &str,
        status: &str,
        created_at: u64,
    ) -> StoredReceipt {
        let preimage = [created_at as u8; 32];
        let payment_hash = sha256::Hash::hash(&preimage);
        let invoice_key = SecretKey::from_slice(&[42_u8; 32]).expect("invoice key should parse");
        let invoice = InvoiceBuilder::new(Currency::Bitcoin)
            .description("Arcadestr history test".to_string())
            .payment_hash(payment_hash)
            .payment_secret(PaymentSecret([11_u8; 32]))
            .amount_milli_satoshis(21_000)
            .duration_since_epoch(Duration::from_secs(1_700_000_000))
            .min_final_cltv_expiry_delta(144)
            .build_signed(|hash| Secp256k1::new().sign_ecdsa_recoverable(hash, &invoice_key))
            .expect("invoice should build");
        let event = EventBuilder::new(Kind::Custom(1020), "")
            .tags([
                Tag::custom(TagKind::p(), [buyer.public_key().to_hex()]),
                Tag::custom(TagKind::a(), [coordinate(merchant, id)]),
                Tag::custom(TagKind::custom("order"), [order]),
                Tag::custom(TagKind::custom("payment_hash"), [payment_hash.to_string()]),
                Tag::custom(TagKind::custom("amount_msat"), ["21000"]),
                Tag::custom(TagKind::custom("settled_at"), [created_at.to_string()]),
                Tag::custom(TagKind::custom("proof"), ["bolt11-preimage"]),
                Tag::custom(TagKind::custom("bolt11"), [invoice.to_string()]),
                Tag::custom(TagKind::custom("preimage"), [hex::encode(preimage)]),
                Tag::custom(TagKind::custom("status"), [status]),
            ])
            .custom_created_at(Timestamp::from(created_at))
            .sign_with_keys(merchant)
            .expect("receipt should sign");
        if status == "paid" {
            return parse_and_validate_receipt(&event, &buyer.public_key().to_hex())
                .expect("receipt should validate");
        }
        StoredReceipt {
            event_id: event.id.to_hex(),
            order_id: order.to_string(),
            listing_coordinate: coordinate(merchant, id),
            buyer_pubkey: buyer.public_key().to_hex(),
            merchant_pubkey: merchant.public_key().to_hex(),
            payment_hash: Some(payment_hash.to_string()),
            status: status.to_string(),
            created_at,
            raw_event: serde_json::to_string(&event).expect("receipt should serialize"),
        }
    }

    fn grant_event(
        recipient: &Keys,
        publisher: &Keys,
        grant_id: &str,
        game_id: &str,
        status: &str,
        predecessor: Option<EventId>,
        created_at: u64,
    ) -> Event {
        let mut tags = vec![
            Tag::custom(TagKind::d(), [grant_id]),
            Tag::custom(TagKind::p(), [recipient.public_key().to_hex()]),
            Tag::custom(TagKind::a(), [coordinate(publisher, game_id)]),
            Tag::custom(TagKind::custom("source_event"), [SOURCE_ID]),
            Tag::custom(TagKind::custom("status"), [status]),
        ];
        if let Some(predecessor) = predecessor {
            tags.push(Tag::custom(TagKind::e(), [predecessor.to_hex()]));
        }
        EventBuilder::new(Kind::Custom(ENTITLEMENT_GRANT_KIND), "")
            .tags(tags)
            .custom_created_at(Timestamp::from(created_at))
            .sign_with_keys(publisher)
            .expect("grant should sign")
    }

    async fn insert_grant(database: &Database, grant: &Event) {
        let parsed =
            crate::entitlements::parse_entitlement_event(grant).expect("grant should parse");
        sqlx::query(
            r#"
            INSERT INTO entitlement_events (
                event_id, grant_id, buyer_pubkey, game_coordinate,
                campaign_root_id, issuer_pubkey, status, predecessor_event_id,
                created_at, raw_event_json, validated
            ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, 1)
            "#,
        )
        .bind(grant.id.to_hex())
        .bind(parsed.grant_id)
        .bind(parsed.recipient.to_hex())
        .bind(parsed.coordinate)
        .bind(parsed.source_event.to_hex())
        .bind(grant.pubkey.to_hex())
        .bind(parsed.status.as_str())
        .bind(parsed.predecessor.map(|id| id.to_hex()))
        .bind(grant.created_at.as_secs() as i64)
        .bind(serde_json::to_string(grant).expect("grant should serialize"))
        .execute(database.pool())
        .await
        .expect("grant should persist");
    }

    #[tokio::test]
    async fn purchase_history_is_account_scoped_deduplicated_and_refund_aware() {
        let (_temp, _database, service, purchases) = setup("purchase-history").await;
        let buyer = Keys::generate();
        let other = Keys::generate();
        let merchant = Keys::generate();
        let paid = receipt(&buyer, &merchant, "game", "order", "paid", 10);
        let refunded = receipt(&buyer, &merchant, "game", "order", "refunded", 20);
        let separate = receipt(&buyer, &merchant, "other-game", "order", "paid", 15);
        let other_paid = receipt(&other, &merchant, "game", "other-order", "paid", 30);
        for stored in [&paid, &refunded, &refunded, &separate, &other_paid] {
            purchases
                .upsert_receipt(stored)
                .await
                .expect("receipt should persist");
        }

        let records = service
            .durable_records_for(&buyer.public_key().to_hex())
            .await
            .expect("history should load");

        assert_eq!(records.len(), 2);
        assert!(records
            .iter()
            .all(|record| { record.record_type == DurableAcquisitionKind::Purchase }));
        assert!(records.iter().any(|record| {
            record.status == DurableCredentialStatus::Refunded
                && record.record_id == refunded.event_id
        }));
        assert!(records.iter().any(|record| {
            record.status == DurableCredentialStatus::Active
                && record.record_id == separate.event_id
        }));
    }

    #[tokio::test]
    async fn entitlement_history_maps_grant_and_terminal_revocation() {
        let (_temp, database, service, _purchases) = setup("grant-history").await;
        let buyer = Keys::generate();
        let publisher = Keys::generate();
        let granted = grant_event(&buyer, &publisher, "grant", "game", "granted", None, 10);
        let revoked = grant_event(
            &buyer,
            &publisher,
            "grant",
            "game",
            "revoked",
            Some(granted.id),
            20,
        );
        let separate = grant_event(
            &buyer,
            &publisher,
            "grant",
            "other-game",
            "granted",
            None,
            15,
        );
        insert_grant(&database, &granted).await;
        insert_grant(&database, &revoked).await;
        insert_grant(&database, &separate).await;

        let records = service
            .durable_records_for(&buyer.public_key().to_hex())
            .await
            .expect("history should load");

        assert_eq!(records.len(), 2);
        assert!(records.iter().all(|record| {
            record.record_type == DurableAcquisitionKind::PromotionClaim
                && record.campaign_id.as_deref() == Some(SOURCE_ID)
        }));
        assert!(records
            .iter()
            .any(|record| record.status == DurableCredentialStatus::Revoked));
        assert!(records
            .iter()
            .any(|record| record.status == DurableCredentialStatus::Active));
    }

    #[tokio::test]
    async fn malformed_stored_receipt_is_returned_as_unverified() {
        let (_temp, database, service, _purchases) = setup("malformed-history").await;
        let buyer = Keys::generate();
        sqlx::query(
            r#"
            INSERT INTO purchases (
                event_id, order_id, listing_coordinate, buyer_pubkey,
                merchant_pubkey, status, created_at, raw_event
            ) VALUES ('bad', 'order', 'malformed', ?, 'merchant', 'paid', 1, '{}')
            "#,
        )
        .bind(buyer.public_key().to_hex())
        .execute(database.pool())
        .await
        .expect("malformed fixture should persist");

        let records = service
            .durable_records_for(&buyer.public_key().to_hex())
            .await
            .expect("history should load");

        assert_eq!(records.len(), 1);
        assert_eq!(records[0].status, DurableCredentialStatus::Unverified);
        assert!(records[0].validation_error.is_some());
    }

    #[tokio::test]
    async fn listings_public_timed_or_unclaimed_never_synthesize_records() {
        let (_temp, database, service, _purchases) = setup("listing-only-history").await;
        let buyer = Keys::generate();
        for (id, title) in [
            ("public", "Public access"),
            ("timed", "Timed access"),
            ("unclaimed", "Unclaimed promotion"),
        ] {
            sqlx::query(
                r#"
                INSERT INTO marketplace_listings (
                    publisher_npub, product_id, title, description, price_sats,
                    download_url, created_at, updated_at
                ) VALUES ('publisher', ?, ?, '', 0, '', 1, 1)
                "#,
            )
            .bind(id)
            .bind(title)
            .execute(database.pool())
            .await
            .expect("listing fixture should persist");
        }

        let records = service
            .durable_records_for(&buyer.public_key().to_hex())
            .await
            .expect("history should load");
        assert!(records.is_empty());
    }

    #[tokio::test]
    async fn records_use_cached_title_only_as_enrichment_and_sort_deterministically() {
        let (_temp, database, service, purchases) = setup("record-order").await;
        let buyer = Keys::generate();
        let merchant = Keys::generate();
        let first = receipt(&buyer, &merchant, "one", "one", "paid", 20);
        let second = receipt(&buyer, &merchant, "two", "two", "paid", 20);
        purchases
            .upsert_receipt(&first)
            .await
            .expect("receipt persists");
        purchases
            .upsert_receipt(&second)
            .await
            .expect("receipt persists");
        sqlx::query(
            r#"
            INSERT INTO marketplace_listings (
                publisher_npub, product_id, title, description, price_sats,
                download_url, created_at, updated_at
            ) VALUES (?, 'two', 'Second game', '', 0, '', 1, 1)
            "#,
        )
        .bind(merchant.public_key().to_hex())
        .execute(database.pool())
        .await
        .expect("listing fixture should persist");

        let records = service
            .durable_records_for(&buyer.public_key().to_hex())
            .await
            .expect("history should load");

        assert_eq!(records.len(), 2);
        let mut expected_ids = vec![first.event_id.clone(), second.event_id.clone()];
        expected_ids.sort();
        assert_eq!(
            records
                .iter()
                .map(|record| record.record_id.clone())
                .collect::<Vec<_>>(),
            expected_ids
        );
        assert_eq!(
            records
                .iter()
                .find(|record| record.record_id == second.event_id)
                .and_then(|record| record.listing_title.as_deref()),
            Some("Second game")
        );
    }
}
