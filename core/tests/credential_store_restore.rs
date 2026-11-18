#![cfg(feature = "native")]
//! Session restore against a locked, hanging, dismissed and later unlocked OS
//! credential store. Runs in its own test binary: the credential gate and the
//! keyring backend are process-wide.

use arcadestr_core::credential_store::{self, CredentialStoreStatus};
use arcadestr_core::nip46::{
    restore_session_on_startup, retry_credential_store, AppSignerState, SessionRestoreResult,
};
use keyring::credential::{Credential, CredentialApi, CredentialBuilderApi};
use nostr::{Keys, ToBech32};
use std::any::Any;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Mode {
    Available,
    Locked,
    Dismissed,
    Hang,
}

#[derive(Default)]
struct Store {
    entries: HashMap<(String, String), String>,
    mode: Option<Mode>,
    deletes: usize,
    writes: usize,
}

type Shared = Arc<Mutex<Store>>;

struct FakeCredential {
    key: (String, String),
    store: Shared,
}

#[derive(Debug)]
struct Platform(&'static str);
impl std::fmt::Display for Platform {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.0)
    }
}
impl std::error::Error for Platform {}

impl FakeCredential {
    fn gate(&self) -> keyring::Result<()> {
        let mode = self
            .store
            .lock()
            .expect("store")
            .mode
            .unwrap_or(Mode::Available);
        match mode {
            Mode::Available => Ok(()),
            Mode::Locked => Err(keyring::Error::NoStorageAccess(Box::new(Platform(
                "Secret Service: collection is locked",
            )))),
            Mode::Dismissed => Err(keyring::Error::NoStorageAccess(Box::new(Platform(
                "Secret Service: unlock prompt was dismissed",
            )))),
            Mode::Hang => {
                std::thread::sleep(Duration::from_secs(8));
                Err(keyring::Error::NoStorageAccess(Box::new(Platform(
                    "Secret Service: unlock prompt was dismissed",
                ))))
            }
        }
    }
}

impl CredentialApi for FakeCredential {
    fn set_secret(&self, secret: &[u8]) -> keyring::Result<()> {
        self.gate()?;
        let mut store = self.store.lock().expect("store");
        store.writes += 1;
        store.entries.insert(
            self.key.clone(),
            String::from_utf8_lossy(secret).into_owned(),
        );
        Ok(())
    }
    fn get_secret(&self) -> keyring::Result<Vec<u8>> {
        self.gate()?;
        self.store
            .lock()
            .expect("store")
            .entries
            .get(&self.key)
            .map(|value| value.as_bytes().to_vec())
            .ok_or(keyring::Error::NoEntry)
    }
    fn delete_credential(&self) -> keyring::Result<()> {
        self.gate()?;
        let mut store = self.store.lock().expect("store");
        store.deletes += 1;
        store
            .entries
            .remove(&self.key)
            .map(|_| ())
            .ok_or(keyring::Error::NoEntry)
    }
    fn as_any(&self) -> &dyn Any {
        self
    }
}

struct FakeBuilder(Shared);
impl CredentialBuilderApi for FakeBuilder {
    fn build(
        &self,
        _target: Option<&str>,
        service: &str,
        user: &str,
    ) -> keyring::Result<Box<Credential>> {
        Ok(Box::new(FakeCredential {
            key: (service.to_string(), user.to_string()),
            store: Arc::clone(&self.0),
        }))
    }
    fn as_any(&self) -> &dyn Any {
        self
    }
}

