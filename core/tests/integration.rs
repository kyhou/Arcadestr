#![cfg(feature = "native")]

use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::Arc;

use arcadestr_core::auth::AuthState;
use arcadestr_core::marketplace::Nip99Listing;
use arcadestr_core::marketplace_cache::MarketplaceCache;
use arcadestr_core::nip05_validator::{IdentityValidationState, Nip05IdentityValidator};
use arcadestr_core::nostr::{
    event_to_game_listing, game_listing_to_event_builder, GameListing, ListingSource,
    KIND_GAME_LISTING,
};
use arcadestr_core::signers::NostrSigner;
use arcadestr_core::storage::Database;
use nostr::{Event, EventBuilder, Keys, Kind, PublicKey, ToBech32};
use serde_json::json;

mod auth {
    pub use arcadestr_core::auth::*;
}

mod http_client {
    pub use arcadestr_core::http_client::*;
}

mod signers {
    pub use arcadestr_core::signers::*;
}

#[path = "../src/test_helpers/http_mocks.rs"]
pub mod http_mocks;

mod test_helpers {
    pub use super::http_mocks;
}

#[path = "../src/test_helpers/nip46_mocks.rs"]
mod nip46_mocks;

#[path = "../src/lightning.rs"]
mod lightning_internal;

use http_mocks::MockHttpClient;
use lightning_internal::{request_zap_invoice_with_http, ZapRequest};
use nip46_mocks::MockNip46Relay;

#[test]
fn parse_valid_badge_definition_extracts_nip58_tags() {
    let keys = Keys::generate();
    let event = EventBuilder::new(Kind::Custom(30009), "")
        .tags(vec![
            nostr::Tag::custom(nostr::TagKind::d(), ["first_clear"]),
            nostr::Tag::custom(nostr::TagKind::custom("name"), ["First Clear"]),
            nostr::Tag::custom(
                nostr::TagKind::custom("description"),
                ["Finished a game once"],
            ),
            nostr::Tag::custom(
                nostr::TagKind::custom("image"),
                ["https://example.com/badge.png", "1024x1024"],
            ),
            nostr::Tag::custom(
                nostr::TagKind::custom("thumb"),
                ["https://example.com/badge-thumb.png", "256x256"],
            ),
        ])
        .sign_with_keys(&keys)
        .expect("test event signs");

    let definition = arcadestr_core::achievements::parse_badge_definition(
        &event,
        Some("wss://relay.example.com".to_string()),
    )
    .expect("definition parses");

    assert_eq!(definition.badge_id, "first_clear");
    assert_eq!(definition.issuer_pubkey, keys.public_key().to_hex());
    assert_eq!(
        definition.coordinate,
        format!("30009:{}:first_clear", keys.public_key().to_hex())
    );
    assert_eq!(definition.name.as_deref(), Some("First Clear"));
    assert_eq!(definition.image_dimensions.as_deref(), Some("1024x1024"));
}

#[test]
fn reject_badge_definition_without_non_empty_d_tag() {
    let keys = Keys::generate();
    let event = EventBuilder::new(Kind::Custom(30009), "")
        .sign_with_keys(&keys)
        .expect("test event signs");

    let error = arcadestr_core::achievements::parse_badge_definition(&event, None)
        .expect_err("missing d tag should fail");

    assert!(error.to_string().contains("d tag"));
}

#[test]
fn parse_profile_badges_requires_immediate_a_then_e_pairs() {
    let owner = Keys::generate();
    let issuer = Keys::generate();
    let badge_coordinate = format!("30009:{}:first", issuer.public_key().to_hex());
    let event = EventBuilder::new(Kind::Custom(10008), "")
        .tags(vec![
            nostr::Tag::custom(nostr::TagKind::a(), [badge_coordinate.as_str()]),
            nostr::Tag::custom(
                nostr::TagKind::e(),
                [
                    "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
                    "wss://relay.example.com",
                ],
            ),
            nostr::Tag::custom(
                nostr::TagKind::e(),
                ["bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"],
            ),
            nostr::Tag::custom(nostr::TagKind::a(), ["30009:issuer:orphan"]),
        ])
        .sign_with_keys(&owner)
        .expect("test event signs");

    let list = arcadestr_core::achievements::parse_profile_badge_list(
        &event,
        &owner.public_key().to_hex(),
    )
    .expect("profile list parses");

    assert_eq!(list.entries.len(), 1);
    assert_eq!(list.entries[0].badge_coordinate, badge_coordinate);
    assert_eq!(list.entries[0].display_order, 0);
}

#[test]
fn malformed_profile_badge_pairs_are_skipped() {
    let owner = Keys::generate();
    let issuer = Keys::generate();
    let valid_coordinate = format!("30009:{}:first", issuer.public_key().to_hex());
    let valid_event_id = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
    let event = EventBuilder::new(Kind::Custom(10008), "")
        .tags(vec![
            nostr::Tag::custom(nostr::TagKind::a(), ["30009:not-a-pubkey:bad"]),
            nostr::Tag::custom(nostr::TagKind::e(), [valid_event_id]),
            nostr::Tag::custom(nostr::TagKind::a(), [valid_coordinate.as_str()]),
            nostr::Tag::custom(nostr::TagKind::e(), ["not-an-event-id"]),
            nostr::Tag::custom(nostr::TagKind::a(), ["30009::empty_issuer"]),
            nostr::Tag::custom(nostr::TagKind::e(), [valid_event_id]),
            nostr::Tag::custom(nostr::TagKind::a(), ["30009:too:few"]),
            nostr::Tag::custom(nostr::TagKind::e(), [valid_event_id]),
            nostr::Tag::custom(nostr::TagKind::a(), [valid_coordinate.as_str()]),
            nostr::Tag::custom(
                nostr::TagKind::e(),
                [valid_event_id, "wss://relay.example.com"],
            ),
        ])
        .sign_with_keys(&owner)
        .expect("test event signs");

    let list = arcadestr_core::achievements::parse_profile_badge_list(
        &event,
        &owner.public_key().to_hex(),
    )
    .expect("profile list parses with malformed pairs skipped");

    assert_eq!(list.entries.len(), 1);
    assert_eq!(list.entries[0].badge_coordinate, valid_coordinate);
    assert_eq!(list.entries[0].award_event_id, valid_event_id);
    assert_eq!(
        list.entries[0].relay_url.as_deref(),
        Some("wss://relay.example.com")
    );
    assert_eq!(list.entries[0].display_order, 0);
}

