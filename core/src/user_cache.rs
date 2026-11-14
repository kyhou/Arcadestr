//! User profile cache for persistent storage of fetched profiles.
//! Mirrors YakiHonne's Dexie users table functionality.

use crate::nostr::UserProfile;
use sqlx::{Pool, Row, Sqlite};
use std::time::{SystemTime, UNIX_EPOCH};

const DEFAULT_CACHE_TTL_SECONDS: i64 = 86400; // 24 hours, matching YakiHonne

pub struct UserCache {
    db: Pool<Sqlite>,
    ttl_seconds: i64,
}

impl UserCache {
    pub fn new(db: Pool<Sqlite>) -> Self {
        Self {
            db,
            ttl_seconds: DEFAULT_CACHE_TTL_SECONDS,
        }
    }

    /// Get a user profile from cache
    pub async fn get(&self, npub: &str) -> Option<UserProfile> {
        let row = sqlx::query(
            r#"
            SELECT npub, name, display_name, picture, about, 
                   nip05, lud16, website, created_at
            FROM users 
            WHERE npub = ?
            "#,
        )
        .bind(npub)
        .fetch_optional(&self.db)
        .await
        .ok()?;

        row.map(|r| UserProfile {
            npub: r.get("npub"),
            name: r.get("name"),
            display_name: r.get("display_name"),
            picture: r.get("picture"),
            about: r.get("about"),
            nip05: r.get("nip05"),
            lud16: r.get("lud16"),
            website: r.get("website"),
            nip05_verified: false, // Will be verified on fetch
        })
    }

    /// Save or update a user profile
    pub async fn put(&self, npub: &str, profile: &UserProfile) -> Result<(), sqlx::Error> {
        let now = Self::now();
        let expires = now + self.ttl_seconds;

        sqlx::query(
            r#"
            INSERT INTO users (npub, name, display_name, picture, about, 
                             nip05, lud16, website, created_at, updated_at, expires_at)
            VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
            ON CONFLICT(npub) DO UPDATE SET
                name = excluded.name,
                display_name = excluded.display_name,
                picture = excluded.picture,
                about = excluded.about,
                nip05 = excluded.nip05,
                lud16 = excluded.lud16,
                website = excluded.website,
                updated_at = excluded.updated_at,
                expires_at = excluded.expires_at
            WHERE NOT EXISTS (SELECT 1 FROM profile_metadata_events WHERE npub = excluded.npub)
            "#,
        )
        .bind(npub)
        .bind(&profile.name)
        .bind(&profile.display_name)
        .bind(&profile.picture)
        .bind(&profile.about)
        .bind(&profile.nip05)
        .bind(&profile.lud16)
        .bind(&profile.website)
        .bind(now) // created_at - use current time since UserProfile doesn't have this field
        .bind(now) // updated_at
        .bind(expires)
        .execute(&self.db)
        .await?;

        Ok(())
    }

    /// Save multiple profiles in a batch transaction
    pub async fn put_many(&self, profiles: &[(String, UserProfile)]) -> Result<(), sqlx::Error> {
        let mut tx = self.db.begin().await?;

        for (npub, profile) in profiles {
            let now = Self::now();
            let expires = now + self.ttl_seconds;

            sqlx::query(
                r#"
                INSERT INTO users (npub, name, display_name, picture, about, 
                                 nip05, lud16, website, created_at, updated_at, expires_at)
                VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
                ON CONFLICT(npub) DO UPDATE SET
                    name = excluded.name,
                    display_name = excluded.display_name,
                    picture = excluded.picture,
                    about = excluded.about,
                    nip05 = excluded.nip05,
                    lud16 = excluded.lud16,
                    website = excluded.website,
                    updated_at = excluded.updated_at,
                    expires_at = excluded.expires_at
                WHERE NOT EXISTS (SELECT 1 FROM profile_metadata_events WHERE npub = excluded.npub)
                "#,
            )
            .bind(npub)
            .bind(&profile.name)
            .bind(&profile.display_name)
            .bind(&profile.picture)
            .bind(&profile.about)
            .bind(&profile.nip05)
            .bind(&profile.lud16)
            .bind(&profile.website)
            .bind(now) // created_at - use current time since UserProfile doesn't have this field
            .bind(now) // updated_at
            .bind(expires)
            .execute(&mut *tx)
            .await?;
        }

        tx.commit().await?;
        Ok(())
    }

    /// Get all cached users
    pub async fn get_all(&self) -> Result<Vec<UserProfile>, sqlx::Error> {
        let rows = sqlx::query(
            r#"
            SELECT npub, name, display_name, picture, about, 
                   nip05, lud16, website, created_at
            FROM users 
            ORDER BY updated_at DESC
            "#,
        )
        .fetch_all(&self.db)
        .await?;

        Ok(rows
            .into_iter()
            .map(|r| UserProfile {
                npub: r.get("npub"),
                name: r.get("name"),
                display_name: r.get("display_name"),
                picture: r.get("picture"),
                about: r.get("about"),
                nip05: r.get("nip05"),
                lud16: r.get("lud16"),
                website: r.get("website"),
                nip05_verified: false,
            })
            .collect())
    }

