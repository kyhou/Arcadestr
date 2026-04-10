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
        let now = Self::now();

        let row = sqlx::query(
            r#"
            SELECT npub, name, display_name, picture, about, 
                   nip05, lud16, website, created_at
            FROM users 
            WHERE npub = ? AND expires_at > ?
            "#,
        )
        .bind(npub)
        .bind(now)
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
        let now = Self::now();

        let rows = sqlx::query(
            r#"
            SELECT npub, name, display_name, picture, about, 
                   nip05, lud16, website, created_at
            FROM users 
            WHERE expires_at > ?
            ORDER BY updated_at DESC
            "#,
        )
        .bind(now)
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
        self.get(npub).await.is_some()
    }

    /// Delete expired profiles
    pub async fn cleanup_expired(&self) -> Result<u64, sqlx::Error> {
        let now = Self::now();

        let result = sqlx::query("DELETE FROM users WHERE expires_at <= ?")
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
}
