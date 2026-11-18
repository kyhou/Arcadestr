#![cfg(feature = "native")]
use arcadestr_core::{
    marketplace::*,
    marketplace_cache::{MarketplaceCache, UpsertOutcome},
    nostr::GameListing,
    storage::Database,
};
use nostr::{Event, EventBuilder, Keys, Kind, Tag, Timestamp};
use serde_json::json;

fn event(keys: &Keys, at: u64, roots: &[String]) -> Event {
    EventBuilder::new(Kind::Custom(30402), "metadata")
        .tags([
            Tag::parse(["d", "game"]).expect("d"),
            Tag::parse(["title", "Projection fixture"]).expect("title"),
            Tag::parse(["price", "0", "SATS"]).expect("price"),
            Tag::parse(["acquisition", "public"]).expect("acquisition"),
        ])
        .tags(roots.iter().map(|root| {
            Tag::parse(["campaign", root, "wss://verified.example.test"]).expect("campaign")
        }))
        .custom_created_at(Timestamp::from_secs(at))
        .sign_with_keys(keys)
        .expect("signature")
}

#[tokio::test]
async fn campaign_cache_round_trip_survives_all_query_shapes_and_database_restart() {
    let temp = tempfile::tempdir().expect("temp");
    let path = temp.path().join("projection.db");
    let db = Database::new(&path).await.expect("db");
    let now = Timestamp::now().as_secs();
    let signed = event(&Keys::generate(), now, &["11".repeat(32), "22".repeat(32)]);
    let listing = GameListing::from_listing(parse_listing(signed.clone()).expect("listing"));
    let json = serde_json::to_string(&listing).expect("serialize");
    let decoded: GameListing = serde_json::from_str(&json).expect("restore JSON");
    assert_eq!(listing.campaigns, decoded.campaigns);
    let cache = MarketplaceCache::new(db.pool().clone());
    cache
        .upsert_listing(&decoded, Some(&signed.id.to_hex()))
        .await
        .expect("cache");
    for (since, until) in [
        (None, None),
        (Some(1), None),
        (None, Some(now + 1)),
        (Some(1), Some(now + 1)),
    ] {
        let restored = cache.load_listings(10, since, until).await.expect("load");
        assert_eq!(restored[0].campaigns, listing.campaigns);
    }
    drop(cache);
    db.pool().close().await;
    let db = Database::new(&path).await.expect("restart");
    let cache = MarketplaceCache::new(db.pool().clone());
    let restored = cache.load_listings(10, None, None).await.expect("restore");
    assert_eq!(restored[0].event_id, Some(signed.id.to_hex()));
    assert_eq!(restored[0].campaigns, listing.campaigns);
}

#[tokio::test]
async fn malformed_cache_pointers_are_bounded_to_advisory_discovery_and_same_revision_can_repair_projection(
) {
    let temp = tempfile::tempdir().expect("temp");
    let db = Database::new(&temp.path().join("cache.db"))
        .await
        .expect("db");
    let cache = MarketplaceCache::new(db.pool().clone());
    let signed = event(&Keys::generate(), 100, &["11".repeat(32)]);
    let listing = GameListing::from_listing(parse_listing(signed.clone()).expect("listing"));
    cache
        .upsert_listing(&listing, Some(&signed.id.to_hex()))
        .await
        .expect("cache");
    let malformed = json!([{"root_event_id":"malformed","relay_hint":null},{"root_event_id":"22".repeat(32),"relay_hint":"not a relay"},{"root_event_id":"11".repeat(32),"relay_hint":null,"authorization":"forged cache data"},{"root_event_id":"11".repeat(32),"relay_hint":null}]);
    sqlx::query("UPDATE marketplace_listings SET campaigns_json=?")
        .bind(malformed.to_string())
        .execute(db.pool())
        .await
        .expect("simulate corrupt projection");
    let restored = cache
        .load_listings(10, None, None)
        .await
        .expect("listing still restores");
    assert_eq!(restored[0].campaigns.len(), 1);
    assert_eq!(
        restored[0].campaigns[0].root_event_id.to_hex(),
        "11".repeat(32)
    );
    assert!(
        restored[0]
            .specs
            .iter()
            .all(|(key, _)| key != "fulfillment_authorization"),
        "campaign hints never become fulfillment evidence"
    );
    assert_eq!(
        cache
            .upsert_listing(&listing, Some(&signed.id.to_hex()))
            .await
            .expect("same verified revision repairs projection"),
        UpsertOutcome::Updated
    );
    assert_eq!(
        cache.load_listings(10, None, None).await.expect("restored")[0].campaigns,
        listing.campaigns
    );
    sqlx::query("UPDATE marketplace_listings SET campaigns_json='not JSON'")
        .execute(db.pool())
        .await
        .expect("malformed JSON");
    assert!(cache
        .load_listings(10, None, None)
        .await
        .expect("safe restore")[0]
        .campaigns
        .is_empty());
}

#[test]
fn metadata_merge_preserves_verified_roots_and_hints_despite_stale_or_poisoned_cache() {
    let signed = event(&Keys::generate(), 100, &["11".repeat(32), "22".repeat(32)]);
    let merged = merge_campaign_references(
        Some(&signed),
        &[
            ("11".repeat(32), Some("wss://cached.example.test".into())),
            ("33".repeat(32), None),
        ],
    )
    .expect("merge");
    assert_eq!(merged.len(), 3);
    assert_eq!(merged[0].1.as_deref(), Some("wss://verified.example.test"));
    assert_eq!(merged[1].0, "22".repeat(32));
    assert_eq!(
        merge_campaign_references(Some(&signed), &[])
            .expect("cache omitted pointers")
            .len(),
        2
    );
    assert!(merge_campaign_references(Some(&signed), &[("not-hex".into(), None)]).is_err());
    assert!(merge_campaign_references(
        Some(&signed),
        &[("33".repeat(32), Some("https://not-a-relay.test".into()))]
    )
    .is_err());
    let mut forged = signed.clone();
    forged.content = "altered cached authority".into();
    assert!(merge_campaign_references(Some(&forged), &[]).is_err());
}

