// Web authentication module for NIP-07 browser extension support.
// This module is only compiled for the WASM web target.

#![cfg(all(target_arch = "wasm32", feature = "web"))]

use arcadestr_core::signers::{Nip07Signer, NostrSigner};
use nostr::nips::nip19::ToBech32;
use serde::{Deserialize, Serialize};
use std::cell::RefCell;
use web_sys::window;

const WEB_ACCOUNTS_KEY: &str = "arcadestr.web.accounts.v1";

#[derive(Debug, Clone, Serialize, Deserialize)]
struct WebAccount {
    id: String,
    npub: String,
    name: Option<String>,
    signing_mode: String,
    last_used: i64,
    is_current: bool,
}

#[derive(Debug, Default, Clone, Serialize, Deserialize)]
struct WebAccountStore {
    accounts: Vec<WebAccount>,
}

thread_local! {
    static WEB_NPUB: RefCell<Option<String>> = const { RefCell::new(None) };
}

fn now_unix() -> i64 {
    (js_sys::Date::now() / 1000.0) as i64
}

fn read_store() -> WebAccountStore {
    let win = match window() {
        Some(win) => win,
        None => return WebAccountStore::default(),
    };

    let storage = match win.local_storage() {
        Ok(Some(storage)) => storage,
        _ => return WebAccountStore::default(),
    };

    let raw = match storage.get_item(WEB_ACCOUNTS_KEY) {
        Ok(Some(raw)) => raw,
        _ => return WebAccountStore::default(),
    };

    serde_json::from_str(&raw).unwrap_or_default()
}

fn write_store(store: &WebAccountStore) -> Result<(), String> {
    let win = window().ok_or("window not available".to_string())?;
    let storage = win
        .local_storage()
        .map_err(|_| "localStorage unavailable".to_string())?
        .ok_or("localStorage unavailable".to_string())?;

    let raw = serde_json::to_string(store).map_err(|e| e.to_string())?;
    storage
        .set_item(WEB_ACCOUNTS_KEY, &raw)
        .map_err(|_| "failed to persist web accounts".to_string())
}

fn account_to_value(account: &WebAccount) -> serde_json::Value {
    serde_json::json!({
        "id": account.id,
        "npub": account.npub,
        "name": account.name,
        "signing_mode": account.signing_mode,
        "last_used": account.last_used,
        "is_current": account.is_current,
        "picture": serde_json::Value::Null,
        "display_name": serde_json::Value::Null,
        "username": serde_json::Value::Null,
        "nip05": serde_json::Value::Null,
        "about": serde_json::Value::Null,
    })
}

pub fn web_upsert_account(
    id: String,
    npub: String,
    name: Option<String>,
    signing_mode: String,
) -> Result<serde_json::Value, String> {
    let mut store = read_store();
    let now = now_unix();
    let mut updated = false;

    for account in &mut store.accounts {
        if account.id == id {
            account.npub = npub.clone();
            account.name = name.clone();
            account.signing_mode = signing_mode.clone();
            account.last_used = now;
            account.is_current = true;
            updated = true;
        } else {
            account.is_current = false;
        }
    }

    if !updated {
        store.accounts.push(WebAccount {
            id: id.clone(),
            npub: npub.clone(),
            name: name.clone(),
            signing_mode,
            last_used: now,
            is_current: true,
        });
    }

    write_store(&store)?;

    WEB_NPUB.with(|cell| {
        *cell.borrow_mut() = Some(npub);
    });

    let account = store
        .accounts
        .iter()
        .find(|acc| acc.id == id)
        .ok_or("Account upsert failed".to_string())?;

    Ok(serde_json::json!({ "account": account_to_value(account) }))
}

pub fn web_has_accounts() -> Result<bool, String> {
    Ok(!read_store().accounts.is_empty())
}

pub fn web_list_saved_profiles() -> Result<serde_json::Value, String> {
    let store = read_store();
    let accounts: Vec<serde_json::Value> = store.accounts.iter().map(account_to_value).collect();
    Ok(serde_json::json!({ "accounts": accounts }))
}

pub fn web_load_active_account() -> Result<serde_json::Value, String> {
    let store = read_store();
    let account = store
        .accounts
        .iter()
        .find(|a| a.is_current)
        .ok_or("No active account".to_string())?;

    WEB_NPUB.with(|cell| {
        *cell.borrow_mut() = Some(account.npub.clone());
    });

    Ok(serde_json::json!({ "account": account_to_value(account) }))
}

pub fn web_switch_profile(profile_id: String) -> Result<serde_json::Value, String> {
    let mut store = read_store();
    let mut found = None;

    for account in &mut store.accounts {
        if account.id == profile_id {
            account.is_current = true;
            account.last_used = now_unix();
            found = Some(account.clone());
        } else {
            account.is_current = false;
        }
    }

    let account = found.ok_or("Account not found".to_string())?;
    write_store(&store)?;

    WEB_NPUB.with(|cell| {
        *cell.borrow_mut() = Some(account.npub.clone());
    });

    Ok(serde_json::json!({ "account": account_to_value(&account) }))
}

pub fn web_delete_profile(profile_id: String) -> Result<(), String> {
    let _ = crate::web_secure_store::delete_nsec(profile_id.clone());

    let mut store = read_store();
    let previous = store.accounts.len();
    store.accounts.retain(|a| a.id != profile_id);

    if store.accounts.len() == previous {
        return Err("Account not found".to_string());
    }

    if !store.accounts.iter().any(|a| a.is_current) {
        if let Some(first) = store.accounts.first_mut() {
            first.is_current = true;
            WEB_NPUB.with(|cell| {
                *cell.borrow_mut() = Some(first.npub.clone());
            });
        } else {
            WEB_NPUB.with(|cell| {
                *cell.borrow_mut() = None;
            });
        }
    }

    write_store(&store)
}

fn web_upsert_nip07_account(npub: String) -> Result<(), String> {
    let id = format!("web_{npub}");
    web_upsert_account(id, npub, None, "nip07".to_string()).map(|_| ())
}

/// Connect via NIP-07 browser extension.
/// Returns the user's npub bech32 string on success.
pub async fn web_connect_nip07() -> Result<String, String> {
    let signer = Nip07Signer::new();
    let pubkey = signer.get_public_key().await.map_err(|e| e.to_string())?;
    let npub = pubkey
        .to_bech32()
        .map_err(|e| format!("Failed to encode npub: {e}"))?;

    WEB_NPUB.with(|cell| {
        *cell.borrow_mut() = Some(npub.clone());
    });

    web_upsert_nip07_account(npub.clone())?;

    Ok(npub)
}

/// Get the current public key if authenticated.
pub fn web_get_public_key() -> Option<String> {
    WEB_NPUB.with(|cell| cell.borrow().clone())
}

/// Check if the user is currently authenticated.
pub fn web_is_authenticated() -> bool {
    WEB_NPUB.with(|cell| cell.borrow().is_some())
}

/// Disconnect the current user.
pub fn web_disconnect() {
    WEB_NPUB.with(|cell| {
        *cell.borrow_mut() = None;
    });
}

pub fn web_logout_active_account() -> Result<(), String> {
    let mut store = read_store();
    for account in &mut store.accounts {
        account.is_current = false;
    }

    write_store(&store)?;
    web_disconnect();
    Ok(())
}