#[test]
fn invalid_profile_badge_kind_returns_explicit_error() {
    let owner = Keys::generate();
    let event = EventBuilder::new(Kind::Custom(30009), "")
        .sign_with_keys(&owner)
        .expect("test event signs");

    let error = arcadestr_core::achievements::parse_profile_badge_list(
        &event,
        &owner.public_key().to_hex(),
    )
    .expect_err("wrong profile badge kind should fail");

    assert!(matches!(
        error,
        arcadestr_core::achievements::AchievementError::InvalidProfileBadgeKind
    ));
}

fn temp_db_path(name: &str) -> PathBuf {
    let mut path = std::env::temp_dir();
    path.push(format!(
        "arcadestr-int-{}-{}-{}.db",
        name,
        std::process::id(),
        uuid::Uuid::new_v4()
    ));
    path
}

fn badge_definition(
    issuer_pubkey: &str,
    badge_id: &str,
    event_id: &str,
    created_at: u64,
) -> arcadestr_core::achievements::BadgeDefinition {
    arcadestr_core::achievements::BadgeDefinition {
        coordinate: format!("30009:{issuer_pubkey}:{badge_id}"),
        issuer_pubkey: issuer_pubkey.to_string(),
        badge_id: badge_id.to_string(),
        name: Some(format!("Badge {badge_id}")),
        description: Some("Achievement description".to_string()),
        image_url: Some("https://example.com/badge.png".to_string()),
        image_dimensions: Some("1024x1024".to_string()),
        thumb_url: Some("https://example.com/thumb.png".to_string()),
        thumb_dimensions: Some("256x256".to_string()),
        relay_url: Some("wss://relay.example.com".to_string()),
        event_id: event_id.to_string(),
        created_at,
    }
}

fn badge_award(
    issuer_pubkey: &str,
    recipient_pubkey: &str,
    badge_coordinate: &str,
    event_id: &str,
    created_at: u64,
) -> arcadestr_core::achievements::BadgeAward {
    arcadestr_core::achievements::BadgeAward {
        event_id: event_id.to_string(),
        issuer_pubkey: issuer_pubkey.to_string(),
        recipient_pubkey: recipient_pubkey.to_string(),
        badge_coordinate: badge_coordinate.to_string(),
        relay_url: Some("wss://relay.example.com".to_string()),
        created_at,
    }
}

#[test]
fn issuer_mismatch_excludes_award_before_cache() {
    let issuer = Keys::generate();
    let attacker = Keys::generate();
    let recipient = Keys::generate();
    let definition = badge_definition(
        &issuer.public_key().to_hex(),
        "first_clear",
        "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
        1,
    );
    let award = badge_award(
        &attacker.public_key().to_hex(),
        &recipient.public_key().to_hex(),
        &definition.coordinate,
        "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
        2,
    );

    let result = arcadestr_core::achievements::validate_award_issuer(&award, &definition);

    assert!(matches!(
        result,
        Err(arcadestr_core::achievements::AchievementError::IssuerMismatch)
    ));
}