#[tokio::test]
async fn stale_cache_revision_cannot_remove_newer_campaign_pointers() {
    let temp = tempfile::tempdir().expect("temp");
    let db = Database::new(&temp.path().join("ordering.db"))
        .await
        .expect("db");
    let cache = MarketplaceCache::new(db.pool().clone());
    let keys = Keys::generate();
    let newer = event(&keys, 200, &["11".repeat(32)]);
    let older = event(&keys, 100, &[]);
    cache
        .upsert_listing(
            &GameListing::from_listing(parse_listing(newer.clone()).expect("newer")),
            Some(&newer.id.to_hex()),
        )
        .await
        .expect("newer");
    assert_eq!(
        cache
            .upsert_listing(
                &GameListing::from_listing(parse_listing(older.clone()).expect("older")),
                Some(&older.id.to_hex())
            )
            .await
            .expect("ignore stale"),
        UpsertOutcome::Unchanged
    );
    assert_eq!(
        cache.load_listings(10, None, None).await.expect("load")[0]
            .campaigns
            .len(),
        1
    );
}

#[tokio::test]
async fn additive_projection_migration_preserves_existing_rows_and_allows_verified_refresh() {
    let temp = tempfile::tempdir().expect("temp");
    let path = temp.path().join("upgrade.db");
    let db = Database::new(&path).await.expect("db");
    let signed = event(&Keys::generate(), 100, &["11".repeat(32)]);
    let listing = GameListing::from_listing(parse_listing(signed.clone()).expect("listing"));
    MarketplaceCache::new(db.pool().clone())
        .upsert_listing(&listing, Some(&signed.id.to_hex()))
        .await
        .expect("old row");
    // Recreate a complete version-24 database in this isolated DB: roll back
    // every later migration's schema and identity row, so the unchanged
    // identity validation sees a consistent pre-25 ledger.
    for rollback in [
        "DROP TABLE profile_metadata_events",
        "DROP TABLE quote_initiations",
        "ALTER TABLE marketplace_listings DROP COLUMN campaigns_json",
    ] {
        sqlx::query(rollback)
            .execute(db.pool())
            .await
            .expect("pre-25 schema");
    }
    sqlx::query("DELETE FROM schema_migrations WHERE version > 24")
        .execute(db.pool())
        .await
        .expect("pre-25 ledger");
    let (latest,): (i64,) = sqlx::query_as("SELECT COUNT(*) FROM schema_migrations")
        .fetch_one(db.pool())
        .await
        .expect("ledger count");
    assert_eq!(latest, 24, "fixture must roll back every later migration");
    sqlx::query("PRAGMA user_version=24")
        .execute(db.pool())
        .await
        .expect("pre-25 version");
    db.pool().close().await;
    let db = Database::new(&path).await.expect("additive upgrade");
    let cache = MarketplaceCache::new(db.pool().clone());
    let loaded = cache
        .load_listings(10, None, None)
        .await
        .expect("old row retained");
    assert_eq!(loaded[0].title, listing.title);
    assert_eq!(loaded[0].event_id, Some(signed.id.to_hex()));
    assert!(
        loaded[0].campaigns.is_empty(),
        "do not invent missing historical pointers"
    );
    cache
        .upsert_listing(&listing, Some(&signed.id.to_hex()))
        .await
        .expect("same signed revision refresh");
    assert_eq!(
        cache
            .load_listings(10, None, None)
            .await
            .expect("backfilled projection")[0]
            .campaigns,
        listing.campaigns
    );
}

#[tokio::test]
async fn same_signed_revision_backfills_a_cached_row_missing_its_signed_lud16() {
    let temp = tempfile::tempdir().expect("temp");
    let db = Database::new(&temp.path().join("lud16.db"))
        .await
        .expect("db");
    let signed = EventBuilder::new(Kind::Custom(30402), "metadata")
        .tags([
            Tag::parse(["d", "paid"]).expect("d"),
            Tag::parse(["title", "Paid fixture"]).expect("title"),
            Tag::parse(["price", "2000", "sat"]).expect("price"),
            Tag::parse(["lud16", "dev@example.com"]).expect("lud16"),
        ])
        .custom_created_at(Timestamp::from_secs(100))
        .sign_with_keys(&Keys::generate())
        .expect("signature");
    let current = GameListing::from_listing(parse_listing(signed.clone()).expect("listing"));
    assert_eq!(current.lud16, "dev@example.com");
    // Projection produced before the signed tag was parsed.
    let mut legacy = current.clone();
    legacy.lud16.clear();
    let cache = MarketplaceCache::new(db.pool().clone());
    cache
        .upsert_listing(&legacy, Some(&signed.id.to_hex()))
        .await
        .expect("legacy row");
    cache
        .upsert_listing(&current, Some(&signed.id.to_hex()))
        .await
        .expect("same signed revision refresh");
    let loaded = cache.load_listings(10, None, None).await.expect("load");
    assert_eq!(loaded[0].lud16, "dev@example.com");
}
