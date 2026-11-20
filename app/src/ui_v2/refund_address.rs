//! Refund Lightning address used when a split purchase commits its refund
//! destination. Settings owns it; the game page only displays it.
//!
//! Precedence: an address saved in Settings, then the `lud16` of the
//! connected NWC wallet, then the active account's Nostr profile `lud16`.
//! Only the Settings address is stored; the backend resolves every address
//! again when a purchase commits it.
use leptos::prelude::*;
use wasm_bindgen_futures::spawn_local;

use crate::tauri_bridge::{
    invoke_clear_refund_address, invoke_get_refund_address, invoke_set_refund_address,
};
use crate::ui_v2::nwc_wallet::NwcWalletContext;
use crate::AuthContext;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RefundAddressSource {
    Custom,
    Wallet,
    Profile,
}

impl RefundAddressSource {
    pub fn label(self) -> &'static str {
        match self {
            Self::Custom => "set in Settings",
            Self::Wallet => "from your connected wallet",
            Self::Profile => "from your Nostr profile",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RefundAddress {
    pub address: String,
    pub source: RefundAddressSource,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RefundAddressStatus {
    Loading,
    /// The saved Settings address could not be read. Falling back to the
    /// wallet or profile could silently replace the user's choice.
    Unavailable,
    Missing,
    Ready(RefundAddress),
}

impl RefundAddressStatus {
    pub fn address(&self) -> Option<&RefundAddress> {
        match self {
            Self::Ready(address) => Some(address),
            _ => None,
        }
    }
}

fn usable(value: Option<&str>) -> Option<String> {
    value
        .map(str::trim)
        .filter(|value| value.contains('@'))
        .map(ToOwned::to_owned)
}

pub fn effective_refund_address(
    custom: Option<&str>,
    wallet: Option<&str>,
    profile: Option<&str>,
) -> Option<RefundAddress> {
    [
        (custom, RefundAddressSource::Custom),
        (wallet, RefundAddressSource::Wallet),
        (profile, RefundAddressSource::Profile),
    ]
    .into_iter()
    .find_map(|(value, source)| usable(value).map(|address| RefundAddress { address, source }))
}

pub fn refund_address_status(
    saved: Option<Result<Option<&str>, ()>>,
    wallet_loading: bool,
    wallet: Option<&str>,
    profile: Option<&str>,
) -> RefundAddressStatus {
    match saved {
        None => RefundAddressStatus::Loading,
        Some(Err(())) => RefundAddressStatus::Unavailable,
        // A saved address wins regardless of the wallet; otherwise wait for
        // the wallet so the profile address does not flash first.
        Some(Ok(custom)) if usable(custom).is_none() && wallet_loading => {
            RefundAddressStatus::Loading
        }
        Some(Ok(custom)) => effective_refund_address(custom, wallet, profile)
            .map_or(RefundAddressStatus::Missing, RefundAddressStatus::Ready),
    }
}

/// Saved Settings address for the active account. `saved` is `None` while
/// loading and `Some(Err)` when it could not be read.
#[derive(Clone, Copy)]
pub struct RefundAddressContext {
    pub account: RwSignal<Option<String>>,
    saved: RwSignal<Option<Result<Option<String>, ()>>>,
    pub busy: RwSignal<bool>,
    pub error: RwSignal<Option<String>>,
    generation: RwSignal<u64>,
}

impl Default for RefundAddressContext {
    fn default() -> Self {
        Self::new()
    }
}

impl RefundAddressContext {
    pub fn new() -> Self {
        Self {
            account: RwSignal::new(None),
            saved: RwSignal::new(None),
            busy: RwSignal::new(false),
            error: RwSignal::new(None),
            generation: RwSignal::new(0),
        }
    }

    pub fn saved_address(self) -> Option<String> {
        self.saved.get().and_then(Result::ok).flatten()
    }

    fn begin(self) -> u64 {
        self.generation
            .update(|value| *value = value.wrapping_add(1));
        self.busy.set(true);
        self.error.set(None);
        self.generation.get_untracked()
    }

    fn current(self, generation: u64, account: &str) -> bool {
        self.generation.get_untracked() == generation
            && self.account.get_untracked().as_deref() == Some(account)
    }

    /// Synchronously drop the previous account's address before any lookup.
    pub fn select_account(self, account: Option<String>) -> u64 {
        let generation = self.begin();
        self.account.set(account.clone());
        self.saved.set(account.is_none().then_some(Ok(None)));
        self.busy.set(account.is_some());
        generation
    }

    pub async fn load_account(self, generation: u64, account: String) {
        let result = invoke_get_refund_address(account.clone()).await;
        if !self.current(generation, &account) {
            return;
        }
        self.saved.set(Some(result.map_err(|_| ())));
        self.busy.set(false);
    }

    /// Re-read the saved address for the active account, e.g. when a page that
    /// may commit it opens. The last known value stays visible meanwhile.
    pub async fn refresh(self) {
        if self.busy.get_untracked() {
            return;
        }
        let Some(account) = self.account.get_untracked() else {
            return;
        };
        let generation = self.begin();
        let result = invoke_get_refund_address(account.clone()).await;
        if !self.current(generation, &account) {
            return;
        }
        self.saved.set(Some(result.map_err(|_| ())));
        self.busy.set(false);
    }

    pub async fn save(self, account: String, lud16: String) -> bool {
        if self.busy.get_untracked() || self.account.get_untracked().as_ref() != Some(&account) {
            return false;
        }
        let generation = self.begin();
        let result = invoke_set_refund_address(account.clone(), lud16).await;
        if !self.current(generation, &account) {
            return false;
        }
        self.busy.set(false);
        match result {
            Ok(stored) => {
                self.saved.set(Some(Ok(Some(stored))));
                true
            }
            Err(error) => {
                self.error.set(Some(error));
                false
            }
        }
    }

    pub async fn clear(self, account: String) {
        if self.busy.get_untracked() || self.account.get_untracked().as_ref() != Some(&account) {
            return;
        }
        let generation = self.begin();
        let result = invoke_clear_refund_address(account.clone()).await;
        if !self.current(generation, &account) {
            return;
        }
        self.busy.set(false);
        match result {
            Ok(()) => self.saved.set(Some(Ok(None))),
            Err(error) => self.error.set(Some(error)),
        }
    }
}

/// The refund address the next split purchase would commit.
pub fn use_refund_address_status() -> Signal<RefundAddressStatus> {
    let refund = use_context::<RefundAddressContext>();
    let wallet = use_context::<NwcWalletContext>();
    let auth = use_context::<AuthContext>();
    Signal::derive(move || {
        let Some(refund) = refund else {
            return RefundAddressStatus::Missing;
        };
        let saved = refund.saved.get();
        let wallet_lud16 = wallet.and_then(|wallet| wallet.metadata.get().and_then(|m| m.lud16));
        let wallet_loading = wallet.is_some_and(|wallet| wallet.busy.get());
        // The profile must belong to the active account.
        let profile_lud16 = auth.as_ref().and_then(|auth| {
            let npub = auth.npub.get();
            auth.profile
                .get()
                .filter(|profile| Some(profile.npub.as_str()) == npub.as_deref())
                .and_then(|profile| profile.lud16)
        });
        refund_address_status(
            saved
                .as_ref()
                .map(|saved| saved.as_ref().map(Option::as_deref).map_err(|_| ())),
            wallet_loading,
            wallet_lud16.as_deref(),
            profile_lud16.as_deref(),
        )
    })
}

const FIELD_STYLE: &str = "min-width:0;width:100%;box-sizing:border-box;padding:10px;border:1px solid oklch(0.32 0.01 60);border-radius:4px;background:#141417;color:#fff;font:400 12px ui-monospace,monospace";
const PRIMARY_STYLE: &str = "padding:11px 20px;border:0;border-radius:5px;background:oklch(0.78 0.16 60);color:#0b0b0d;font:800 13px ui-monospace,monospace";
const SECONDARY_STYLE: &str = "padding:8px;border:1px solid oklch(0.5 0.13 210);border-radius:4px;background:transparent;color:oklch(0.8 0.13 210);font:800 11px ui-monospace,monospace";

#[component]
pub fn RefundAddressPanel() -> impl IntoView {
    let refund = use_context::<RefundAddressContext>().expect("RefundAddressContext not provided");
    let status = use_refund_address_status();
    let input = RwSignal::new(String::new());
    let editing = RwSignal::new(false);
    Effect::new(move |_| {
        let _account = refund.account.get();
        input.set(String::new());
        editing.set(false);
    });
    // Ask only when nothing is configured; otherwise editing is opt-in.
    let show_field = move || editing.get() || status.get() == RefundAddressStatus::Missing;
    let save = move |_| {
        let Some(account) = refund.account.get_untracked() else {
            return;
        };
        let address = input.get_untracked();
        if address.trim().is_empty() {
            return;
        }
        spawn_local(async move {
            if refund.save(account, address).await {
                input.set(String::new());
                editing.set(false);
            }
        });
    };
    view! {
        <section class="v2-settings-card" aria-labelledby="settings-refund-title" data-testid="refund-address-panel" style="font:400 12.5px/1.6 ui-monospace,monospace;color:oklch(0.68 0.01 60)">
            <header class="v2-settings-card-header">
                <span class="v2-settings-icon material-symbols-outlined" aria-hidden="true">"undo"</span>
                <div><p class="v2-store-kicker">"Payments"</p><h2 id="settings-refund-title">"Refund address"</h2></div>
            </header>
            <p style="margin:0">"Some games split your payment between several recipients. If part of a purchase fails, the recipients already paid send your money back to this Lightning address. It only receives refunds; it is not a wallet connection and cannot authorize payments."</p>
            <p style="margin:0">"Arcadestr uses your connected wallet’s Lightning address, or the one on your Nostr profile. Set a different address here only if you want refunds to go elsewhere."</p>
            <strong role="status" style="color:oklch(0.8 0.14 195);overflow-wrap:anywhere">{move || match status.get() {
                _ if refund.account.get().is_none() => "Sign in to set a refund address".to_string(),
                RefundAddressStatus::Loading => "Loading refund address…".to_string(),
                RefundAddressStatus::Unavailable => "Saved refund address unavailable. Reopen Settings to retry.".to_string(),
                RefundAddressStatus::Missing => "No refund address. Add one to buy games with split payments.".to_string(),
                RefundAddressStatus::Ready(address) => format!("Refunds go to {} ({})", address.address, address.source.label()),
            }}</strong>
            <Show when=show_field>
                <label style="display:grid;gap:6px;font:800 11px ui-monospace,monospace">"REFUND LIGHTNING ADDRESS"
                    <input type="text" autocomplete="off" spellcheck="false" autocapitalize="off" placeholder="you@example.com" style=FIELD_STYLE
                        prop:value=move || input.get() on:input=move |event| input.set(event_target_value(&event))
                        disabled=move || refund.busy.get() || refund.account.get().is_none() />
                </label>
                <div style="display:flex;gap:8px;flex-wrap:wrap">
                    <button type="button" style=PRIMARY_STYLE on:click=save
                        disabled=move || refund.busy.get() || refund.account.get().is_none() || input.get().trim().is_empty()>
                        {move || if refund.busy.get() { "CHECKING…" } else { "CHECK AND SAVE" }}
                    </button>
                    <Show when=move || editing.get()>
                        <button type="button" style=SECONDARY_STYLE on:click=move |_| { editing.set(false); input.set(String::new()); refund.error.set(None); }>"CANCEL"</button>
                    </Show>
                </div>
                <p style="margin:0">"The address is checked before it is saved. Each purchase checks it again before any payment."</p>
            </Show>
            <div style="display:flex;gap:8px;flex-wrap:wrap">
                <Show when=move || !show_field() && status.get().address().is_some()>
                    <button type="button" style=SECONDARY_STYLE on:click=move |_| editing.set(true) disabled=move || refund.busy.get()>"USE A DIFFERENT ADDRESS"</button>
                </Show>
                <Show when=move || refund.saved_address().is_some()>
                    <button type="button" style=SECONDARY_STYLE disabled=move || refund.busy.get()
                        on:click=move |_| { if let Some(account) = refund.account.get_untracked() { spawn_local(async move { refund.clear(account).await; }); } }>
                        "REMOVE SAVED ADDRESS"
                    </button>
                </Show>
            </div>
            {move || refund.error.get().map(|error| view! { <p role="alert" style="margin:0;color:oklch(0.65 0.18 25)">{error}</p> })}
        </section>
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ready(address: &str, source: RefundAddressSource) -> RefundAddressStatus {
        RefundAddressStatus::Ready(RefundAddress {
            address: address.into(),
            source,
        })
    }

    #[test]
    fn settings_address_beats_wallet_which_beats_profile() {
        let custom = Some("custom@example.com");
        let wallet = Some("wallet@example.com");
        let profile = Some("profile@example.com");
        assert_eq!(
            refund_address_status(Some(Ok(custom)), false, wallet, profile),
            ready("custom@example.com", RefundAddressSource::Custom)
        );
        assert_eq!(
            refund_address_status(Some(Ok(None)), false, wallet, profile),
            ready("wallet@example.com", RefundAddressSource::Wallet)
        );
        assert_eq!(
            refund_address_status(Some(Ok(None)), false, None, profile),
            ready("profile@example.com", RefundAddressSource::Profile)
        );
    }

    #[test]
    fn blank_or_malformed_sources_are_skipped_and_trimmed() {
        assert_eq!(
            refund_address_status(Some(Ok(Some("  "))), false, Some(""), Some(" me@x.io ")),
            ready("me@x.io", RefundAddressSource::Profile)
        );
        assert_eq!(
            refund_address_status(Some(Ok(None)), false, Some("not-an-address"), None),
            RefundAddressStatus::Missing
        );
    }

    #[test]
    fn nothing_configured_is_missing_so_settings_asks() {
        assert_eq!(
            refund_address_status(Some(Ok(None)), false, None, None),
            RefundAddressStatus::Missing
        );
    }

    #[test]
    fn unknown_saved_address_never_falls_back() {
        let wallet = Some("wallet@example.com");
        assert_eq!(
            refund_address_status(None, false, wallet, None),
            RefundAddressStatus::Loading
        );
        assert_eq!(
            refund_address_status(Some(Err(())), false, wallet, None),
            RefundAddressStatus::Unavailable
        );
    }

    #[test]
    fn waits_for_wallet_unless_settings_address_exists() {
        let profile = Some("profile@example.com");
        assert_eq!(
            refund_address_status(Some(Ok(None)), true, None, profile),
            RefundAddressStatus::Loading
        );
        assert_eq!(
            refund_address_status(Some(Ok(Some("c@example.com"))), true, None, profile),
            ready("c@example.com", RefundAddressSource::Custom)
        );
    }

    #[test]
    fn switching_account_drops_the_previous_saved_address() {
        let owner = Owner::new();
        owner.with(|| {
            let refund = RefundAddressContext::new();
            refund.select_account(Some("a".into()));
            refund.saved.set(Some(Ok(Some("a@example.com".into()))));
            assert_eq!(refund.saved_address().as_deref(), Some("a@example.com"));
            refund.select_account(Some("b".into()));
            assert_eq!(refund.saved_address(), None);
            assert_eq!(refund.saved.get_untracked(), None, "b is loading");
            refund.select_account(None);
            assert_eq!(refund.saved.get_untracked(), Some(Ok(None)));
        });
    }
}