#[tokio::test]
async fn cache_badge_definition_keeps_latest_by_coordinate() {
    let db_path = temp_db_path("badge-definition-upsert");
    let db = Database::new(&db_path)
        .await
        .expect("database should initialize");
    let issuer = Keys::generate().public_key().to_hex();
    let initial = badge_definition(
        &issuer,
        "first_clear",
        "cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc",
        10,
    );
    let older = badge_definition(
        &issuer,
        "first_clear",
        "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
        9,
    );
    let tie_lower_event_id = badge_definition(
        &issuer,
        "first_clear",
        "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
        10,
    );
    let newer = badge_definition(
        &issuer,
        "first_clear",
        "dddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddd",
        11,
    );

    db.cache_badge_definition(&initial, r#"{"event":"initial"}"#)
        .await
        .expect("initial definition should cache");
    db.cache_badge_definition(&older, r#"{"event":"older"}"#)
        .await
        .expect("older definition should be ignored");
    db.cache_badge_definition(&tie_lower_event_id, r#"{"event":"tie"}"#)
        .await
        .expect("lower event id tie should replace");

    let tied_event_id: String =
        sqlx::query_scalar("SELECT event_id FROM badge_definitions WHERE coordinate = ?")
            .bind(&initial.coordinate)
            .fetch_one(db.pool())
            .await
            .expect("definition row should exist after tie replacement");

    assert_eq!(tied_event_id, tie_lower_event_id.event_id);

    db.cache_badge_definition(&newer, r#"{"event":"newer"}"#)
        .await
        .expect("newer definition should replace");

    let cached_event_id: String =
        sqlx::query_scalar("SELECT event_id FROM badge_definitions WHERE coordinate = ?")
            .bind(&initial.coordinate)
            .fetch_one(db.pool())
            .await
            .expect("definition row should exist");

    assert_eq!(cached_event_id, newer.event_id);

    db.close().await;
    std::fs::remove_file(db_path).expect("test database should be removed");
}

#[tokio::test]
async fn earned_badges_for_profile_joins_awards_to_definitions() {
    let db_path = temp_db_path("earned-badges-join");
    let db = Database::new(&db_path)
        .await
        .expect("database should initialize");
    let issuer = Keys::generate().public_key().to_hex();
    let recipient = Keys::generate().public_key().to_hex();
    let definition = badge_definition(
        &issuer,
        "first_clear",
        "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
        1,
    );
    let award = badge_award(
        &issuer,
        &recipient,
        &definition.coordinate,
        "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
        2,
    );

    db.cache_badge_definition(&definition, r#"{"kind":30009}"#)
        .await
        .expect("definition should cache");
    db.cache_badge_award(&award, r#"{"kind":8}"#)
        .await
        .expect("award should cache");

    let earned = db
        .earned_badges_for_profile(&recipient)
        .await
        .expect("earned badges should query");

    assert_eq!(earned.len(), 1);
    assert_eq!(earned[0].definition.coordinate, definition.coordinate);
    assert_eq!(earned[0].award.event_id, award.event_id);
    assert!(!earned[0].visible_on_profile);

    db.close().await;
    std::fs::remove_file(db_path).expect("test database should be removed");
}

#[tokio::test]
async fn cache_profile_badge_list_replaces_entries_in_transaction() {
    let db_path = temp_db_path("profile-badge-replace");
    let db = Database::new(&db_path)
        .await
        .expect("database should initialize");
    let issuer = Keys::generate().public_key().to_hex();
    let profile = Keys::generate().public_key().to_hex();
    let first_definition = badge_definition(
        &issuer,
        "first_clear",
        "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
        1,
    );
    let second_definition = badge_definition(
        &issuer,
        "second_clear",
        "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
        2,
    );
    let first_award = badge_award(
        &issuer,
        &profile,
        &first_definition.coordinate,
        "cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc",
        3,
    );
    let second_award = badge_award(
        &issuer,
        &profile,
        &second_definition.coordinate,
        "dddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddd",
        4,
    );
    db.cache_badge_definition(&first_definition, r#"{"badge":"first"}"#)
        .await
        .expect("first definition should cache");
    db.cache_badge_definition(&second_definition, r#"{"badge":"second"}"#)
        .await
        .expect("second definition should cache");
    db.cache_badge_award(&first_award, r#"{"award":"first"}"#)
        .await
        .expect("first award should cache");
    db.cache_badge_award(&second_award, r#"{"award":"second"}"#)
        .await
        .expect("second award should cache");

    let first_list = arcadestr_core::achievements::ProfileBadgeList {
        profile_pubkey: profile.clone(),
        event_id: "eeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee".to_string(),
        kind: arcadestr_core::achievements::KIND_PROFILE_BADGES_CURRENT,
        created_at: 5,
        entries: vec![arcadestr_core::achievements::ProfileBadgeSelection {
            badge_coordinate: first_definition.coordinate.clone(),
            award_event_id: first_award.event_id.clone(),
            relay_url: Some("wss://relay.example.com".to_string()),
            display_order: 0,
        }],
    };
    let replacement_list = arcadestr_core::achievements::ProfileBadgeList {
        profile_pubkey: profile.clone(),
        event_id: "ffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff".to_string(),
        kind: arcadestr_core::achievements::KIND_PROFILE_BADGES_CURRENT,
        created_at: 6,
        entries: vec![arcadestr_core::achievements::ProfileBadgeSelection {
            badge_coordinate: second_definition.coordinate.clone(),
            award_event_id: second_award.event_id.clone(),
            relay_url: Some("wss://relay.example.com".to_string()),
            display_order: 0,
        }],
    };

    db.cache_profile_badge_list(&first_list, r#"{"list":"first"}"#)
        .await
        .expect("first profile list should cache");
    db.cache_profile_badge_list(&replacement_list, r#"{"list":"replacement"}"#)
        .await
        .expect("replacement profile list should cache");

    let profile_badges = db
        .profile_badges_for_profile(&profile)
        .await
        .expect("profile badges should query");

    assert_eq!(profile_badges.len(), 1);
    assert_eq!(
        profile_badges[0].definition.coordinate,
        second_definition.coordinate
    );
    assert_eq!(profile_badges[0].award.event_id, second_award.event_id);
    assert_eq!(profile_badges[0].display_order, 0);
    assert!(profile_badges[0].visible);

    db.close().await;
    std::fs::remove_file(db_path).expect("test database should be removed");
}

#[tokio::test]
async fn stale_profile_badge_list_does_not_replace_newer_entries() {
    let db_path = temp_db_path("profile-badge-stale");
    let db = Database::new(&db_path)
        .await
        .expect("database should initialize");
    let issuer = Keys::generate().public_key().to_hex();
    let profile = Keys::generate().public_key().to_hex();
    let first_definition = badge_definition(
        &issuer,
        "first_clear",
        "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
        1,
    );
    let second_definition = badge_definition(
        &issuer,
        "second_clear",
        "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
        2,
    );
    let first_award = badge_award(
        &issuer,
        &profile,
        &first_definition.coordinate,
        "cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc",
        3,
    );
    let second_award = badge_award(
        &issuer,
        &profile,
        &second_definition.coordinate,
        "dddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddd",
        4,
    );
    db.cache_badge_definition(&first_definition, r#"{"badge":"first"}"#)
        .await
        .expect("first definition should cache");
    db.cache_badge_definition(&second_definition, r#"{"badge":"second"}"#)
        .await
        .expect("second definition should cache");
    db.cache_badge_award(&first_award, r#"{"award":"first"}"#)
        .await
        .expect("first award should cache");
    db.cache_badge_award(&second_award, r#"{"award":"second"}"#)
        .await
        .expect("second award should cache");

    let newer_list = arcadestr_core::achievements::ProfileBadgeList {
        profile_pubkey: profile.clone(),
        event_id: "eeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee".to_string(),
        kind: arcadestr_core::achievements::KIND_PROFILE_BADGES_CURRENT,
        created_at: 10,
        entries: vec![arcadestr_core::achievements::ProfileBadgeSelection {
            badge_coordinate: first_definition.coordinate.clone(),
            award_event_id: first_award.event_id.clone(),
            relay_url: Some("wss://relay.example.com".to_string()),
            display_order: 0,
        }],
    };
    let stale_list = arcadestr_core::achievements::ProfileBadgeList {
        profile_pubkey: profile.clone(),
        event_id: "ffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff".to_string(),
        kind: arcadestr_core::achievements::KIND_PROFILE_BADGES_CURRENT,
        created_at: 9,
        entries: vec![arcadestr_core::achievements::ProfileBadgeSelection {
            badge_coordinate: second_definition.coordinate.clone(),
            award_event_id: second_award.event_id.clone(),
            relay_url: Some("wss://relay.example.com".to_string()),
            display_order: 0,
        }],
    };

    db.cache_profile_badge_list(&newer_list, r#"{"list":"newer"}"#)
        .await
        .expect("newer profile list should cache");
    db.cache_profile_badge_list(&stale_list, r#"{"list":"stale"}"#)
        .await
        .expect("stale profile list should be ignored");

    let profile_badges = db
        .profile_badges_for_profile(&profile)
        .await
        .expect("profile badges should query");

    assert_eq!(profile_badges.len(), 1);
    assert_eq!(profile_badges[0].award.event_id, first_award.event_id);

    db.close().await;
    std::fs::remove_file(db_path).expect("test database should be removed");
}

#[tokio::test]
async fn current_profile_badge_list_replaces_deprecated_list() {
    let db_path = temp_db_path("profile-badge-current-over-deprecated");
    let db = Database::new(&db_path)
        .await
        .expect("database should initialize");
    let issuer = Keys::generate().public_key().to_hex();
    let profile = Keys::generate().public_key().to_hex();
    let deprecated_definition = badge_definition(
        &issuer,
        "deprecated_choice",
        "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
        1,
    );
    let current_definition = badge_definition(
        &issuer,
        "current_choice",
        "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
        2,
    );
    let deprecated_award = badge_award(
        &issuer,
        &profile,
        &deprecated_definition.coordinate,
        "cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc",
        3,
    );
    let current_award = badge_award(
        &issuer,
        &profile,
        &current_definition.coordinate,
        "dddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddd",
        4,
    );
    db.cache_badge_definition(&deprecated_definition, r#"{"badge":"deprecated"}"#)
        .await
        .expect("deprecated definition should cache");
    db.cache_badge_definition(&current_definition, r#"{"badge":"current"}"#)
        .await
        .expect("current definition should cache");
    db.cache_badge_award(&deprecated_award, r#"{"award":"deprecated"}"#)
        .await
        .expect("deprecated award should cache");
    db.cache_badge_award(&current_award, r#"{"award":"current"}"#)
        .await
        .expect("current award should cache");

    let deprecated_list = arcadestr_core::achievements::ProfileBadgeList {
        profile_pubkey: profile.clone(),
        event_id: "eeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee".to_string(),
        kind: arcadestr_core::achievements::KIND_PROFILE_BADGES_DEPRECATED,
        created_at: 10,
        entries: vec![arcadestr_core::achievements::ProfileBadgeSelection {
            badge_coordinate: deprecated_definition.coordinate.clone(),
            award_event_id: deprecated_award.event_id.clone(),
            relay_url: Some("wss://relay.example.com".to_string()),
            display_order: 0,
        }],
    };
    let current_list = arcadestr_core::achievements::ProfileBadgeList {
        profile_pubkey: profile.clone(),
        event_id: "ffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff".to_string(),
        kind: arcadestr_core::achievements::KIND_PROFILE_BADGES_CURRENT,
        created_at: 9,
        entries: vec![arcadestr_core::achievements::ProfileBadgeSelection {
            badge_coordinate: current_definition.coordinate.clone(),
            award_event_id: current_award.event_id.clone(),
            relay_url: Some("wss://relay.example.com".to_string()),
            display_order: 0,
        }],
    };

    db.cache_profile_badge_list(&deprecated_list, r#"{"list":"deprecated"}"#)
        .await
        .expect("deprecated list should cache");
    db.cache_profile_badge_list(&current_list, r#"{"list":"current"}"#)
        .await
        .expect("current list should replace deprecated list");

    let profile_badges = db
        .profile_badges_for_profile(&profile)
        .await
        .expect("profile badges should query");

    assert_eq!(profile_badges.len(), 1);
    assert_eq!(profile_badges[0].award.event_id, current_award.event_id);

    db.close().await;
    std::fs::remove_file(db_path).expect("test database should be removed");
}

#[tokio::test]
async fn deprecated_profile_badge_list_does_not_replace_current_list() {
    let db_path = temp_db_path("profile-badge-deprecated-after-current");
    let db = Database::new(&db_path)
        .await
        .expect("database should initialize");
    let issuer = Keys::generate().public_key().to_hex();
    let profile = Keys::generate().public_key().to_hex();
    let current_definition = badge_definition(
        &issuer,
        "current_choice",
        "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
        1,
    );
    let deprecated_definition = badge_definition(
        &issuer,
        "deprecated_choice",
        "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
        2,
    );
    let current_award = badge_award(
        &issuer,
        &profile,
        &current_definition.coordinate,
        "cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc",
        3,
    );
    let deprecated_award = badge_award(
        &issuer,
        &profile,
        &deprecated_definition.coordinate,
        "dddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddd",
        4,
    );
    db.cache_badge_definition(&current_definition, r#"{"badge":"current"}"#)
        .await
        .expect("current definition should cache");
    db.cache_badge_definition(&deprecated_definition, r#"{"badge":"deprecated"}"#)
        .await
        .expect("deprecated definition should cache");
    db.cache_badge_award(&current_award, r#"{"award":"current"}"#)
        .await
        .expect("current award should cache");
    db.cache_badge_award(&deprecated_award, r#"{"award":"deprecated"}"#)
        .await
        .expect("deprecated award should cache");

    let current_list = arcadestr_core::achievements::ProfileBadgeList {
        profile_pubkey: profile.clone(),
        event_id: "eeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee".to_string(),
        kind: arcadestr_core::achievements::KIND_PROFILE_BADGES_CURRENT,
        created_at: 10,
        entries: vec![arcadestr_core::achievements::ProfileBadgeSelection {
            badge_coordinate: current_definition.coordinate.clone(),
            award_event_id: current_award.event_id.clone(),
            relay_url: Some("wss://relay.example.com".to_string()),
            display_order: 0,
        }],
    };
    let deprecated_list = arcadestr_core::achievements::ProfileBadgeList {
        profile_pubkey: profile.clone(),
        event_id: "ffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff".to_string(),
        kind: arcadestr_core::achievements::KIND_PROFILE_BADGES_DEPRECATED,
        created_at: 11,
        entries: vec![arcadestr_core::achievements::ProfileBadgeSelection {
            badge_coordinate: deprecated_definition.coordinate.clone(),
            award_event_id: deprecated_award.event_id.clone(),
            relay_url: Some("wss://relay.example.com".to_string()),
            display_order: 0,
        }],
    };

    db.cache_profile_badge_list(&current_list, r#"{"list":"current"}"#)
        .await
        .expect("current list should cache");
    db.cache_profile_badge_list(&deprecated_list, r#"{"list":"deprecated"}"#)
        .await
        .expect("deprecated list should be ignored after current list");

    let profile_badges = db
        .profile_badges_for_profile(&profile)
        .await
        .expect("profile badges should query");

    assert_eq!(profile_badges.len(), 1);
    assert_eq!(profile_badges[0].award.event_id, current_award.event_id);

    db.close().await;
    std::fs::remove_file(db_path).expect("test database should be removed");
}

#[tokio::test]
async fn earned_badges_query_rejects_negative_cached_timestamps() {
    let db_path = temp_db_path("earned-badges-negative-ts");
    let db = Database::new(&db_path)
        .await
        .expect("database should initialize");
    let issuer = Keys::generate().public_key().to_hex();
    let profile = Keys::generate().public_key().to_hex();
    let coordinate = format!("30009:{issuer}:first_clear");

    sqlx::query(
        r#"
        INSERT INTO badge_definitions (
            coordinate, issuer_pubkey, badge_id, event_id, created_at, raw_event_json
        ) VALUES (?, ?, ?, ?, ?, ?)
        "#,
    )
    .bind(&coordinate)
    .bind(&issuer)
    .bind("first_clear")
    .bind("aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa")
    .bind(-1_i64)
    .bind(r#"{"kind":30009}"#)
    .execute(db.pool())
    .await
    .expect("corrupt definition fixture should insert");

    sqlx::query(
        r#"
        INSERT INTO badge_awards (
            event_id, issuer_pubkey, recipient_pubkey, badge_coordinate, created_at, raw_event_json
        ) VALUES (?, ?, ?, ?, ?, ?)
        "#,
    )
    .bind("bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb")
    .bind(&issuer)
    .bind(&profile)
    .bind(&coordinate)
    .bind(2_i64)
    .bind(r#"{"kind":8}"#)
    .execute(db.pool())
    .await
    .expect("award fixture should insert");

    let error = db
        .earned_badges_for_profile(&profile)
        .await
        .expect_err("negative timestamps should be rejected");

    assert!(error.to_string().contains("Timestamp out of range"));

    db.close().await;
    std::fs::remove_file(db_path).expect("test database should be removed");
}

#[tokio::test]
async fn profile_badges_query_rejects_negative_award_timestamps() {
    let db_path = temp_db_path("profile-badges-negative-award-ts");
    let db = Database::new(&db_path)
        .await
        .expect("database should initialize");
    let issuer = Keys::generate().public_key().to_hex();
    let profile = Keys::generate().public_key().to_hex();
    let coordinate = format!("30009:{issuer}:first_clear");
    let award_event_id = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";

    sqlx::query(
        r#"
        INSERT INTO badge_definitions (
            coordinate, issuer_pubkey, badge_id, event_id, created_at, raw_event_json
        ) VALUES (?, ?, ?, ?, ?, ?)
        "#,
    )
    .bind(&coordinate)
    .bind(&issuer)
    .bind("first_clear")
    .bind("aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa")
    .bind(1_i64)
    .bind(r#"{"kind":30009}"#)
    .execute(db.pool())
    .await
    .expect("definition fixture should insert");

    sqlx::query(
        r#"
        INSERT INTO badge_awards (
            event_id, issuer_pubkey, recipient_pubkey, badge_coordinate, created_at, raw_event_json
        ) VALUES (?, ?, ?, ?, ?, ?)
        "#,
    )
    .bind(award_event_id)
    .bind(&issuer)
    .bind(&profile)
    .bind(&coordinate)
    .bind(-1_i64)
    .bind(r#"{"kind":8}"#)
    .execute(db.pool())
    .await
    .expect("corrupt award fixture should insert");

    sqlx::query(
        r#"
        INSERT INTO profile_badge_lists (
            profile_pubkey, event_id, kind, created_at, raw_event_json, updated_at
        ) VALUES (?, ?, ?, ?, ?, ?)
        "#,
    )
    .bind(&profile)
    .bind("cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc")
    .bind(i64::from(
        arcadestr_core::achievements::KIND_PROFILE_BADGES_CURRENT,
    ))
    .bind(2_i64)
    .bind(r#"{"kind":10008}"#)
    .bind(3_i64)
    .execute(db.pool())
    .await
    .expect("profile list fixture should insert");

    sqlx::query(
        r#"
        INSERT INTO profile_badge_entries (
            profile_pubkey, badge_coordinate, award_event_id, display_order
        ) VALUES (?, ?, ?, ?)
        "#,
    )
    .bind(&profile)
    .bind(&coordinate)
    .bind(award_event_id)
    .bind(0_i64)
    .execute(db.pool())
    .await
    .expect("profile entry fixture should insert");

    let error = db
        .profile_badges_for_profile(&profile)
        .await
        .expect_err("negative award timestamps should be rejected");

    assert!(error.to_string().contains("Timestamp out of range"));

    db.close().await;
    std::fs::remove_file(db_path).expect("test database should be removed");
}

#[tokio::test]
async fn profile_badge_queries_require_matching_award_recipient_and_coordinate() {
    let db_path = temp_db_path("profile-badge-proof-match");
    let db = Database::new(&db_path)
        .await
        .expect("database should initialize");
    let issuer = Keys::generate().public_key().to_hex();
    let profile = Keys::generate().public_key().to_hex();
    let first_definition = badge_definition(
        &issuer,
        "first_clear",
        "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
        1,
    );
    let second_definition = badge_definition(
        &issuer,
        "second_clear",
        "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
        2,
    );
    let award = badge_award(
        &issuer,
        &profile,
        &first_definition.coordinate,
        "cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc",
        3,
    );
    let mismatched_list = arcadestr_core::achievements::ProfileBadgeList {
        profile_pubkey: profile.clone(),
        event_id: "dddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddd".to_string(),
        kind: arcadestr_core::achievements::KIND_PROFILE_BADGES_CURRENT,
        created_at: 4,
        entries: vec![arcadestr_core::achievements::ProfileBadgeSelection {
            badge_coordinate: second_definition.coordinate.clone(),
            award_event_id: award.event_id.clone(),
            relay_url: Some("wss://relay.example.com".to_string()),
            display_order: 0,
        }],
    };

    db.cache_badge_definition(&first_definition, r#"{"badge":"first"}"#)
        .await
        .expect("first definition should cache");
    db.cache_badge_definition(&second_definition, r#"{"badge":"second"}"#)
        .await
        .expect("second definition should cache");
    db.cache_badge_award(&award, r#"{"award":"first"}"#)
        .await
        .expect("award should cache");
    db.cache_profile_badge_list(&mismatched_list, r#"{"list":"mismatch"}"#)
        .await
        .expect("mismatched profile list should cache as relay data");

    let earned = db
        .earned_badges_for_profile(&profile)
        .await
        .expect("earned badges should query");
    let profile_badges = db
        .profile_badges_for_profile(&profile)
        .await
        .expect("profile badges should query");

    assert_eq!(earned.len(), 1);
    assert!(!earned[0].visible_on_profile);
    assert!(profile_badges.is_empty());

    db.close().await;
    std::fs::remove_file(db_path).expect("test database should be removed");
}

fn test_game_listing(publisher_npub: String, id: &str) -> GameListing {
    GameListing {
        id: id.to_string(),
        source: ListingSource::Legacy,
        title: "Test Game".to_string(),
        description: "Integration scenario listing".to_string(),
        price_sats: 1_000,
        download_url: "https://example.com/game.zip".to_string(),
        publisher_npub,
        created_at: 1_710_000_000,
        tags: vec!["action".to_string(), "indie".to_string()],
        lud16: "seller@example.com".to_string(),
        images: vec!["https://example.com/cover.png".to_string()],
        summary: Some("short summary".to_string()),
        published_at: Some(1_710_000_000),
        location: Some("online".to_string()),
        geohash: None,
        status: Some("active".to_string()),
    }
}

fn test_nip99_listing(id: &str, merchant_npub: &str, created_at: u64) -> Nip99Listing {
    Nip99Listing {
        id: id.to_string(),
        title: format!("Listing {id}"),
        content: "Markdown body".to_string(),
        summary: Some("summary".to_string()),
        published_at: Some(created_at as i64),
        location: Some("online".to_string()),
        price_amount: Some("1000".to_string()),
        price_currency: Some("SATS".to_string()),
        price_frequency: None,
        images: vec!["https://example.com/img.png".to_string()],
        geohash: None,
        tags: vec!["indie".to_string()],
        status: Some("active".to_string()),
        #[cfg(debug_assertions)]
        raw_event_json: None,
        merchant_npub: merchant_npub.to_string(),
        created_at,
    }
}

fn test_auth_state() -> AuthState {
    let mut auth = AuthState::new();
    let keys = Keys::generate();
    auth.connect_with_key(&keys.secret_key().to_secret_hex())
        .expect("test key should authenticate auth state");
    auth
}

#[tokio::test]
async fn int_01_full_listing_publish_retrieve_cycle() {
    let auth = test_auth_state();
    let signer = auth.signer().expect("auth signer should exist");
    let signer_pubkey = signer
        .get_public_key()
        .await
        .expect("signer should provide pubkey");
    let publisher_npub = signer_pubkey
        .to_bech32()
        .expect("pubkey should encode to npub");

    let listing = test_game_listing(publisher_npub, "int-01-listing");
    let builder = game_listing_to_event_builder(&listing);
    let unsigned = builder.build(signer_pubkey);
    let event = signer
        .sign_event(unsigned)
        .await
        .expect("sign_event should succeed");

    assert_eq!(event.kind, Kind::Custom(KIND_GAME_LISTING));
    assert_eq!(event.id.to_hex().len(), 64);
    assert!(event.id.to_hex().bytes().all(|b| b.is_ascii_hexdigit()));
    assert!(event.verify().is_ok(), "signed event should verify");

    let roundtrip = event_to_game_listing(&event).expect("event should parse into listing");
    assert_eq!(roundtrip.id, listing.id);
    assert_eq!(roundtrip.title, listing.title);
    assert_eq!(roundtrip.publisher_npub, listing.publisher_npub);

    let db_path = temp_db_path("int01");
    let db = Database::new(&db_path)
        .await
        .expect("integration database should initialize");
    let cache = MarketplaceCache::new(db.pool().clone());

    cache
        .upsert_listing(&roundtrip, Some(&event.id.to_hex()))
        .await
        .expect("cache upsert should succeed");

    let loaded = cache
        .load_listings(20, None, None)
        .await
        .expect("cache load should succeed");

    let found = loaded
        .iter()
        .find(|entry| entry.publisher_npub == listing.publisher_npub && entry.id == listing.id)
        .expect("listing should be retrievable from cache");

    assert_eq!(found.title, listing.title);

    let _ = std::fs::remove_file(db_path);
}

#[tokio::test]
async fn int_02_nip05_validation_with_relay_discovery() {
    let keys = Keys::generate();
    let npub = keys
        .public_key()
        .to_bech32()
        .expect("pubkey should encode to npub");
    let expected_hex = keys.public_key().to_hex();

    let url = "https://example.com/.well-known/nostr.json?name=alice";
    let mock = MockHttpClient::new().with_json_response(
        url,
        json!({
            "names": {
                "alice": expected_hex,
            },
            "relays": {
                expected_hex: [
                    "wss://relay.damus.io",
                    "wss://relay.primal.net",
                    "wss://relay.primal.net"
                ]
            }
        }),
    );

    let validator = Nip05IdentityValidator::with_http_client(Arc::new(mock.clone()));

    let first = validator.validate(&npub, "alice@example.com").await;
    let second = validator.validate(&npub, "alice@example.com").await;

    assert_eq!(first.state, IdentityValidationState::Valid);
    assert_eq!(second.state, IdentityValidationState::Valid);
    assert_eq!(
        first.relays,
        vec![
            "wss://relay.damus.io".to_string(),
            "wss://relay.primal.net".to_string()
        ]
    );
    assert_eq!(mock.call_count(url), 1, "second call should use cache");
}

#[tokio::test]
async fn int_03_nip46_session_lifecycle_mocked_sequence() {
    let signer_keys = Keys::generate();
    let user_signing_keys = Keys::generate();
    let app_keys = Keys::generate();

    let signer_pubkey = signer_keys.public_key();
    let user_pubkey_hex = user_signing_keys.public_key().to_hex();

    let mut relay = MockNip46Relay::new(signer_keys, user_signing_keys);
    relay.set_expected_secret("sec-123");

    let connect_req = MockNip46Relay::build_client_request_event(
        &app_keys,
        signer_pubkey,
        "connect",
        json!(["sec-123"]),
        "id-connect",
    );
    let connect_resp = relay
        .process_client_event(&connect_req)
        .expect("connect request should succeed");
    let connect_json = MockNip46Relay::decrypt_relay_response(&app_keys, &connect_resp)
        .expect("connect response should decrypt");
    assert_eq!(connect_json["result"], json!("sec-123"));

    let pubkey_req = MockNip46Relay::build_client_request_event(
        &app_keys,
        signer_pubkey,
        "get_public_key",
        json!([]),
        "id-pubkey",
    );
    let pubkey_resp = relay
        .process_client_event(&pubkey_req)
        .expect("get_public_key should succeed");
    let pubkey_json = MockNip46Relay::decrypt_relay_response(&app_keys, &pubkey_resp)
        .expect("pubkey response should decrypt");
    assert_eq!(pubkey_json["result"], json!(user_pubkey_hex));

    let unsigned = EventBuilder::new(Kind::TextNote, "int-03 sign")
        .build(PublicKey::parse(&user_pubkey_hex).expect("hex pubkey should parse"));
    let sign_req = MockNip46Relay::build_client_request_event(
        &app_keys,
        signer_pubkey,
        "sign_event",
        json!([serde_json::to_value(unsigned).expect("unsigned event should serialize")]),
        "id-sign",
    );
    let sign_resp = relay
        .process_client_event(&sign_req)
        .expect("sign_event should succeed");
    let sign_json = MockNip46Relay::decrypt_relay_response(&app_keys, &sign_resp)
        .expect("sign response should decrypt");
    let signed_event: Event = serde_json::from_value(sign_json["result"].clone())
        .expect("signed event payload should deserialize");
    assert!(signed_event.verify().is_ok(), "signed event should verify");

    relay.set_method_disconnect("ping");
    let ping_req = MockNip46Relay::build_client_request_event(
        &app_keys,
        signer_pubkey,
        "ping",
        json!([]),
        "id-ping-disconnect",
    );
    let err = relay
        .process_client_event(&ping_req)
        .expect_err("ping should fail after forced disconnect");
    assert!(err.contains("disconnected"));

    relay.reconnect();
    relay.clear_method_behavior("ping");

    let ping_req_ok = MockNip46Relay::build_client_request_event(
        &app_keys,
        signer_pubkey,
        "ping",
        json!([]),
        "id-ping-ok",
    );
    let ping_resp = relay
        .process_client_event(&ping_req_ok)
        .expect("ping should succeed after reconnect");
    let ping_json = MockNip46Relay::decrypt_relay_response(&app_keys, &ping_resp)
        .expect("ping response should decrypt");
    assert_eq!(ping_json["result"], json!("pong"));

    let mut auth = test_auth_state();
    assert!(auth.is_authenticated());
    auth.disconnect();
    assert!(!auth.is_authenticated());
}

#[tokio::test]
async fn int_04_marketplace_cache_streaming_callback_collection() {
    let db_path = temp_db_path("int04");
    let db = Database::new(&db_path)
        .await
        .expect("integration database should initialize");
    let cache = MarketplaceCache::new(db.pool().clone());

    for idx in 0..5 {
        let listing = test_game_listing(format!("npub-cached-{idx}"), &format!("cached-{idx}"));
        cache
            .upsert_listing(&listing, None)
            .await
            .expect("cache preload should succeed");
    }

    let cached_first = cache
        .load_listings(20, None, None)
        .await
        .expect("cache load should succeed");

    let mut streamed: Vec<Nip99Listing> = Vec::new();
    let mut seen_ids: HashSet<String> = HashSet::new();
    let mut on_product = |listing: Nip99Listing| {
        if seen_ids.insert(listing.id.clone()) {
            streamed.push(listing);
        }
    };

    for idx in 0..20 {
        let listing = test_nip99_listing(
            &format!("stream-{idx}"),
            "npub-stream-merchant",
            1_710_000_100 + idx,
        );
        on_product(listing);
    }

    on_product(test_nip99_listing(
        "stream-1",
        "npub-stream-merchant",
        1_710_000_500,
    ));

    for listing in &streamed {
        let game_listing = arcadestr_core::nostr::GameListing::from_listing(listing.clone());
        cache
            .upsert_listing(&game_listing, None)
            .await
            .expect("stream upsert should succeed");
    }

    assert_eq!(cached_first.len(), 5, "cached listings should load first");
    assert_eq!(streamed.len(), 20, "callback collection should dedup by id");

    let loaded = cache
        .load_listings(50, None, None)
        .await
        .expect("final cache load should succeed");

    assert!(
        loaded.len() >= 25,
        "cache should include cached + streamed listings"
    );

    let keyset: HashSet<(String, String)> = loaded
        .iter()
        .map(|entry| (entry.publisher_npub.clone(), entry.id.clone()))
        .collect();
    assert_eq!(
        keyset.len(),
        loaded.len(),
        "cache should not contain duplicates"
    );

    let _ = std::fs::remove_file(db_path);
}

#[tokio::test]
async fn int_05_nip57_zap_invoice_request_with_mock_http() {
    let auth = test_auth_state();

    let request = ZapRequest {
        seller_npub: "d94a3f0b5b907fda6c1d2716af34e4d533ddf8f6f6f0f8f1f4a3f605f6c9a3b4".to_string(),
        seller_lud16: "seller@example.com".to_string(),
        listing_event_id: "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
            .to_string(),
        amount_sats: 21,
        buyer_npub: Keys::generate()
            .public_key()
            .to_bech32()
            .expect("pubkey should encode to npub"),
        relays: vec![
            "wss://relay1.example.com".to_string(),
            "wss://relay2.example.com".to_string(),
        ],
    };

    let callback_base = "https://ln.example.com/callback";
    let mock = MockHttpClient::new()
        .with_json_response(
            "https://example.com/.well-known/lnurlp/seller",
            json!({
                "callback": callback_base,
                "minSendable": 1000,
                "maxSendable": 1000000,
                "allowsNostr": true,
                "nostrPubkey": "d94a3f0b5b907fda6c1d2716af34e4d533ddf8f6f6f0f8f1f4a3f605f6c9a3b4"
            }),
        )
        .with_prefix_json_response(callback_base, json!({ "pr": "lnbc10n1p0testpp5qqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqq" }));

    let invoice = request_zap_invoice_with_http(&request, &auth, &mock)
        .await
        .expect("zap invoice request should succeed");

    assert_eq!(invoice.amount_sats, request.amount_sats);
    assert!(!invoice.zap_request_event_id.is_empty());

    let callback_url = mock
        .last_requested_url()
        .expect("callback request URL should be captured");
    assert!(callback_url.contains("amount=21000"));
    assert!(callback_url.contains("nostr="));

    let mock_no_nostr = MockHttpClient::new()
        .with_json_response(
            "https://example.com/.well-known/lnurlp/seller",
            json!({
                "callback": callback_base,
                "minSendable": 1000,
                "maxSendable": 1000000
            }),
        )
        .with_prefix_json_response(callback_base, json!({ "pr": "lnbc10n1p0testpp5qqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqq" }));

    let invoice_no_nostr = request_zap_invoice_with_http(&request, &auth, &mock_no_nostr)
        .await
        .expect("invoice request should succeed without nostr mode");

    assert!(
        invoice_no_nostr.zap_request_event_id.is_empty(),
        "allowsNostr=false path should not create kind-9734 event"
    );
}
