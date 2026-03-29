// Saved users management: persist user login info for quick reconnection.

use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;
use tracing::{debug, error, info};

/// Method used for authentication
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum LoginMethod {
    /// NIP-46 nostrconnect:// flow (client-initiated)
    Nostrconnect,
    /// NIP-46 bunker:// flow (signer-initiated)
    Bunker,
    /// Direct private key (for testing)
    DirectKey,
}

impl std::fmt::Display for LoginMethod {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            LoginMethod::Nostrconnect => write!(f, "nostrconnect"),
            LoginMethod::Bunker => write!(f, "bunker"),
            LoginMethod::DirectKey => write!(f, "direct_key"),
        }
    }
}

/// A saved user/login entry
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SavedUser {
    /// Unique identifier
    pub id: String,
    /// User-friendly name (optional, auto-generated if not provided)
    pub name: String,
    /// Login method used
    pub method: LoginMethod,
    /// Relay URL (for nostrconnect/bunker)
    pub relay: Option<String>,
    /// The URI (for bunker/nostrconnect) - can be None if we just reconnect with keys
    pub uri: Option<String>,
    /// Private key (for direct_key method, stored encrypted in production)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub private_key: Option<String>,
    /// The user's public key (npub)
    pub npub: String,
    /// When this user was first added
    pub created_at: i64,
    /// Last time this user was used to login
    pub last_used_at: Option<i64>,
}

/// Saved users storage
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct SavedUsers {
    pub users: Vec<SavedUser>,
}

/// Directory for storing user data.
/// Set dynamically at runtime via `set_users_dir()`.
static USERS_DIR: std::sync::OnceLock<Option<PathBuf>> = std::sync::OnceLock::new();

/// Sets the directory where user data will be stored.
pub fn set_users_dir(path: PathBuf) {
    let _ = USERS_DIR.set(Some(path));
}

/// Gets the path to the users data file.
fn get_users_path() -> Option<PathBuf> {
    USERS_DIR
        .get()
        .and_then(|dir| dir.as_ref().map(|p| p.join("saved_users.json")))
}

/// Loads all saved users from disk.
pub fn load_saved_users() -> Result<SavedUsers, String> {
    let path = match get_users_path() {
        Some(p) => p,
        None => {
            error!("Users directory not set");
            return Err("Users directory not set".to_string());
        }
    };

    if !path.exists() {
        debug!("No saved users file, returning empty list");
        return Ok(SavedUsers::default());
    }

    let content =
        fs::read_to_string(&path).map_err(|e| format!("Failed to read users file: {}", e))?;

    let users: SavedUsers =
        serde_json::from_str(&content).map_err(|e| format!("Failed to parse users file: {}", e))?;

    info!("Loaded {} saved users", users.users.len());
    Ok(users)
}

/// Saves all saved users to disk.
pub fn save_saved_users(users: &SavedUsers) -> Result<(), String> {
    let path = match get_users_path() {
        Some(p) => p,
        None => {
            error!("Users directory not set");
            return Err("Users directory not set".to_string());
        }
    };

    // Ensure parent directory exists
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .map_err(|e| format!("Failed to create users directory: {}", e))?;
    }

    let content = serde_json::to_string_pretty(users)
        .map_err(|e| format!("Failed to serialize users: {}", e))?;

    fs::write(&path, content).map_err(|e| format!("Failed to write users file: {}", e))?;

    info!("Saved {} users to {}", users.users.len(), path.display());
    Ok(())
}

/// Adds a new saved user.
pub fn add_saved_user(user: SavedUser) -> Result<SavedUsers, String> {
    let user_name = user.name.clone();
    let user_npub = user.npub.clone();
    info!("Adding saved user: {} ({})", user_name, user_npub);
    let mut users = load_saved_users()?;

    // Check if user with same npub already exists
    if users.users.iter().any(|u| u.npub == user_npub) {
        info!("User with npub {} already exists, skipping save", user_npub);
        return Ok(users); // Return existing users instead of error
    }

    users.users.push(user);
    save_saved_users(&users)?;
    info!("Successfully saved user: {} ({})", user_name, user_npub);
    Ok(users)
}

/// Updates an existing saved user.
pub fn update_saved_user(user: SavedUser) -> Result<SavedUsers, String> {
    let mut users = load_saved_users()?;

    if let Some(existing) = users.users.iter_mut().find(|u| u.id == user.id) {
        *existing = user;
        save_saved_users(&users)?;
        Ok(users)
    } else {
        Err("User not found".to_string())
    }
}

/// Removes a saved user by ID.
pub fn remove_saved_user(user_id: &str) -> Result<SavedUsers, String> {
    let mut users = load_saved_users()?;
    let initial_len = users.users.len();
    users.users.retain(|u| u.id != user_id);

    if users.users.len() == initial_len {
        return Err("User not found".to_string());
    }

    save_saved_users(&users)?;
    Ok(users)
}

/// Gets a saved user by ID.
pub fn get_saved_user(user_id: &str) -> Result<SavedUser, String> {
    let users = load_saved_users()?;
    users
        .users
        .into_iter()
        .find(|u| u.id == user_id)
        .ok_or_else(|| "User not found".to_string())
}

/// Updates the last_used_at timestamp for a user.
pub fn mark_user_as_used(user_id: &str) -> Result<(), String> {
    let mut users = load_saved_users()?;

    if let Some(user) = users.users.iter_mut().find(|u| u.id == user_id) {
        user.last_used_at = Some(chrono::Utc::now().timestamp());
        save_saved_users(&users)?;
        Ok(())
    } else {
        Err("User not found".to_string())
    }
}

/// Generates a unique ID for a new user.
fn generate_user_id() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_millis();
    format!("user_{}", timestamp)
}

/// Creates a new SavedUser from login info.
pub fn create_saved_user(
    method: LoginMethod,
    relay: Option<String>,
    uri: Option<String>,
    private_key: Option<String>,
    npub: &str,
) -> SavedUser {
    let name = match &method {
        LoginMethod::Nostrconnect => format!("NostrConnect User"),
        LoginMethod::Bunker => format!("Bunker User"),
        LoginMethod::DirectKey => format!("Direct Key User"),
    };

    SavedUser {
        id: generate_user_id(),
        name,
        method,
        relay,
        uri,
        private_key,
        npub: npub.to_string(),
        created_at: chrono::Utc::now().timestamp(),
        last_used_at: None,
    }
}
