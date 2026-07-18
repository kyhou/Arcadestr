use std::path::PathBuf;

use arcadestr_core::adp_protocol::{ADP_CAMPAIGN_KIND, ENTITLEMENT_GRANT_KIND};
use arcadestr_core::campaign::{parse_campaign_event, resolve_campaign};
use arcadestr_core::entitlements::{GrantStatus, IssuanceDelegation};
use arcadestr_core::entitlements_repository::EntitlementsRepository;
use arcadestr_core::ownership::{OwnershipService, OwnershipSource};
use arcadestr_core::purchases::PurchasesRepository;
use arcadestr_core::storage::Database;
use nostr::{Event, EventBuilder, EventId, Keys, Kind, Tag, Timestamp};

fn tag(values: &[&str]) -> Tag {
    Tag::parse(values.iter().copied()).expect("test tag must be valid")
}

fn unique_db_path(name: &str) -> PathBuf {
    std::env::temp_dir().join(format!(
        "arcadestr-{name}-{}-{}.db",
        std::process::id(),
        Timestamp::now().as_secs()
    ))
}

fn campaign(publisher: &Keys, coordinate: &str) -> arcadestr_core::campaign::ResolvedCampaign {
    let event = EventBuilder::new(Kind::Custom(ADP_CAMPAIGN_KIND), "")
        .tags([
            tag(&["d", "campaign-1"]),
            tag(&["a", coordinate]),
            tag(&["mode", "claim"]),
            tag(&["starts", "120"]),
            tag(&["ends", "300"]),
            tag(&["status", "active"]),
        ])
        .custom_created_at(Timestamp::from_secs(100))
        .sign_with_keys(publisher)
        .expect("campaign signs");
    resolve_campaign(
        &[parse_campaign_event(&event).expect("campaign parses")],
        publisher.public_key(),
        coordinate,
    )
    .expect("campaign resolves")
}

#[allow(clippy::too_many_arguments)]
fn grant(
    signer: &Keys,
    recipient: &Keys,
    coordinate: &str,
    grant_id: &str,
    source_event: EventId,
    status: GrantStatus,
    predecessor: Option<EventId>,
    created_at: u64,
) -> Event {
    let mut tags = vec![
        tag(&["d", grant_id]),
        tag(&["p", &recipient.public_key().to_hex()]),
        tag(&["a", coordinate]),
        tag(&["source_event", &source_event.to_hex()]),
        tag(&["status", status.as_str()]),
    ];
    if let Some(predecessor) = predecessor {
        tags.push(tag(&["e", &predecessor.to_hex()]));
    }
    EventBuilder::new(Kind::Custom(ENTITLEMENT_GRANT_KIND), "")
        .tags(tags)
        .custom_created_at(Timestamp::from_secs(created_at))
        .sign_with_keys(signer)
        .expect("grant signs")
}

#[tokio::test]
async fn migration_creates_entitlement_history_table() {
    let path = unique_db_path("entitlement-migration");
    let db = Database::new(&path).await.expect("database initializes");

    let table: String = sqlx::query_scalar(
        "SELECT name FROM sqlite_master WHERE type = 'table' AND name = 'entitlement_events'",
    )
    .fetch_one(db.pool())
    .await
    .expect("entitlement table exists");

    assert_eq!(table, "entitlement_events");
    db.close().await;
    let _ = std::fs::remove_file(path);
}

#[tokio::test]
async fn valid_grant_is_idempotently_persisted_and_proves_ownership() {
    let path = unique_db_path("entitlement-owned");
    let db = Database::new(&path).await.expect("database initializes");
    let publisher = Keys::generate();
    let recipient = Keys::generate();
    let coordinate = format!("30402:{}:game", publisher.public_key());
    let campaign = campaign(&publisher, &coordinate);
    let event = grant(
        &publisher,
        &recipient,
        &coordinate,
        "grant-1",
        campaign.root_event_id,
        GrantStatus::Granted,
        None,
        150,
    );
    let repository = EntitlementsRepository::new(db.pool().clone());

    repository
        .ingest_event(&event, &campaign, None, &[])
        .await
        .expect("grant ingests");
    repository
        .ingest_event(&event, &campaign, None, &[])
        .await
        .expect("duplicate is idempotent");

    assert!(repository
        .is_owned(&recipient.public_key().to_hex(), &coordinate)
        .await
        .expect("ownership query succeeds"));
    assert_eq!(
        repository
            .chains_for(&recipient.public_key().to_hex(), &coordinate)
            .await
            .expect("chains load")
            .len(),
        1
    );
    db.close().await;
    let _ = std::fs::remove_file(path);
}

