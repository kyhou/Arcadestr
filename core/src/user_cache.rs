//! User profile cache for persistent storage of fetched profiles.
//! Mirrors YakiHonne's Dexie users table functionality.

use std::time::{SystemTime, UNIX_EPOCH};
use sqlx::{Pool, Sqlite, Row};
use crate::nostr::UserProfile;

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
            "#
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
            "#
        )
        .bind(npub)
        .bind(&profile.name)
        .bind(&profile.display_name)
        .bind(&profile.picture)
        .bind(&profile.about)
        .bind(&profile.nip05)
        .bind(&profile.lud16)
        .bind(&profile.website)
        .bind(now)  // created_at - use current time since UserProfile doesn't have this field
        .bind(now)  // updated_at
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
                "#
            )
            .bind(npub)
            .bind(&profile.name)
            .bind(&profile.display_name)
            .bind(&profile.picture)
            .bind(&profile.about)
            .bind(&profile.nip05)
            .bind(&profile.lud16)
            .bind(&profile.website)
            .bind(now)  // created_at - use current time since UserProfile doesn't have this field
            .bind(now)  // updated_at
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
            "#
        )
        .bind(now)
        .fetch_all(&self.db)
        .await?;
        
        Ok(rows.into_iter().map(|r| UserProfile {
            npub: r.get("npub"),
            name: r.get("name"),
            display_name: r.get("display_name"),
            picture: r.get("picture"),
            about: r.get("about"),
            nip05: r.get("nip05"),
            lud16: r.get("lud16"),
            website: r.get("website"),
            nip05_verified: false,
        }).collect())
    }

    /// Check if profile exists and is fresh
    pub async fn is_fresh(&self, npub: &str) -> bool {
        self.get(npub).await.is_some()
    }

    /// Delete expired profiles
    pub async fn cleanup_expired(&self) -> Result<u64, sqlx::Error> {
        let now = Self::now();
        
        let result = sqlx::query(
            "DELETE FROM users WHERE expires_at <= ?"
        )
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
    
    // Tests would need actual DB - integration tests in tests/ dir
}
