// Browser secure store for encrypted nsec persistence in web mode.

#![cfg(all(target_arch = "wasm32", feature = "web"))]

use aes_gcm::aead::{Aead, KeyInit};
use aes_gcm::{Aes256Gcm, Nonce};
use base64::{engine::general_purpose::STANDARD as BASE64, Engine as _};
use getrandom::getrandom;
use serde::{Deserialize, Serialize};
use web_sys::Storage;

const WEB_SECRET_KEY: &str = "arcadestr.web.secrets.key.v1";
const WEB_SECRETS_KEY: &str = "arcadestr.web.secrets.nsec.v1";

#[derive(Debug, Clone, Serialize, Deserialize)]
struct EncryptedSecret {
    account_id: String,
    nonce_b64: String,
    ciphertext_b64: String,
}

#[derive(Debug, Default, Clone, Serialize, Deserialize)]
struct SecretStore {
    items: Vec<EncryptedSecret>,
}

fn local_storage() -> Result<Storage, String> {
    let window = web_sys::window().ok_or("window not available".to_string())?;
    window
        .local_storage()
        .map_err(|_| "localStorage unavailable".to_string())?
        .ok_or("localStorage unavailable".to_string())
}

fn load_store(storage: &Storage) -> SecretStore {
    match storage.get_item(WEB_SECRETS_KEY) {
        Ok(Some(raw)) => serde_json::from_str(&raw).unwrap_or_default(),
        _ => SecretStore::default(),
    }
}

fn save_store(storage: &Storage, store: &SecretStore) -> Result<(), String> {
    let raw = serde_json::to_string(store).map_err(|e| e.to_string())?;
    storage
        .set_item(WEB_SECRETS_KEY, &raw)
        .map_err(|_| "failed to persist encrypted secrets".to_string())
}

fn load_or_create_key(storage: &Storage) -> Result<[u8; 32], String> {
    if let Ok(Some(raw)) = storage.get_item(WEB_SECRET_KEY) {
        let bytes = BASE64.decode(raw).map_err(|e| e.to_string())?;
        if bytes.len() == 32 {
            let mut key = [0u8; 32];
            key.copy_from_slice(&bytes);
            return Ok(key);
        }
    }

    let mut key = [0u8; 32];
    getrandom(&mut key).map_err(|e| e.to_string())?;

    let encoded = BASE64.encode(key);
    storage
        .set_item(WEB_SECRET_KEY, &encoded)
        .map_err(|_| "failed to persist web secret key".to_string())?;

    Ok(key)
}

fn encrypt_secret(key: &[u8; 32], plaintext: &str) -> Result<(String, String), String> {
    let cipher = Aes256Gcm::new_from_slice(key).map_err(|e| e.to_string())?;
    let mut nonce = [0u8; 12];
    getrandom(&mut nonce).map_err(|e| e.to_string())?;

    let ciphertext = cipher
        .encrypt(Nonce::from_slice(&nonce), plaintext.as_bytes())
        .map_err(|_| "encryption failed".to_string())?;

    Ok((BASE64.encode(nonce), BASE64.encode(ciphertext)))
}

fn decrypt_secret(key: &[u8; 32], nonce_b64: &str, ciphertext_b64: &str) -> Result<String, String> {
    let nonce = BASE64.decode(nonce_b64).map_err(|e| e.to_string())?;
    if nonce.len() != 12 {
        return Err("invalid nonce length".to_string());
    }

    let ciphertext = BASE64.decode(ciphertext_b64).map_err(|e| e.to_string())?;
    let cipher = Aes256Gcm::new_from_slice(key).map_err(|e| e.to_string())?;

    let plaintext = cipher
        .decrypt(Nonce::from_slice(&nonce), ciphertext.as_ref())
        .map_err(|_| "decryption failed".to_string())?;

    String::from_utf8(plaintext).map_err(|e| e.to_string())
}

pub fn save_nsec(account_id: String, nsec: String) -> Result<(), String> {
    let storage = local_storage()?;
    let key = load_or_create_key(&storage)?;
    let (nonce_b64, ciphertext_b64) = encrypt_secret(&key, &nsec)?;

    let mut store = load_store(&storage);
    store.items.retain(|item| item.account_id != account_id);
    store.items.push(EncryptedSecret {
        account_id,
        nonce_b64,
        ciphertext_b64,
    });

    save_store(&storage, &store)
}

pub fn load_nsec(account_id: String) -> Result<Option<String>, String> {
    let storage = local_storage()?;
    let store = load_store(&storage);

    let Some(item) = store
        .items
        .into_iter()
        .find(|item| item.account_id == account_id)
    else {
        return Ok(None);
    };

    let key = load_or_create_key(&storage)?;
    decrypt_secret(&key, &item.nonce_b64, &item.ciphertext_b64).map(Some)
}

pub fn delete_nsec(account_id: String) -> Result<(), String> {
    let storage = local_storage()?;
    let mut store = load_store(&storage);
    store.items.retain(|item| item.account_id != account_id);
    save_store(&storage, &store)
}

pub fn list_secret_refs() -> Result<Vec<String>, String> {
    let storage = local_storage()?;
    let store = load_store(&storage);
    Ok(store
        .items
        .into_iter()
        .map(|item| item.account_id)
        .collect())
}

pub fn clear_all() -> Result<(), String> {
    let storage = local_storage()?;
    storage
        .remove_item(WEB_SECRETS_KEY)
        .map_err(|_| "failed to clear encrypted secrets".to_string())?;
    storage
        .remove_item(WEB_SECRET_KEY)
        .map_err(|_| "failed to clear web secret key".to_string())?;
    Ok(())
}
