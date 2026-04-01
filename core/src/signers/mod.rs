//! Signers module for Nostr event signing
//! 
//! This module provides multiple signing backends:
//! - LocalSigner: Fast local signing with encrypted nsec storage (native only)
//! - Nip46Signer: NIP-46 remote signer (Amber, Nsec.app, etc.) (native only)
//! - LazyNip46Signer: Deferred connection NIP-46 signer (native only)
//! - Nip07Signer: Browser extension signer (WASM only)

mod local;
mod nip46;

#[cfg(not(target_arch = "wasm32"))]
mod lazy_nip46;

// LocalSigner is native-only (requires sqlx, encryption)
#[cfg(not(target_arch = "wasm32"))]
pub use local::LocalSigner;

// These are native-only
#[cfg(not(target_arch = "wasm32"))]
pub use nip46::{ActiveSigner, DirectKeySigner, Nip46Signer, NostrSigner, SignerError, load_or_create_client_keys, reset_client_keys, set_keys_dir};

// LazyNip46Signer is native-only
#[cfg(not(target_arch = "wasm32"))]
pub use lazy_nip46::LazyNip46Signer;

// WASM-only exports
#[cfg(target_arch = "wasm32")]
pub use nip46::{NostrSigner, SignerError, Nip07Signer};
