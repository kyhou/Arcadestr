//! Migration from legacy saved_users.json to new secure SQLite storage

use crate::auth::{AccountManager, Account, SigningMode};
use crate::saved_users::{get_saved_users, SavedUser, LoginMethod};
use std::path::Path;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum MigrationError {
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
    #[error("JSON error: {0}")]
    Json(#[from] serde_json::Error),
    #[error("Account manager error: {0}")]
    AccountManager(#[from] crate::auth::AccountManagerError),
    #[error("No legacy data found")]
    NoLegacyData,
    #[error("Migration failed for user {user_id}: {reason}")]
    MigrationFailed { user_id: String, reason: String },
}

/// Result of migrating a single user
#[derive(Debug, Clone)]
pub struct MigrationResult {
    pub user_id: String,
    pub npub: String,
    pub status: MigrationStatus,
}

#[derive(Debug, Clone)]
pub enum MigrationStatus {
    /// Successfully migrated to new storage
    Success,
    /// Already exists in new storage
    AlreadyMigrated,
    /// Failed to migrate
    Failed(String),
}

/// Migrate all accounts from legacy saved_users.json to new SQLite database
/// 
/// This function:
/// 1. Reads the legacy saved_users.json file
/// 2. Converts each user to the new Account format
/// 3. Stores in the SQLite database
/// 4. Renames the legacy file to .legacy (preserves backup)
/// 
/// # Arguments
/// * `data_dir` - Data directory containing saved_users.json
/// * `account_manager` - The account manager to use for storage
/// 
/// # Returns
/// List of migration results for each user
pub async fn migrate_from_legacy(
    data_dir: &Path,
    account_manager: &AccountManager,
) -> Result<Vec<MigrationResult>, MigrationError> {
    let legacy_path = data_dir.join("saved_users.json");
    
    if !legacy_path.exists() {
        return Err(MigrationError::NoLegacyData);
    }
    
    tracing::info!("Starting migration from legacy storage: {}", legacy_path.display());
    
    // Load legacy data
    let legacy_data = tokio::fs::read_to_string(&legacy_path).await?;
    let legacy_users: Vec<SavedUser> = serde_json::from_str(&legacy_data)?;
    
    tracing::info!("Found {} legacy users to migrate", legacy_users.len());
    
    let mut results = Vec::new();
    
    for user in legacy_users {
        let result = migrate_user(&user, account_manager).await;
        results.push(result);
    }
    
    // Rename legacy file (don't delete for safety)
    let backup_path = data_dir.join("saved_users.json.legacy");
    tokio::fs::rename(&legacy_path, &backup_path).await?;
    
    let success_count = results.iter().filter(|r| matches!(r.status, MigrationStatus::Success)).count();
    let already_count = results.iter().filter(|r| matches!(r.status, MigrationStatus::AlreadyMigrated)).count();
    let failed_count = results.iter().filter(|r| matches!(r.status, MigrationStatus::Failed(_))).count();
    
    tracing::info!(
        "Migration complete: {} succeeded, {} already migrated, {} failed",
        success_count,
        already_count,
        failed_count
    );
    
    Ok(results)
}

/// Migrate a single user from legacy format
async fn migrate_user(
    user: &SavedUser,
    account_manager: &AccountManager,
) -> MigrationResult {
    // Check if already migrated
    match account_manager.get_account_by_npub(&user.npub).await {
        Ok(Some(_)) => {
            return MigrationResult {
                user_id: user.id.clone(),
                npub: user.npub.clone(),
                status: MigrationStatus::AlreadyMigrated,
            };
        }
        Ok(None) => {} // Continue with migration
        Err(e) => {
            return MigrationResult {
                user_id: user.id.clone(),
                npub: user.npub.clone(),
                status: MigrationStatus::Failed(format!("Database check failed: {}", e)),
            };
        }
    }
    
    match user.method {
        LoginMethod::DirectKey => {
            // DirectKey with nsec - migrate to Local signing mode
            if let Some(nsec) = &user.nsec {
                match account_manager.login_with_nsec(nsec).await {
                    Ok(account) => {
                        // Update with saved metadata
                        if let Err(e) = account_manager.update_profile(
                            &account.id,
                            user.display_name.clone(),
                            user.picture.clone(),
                        ).await {
                            tracing::warn!("Failed to update profile for {}: {}", user.id, e);
                        }
                        
                        MigrationResult {
                            user_id: user.id.clone(),
                            npub: account.npub,
                            status: MigrationStatus::Success,
                        }
                    }
                    Err(e) => MigrationResult {
                        user_id: user.id.clone(),
                        npub: user.npub.clone(),
                        status: MigrationStatus::Failed(format!("Login failed: {}", e)),
                    },
                }
            } else {
                MigrationResult {
                    user_id: user.id.clone(),
                    npub: user.npub.clone(),
                    status: MigrationStatus::Failed("No nsec found for DirectKey user".to_string()),
                }
            }
        }
        LoginMethod::Nostrconnect | LoginMethod::Bunker => {
            // NIP-46 accounts - migrate to Remote signing mode
            let now = chrono::Utc::now().timestamp();
            let account = Account {
                id: user.id.clone(),
                pubkey: user.pubkey.clone(),
                npub: user.npub.clone(),
                signing_mode: SigningMode::Remote,
                encrypted_nsec: None,
                display_name: user.display_name.clone(),
                picture: user.picture.clone(),
                created_at: user.created_at,
                last_used: user.last_used,
                is_active: user.is_active,
            };
            
            match account_manager.save_account(&account).await {
                Ok(_) => {
                    // Store URI separately for reconnection
                    if let Some(uri) = &user.uri {
                        if let Err(e) = store_remote_uri(account_manager, &user.id, uri, &user.client_key.unwrap_or_default()).await {
                            tracing::warn!("Failed to store remote URI for {}: {}", user.id, e);
                        }
                    }
                    
                    MigrationResult {
                        user_id: user.id.clone(),
                        npub: user.npub.clone(),
                        status: MigrationStatus::Success,
                    }
                }
                Err(e) => MigrationResult {
                    user_id: user.id.clone(),
                    npub: user.npub.clone(),
                    status: MigrationStatus::Failed(format!("Save failed: {}", e)),
                },
            }
        }
    }
}

/// Store remote URI for NIP-46 accounts
async fn store_remote_uri(
    account_manager: &AccountManager,
    account_id: &str,
    uri: &str,
    client_key: &str,
) -> Result<(), sqlx::Error> {
    let now = chrono::Utc::now().timestamp();
    
    sqlx::query(
        "INSERT INTO remote_uris (account_id, uri, client_key, updated_at)
         VALUES (?, ?, ?, ?)
         ON CONFLICT(account_id) DO UPDATE SET
         uri = excluded.uri,
         client_key = excluded.client_key,
         updated_at = excluded.updated_at"
    )
    .bind(account_id)
    .bind(uri)
    .bind(client_key)
    .bind(now)
    .execute(account_manager.db_pool())
    .await?;
    
    Ok(())
}

/// Check if migration is needed
pub fn migration_needed(data_dir: &Path) -> bool {
    data_dir.join("saved_users.json").exists()
}

/// Get migration status summary
pub async fn get_migration_status(
    data_dir: &Path,
    account_manager: &AccountManager,
) -> Result<MigrationSummary, MigrationError> {
    let legacy_path = data_dir.join("saved_users.json");
    
    if !legacy_path.exists() {
        return Ok(MigrationSummary {
            legacy_exists: false,
            legacy_count: 0,
            migrated_count: 0,
            pending_count: 0,
        });
    }
    
    let legacy_data = tokio::fs::read_to_string(&legacy_path).await?;
    let legacy_users: Vec<SavedUser> = serde_json::from_str(&legacy_data)?;
    
    let mut migrated = 0;
    let mut pending = 0;
    
    for user in &legacy_users {
        match account_manager.get_account_by_npub(&user.npub).await {
            Ok(Some(_)) => migrated += 1,
            Ok(None) => pending += 1,
            Err(_) => pending += 1, // Count as pending if check fails
        }
    }
    
    Ok(MigrationSummary {
        legacy_exists: true,
        legacy_count: legacy_users.len(),
        migrated_count: migrated,
        pending_count: pending,
    })
}

/// Summary of migration status
#[derive(Debug, Clone)]
pub struct MigrationSummary {
    pub legacy_exists: bool,
    pub legacy_count: usize,
    pub migrated_count: usize,
    pub pending_count: usize,
}

impl MigrationSummary {
    pub fn is_complete(&self) -> bool {
        self.legacy_exists && self.pending_count == 0
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;
    
    #[tokio::test]
    async fn test_migration_needed() {
        let temp_dir = TempDir::new().unwrap();
        
        // No legacy file - should not need migration
        assert!(!migration_needed(temp_dir.path()));
        
        // Create legacy file
        let legacy_file = temp_dir.path().join("saved_users.json");
        tokio::fs::write(&legacy_file, "[]").await.unwrap();
        
        // Now it should need migration
        assert!(migration_needed(temp_dir.path()));
    }
}
