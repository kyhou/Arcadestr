use std::time::{SystemTime, UNIX_EPOCH};

use nostr::Event;
use sqlx::{Row, SqlitePool};
use thiserror::Error;

use crate::is_replaceable_event_newer;
use crate::store_page::{
    listing_coordinate, parse_store_page_event, store_page_coordinate, ParsedStorePage,
    STORE_PAGE_SCHEMA_VERSION,
};
use crate::store_page_content_policy::STORE_PAGE_CONTENT_POLICY_VERSION;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StorePageCacheUpsertOutcome {
    Stored,
    Unchanged,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StorePageCacheEntry {
    pub coordinate: String,
    pub parsed: ParsedStorePage,
    pub sanitizer_policy_version: u32,
    pub policy_recomputed: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StorePageCacheLookup {
    Missing,
    Current(StorePageCacheEntry),
    StalePolicy {
        coordinate: String,
        cached_policy_version: u32,
        reason: String,
    },
    InvalidCachedEvent {
        coordinate: String,
        reason: String,
    },
}

#[derive(Debug, Error)]
pub enum StorePageRepositoryError {
    #[error(transparent)]
    Database(#[from] sqlx::Error),
    #[error("failed to serialize Store Page cache data: {0}")]
    Serialization(#[from] serde_json::Error),
    #[error("Store Page timestamp cannot be stored in SQLite")]
    InvalidTimestamp,
    #[error("Store Page cache coordinate does not match its signed event")]
    CoordinateMismatch,
    #[error("Store Page failed validation before persistence: {0}")]
    InvalidStorePage(String),
}

pub struct StorePageRepository {
    pool: SqlitePool,
}

impl StorePageRepository {
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }

    pub async fn upsert_listing_event(
        &self,
        event: &Event,
    ) -> Result<StorePageCacheUpsertOutcome, StorePageRepositoryError> {
        if event.kind.as_u16() != crate::adp_protocol::NIP99_LISTING_KIND || event.verify().is_err()
        {
            return Err(StorePageRepositoryError::InvalidStorePage(
                "invalid signed kind:30402 listing".to_string(),
            ));
        }
        let coordinate = listing_coordinate(event)
            .map_err(|error| StorePageRepositoryError::InvalidStorePage(error.to_string()))?;
        let d_tag = coordinate.splitn(3, ':').nth(2).ok_or_else(|| {
            StorePageRepositoryError::InvalidStorePage("missing listing d tag".into())
        })?;
        let raw_event_json = serde_json::to_string(event)?;
        let created_at = i64::try_from(event.created_at.as_secs())
            .map_err(|_| StorePageRepositoryError::InvalidTimestamp)?;
        let result = sqlx::query(
            r#"
            INSERT INTO store_page_listing_events (
                listing_coordinate, event_id, publisher_pubkey, d_tag, created_at,
                raw_event_json, updated_at
            ) VALUES (?, ?, ?, ?, ?, ?, ?)
            ON CONFLICT(listing_coordinate) DO UPDATE SET
                event_id = excluded.event_id,
                publisher_pubkey = excluded.publisher_pubkey,
                d_tag = excluded.d_tag,
                created_at = excluded.created_at,
                raw_event_json = excluded.raw_event_json,
                updated_at = excluded.updated_at
            WHERE excluded.created_at > store_page_listing_events.created_at
               OR (
                    excluded.created_at = store_page_listing_events.created_at
                    AND excluded.event_id < store_page_listing_events.event_id
               )
            "#,
        )
        .bind(&coordinate)
        .bind(event.id.to_hex())
        .bind(event.pubkey.to_hex())
        .bind(d_tag)
        .bind(created_at)
        .bind(raw_event_json)
        .bind(now_secs())
        .execute(&self.pool)
        .await?;
        Ok(if result.rows_affected() == 0 {
            StorePageCacheUpsertOutcome::Unchanged
        } else {
            StorePageCacheUpsertOutcome::Stored
        })
    }

    pub async fn load_listing_event(
        &self,
        coordinate: &str,
    ) -> Result<Option<Event>, StorePageRepositoryError> {
        let Some(row) = sqlx::query(
            r#"
            SELECT event_id, publisher_pubkey, d_tag, created_at, raw_event_json
            FROM store_page_listing_events
            WHERE listing_coordinate = ?
            "#,
        )
        .bind(coordinate)
        .fetch_optional(&self.pool)
        .await?
        else {
            return Ok(None);
        };
        let raw_event_json: String = row.get("raw_event_json");
        let event: Event = serde_json::from_str(&raw_event_json)?;
        let parsed_coordinate = listing_coordinate(&event)
            .map_err(|error| StorePageRepositoryError::InvalidStorePage(error.to_string()))?;
        if event.kind.as_u16() != crate::adp_protocol::NIP99_LISTING_KIND
            || event.verify().is_err()
            || parsed_coordinate != coordinate
            || row.get::<String, _>("event_id") != event.id.to_hex()
            || row.get::<String, _>("publisher_pubkey") != event.pubkey.to_hex()
            || row.get::<String, _>("d_tag") != coordinate.splitn(3, ':').nth(2).unwrap_or_default()
            || row.get::<i64, _>("created_at") < 0
            || row.get::<i64, _>("created_at") as u64 != event.created_at.as_secs()
        {
            return Err(StorePageRepositoryError::InvalidStorePage(
                "cached listing metadata does not match signed event".to_string(),
            ));
        }
        Ok(Some(event))
    }

    pub async fn upsert_valid(
        &self,
        parsed: &ParsedStorePage,
    ) -> Result<StorePageCacheUpsertOutcome, StorePageRepositoryError> {
        let parsed = parse_store_page_event(&parsed.event)
            .map_err(|error| StorePageRepositoryError::InvalidStorePage(error.to_string()))?;
        let parsed = &parsed;
        let coordinate = store_page_coordinate(parsed.event.pubkey, &parsed.presentation_id);
        let existing = sqlx::query(
            "SELECT event_id, created_at FROM store_pages WHERE store_page_coordinate = ?",
        )
        .bind(&coordinate)
        .fetch_optional(&self.pool)
        .await?;

        if let Some(row) = existing {
            let current_event_id: String = row.get("event_id");
            let current_created_at: i64 = row.get("created_at");
            if current_created_at < 0
                || !is_replaceable_event_newer(
                    parsed.event.created_at.as_secs(),
                    Some(parsed.event.id.to_hex().as_str()),
                    current_created_at as u64,
                    Some(current_event_id.as_str()),
                )
            {
                return Ok(StorePageCacheUpsertOutcome::Unchanged);
            }
        }

        if !self.write_valid(&coordinate, parsed).await? {
            return Ok(StorePageCacheUpsertOutcome::Unchanged);
        }
        Ok(StorePageCacheUpsertOutcome::Stored)
    }

    pub async fn load(
        &self,
        coordinate: &str,
    ) -> Result<StorePageCacheLookup, StorePageRepositoryError> {
        let Some(row) = sqlx::query(
            r#"
            SELECT event_id, publisher_pubkey, d_tag, created_at, raw_event_json,
                   sanitizer_policy_version, sanitized_content_json, diagnostics_json
            FROM store_pages
            WHERE store_page_coordinate = ?
            "#,
        )
        .bind(coordinate)
        .fetch_optional(&self.pool)
        .await?
        else {
            return Ok(StorePageCacheLookup::Missing);
        };

        let cached_policy_version: i64 = row.get("sanitizer_policy_version");
        let raw_event_json: String = row.get("raw_event_json");
        let event = match serde_json::from_str::<Event>(&raw_event_json) {
            Ok(event) => event,
            Err(error) => {
                return Ok(self.invalid_load(coordinate, cached_policy_version, error.to_string()))
            }
        };
        let parsed = match parse_store_page_event(&event) {
            Ok(parsed) => parsed,
            Err(error) => {
                return Ok(self.invalid_load(coordinate, cached_policy_version, error.to_string()))
            }
        };
        let parsed_coordinate = store_page_coordinate(parsed.event.pubkey, &parsed.presentation_id);
        if parsed_coordinate != coordinate
            || row.get::<String, _>("event_id") != parsed.event.id.to_hex()
            || row.get::<String, _>("publisher_pubkey") != parsed.event.pubkey.to_hex()
            || row.get::<String, _>("d_tag") != parsed.presentation_id
            || row.get::<i64, _>("created_at") < 0
            || row.get::<i64, _>("created_at") as u64 != parsed.event.created_at.as_secs()
        {
            return Ok(StorePageCacheLookup::InvalidCachedEvent {
                coordinate: coordinate.to_string(),
                reason: "cached metadata does not match signed event".to_string(),
            });
        }

        let sanitized_json = serde_json::to_string(parsed.sanitized_content())?;
        let diagnostics_json = serde_json::to_string(&parsed.diagnostics)?;
        let policy_recomputed = cached_policy_version
            != i64::from(STORE_PAGE_CONTENT_POLICY_VERSION)
            || row.get::<String, _>("sanitized_content_json") != sanitized_json
            || row.get::<String, _>("diagnostics_json") != diagnostics_json;
        if policy_recomputed {
            self.rewrite_policy_data(coordinate, &parsed).await?;
        }

        Ok(StorePageCacheLookup::Current(StorePageCacheEntry {
            coordinate: coordinate.to_string(),
            parsed,
            sanitizer_policy_version: STORE_PAGE_CONTENT_POLICY_VERSION,
            policy_recomputed,
        }))
    }

    async fn write_valid(
        &self,
        coordinate: &str,
        parsed: &ParsedStorePage,
    ) -> Result<bool, StorePageRepositoryError> {
        let created_at = i64::try_from(parsed.event.created_at.as_secs())
            .map_err(|_| StorePageRepositoryError::InvalidTimestamp)?;
        let raw_event_json = serde_json::to_string(&parsed.event)?;
        let sanitized_content_json = serde_json::to_string(parsed.sanitized_content())?;
        let diagnostics_json = serde_json::to_string(&parsed.diagnostics)?;
        let result = sqlx::query(
            r#"
            INSERT INTO store_pages (
                store_page_coordinate, event_id, publisher_pubkey, d_tag, created_at,
                raw_event_json, schema_version, sanitizer_policy_version,
                sanitized_content_json, diagnostics_json, updated_at
            ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
            ON CONFLICT(store_page_coordinate) DO UPDATE SET
                event_id = excluded.event_id,
                publisher_pubkey = excluded.publisher_pubkey,
                d_tag = excluded.d_tag,
                created_at = excluded.created_at,
                raw_event_json = excluded.raw_event_json,
                schema_version = excluded.schema_version,
                sanitizer_policy_version = excluded.sanitizer_policy_version,
                sanitized_content_json = excluded.sanitized_content_json,
                diagnostics_json = excluded.diagnostics_json,
                updated_at = excluded.updated_at
            WHERE excluded.created_at > store_pages.created_at
               OR (
                    excluded.created_at = store_pages.created_at
                    AND excluded.event_id < store_pages.event_id
               )
            "#,
        )
        .bind(coordinate)
        .bind(parsed.event.id.to_hex())
        .bind(parsed.event.pubkey.to_hex())
        .bind(&parsed.presentation_id)
        .bind(created_at)
        .bind(raw_event_json)
        .bind(STORE_PAGE_SCHEMA_VERSION as i64)
        .bind(i64::from(STORE_PAGE_CONTENT_POLICY_VERSION))
        .bind(sanitized_content_json)
        .bind(diagnostics_json)
        .bind(now_secs())
        .execute(&self.pool)
        .await?
        .rows_affected();
        Ok(result > 0)
    }

    async fn rewrite_policy_data(
        &self,
        coordinate: &str,
        parsed: &ParsedStorePage,
    ) -> Result<(), StorePageRepositoryError> {
        let sanitized_content_json = serde_json::to_string(parsed.sanitized_content())?;
        let diagnostics_json = serde_json::to_string(&parsed.diagnostics)?;
        sqlx::query(
            r#"
            UPDATE store_pages
            SET schema_version = ?, sanitizer_policy_version = ?,
                sanitized_content_json = ?, diagnostics_json = ?, updated_at = ?
            WHERE store_page_coordinate = ? AND event_id = ?
            "#,
        )
        .bind(STORE_PAGE_SCHEMA_VERSION as i64)
        .bind(i64::from(STORE_PAGE_CONTENT_POLICY_VERSION))
        .bind(sanitized_content_json)
        .bind(diagnostics_json)
        .bind(now_secs())
        .bind(coordinate)
        .bind(parsed.event.id.to_hex())
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    fn invalid_load(
        &self,
        coordinate: &str,
        cached_policy_version: i64,
        reason: String,
    ) -> StorePageCacheLookup {
        if cached_policy_version != i64::from(STORE_PAGE_CONTENT_POLICY_VERSION) {
            StorePageCacheLookup::StalePolicy {
                coordinate: coordinate.to_string(),
                cached_policy_version: u32::try_from(cached_policy_version).unwrap_or_default(),
                reason,
            }
        } else {
            StorePageCacheLookup::InvalidCachedEvent {
                coordinate: coordinate.to_string(),
                reason,
            }
        }
    }
}

fn now_secs() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .ok()
        .and_then(|duration| i64::try_from(duration.as_secs()).ok())
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use nostr::{EventBuilder, Keys, Kind, Tag, Timestamp};
    use tempfile::TempDir;

    use super::*;
    use crate::storage::Database;
    use crate::store_page::{
        build_store_page_event_builder, StorePageBasic, StorePageBuildParams, StorePageCompactTags,
        StorePageContentV1,
    };

    async fn repository() -> (TempDir, StorePageRepository) {
        let directory = TempDir::new().expect("temp directory");
        let database = Database::new(&directory.path().join("store-pages.db"))
            .await
            .expect("database");
        let repository = StorePageRepository::new(database.pool().clone());
        (directory, repository)
    }

    fn parsed_page(keys: &Keys, title: &str, created_at: u64) -> ParsedStorePage {
        let listing = format!("30402:{}:game", keys.public_key().to_hex());
        let content = StorePageContentV1 {
            basic: StorePageBasic {
                title: Some(title.to_string()),
                ..StorePageBasic::default()
            },
            ..StorePageContentV1::default()
        };
        let event = build_store_page_event_builder(&StorePageBuildParams {
            publisher: keys.public_key(),
            presentation_id: "page".to_string(),
            listing_coordinates: vec![listing],
            content,
            compact_tags: StorePageCompactTags::default(),
        })
        .expect("builder")
        .custom_created_at(Timestamp::from(created_at))
        .sign_with_keys(keys)
        .expect("signed page");
        parse_store_page_event(&event).expect("parsed page")
    }

    #[tokio::test]
    async fn signed_listing_round_trips_for_offline_association_validation() {
        let (_directory, repository) = repository().await;
        let keys = Keys::generate();
        let listing = EventBuilder::new(Kind::Custom(30402), "listing")
            .tags([Tag::parse(["d", "game"]).expect("d tag")])
            .sign_with_keys(&keys)
            .expect("signed listing");
        let coordinate = listing_coordinate(&listing).expect("coordinate");

        assert_eq!(
            repository
                .upsert_listing_event(&listing)
                .await
                .expect("store listing"),
            StorePageCacheUpsertOutcome::Stored
        );
        assert_eq!(
            repository
                .load_listing_event(&coordinate)
                .await
                .expect("load listing")
                .map(|event| event.id),
            Some(listing.id)
        );
    }

    #[tokio::test]
    async fn valid_newer_replaces_valid_cache_and_older_does_not() {
        let (_directory, repository) = repository().await;
        let keys = Keys::generate();
        let older = parsed_page(&keys, "Older", 10);
        let newer = parsed_page(&keys, "Newer", 20);
        assert_eq!(
            repository.upsert_valid(&older).await.expect("insert"),
            StorePageCacheUpsertOutcome::Stored
        );
        assert_eq!(
            repository.upsert_valid(&newer).await.expect("update"),
            StorePageCacheUpsertOutcome::Stored
        );
        assert_eq!(
            repository.upsert_valid(&older).await.expect("stale"),
            StorePageCacheUpsertOutcome::Unchanged
        );
        let coordinate = store_page_coordinate(keys.public_key(), "page");
        let StorePageCacheLookup::Current(entry) =
            repository.load(&coordinate).await.expect("load")
        else {
            panic!("expected current entry");
        };
        assert_eq!(entry.parsed.event.id, newer.event.id);
    }

    #[tokio::test]
    async fn invalid_newer_candidate_cannot_erase_valid_cache() {
        let (_directory, repository) = repository().await;
        let keys = Keys::generate();
        let valid = parsed_page(&keys, "Valid", 10);
        repository.upsert_valid(&valid).await.expect("insert");

        let mut unsupported = valid.content.clone();
        unsupported.version += 1;
        let invalid = build_store_page_event_builder(&StorePageBuildParams {
            publisher: keys.public_key(),
            presentation_id: "page".to_string(),
            listing_coordinates: valid.listing_coordinates.clone(),
            content: StorePageContentV1::default(),
            compact_tags: StorePageCompactTags::default(),
        })
        .expect("builder")
        .custom_created_at(Timestamp::from(20))
        .sign_with_keys(&keys)
        .expect("signed event");
        let mut invalid_json: serde_json::Value =
            serde_json::from_str(&invalid.content).expect("content JSON");
        invalid_json["version"] = serde_json::json!(unsupported.version);
        let invalid = nostr::EventBuilder::new(invalid.kind, invalid_json.to_string())
            .tags(invalid.tags)
            .custom_created_at(Timestamp::from(20))
            .sign_with_keys(&keys)
            .expect("invalid-schema signed event");
        assert!(parse_store_page_event(&invalid).is_err());

        let coordinate = store_page_coordinate(keys.public_key(), "page");
        let StorePageCacheLookup::Current(entry) =
            repository.load(&coordinate).await.expect("load")
        else {
            panic!("expected current entry");
        };
        assert_eq!(entry.parsed.event.id, valid.event.id);
    }

    #[tokio::test]
    async fn equal_timestamp_uses_lower_event_id() {
        let (_directory, repository) = repository().await;
        let keys = Keys::generate();
        let first = parsed_page(&keys, "First", 10);
        let second = parsed_page(&keys, "Second", 10);
        repository.upsert_valid(&first).await.expect("first");
        repository.upsert_valid(&second).await.expect("second");
        let expected = if first.event.id.to_hex() < second.event.id.to_hex() {
            first.event.id
        } else {
            second.event.id
        };
        let coordinate = store_page_coordinate(keys.public_key(), "page");
        let StorePageCacheLookup::Current(entry) =
            repository.load(&coordinate).await.expect("load")
        else {
            panic!("expected current entry");
        };
        assert_eq!(entry.parsed.event.id, expected);
    }

    #[tokio::test]
    async fn stale_policy_is_recomputed_from_signed_raw_event() {
        let (_directory, repository) = repository().await;
        let keys = Keys::generate();
        let parsed = parsed_page(&keys, "Safe", 10);
        repository.upsert_valid(&parsed).await.expect("insert");
        let coordinate = store_page_coordinate(keys.public_key(), "page");
        sqlx::query(
            "UPDATE store_pages SET sanitizer_policy_version = 0, sanitized_content_json = ? WHERE store_page_coordinate = ?",
        )
        .bind(r#"{"description_html":"<script>unsafe</script>"}"#)
        .bind(&coordinate)
        .execute(&repository.pool)
        .await
        .expect("mark stale");

        let StorePageCacheLookup::Current(entry) =
            repository.load(&coordinate).await.expect("load")
        else {
            panic!("expected recomputed entry");
        };
        assert!(entry.policy_recomputed);
        assert_eq!(
            entry.sanitizer_policy_version,
            STORE_PAGE_CONTENT_POLICY_VERSION
        );
        assert!(!entry
            .parsed
            .sanitized_content()
            .description_html
            .as_str()
            .contains("script"));
    }
}
