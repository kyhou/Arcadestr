//! Active-account wallet metadata only. Credentials never live in this context.
use leptos::prelude::*;
use wasm_bindgen_futures::spawn_local;

use crate::tauri_bridge::{
    invoke_connect_nwc_wallet, invoke_disconnect_nwc_wallet, invoke_get_nwc_wallet,
    invoke_verify_nwc_wallet, ConnectNwcWalletRequest, ConnectNwcWalletResponse,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Verification {
    #[default]
    Unknown,
    Verified,
    Unsupported,
    Failed,
}

#[derive(Clone, Copy)]
pub struct NwcWalletContext {
    pub account: RwSignal<Option<String>>,
    pub metadata: RwSignal<Option<ConnectNwcWalletResponse>>,
    pub loaded: RwSignal<bool>,
    pub busy: RwSignal<bool>,
    pub operation: RwSignal<&'static str>,
    pub error: RwSignal<Option<String>>,
    pub verification: RwSignal<Verification>,
    generation: RwSignal<u64>,
}

impl NwcWalletContext {
    pub fn new() -> Self {
        Self {
            account: RwSignal::new(None),
            metadata: RwSignal::new(None),
            loaded: RwSignal::new(false),
            busy: RwSignal::new(false),
            operation: RwSignal::new("Loading wallet…"),
            error: RwSignal::new(None),
            verification: RwSignal::new(Verification::Unknown),
            generation: RwSignal::new(0),
        }
    }

    pub fn payment_available(self) -> bool {
        self.account.get().is_some()
            && wallet_payment_available(
                self.loaded.get(),
                self.metadata.get().is_some(),
                self.busy.get(),
            )
    }

    /// Synchronously invalidate old results before starting any new account lookup.
    pub fn select_account(self, account: Option<String>) -> u64 {
        let generation = self.begin("Loading wallet…");
        self.account.set(account.clone());
        self.metadata.set(None);
        self.verification.set(Verification::Unknown);
        self.loaded.set(account.is_none());
        self.busy.set(account.is_some());
        generation
    }

    pub async fn load_account(self, generation: u64, account: String) {
        if self.generation.get_untracked() != generation {
            return;
        }
        let result = invoke_get_nwc_wallet(account).await;
        self.complete_configuration(generation, result);
    }

    pub fn begin_payment(self, account: &str) -> Option<u64> {
        if !self.payment_available() || self.account.get_untracked().as_deref() != Some(account) {
            return None;
        }
        Some(self.begin("Paying invoice…"))
    }

    pub fn finish_payment(self, generation: u64) -> bool {
        if self.generation.get_untracked() != generation {
            return false;
        }
        self.busy.set(false);
        true
    }

    fn begin(self, operation: &'static str) -> u64 {
        self.generation
            .update(|value| *value = value.wrapping_add(1));
        self.busy.set(true);
        self.operation.set(operation);
        self.error.set(None);
        self.generation.get_untracked()
    }

    fn complete_configuration(
        self,
        generation: u64,
        result: Result<Option<ConnectNwcWalletResponse>, String>,
    ) {
        if self.generation.get_untracked() != generation {
            return;
        }
        self.verification.set(Verification::Unknown);
        match result {
            Ok(metadata) => {
                self.metadata.set(metadata);
                self.loaded.set(true);
            }
            Err(error) => {
                self.metadata.set(None);
                self.loaded.set(false);
                self.error.set(Some(error));
            }
        }
        self.busy.set(false);
    }

    fn complete_failed_save(
        self,
        generation: u64,
        error: String,
        current: Result<Option<ConnectNwcWalletResponse>, String>,
    ) {
        if self.generation.get_untracked() != generation {
            return;
        }
        self.complete_configuration(generation, current);
        self.error.set(Some(error));
    }

    pub async fn refresh(self) {
        if self.busy.get_untracked() {
            return;
        }
        let Some(account) = self.account.get_untracked() else {
            return;
        };
        let generation = self.begin("Loading wallet…");
        self.loaded.set(false);
        self.verification.set(Verification::Unknown);
        let result = invoke_get_nwc_wallet(account).await;
        self.complete_configuration(generation, result);
    }

    pub async fn save(self, account: String, connection_string: String) {
        if self.busy.get_untracked() || self.account.get_untracked().as_ref() != Some(&account) {
            return;
        }
        let generation = self.begin("Saving wallet…");
        self.verification.set(Verification::Unknown);
        let result = invoke_connect_nwc_wallet(ConnectNwcWalletRequest {
            expected_account_pubkey: account.clone(),
            connection_string,
        })
        .await;
        match result {
            Ok(metadata) => self.complete_configuration(generation, Ok(Some(metadata))),
            Err(error) => {
                if self.generation.get_untracked() != generation {
                    return;
                }
                // A failed replacement must not hide a still-configured wallet.
                let current = invoke_get_nwc_wallet(account).await;
                self.complete_failed_save(generation, error, current);
            }
        }
    }

    pub async fn remove(self, account: String) {
        if self.busy.get_untracked() || self.account.get_untracked().as_ref() != Some(&account) {
            return;
        }
        let generation = self.begin("Disconnecting wallet…");
        self.verification.set(Verification::Unknown);
        let result = invoke_disconnect_nwc_wallet(account).await;
        self.complete_configuration(generation, result.map(|()| None));
    }

    pub async fn verify(self, account: String) {
        if self.busy.get_untracked() || self.account.get_untracked().as_ref() != Some(&account) {
            return;
        }
        let generation = self.begin("Verifying wallet…");
        self.verification.set(Verification::Unknown);
        let result = invoke_verify_nwc_wallet(account).await;
        self.complete_verification(generation, result);
    }

    fn complete_verification(self, generation: u64, result: Result<String, String>) {
        if self.generation.get_untracked() != generation {
            return;
        }
        match result {
            Ok(status) => self.verification.set(if status == "verified" {
                Verification::Verified
            } else {
                Verification::Unsupported
            }),
            Err(error) => {
                self.verification.set(Verification::Failed);
                self.error.set(Some(error));
            }
        }
        self.busy.set(false);
    }
}

// Verification is deliberately absent: get_info support is not a prerequisite for payment.
fn wallet_payment_available(loaded: bool, configured: bool, busy: bool) -> bool {
    loaded && configured && !busy
}

#[component]
pub fn NwcWalletPanel() -> impl IntoView {
    let wallet = use_context::<NwcWalletContext>().expect("NwcWalletContext not provided");
    let input = RwSignal::new(String::new());
    let confirming = RwSignal::new(false);
    Effect::new(move |_| {
        let _account = wallet.account.get();
        input.set(String::new());
        confirming.set(false);
    });
    let save = move |_| {
        if wallet.busy.get_untracked() {
            return;
        }
        let Some(account) = wallet.account.get_untracked() else {
            return;
        };
        let uri = input.get_untracked();
        if uri.trim().is_empty() {
            return;
        }
        // Clear the field before submission; even failed submissions retain no URI in UI state.
        input.set(String::new());
        confirming.set(false);
        spawn_local(async move {
            wallet.save(account, uri).await;
        });
    };
    view! {
        <section class="v2-settings-card" aria-labelledby="settings-wallet-title" style="font:400 12.5px/1.6 ui-monospace,monospace;color:oklch(0.68 0.01 60)">
            <header class="v2-settings-card-header">
                <span class="v2-settings-icon material-symbols-outlined" aria-hidden="true">"bolt"</span>
                <div><p class="v2-store-kicker">"Payments"</p><h2 id="settings-wallet-title">"Lightning wallet"</h2></div>
            </header>
            <p style="margin:0">"Connect a compatible Lightning wallet to pay from Arcadestr. In your wallet, create a Nostr Wallet Connect (NWC) connection and paste it below. Treat this address like a password: it can authorize payments. Set a spending limit in your wallet before connecting."</p>
            <p style="margin:0">"The connection is saved securely on this device for your current account. Disconnecting here removes the saved connection; revoke it in your wallet too if you no longer want it used."</p>
            <strong role="status" style="color:oklch(0.8 0.14 195)">{move || {
                if wallet.account.get().is_none() { "Sign in to configure a wallet".into() }
                else if wallet.busy.get() { wallet.operation.get().to_string() }
                else if !wallet.loaded.get() { "Wallet status unavailable".into() }
                else if wallet.metadata.get().is_none() { "Not configured".into() }
                else { match wallet.verification.get() {
                    Verification::Unknown => "Configured · not verified",
                    Verification::Verified => "Configured · verified this session",
                    Verification::Unsupported => "Configured · verification unsupported; payments remain available",
                    Verification::Failed => "Configured · verification failed; payment can still be attempted",
                }.into() }
            }}</strong>
            <crate::ui_v2::components::AdvancedOptions title="WALLET CONNECTION DETAILS">
            {move || wallet.metadata.get().map(|metadata| view! {
                <div style="display:grid;gap:4px;overflow-wrap:anywhere">
                    <span>{format!("Wallet pubkey: {}…{}", &metadata.wallet_pubkey[..8], &metadata.wallet_pubkey[metadata.wallet_pubkey.len()-8..])}</span>
                    <span>{format!("Wallet relay origins: {}", metadata.relays.join(", "))}</span>
                    {metadata.lud16.map(|address| view! { <span>{format!("Lightning address: {address}")}</span> })}
                </div>
            })}
            </crate::ui_v2::components::AdvancedOptions>
            <label style="display:grid;gap:6px;font:800 11px ui-monospace,monospace">"WALLET CONNECTION ADDRESS (NWC)"
                <input type="password" autocomplete="off" spellcheck="false" autocapitalize="off" aria-label="NWC connection URI" placeholder="nostr+walletconnect://…"
                    style="min-width:0;width:100%;box-sizing:border-box;padding:10px;border:1px solid oklch(0.32 0.01 60);border-radius:4px;background:#141417;color:#fff;font:400 12px ui-monospace,monospace"
                    prop:value=move || input.get() on:input=move |event| input.set(event_target_value(&event)) disabled=move || wallet.busy.get() || wallet.account.get().is_none() />
            </label>
            <button type="button" on:click=save disabled=move || wallet.busy.get() || wallet.account.get().is_none() || input.get().trim().is_empty()
                style="padding:11px 20px;border:0;border-radius:5px;background:oklch(0.78 0.16 60);color:#0b0b0d;font:800 13px ui-monospace,monospace">
                {move || if wallet.metadata.get().is_some() { "REPLACE WALLET" } else { "SAVE WALLET" }}
            </button>
            <p style="margin:0">"After saving, select Verify connection to check whether your wallet responds. This check does not send a payment. Some wallets do not support it."</p>
            <div style="display:flex;gap:8px;flex-wrap:wrap">
                <button type="button" on:click=move |_| { spawn_local(async move { wallet.refresh().await; }); } disabled=move || wallet.busy.get() || wallet.account.get().is_none()
                    style="padding:8px;border:1px solid oklch(0.5 0.13 210);border-radius:4px;background:transparent;color:oklch(0.8 0.13 210);font:800 11px ui-monospace,monospace">"REFRESH SAVED WALLET"</button>
                <button type="button" on:click=move |_| { if let Some(account) = wallet.account.get_untracked() { spawn_local(async move { wallet.verify(account).await; }); } } disabled=move || !wallet.payment_available()
                    style="padding:8px;border:1px solid oklch(0.5 0.13 210);border-radius:4px;background:transparent;color:oklch(0.8 0.13 210);font:800 11px ui-monospace,monospace">"VERIFY CONNECTION"</button>
                <button type="button" on:click=move |_| confirming.set(true) disabled=move || wallet.account.get().is_none() || wallet.busy.get() || (wallet.loaded.get() && wallet.metadata.get().is_none())
                    style="padding:8px;border:1px solid oklch(0.65 0.18 25);border-radius:4px;background:transparent;color:oklch(0.65 0.18 25);font:800 11px ui-monospace,monospace">"DISCONNECT"</button>
            </div>
            <Show when=move || confirming.get()>
                <p>"Remove the active account’s saved wallet from this device? Other accounts, purchases, and recovery records are retained."</p>
                <button type="button" disabled=move || wallet.busy.get() || wallet.account.get().is_none() on:click=move |_| { input.set(String::new()); confirming.set(false); if let Some(account) = wallet.account.get_untracked() { spawn_local(async move { wallet.remove(account).await; }); } }
                    style="padding:8px;border:1px solid oklch(0.65 0.18 25);background:transparent;color:oklch(0.65 0.18 25);font:800 11px ui-monospace,monospace">"CONFIRM DISCONNECT"</button>
                <button type="button" on:click=move |_| confirming.set(false)
                    style="padding:8px;border:1px solid rgba(255,255,255,.22);background:transparent;color:oklch(0.68 0.01 60);font:800 11px ui-monospace,monospace">"CANCEL"</button>
            </Show>
            {move || wallet.error.get().map(|error| view! { <p role="alert" style="margin:0;color:oklch(0.65 0.18 25)">{error}</p> })}
        </section>
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nwc_failed_save_keeps_authoritative_configuration_and_error() {
        let owner = Owner::new();
        owner.with(|| {
            let wallet = NwcWalletContext::new();
            wallet.select_account(Some("account-a".into()));
            let missing = wallet.begin("Saving…");
            wallet.complete_failed_save(missing, "Invalid URI".into(), Ok(None));
            assert!(wallet.loaded.get());
            assert!(wallet.metadata.get().is_none());
            assert_eq!(wallet.error.get().as_deref(), Some("Invalid URI"));
            let existing = ConnectNwcWalletResponse {
                wallet_pubkey: "a".repeat(64),
                relays: vec!["ws://127.0.0.1:7778".into()],
                lud16: None,
            };
            let failed_replace = wallet.begin("Replacing…");
            wallet.complete_failed_save(
                failed_replace,
                "Invalid URI".into(),
                Ok(Some(existing.clone())),
            );
            assert_eq!(wallet.metadata.get(), Some(existing));
            assert!(wallet.payment_available());
            assert_eq!(wallet.verification.get(), Verification::Unknown);
            let remove = wallet.begin("Removing…");
            wallet.complete_configuration(remove, Ok(None));
            wallet.complete_failed_save(failed_replace, "Stale error".into(), Ok(None));
            assert!(wallet.error.get().is_none());
        });
    }

    #[test]
    fn nwc_payment_availability_is_independent_of_verification() {
        for _verification in [
            Verification::Unknown,
            Verification::Verified,
            Verification::Unsupported,
            Verification::Failed,
        ] {
            assert!(wallet_payment_available(true, true, false));
        }
        assert!(!wallet_payment_available(false, true, false));
        assert!(!wallet_payment_available(true, false, false));
        assert!(!wallet_payment_available(true, true, true));
    }

    #[test]
    fn nwc_shared_state_replacement_removal_and_restart() {
        let owner = Owner::new();
        owner.with(|| {
            let wallet = NwcWalletContext::new();
            wallet.select_account(Some("account-a".into()));
            assert!(!wallet.payment_available());
            let metadata = ConnectNwcWalletResponse {
                wallet_pubkey: "a".repeat(64),
                relays: vec!["ws://127.0.0.1:7778".into()],
                lud16: None,
            };
            let startup = wallet.begin("Loading…");
            wallet.complete_configuration(startup, Ok(Some(metadata.clone())));
            assert!(wallet.payment_available());
            for verification in [
                Verification::Unknown,
                Verification::Verified,
                Verification::Unsupported,
                Verification::Failed,
            ] {
                wallet.verification.set(verification);
                assert!(wallet.payment_available());
            }
            let replace = wallet.begin("Replacing…");
            assert!(!wallet.payment_available());
            let replacement = ConnectNwcWalletResponse {
                wallet_pubkey: "b".repeat(64),
                ..metadata.clone()
            };
            wallet.complete_configuration(replace, Ok(Some(replacement.clone())));
            assert_eq!(wallet.metadata.get(), Some(replacement));
            assert_eq!(wallet.verification.get(), Verification::Unknown);
            let remove = wallet.begin("Removing…");
            wallet.complete_configuration(remove, Ok(None));
            assert!(!wallet.payment_available());
            wallet.complete_configuration(startup, Ok(Some(metadata)));
            assert!(
                wallet.metadata.get().is_none(),
                "stale load cannot restore removed wallet"
            );
            let failed = wallet.begin("Loading…");
            wallet.complete_configuration(failed, Err("Keyring locked".into()));
            assert!(!wallet.loaded.get());
            assert!(wallet.error.get().is_some());
            let missing = wallet.begin("Loading…");
            wallet.complete_configuration(missing, Ok(None));
            assert!(wallet.loaded.get());
            assert!(wallet.error.get().is_none());
        });
    }

    #[test]
    fn nwc_account_switch_logout_and_stale_operation_results_are_isolated() {
        let owner = Owner::new();
        owner.with(|| {
            let wallet = NwcWalletContext::new();
            let a = wallet.select_account(Some("account-a".into()));
            let metadata = ConnectNwcWalletResponse {
                wallet_pubkey: "a".repeat(64),
                relays: vec!["ws://127.0.0.1:7778".into()],
                lud16: None,
            };
            wallet.complete_configuration(a, Ok(Some(metadata.clone())));
            wallet.verification.set(Verification::Verified);
            let paying_a = wallet.begin_payment("account-a").expect("A can pay");
            let b = wallet.select_account(Some("account-b".into()));
            assert!(wallet.metadata.get().is_none());
            assert_eq!(wallet.verification.get(), Verification::Unknown);
            assert!(!wallet.payment_available());
            assert!(
                !wallet.finish_payment(paying_a),
                "A's completion must not unlock B's operation"
            );
            assert!(wallet.busy.get());
            wallet.complete_configuration(paying_a, Ok(Some(metadata.clone())));
            assert!(wallet.metadata.get().is_none());
            wallet.complete_configuration(b, Ok(None));
            assert!(!wallet.payment_available(), "B has no saved wallet");
            assert!(wallet.begin_payment("account-a").is_none());
            let back_to_a = wallet.select_account(Some("account-a".into()));
            wallet.complete_configuration(back_to_a, Ok(Some(metadata.clone())));
            assert!(wallet.payment_available());
            let verify_a = wallet.begin("Verifying…");
            wallet.select_account(None);
            wallet.complete_verification(verify_a, Ok("verified".into()));
            assert_eq!(wallet.verification.get(), Verification::Unknown);
            wallet.complete_configuration(verify_a, Ok(Some(metadata)));
            assert!(wallet.metadata.get().is_none());
            assert!(wallet.account.get().is_none());
            assert!(!wallet.payment_available());
            assert!(!wallet.busy.get());
        });
    }
}
