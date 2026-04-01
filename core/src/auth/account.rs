use serde::{Deserialize, Serialize};
use sqlx::FromRow;

/// User account with signing capabilities
#[derive(Debug, Clone, FromRow, Serialize, Deserialize)]
pub struct Account {
    pub id: String,
    pub pubkey: String,
    pub npub: String,
    pub signing_mode: SigningMode,
    pub encrypted_nsec: Option<Vec<u8>>,
    pub display_name: Option<String>,
    pub picture: Option<String>,
    pub created_at: i64,
    pub last_used: i64,
    pub is_active: bool,
}

/// Signing mode for an account
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, sqlx::Type)]
#[sqlx(rename_all = "PascalCase")]
pub enum SigningMode {
    /// nsec stored locally, encrypted with AES-256-GCM
    Local,
    /// NIP-46 remote signer (Amber, etc.)
    Remote,
    /// npub only, no signing capability
    ReadOnly,
}

impl Account {
    /// Check if this account can sign events
    pub fn can_sign(&self) -> bool {
        matches!(self.signing_mode, SigningMode::Local | SigningMode::Remote)
    }

    /// Check if this is a local (fast) account
    pub fn is_local(&self) -> bool {
        self.signing_mode == SigningMode::Local
    }

    /// Check if this is a remote (NIP-46) account
    pub fn is_remote(&self) -> bool {
        self.signing_mode == SigningMode::Remote
    }

    /// Check if this is a read-only account
    pub fn is_readonly(&self) -> bool {
        self.signing_mode == SigningMode::ReadOnly
    }

    /// Get a display name for the account (falls back to truncated npub)
    pub fn display_name_or_npub(&self) -> String {
        self.display_name.clone().unwrap_or_else(|| {
            // Return truncated npub like "npub1abc...xyz"
            if self.npub.len() > 20 {
                format!(
                    "{}...{}",
                    &self.npub[..10],
                    &self.npub[self.npub.len() - 10..]
                )
            } else {
                self.npub.clone()
            }
        })
    }
}

/// Account information for UI display (without sensitive data)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AccountInfo {
    pub id: String,
    pub npub: String,
    pub display_name: Option<String>,
    pub picture: Option<String>,
    pub signing_mode: SigningMode,
    pub is_active: bool,
    pub last_used: i64,
}

impl From<Account> for AccountInfo {
    fn from(account: Account) -> Self {
        Self {
            id: account.id,
            npub: account.npub,
            display_name: account.display_name,
            picture: account.picture,
            signing_mode: account.signing_mode,
            is_active: account.is_active,
            last_used: account.last_used,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_account_signing_modes() {
        let local_account = Account {
            id: "test".to_string(),
            pubkey: "abc".to_string(),
            npub: "npub1test".to_string(),
            signing_mode: SigningMode::Local,
            encrypted_nsec: Some(vec![1, 2, 3]),
            display_name: None,
            picture: None,
            created_at: 0,
            last_used: 0,
            is_active: true,
        };

        assert!(local_account.can_sign());
        assert!(local_account.is_local());
        assert!(!local_account.is_remote());
        assert!(!local_account.is_readonly());

        let readonly_account = Account {
            signing_mode: SigningMode::ReadOnly,
            ..local_account.clone()
        };

        assert!(!readonly_account.can_sign());
        assert!(readonly_account.is_readonly());
    }

    #[test]
    fn test_display_name_fallback() {
        let account = Account {
            id: "test".to_string(),
            pubkey: "abc".to_string(),
            npub: "npub1abcdefghijklmnopqrstuvwxyz".to_string(),
            signing_mode: SigningMode::Local,
            encrypted_nsec: None,
            display_name: None,
            picture: None,
            created_at: 0,
            last_used: 0,
            is_active: true,
        };

        let display = account.display_name_or_npub();
        assert!(display.starts_with("npub1"));
        assert!(display.contains("..."));

        let account_with_name = Account {
            display_name: Some("Test User".to_string()),
            ..account
        };

        assert_eq!(account_with_name.display_name_or_npub(), "Test User");
    }
}