fn set_mode(store: &Shared, mode: Mode) {
    store.lock().expect("store").mode = Some(mode);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn restore_is_bounded_non_destructive_and_recovers_after_unlock() {
    let cache = tempfile::tempdir().expect("tempdir");
    arcadestr_core::nip46::set_profile_cache_dir(cache.path().to_path_buf());

    // Seed one saved remote-signer profile exactly as storage writes it.
    let user = Keys::generate().public_key();
    let signer = Keys::generate().public_key().to_hex();
    let app_keys = Keys::generate();
    let uri = format!("bunker://{signer}?relay=ws%3A%2F%2F127.0.0.1%3A1&secret=SEEDSECRET");
    let index = serde_json::json!([{
        "id": "profile-1",
        "name": uri,
        "pubkey_bech32": user.to_bech32().expect("npub"),
        "pubkey_hex": user.to_hex(),
        "bunker_pubkey_hex": signer,
    }]);
    let store: Shared = Arc::new(Mutex::new(Store::default()));
    {
        let mut seeded = store.lock().expect("store");
        let svc = "arcadestr-auth".to_string();
        seeded.entries.insert(
            (svc.clone(), "last_active_profile".into()),
            "profile-1".into(),
        );
        seeded
            .entries
            .insert((svc.clone(), "profile_index".into()), index.to_string());
        seeded.entries.insert(
            (svc.clone(), signer.clone()),
            app_keys.secret_key().to_bech32().expect("nsec"),
        );
        seeded
            .entries
            .insert((svc.clone(), format!("{signer}_uri")), uri.clone());
    }
    keyring::set_default_credential_builder(Box::new(FakeBuilder(Arc::clone(&store))));
    let snapshot = store.lock().expect("store").entries.clone();

    // 1. Locked store: restore reports it, quickly, and changes nothing.
    set_mode(&store, Mode::Locked);
    let state = Arc::new(tokio::sync::Mutex::new(AppSignerState::new()));
    let started = Instant::now();
    let result = restore_session_on_startup(&state).await;
    assert!(
        matches!(result, SessionRestoreResult::CredentialStoreUnavailable(_)),
        "{result:?}"
    );
    assert!(started.elapsed() < Duration::from_secs(2));
    assert_eq!(credential_store::status(), CredentialStoreStatus::Locked);
    assert!(state.lock().await.active_profile_id.is_none());

    // Automatic access is suppressed: no further prompts, immediate answers.
    let started = Instant::now();
    assert!(matches!(
        restore_session_on_startup(&state).await,
        SessionRestoreResult::CredentialStoreUnavailable(_)
    ));
    assert!(started.elapsed() < Duration::from_millis(500));

    // 2. Dismissed prompt on explicit retry: still reported, still nothing lost.
    set_mode(&store, Mode::Dismissed);
    assert_eq!(retry_credential_store(), CredentialStoreStatus::Dismissed);

    // 3. Hanging store: explicit retry is bounded by its deadline only via the
    //    gate; an automatic restore answers within the automatic deadline.
    set_mode(&store, Mode::Hang);
    let hung = std::thread::spawn(retry_credential_store);
    std::thread::sleep(Duration::from_millis(200));
    let started = Instant::now();
    assert!(matches!(
        restore_session_on_startup(&state).await,
        SessionRestoreResult::CredentialStoreUnavailable(_)
    ));
    assert!(
        started.elapsed() < Duration::from_secs(6),
        "automatic restore must not wait for the hung prompt"
    );
    let _ = hung.join();

    {
        let store = store.lock().expect("store");
        assert_eq!(
            store.deletes, 0,
            "unavailable store never clears credentials"
        );
        for (key, value) in &snapshot {
            if key.1 != "profile_index" {
                assert_eq!(store.entries.get(key), Some(value), "{key:?} preserved");
            }
        }
    }

    // 4. A newer session (explicit login/switch) wins over a late restore.
    set_mode(&store, Mode::Available);
    state.lock().await.auth_generation = 1;
    assert_eq!(retry_credential_store(), CredentialStoreStatus::Available);
    assert!(matches!(
        restore_session_on_startup(&state).await,
        SessionRestoreResult::NoSession
    ));
    assert!(state.lock().await.active_profile_id.is_none());

    // 5. Unlocked store, no newer session: the saved session is restored.
    let fresh = Arc::new(tokio::sync::Mutex::new(AppSignerState::new()));
    assert!(matches!(
        restore_session_on_startup(&fresh).await,
        SessionRestoreResult::Success
    ));
    assert_eq!(
        fresh.lock().await.active_profile_id.as_deref(),
        Some("profile-1")
    );

    // Legacy label carrying the connection secret was sanitized in the index
    // and cache; the secret survives only in its credential entry.
    let store = store.lock().expect("store");
    let index = store
        .entries
        .get(&("arcadestr-auth".into(), "profile_index".into()))
        .expect("index");
    assert!(!index.contains("SEEDSECRET"));
    assert_eq!(
        store
            .entries
            .get(&("arcadestr-auth".into(), format!("{signer}_uri"))),
        Some(&uri),
        "credential entry unchanged"
    );
    let cache_file = std::fs::read_to_string(cache.path().join("profile_metadata_cache.json"))
        .unwrap_or_default();
    assert!(!cache_file.contains("SEEDSECRET"));
}