    /// Check if profile exists and is fresh
    pub async fn is_fresh(&self, npub: &str) -> bool {
        sqlx::query_scalar::<_, bool>(
            "SELECT EXISTS(SELECT 1 FROM users WHERE npub = ? AND expires_at > ?)",
        )
        .bind(npub)
        .bind(Self::now())
        .fetch_one(&self.db)
        .await
        .unwrap_or(false)
    }

    /// Persist a validated signed event and its projection atomically. Arrival order
    /// and unsigned legacy/batch writes cannot roll a signed profile backwards.
    pub async fn put_event(&self, npub: &str, event: &nostr::Event) -> Result<bool, String> {
        let author = nostr::PublicKey::parse(npub).map_err(|error| error.to_string())?;
        let profile = crate::profile_metadata::validate_metadata_event(event, author)?;
        let raw = serde_json::to_string(event).map_err(|error| error.to_string())?;
        let timestamp = i64::try_from(event.created_at.as_secs())
            .map_err(|_| "Metadata timestamp exceeds storage range".to_string())?;
        let mut tx = self.db.begin().await.map_err(|error| error.to_string())?;
        let changed = sqlx::query(
            "INSERT INTO profile_metadata_events(npub,event_id,event_created_at,raw_event_json) VALUES(?,?,?,?)
             ON CONFLICT(npub) DO UPDATE SET event_id=excluded.event_id,
             event_created_at=excluded.event_created_at,raw_event_json=excluded.raw_event_json
             WHERE excluded.event_created_at > profile_metadata_events.event_created_at
             OR (excluded.event_created_at = profile_metadata_events.event_created_at
                 AND excluded.event_id < profile_metadata_events.event_id)",
        )
        .bind(npub).bind(event.id.to_hex()).bind(timestamp).bind(raw)
        .execute(&mut *tx).await.map_err(|error| error.to_string())?
        .rows_affected() > 0;
        if changed {
            let now = Self::now();
            sqlx::query(
                "INSERT INTO users(npub,name,display_name,picture,about,nip05,lud16,website,created_at,updated_at,expires_at)
                 VALUES(?,?,?,?,?,?,?,?,?,?,?) ON CONFLICT(npub) DO UPDATE SET
                 name=excluded.name,display_name=excluded.display_name,picture=excluded.picture,
                 about=excluded.about,nip05=excluded.nip05,lud16=excluded.lud16,website=excluded.website,
                 updated_at=excluded.updated_at,expires_at=excluded.expires_at",
            )
            .bind(npub).bind(profile.name).bind(profile.display_name).bind(profile.picture)
            .bind(profile.about).bind(profile.nip05).bind(profile.lud16).bind(profile.website)
            .bind(now).bind(now).bind(now + self.ttl_seconds)
            .execute(&mut *tx).await.map_err(|error| error.to_string())?;
        } else {
            // An identical event confirms freshness without changing its ordering.
            sqlx::query("UPDATE users SET expires_at = ? WHERE npub = ? AND EXISTS (SELECT 1 FROM profile_metadata_events p WHERE p.npub = users.npub AND p.event_id = ?)")
                .bind(Self::now() + self.ttl_seconds).bind(npub).bind(event.id.to_hex())
                .execute(&mut *tx).await.map_err(|error| error.to_string())?;
        }
        tx.commit().await.map_err(|error| error.to_string())?;
        Ok(changed)
    }

    /// Delete expired profiles
    pub async fn cleanup_expired(&self) -> Result<u64, sqlx::Error> {
        let now = Self::now();

        let result = sqlx::query("DELETE FROM users WHERE expires_at <= ? AND NOT EXISTS (SELECT 1 FROM profile_metadata_events p WHERE p.npub = users.npub) AND NOT EXISTS (SELECT 1 FROM accounts a WHERE a.npub = users.npub)")
            .bind(now)
            .execute(&self.db)
            .await?;

        Ok(result.rows_affected())
    }

