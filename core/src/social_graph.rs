// Social Graph Database - Stores "followed-by" relationships for extended network discovery
// Tracks which 1st-degree follows follow which 2nd-degree pubkeys

use rusqlite::Connection;
use std::collections::HashMap;
use std::path::Path;
use std::sync::Mutex;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum SocialGraphError {
    #[error("SQLite error: {0}")]
    Sqlite(#[from] rusqlite::Error),
    #[error("Lock error")]
    Lock,
}

/// Stores "followed-by" relationships for extended network discovery.
/// Tracks which 1st-degree follows follow which 2nd-degree pubkeys.
pub struct SocialGraphDb {
    conn: Mutex<Connection>,
}

impl SocialGraphDb {
    /// Create/open social graph database at path
    pub fn new<P: AsRef<Path>>(path: P) -> Result<Self, SocialGraphError> {
        let conn = Connection::open(path)?;

        // Create table for followed-by relationships
        conn.execute(
            "CREATE TABLE IF NOT EXISTS followed_by (
                target_pubkey TEXT NOT NULL,
                follower_pubkey TEXT NOT NULL,
                PRIMARY KEY (target_pubkey, follower_pubkey)
            )",
            [],
        )?;

        // Index for fast lookups
        conn.execute(
            "CREATE INDEX IF NOT EXISTS idx_target ON followed_by (target_pubkey)",
            [],
        )?;

        Ok(Self {
            conn: Mutex::new(conn),
        })
    }

    /// Insert a batch of followed-by relationships
    /// Each pair is (target_pubkey, follower_pubkey) meaning follower follows target
    pub fn insert_batch(&self, pairs: &[(String, String)]) -> Result<(), SocialGraphError> {
        if pairs.is_empty() {
            return Ok(());
        }

        let conn = self.conn.lock().map_err(|_| SocialGraphError::Lock)?;

        let tx = conn.unchecked_transaction()?;

        {
            let mut stmt = tx.prepare(
                "INSERT OR REPLACE INTO followed_by (target_pubkey, follower_pubkey) VALUES (?, ?)",
            )?;

            for (target, follower) in pairs {
                stmt.execute(rusqlite::params![target, follower])?;
            }
        }

        tx.commit()?;
        Ok(())
    }

    /// Get all followers (1st-degree) who follow the given target pubkey
    pub fn get_followers(&self, target_pubkey: &str) -> Result<Vec<String>, SocialGraphError> {
        let conn = self.conn.lock().map_err(|_| SocialGraphError::Lock)?;

        let mut stmt =
            conn.prepare("SELECT follower_pubkey FROM followed_by WHERE target_pubkey = ?")?;

        let followers: Result<Vec<String>, _> =
            stmt.query_map([target_pubkey], |row| row.get(0))?.collect();

        Ok(followers?)
    }

    /// Count how many followers each target pubkey has
    /// Returns map of target_pubkey -> follower_count
    pub fn count_followers(
        &self,
        target_pubkeys: &[String],
    ) -> Result<HashMap<String, i32>, SocialGraphError> {
        if target_pubkeys.is_empty() {
            return Ok(HashMap::new());
        }

        let conn = self.conn.lock().map_err(|_| SocialGraphError::Lock)?;

        let placeholders: Vec<&str> = target_pubkeys.iter().map(|_| "?").collect();
        let query = format!(
            "SELECT target_pubkey, COUNT(follower_pubkey) as count 
             FROM followed_by 
             WHERE target_pubkey IN ({}) 
             GROUP BY target_pubkey",
            placeholders.join(",")
        );

        let mut stmt = conn.prepare(&query)?;
        let params: Vec<&dyn rusqlite::ToSql> = target_pubkeys
            .iter()
            .map(|p| p as &dyn rusqlite::ToSql)
            .collect();

        let counts: Result<HashMap<String, i32>, _> = stmt
            .query_map(rusqlite::params_from_iter(params.iter()), |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, i32>(1)?))
            })?
            .collect();

        Ok(counts?)
    }

    /// Clear all data (e.g., on logout or fresh discovery)
    pub fn clear_all(&self) -> Result<(), SocialGraphError> {
        let conn = self.conn.lock().map_err(|_| SocialGraphError::Lock)?;
        conn.execute("DELETE FROM followed_by", [])?;
        Ok(())
    }

    /// Get total relationship count
    pub fn get_relationship_count(&self) -> Result<i64, SocialGraphError> {
        let conn = self.conn.lock().map_err(|_| SocialGraphError::Lock)?;
        let count: i64 =
            conn.query_row("SELECT COUNT(*) FROM followed_by", [], |row| row.get(0))?;
        Ok(count)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::env;
    use std::fs;

    fn get_temp_db_path() -> std::path::PathBuf {
        let temp_dir = env::temp_dir();
        let unique_name = format!("test_social_graph_{}.db", std::process::id());
        temp_dir.join(unique_name)
    }

    fn cleanup_db(path: &std::path::Path) {
        let _ = fs::remove_file(path);
    }

    #[test]
    fn test_insert_and_query() {
        let db_path = get_temp_db_path();
        cleanup_db(&db_path);

        let db = SocialGraphDb::new(&db_path).unwrap();

        let pairs = vec![
            ("pubkey_a".to_string(), "pubkey_1".to_string()), // 1 follows a
            ("pubkey_a".to_string(), "pubkey_2".to_string()), // 2 follows a
            ("pubkey_b".to_string(), "pubkey_1".to_string()), // 1 follows b
        ];

        db.insert_batch(&pairs).unwrap();

        let followers_a = db.get_followers("pubkey_a").unwrap();
        assert_eq!(followers_a.len(), 2);
        assert!(followers_a.contains(&"pubkey_1".to_string()));
        assert!(followers_a.contains(&"pubkey_2".to_string()));

        let followers_b = db.get_followers("pubkey_b").unwrap();
        assert_eq!(followers_b.len(), 1);
        assert!(followers_b.contains(&"pubkey_1".to_string()));

        cleanup_db(&db_path);
    }

    #[test]
    fn test_count_followers() {
        let db_path = get_temp_db_path();
        cleanup_db(&db_path);

        let db = SocialGraphDb::new(&db_path).unwrap();

        let pairs: Vec<(String, String)> = (0..100)
            .map(|i| ("target".to_string(), format!("follower_{}", i)))
            .collect();

        db.insert_batch(&pairs).unwrap();

        let counts = db.count_followers(&["target".to_string()]).unwrap();
        assert_eq!(counts.get("target"), Some(&100));

        cleanup_db(&db_path);
    }

    #[test]
    fn test_count_followers_empty_input() {
        let db_path = get_temp_db_path();
        cleanup_db(&db_path);

        let db = SocialGraphDb::new(&db_path).unwrap();

        // Test with empty input - should return empty map without error
        let counts = db.count_followers(&[]).unwrap();
        assert!(counts.is_empty());

        cleanup_db(&db_path);
    }

    #[test]
    fn test_clear_all() {
        let db_path = get_temp_db_path();
        cleanup_db(&db_path);

        let db = SocialGraphDb::new(&db_path).unwrap();

        let pairs = vec![("a".to_string(), "1".to_string())];
        db.insert_batch(&pairs).unwrap();

        assert_eq!(db.get_relationship_count().unwrap(), 1);
        db.clear_all().unwrap();
        assert_eq!(db.get_relationship_count().unwrap(), 0);

        cleanup_db(&db_path);
    }

    #[test]
    fn test_insert_batch_empty() {
        let db_path = get_temp_db_path();
        cleanup_db(&db_path);

        let db = SocialGraphDb::new(&db_path).unwrap();

        // Empty batch should succeed without error
        let empty_pairs: Vec<(String, String)> = vec![];
        db.insert_batch(&empty_pairs).unwrap();

        assert_eq!(db.get_relationship_count().unwrap(), 0);

        cleanup_db(&db_path);
    }

    #[test]
    fn test_multiple_targets_count() {
        let db_path = get_temp_db_path();
        cleanup_db(&db_path);

        let db = SocialGraphDb::new(&db_path).unwrap();

        // Create relationships for multiple targets
        let pairs = vec![
            ("target_a".to_string(), "follower_1".to_string()),
            ("target_a".to_string(), "follower_2".to_string()),
            ("target_a".to_string(), "follower_3".to_string()),
            ("target_b".to_string(), "follower_1".to_string()),
            ("target_b".to_string(), "follower_2".to_string()),
            ("target_c".to_string(), "follower_1".to_string()),
        ];

        db.insert_batch(&pairs).unwrap();

        let counts = db
            .count_followers(&[
                "target_a".to_string(),
                "target_b".to_string(),
                "target_c".to_string(),
            ])
            .unwrap();

        assert_eq!(counts.get("target_a"), Some(&3));
        assert_eq!(counts.get("target_b"), Some(&2));
        assert_eq!(counts.get("target_c"), Some(&1));

        cleanup_db(&db_path);
    }
}