#[tokio::test]
async fn publisher_revocation_removes_grant_ownership() {
    let path = unique_db_path("entitlement-revoked");
    let db = Database::new(&path).await.expect("database initializes");
    let publisher = Keys::generate();
    let recipient = Keys::generate();
    let coordinate = format!("30402:{}:game", publisher.public_key());
    let campaign = campaign(&publisher, &coordinate);
    let root = grant(
        &publisher,
        &recipient,
        &coordinate,
        "grant-1",
        campaign.root_event_id,
        GrantStatus::Granted,
        None,
        150,
    );
    let revoke = grant(
        &publisher,
        &recipient,
        &coordinate,
        "grant-1",
        campaign.root_event_id,
        GrantStatus::Revoked,
        Some(root.id),
        160,
    );
    let repository = EntitlementsRepository::new(db.pool().clone());
    repository
        .ingest_event(&root, &campaign, None, &[])
        .await
        .expect("root ingests");
    repository
        .ingest_event(&revoke, &campaign, None, &[])
        .await
        .expect("revocation ingests");

    assert!(!repository
        .is_owned(&recipient.public_key().to_hex(), &coordinate)
        .await
        .expect("ownership query succeeds"));
    db.close().await;
    let _ = std::fs::remove_file(path);
}

#[tokio::test]
async fn malformed_unrelated_history_does_not_hide_valid_grant() {
    let path = unique_db_path("entitlement-malformed");
    let db = Database::new(&path).await.expect("database initializes");
    let publisher = Keys::generate();
    let recipient = Keys::generate();
    let coordinate = format!("30402:{}:game", publisher.public_key());
    let campaign = campaign(&publisher, &coordinate);
    let event = grant(
        &publisher,
        &recipient,
        &coordinate,
        "valid-grant",
        campaign.root_event_id,
        GrantStatus::Granted,
        None,
        150,
    );
    let repository = EntitlementsRepository::new(db.pool().clone());
    repository
        .ingest_event(&event, &campaign, None, &[])
        .await
        .expect("valid grant ingests");
    sqlx::query(
        "INSERT INTO entitlement_events
         (event_id, grant_id, buyer_pubkey, game_coordinate, campaign_root_id,
          issuer_pubkey, status, predecessor_event_id, created_at, raw_event_json)
         VALUES (?, ?, ?, ?, ?, ?, ?, NULL, ?, ?)",
    )
    .bind("malformed-event")
    .bind("broken-grant")
    .bind(recipient.public_key().to_hex())
    .bind(&coordinate)
    .bind(campaign.root_event_id.to_hex())
    .bind(publisher.public_key().to_hex())
    .bind("granted")
    .bind(151_i64)
    .bind("not-json")
    .execute(db.pool())
    .await
    .expect("fixture inserts");

    assert!(repository
        .is_owned(&recipient.public_key().to_hex(), &coordinate)
        .await
        .expect("valid chain still grants ownership"));
    db.close().await;
    let _ = std::fs::remove_file(path);
}

#[tokio::test]
async fn shared_ownership_service_reports_entitlement_source() {
    let path = unique_db_path("ownership-service");
    let db = Database::new(&path).await.expect("database initializes");
    let publisher = Keys::generate();
    let recipient = Keys::generate();
    let coordinate = format!("30402:{}:game", publisher.public_key());
    let campaign = campaign(&publisher, &coordinate);
    let event = grant(
        &publisher,
        &recipient,
        &coordinate,
        "grant-1",
        campaign.root_event_id,
        GrantStatus::Granted,
        None,
        150,
    );
    let entitlements = EntitlementsRepository::new(db.pool().clone());
    entitlements
        .ingest_event(&event, &campaign, None, &[] as &[IssuanceDelegation])
        .await
        .expect("grant ingests");
    let ownership =
        OwnershipService::new(PurchasesRepository::new(db.pool().clone()), entitlements);

    assert_eq!(
        ownership
            .source_for(&recipient.public_key().to_hex(), &coordinate)
            .await
            .expect("ownership resolves"),
        OwnershipSource::EntitlementGrant
    );
    db.close().await;
    let _ = std::fs::remove_file(path);
}