    fn now() -> i64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_secs() as i64)
            .unwrap_or(0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::Database;
    use tempfile::TempDir;

    fn create_test_profile() -> UserProfile {
        UserProfile {
            npub: "npub1test123".to_string(),
            name: Some("testuser".to_string()),
            display_name: Some("Test User".to_string()),
            picture: Some("https://example.com/pic.jpg".to_string()),
            about: Some("Test bio".to_string()),
            nip05: Some("test@example.com".to_string()),
            lud16: None,
            website: Some("https://example.com".to_string()),
            nip05_verified: false,
        }
    }

    #[tokio::test]
    async fn get_returns_none_for_unknown_npub() {
        let temp_dir = TempDir::new().expect("temp dir should be created");
        let db_path = temp_dir.path().join("user_cache.db");
        let db = Database::new(&db_path)
            .await
            .expect("database should initialize");

        let cache = UserCache::new(db.pool().clone());
        let unknown = cache.get("npub1unknown").await;

        assert!(unknown.is_none());
    }

    #[tokio::test]
    async fn put_and_get_roundtrip() {
        let temp_dir = TempDir::new().expect("temp dir should be created");
        let db_path = temp_dir.path().join("user_cache.db");
        let db = Database::new(&db_path)
            .await
            .expect("database should initialize");

        let cache = UserCache::new(db.pool().clone());
        let profile = create_test_profile();

        cache
            .put(&profile.npub, &profile)
            .await
            .expect("put should succeed");

        let loaded = cache.get(&profile.npub).await;
        assert!(loaded.is_some());
        let loaded = loaded.expect("profile should be present");

        assert_eq!(loaded.npub, profile.npub);
        assert_eq!(loaded.name, profile.name);
        assert_eq!(loaded.display_name, profile.display_name);
        assert_eq!(loaded.picture, profile.picture);
        assert_eq!(loaded.about, profile.about);
        assert_eq!(loaded.nip05, profile.nip05);
        assert_eq!(loaded.lud16, profile.lud16);
        assert_eq!(loaded.website, profile.website);
    }

    #[tokio::test]
    async fn put_overwrites_stale_profile() {
        let temp_dir = TempDir::new().expect("temp dir should be created");
        let db_path = temp_dir.path().join("user_cache.db");
        let db = Database::new(&db_path)
            .await
            .expect("database should initialize");

        let cache = UserCache::new(db.pool().clone());

        let mut v1 = create_test_profile();
        v1.name = Some("old-name".to_string());

        cache
            .put(&v1.npub, &v1)
            .await
            .expect("first put should succeed");

        let mut v2 = v1.clone();
        v2.name = Some("new-name".to_string());
        v2.about = Some("new about".to_string());

        cache
            .put(&v2.npub, &v2)
            .await
            .expect("second put should succeed");

        let loaded = cache
            .get(&v2.npub)
            .await
            .expect("profile should still exist");

        assert_eq!(loaded.name, v2.name);
        assert_eq!(loaded.about, v2.about);
    }

    #[tokio::test]
    async fn expired_profile_remains_available_after_reopening_database() {
        let temp = TempDir::new().expect("temp directory");
        let path = temp.path().join("profiles.db");
        let db = Database::new(&path).await.expect("database");
        let cache = UserCache::new(db.pool().clone());
        let profile = create_test_profile();
        cache.put(&profile.npub, &profile).await.expect("save");
        sqlx::query("UPDATE users SET expires_at = 0")
            .execute(db.pool())
            .await
            .expect("expire");
        assert!(!cache.is_fresh(&profile.npub).await);
        drop(cache);
        db.pool().close().await;
        let reopened = Database::new(&path).await.expect("reopen");
        let cache = UserCache::new(reopened.pool().clone());
        assert_eq!(
            cache.get(&profile.npub).await.expect("saved").name,
            profile.name
        );
        assert_eq!(cache.get_all().await.expect("all").len(), 1);
    }

    #[tokio::test]
    async fn signed_metadata_cannot_be_rolled_back_or_overwritten_by_unsigned_profiles() {
        use nostr::{EventBuilder, Keys, Kind, Timestamp, ToBech32};
        let temp = TempDir::new().expect("temp directory");
        let path = temp.path().join("profiles.db");
        let db = Database::new(&path).await.expect("database");
        let cache = UserCache::new(db.pool().clone());
        let keys = Keys::generate();
        let npub = keys.public_key().to_bech32().expect("npub");
        let make = |name: &str, time| {
            EventBuilder::new(Kind::Metadata, format!(r#"{{"name":"{name}"}}"#))
                .custom_created_at(Timestamp::from(time))
                .sign_with_keys(&keys)
                .expect("signed event")
        };
        let latest = make("latest", 20);
        assert!(cache.put_event(&npub, &latest).await.expect("save event"));
        assert!(!cache
            .put_event(&npub, &make("old", 10))
            .await
            .expect("stale"));
        assert!(!cache.put_event(&npub, &latest).await.expect("duplicate"));
        let mut unsigned = create_test_profile();
        unsigned.npub = npub.clone();
        cache.put(&npub, &unsigned).await.expect("unsigned write");
        cache
            .put_many(&[(npub.clone(), unsigned)])
            .await
            .expect("unsigned batch");
        assert_eq!(
            cache.get(&npub).await.expect("profile").name.as_deref(),
            Some("latest")
        );
        let raw: String =
            sqlx::query_scalar("SELECT raw_event_json FROM profile_metadata_events WHERE npub = ?")
                .bind(&npub)
                .fetch_one(db.pool())
                .await
                .expect("raw event");
        assert_eq!(
            serde_json::from_str::<nostr::Event>(&raw).expect("event"),
            latest
        );
        sqlx::query("UPDATE users SET expires_at = 0")
            .execute(db.pool())
            .await
            .expect("expire");
        cache.cleanup_expired().await.expect("cleanup");
        assert!(cache.get(&npub).await.is_some());
        let wrong_author = Keys::generate()
            .public_key()
            .to_bech32()
            .expect("other npub");
        assert!(cache.put_event(&wrong_author, &latest).await.is_err());
        let malformed = EventBuilder::new(Kind::Metadata, "not json")
            .sign_with_keys(&keys)
            .expect("signed invalid content");
        assert!(cache.put_event(&npub, &malformed).await.is_err());
    }
}
