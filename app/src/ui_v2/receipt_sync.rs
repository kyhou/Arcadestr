//! Shared account-scoped receipt state. No credentials or decrypted content.
use crate::{
    tauri_bridge::{
        invoke_import_purchase_receipts, invoke_sync_purchase_receipts, ReceiptImportRequest,
        ReceiptSyncReport, ReceiptSyncRequest, ReceiptVerification,
    },
    AuthContext,
};
use leptos::prelude::*;
use wasm_bindgen_futures::spawn_local;

#[derive(Clone, Copy)]
pub struct ReceiptSyncContext {
    pub grants_report: RwSignal<Option<crate::tauri_bridge::EntitlementSyncReport>>,
    pub grants_error: RwSignal<Option<String>>,
    grants_busy: RwSignal<bool>,
    grants_retry: RwSignal<bool>,
    pub account: RwSignal<Option<String>>,
    pub busy: RwSignal<bool>,
    pub report: RwSignal<Option<ReceiptSyncReport>>,
    pub error: RwSignal<Option<String>>,
    pub revision: RwSignal<u64>,
    generation: RwSignal<u64>,
    retry: RwSignal<bool>,
}
impl ReceiptSyncContext {
    pub fn new() -> Self {
        Self {
            grants_report: RwSignal::new(None),
            grants_error: RwSignal::new(None),
            grants_busy: RwSignal::new(false),
            grants_retry: RwSignal::new(false),
            account: RwSignal::new(None),
            busy: RwSignal::new(false),
            report: RwSignal::new(None),
            error: RwSignal::new(None),
            revision: RwSignal::new(0),
            generation: RwSignal::new(0),
            retry: RwSignal::new(false),
        }
    }
    pub fn select_account(self, account: Option<String>) {
        self.generation.update(|n| *n = n.wrapping_add(1));
        self.account.set(account);
        self.grants_report.set(None);
        self.grants_error.set(None);
        self.grants_busy.set(false);
        self.grants_retry.set(false);
        self.report.set(None);
        self.error.set(None);
        self.busy.set(false);
        self.retry.set(false);
        self.revision.update(|n| *n = n.wrapping_add(1));
    }
    fn finish(self, generation: u64, result: Result<ReceiptSyncReport, String>) -> bool {
        if self.generation.try_get_untracked() != Some(generation) {
            return false;
        }
        self.busy.set(false);
        match result {
            Ok(report) => {
                self.report.set(Some(report));
                self.error.set(None);
                self.revision.update(|n| *n = n.wrapping_add(1));
            }
            Err(e) => self.error.set(Some(e)),
        }
        true
    }
    pub fn synchronize(self, full: bool) {
        if cfg!(feature = "web") {
            return;
        }
        let Some(account) = self.account.get_untracked() else {
            return;
        };
        self.reconcile_grants();
        if self.busy.get_untracked() {
            self.retry.set(true);
            return;
        }
        let generation = self.generation.get_untracked();
        self.busy.set(true);
        self.error.set(None);
        spawn_local(async move {
            let result = invoke_sync_purchase_receipts(ReceiptSyncRequest {
                expected_account_pubkey: account,
                full_rescan: full,
            })
            .await;
            if self.finish(generation, result) && self.retry.get_untracked() {
                self.retry.set(false);
                self.synchronize(false);
            }
        });
    }
    pub fn import(self, json: String) {
        if cfg!(feature = "web") {
            return;
        }
        let Some(account) = self.account.get_untracked() else {
            return;
        };
        if self.busy.get_untracked() {
            return;
        }
        let generation = self.generation.get_untracked();
        self.busy.set(true);
        self.error.set(None);
        spawn_local(async move {
            let result = invoke_import_purchase_receipts(ReceiptImportRequest {
                expected_account_pubkey: account,
                events_json: json,
            })
            .await;
            if self.finish(generation, result) && self.retry.get_untracked() {
                self.retry.set(false);
                self.synchronize(false);
            }
        });
    }

    fn finish_grants(
        self,
        generation: u64,
        result: Result<crate::tauri_bridge::EntitlementSyncReport, String>,
    ) -> bool {
        if self.generation.try_get_untracked() != Some(generation) {
            return false;
        }
        self.grants_busy.set(false);
        match result {
            Ok(report) => {
                self.grants_report.set(Some(report));
                self.grants_error.set(None);
                // The existing History, Library and details resources share this
                // invalidation signal. No optimistic local ownership projection.
                self.revision.update(|n| *n = n.wrapping_add(1));
            }
            Err(error) => self.grants_error.set(Some(error)),
        }
        true
    }

    pub(crate) fn reconcile_grants(self) {
        if cfg!(feature = "web") {
            return;
        }
        if self.grants_busy.get_untracked() {
            self.grants_retry.set(true);
            return;
        }
        let Some(account) = self.account.get_untracked() else {
            return;
        };
        let generation = self.generation.get_untracked();
        self.grants_busy.set(true);
        spawn_local(async move {
            let result = crate::tauri_bridge::invoke_reconcile_known_entitlements(
                crate::tauri_bridge::EntitlementSyncRequest {
                    expected_account_pubkey: account,
                },
            )
            .await;
            if self.finish_grants(generation, result) && self.grants_retry.get_untracked() {
                self.grants_retry.set(false);
                self.reconcile_grants();
            }
        });
    }
}

pub fn provide_receipt_sync(auth: AuthContext) -> ReceiptSyncContext {
    let receipts = ReceiptSyncContext::new();
    provide_context(receipts);
    Effect::new(move |_| {
        let _epoch = auth.current_auth_flow();
        let account = auth
            .npub
            .get()
            .as_deref()
            .and_then(super::components::blossom_media_upload::publisher_hex);
        receipts.select_account(account);
        receipts.synchronize(false);
    });
    #[cfg(all(target_arch = "wasm32", not(feature = "web")))]
    {
        let timer = send_wrapper::SendWrapper::new(gloo_timers::callback::Interval::new(
            60_000,
            move || receipts.synchronize(false),
        ));
        on_cleanup(move || drop(timer));
        listen_ownership_events(receipts);
    }
    receipts
}

#[cfg(all(target_arch = "wasm32", not(feature = "web")))]
fn listen_ownership_events(receipts: ReceiptSyncContext) {
    use std::{
        cell::{Cell, RefCell},
        rc::Rc,
    };
    let alive = Rc::new(Cell::new(true));
    let cleanup: Rc<RefCell<Vec<Box<dyn FnOnce()>>>> = Rc::new(RefCell::new(Vec::new()));
    let alive_cleanup = send_wrapper::SendWrapper::new(alive.clone());
    let listener_cleanup = send_wrapper::SendWrapper::new(cleanup.clone());
    on_cleanup(move || {
        alive_cleanup.set(false);
        for unlisten in listener_cleanup.borrow_mut().drain(..) {
            unlisten();
        }
    });
    let callback_alive = alive.clone();
    spawn_local(async move {
        if let Ok(unlisten) =
            crate::tauri_invoke::listen("receipt-ownership-updated", move |payload| {
                if !callback_alive.get() {
                    return;
                }
                if payload.get("account").and_then(|v| v.as_str())
                    == receipts.account.get_untracked().as_deref()
                    && !receipts.busy.get_untracked()
                {
                    receipts.revision.update(|v| *v = v.wrapping_add(1));
                }
            })
            .await
        {
            if alive.get() {
                cleanup.borrow_mut().push(Box::new(unlisten));
            } else {
                unlisten();
            }
        }
        let connected_alive = alive.clone();
        if let Ok(unlisten) = crate::tauri_invoke::listen("relay-connection", move |payload| {
            if connected_alive.get()
                && payload.get("type").and_then(|v| v.as_str()) == Some("connected")
            {
                receipts.reconcile_grants();
            }
        })
        .await
        {
            if alive.get() {
                cleanup.borrow_mut().push(Box::new(unlisten));
            } else {
                unlisten();
            }
        }
    });
}

#[component]
pub fn ReceiptPortabilityPanel() -> impl IntoView {
    let receipts = use_context::<ReceiptSyncContext>();
    let input = RwSignal::new(String::new());
    if let Some(context) = receipts {
        Effect::new(move |_| {
            let _ = context.account.get();
            input.set(String::new());
        });
    }
    view! {
        <section data-testid="receipt-portability" aria-label="Portable purchase receipts" style="margin:18px 0;padding:20px;max-width:1440px;box-sizing:border-box;min-width:0;overflow-wrap:anywhere;background:#121215;border:1px solid oklch(0.32 0.01 60);clip-path:polygon(0 0,calc(100% - 12px) 0,100% 12px,100% 100%,12px 100%,0 calc(100% - 12px));font:400 12.5px/1.65 ui-monospace,monospace;color:oklch(0.68 0.01 60)">
            <h2 style="font:800 12px ui-monospace,monospace;letter-spacing:1.5px;color:oklch(0.8 0.16 60)">"◆ PORTABLE PURCHASE RECEIPTS"</h2>
            <p>"Recover purchases for this Nostr identity through configured relays or import signed receipt events. Relay availability and retention limit discovery: automatic synchronization does not guarantee recovery of every historical receipt. Manual import is an independent recovery mechanism."</p>
            <p>"This is read-only. It never pays or publishes receipts. Encrypted receipt content stays encrypted. Private NIP-59-wrapped receipts are not discovered by public p-tag queries."</p>
            <p role="status" data-testid="receipt-sync-status">{move||match receipts {_ if cfg!(feature="web")=>"Receipt import, synchronization and grant reconciliation require the desktop application.".into(),Some(c) if c.busy.get()=>"Synchronizing / verifying receipt evidence…".to_string(),Some(c) if c.account.get().is_none()=>"Connect the original buyer account.".into(),Some(c)=>c.report.get().map(|r|r.message).unwrap_or_else(||"No synchronization result yet. Existing local purchases are preserved.".into()),None=>"Receipt synchronization requires the desktop application.".into()}}</p>
            {move||receipts.and_then(|c|c.error.get()).map(|e|view!{<p role="alert" style="color:oklch(0.65 0.18 25)">{e}</p>})}
            {move||receipts.and_then(|c|c.report.get()).map(|r|view!{
                <p data-testid="receipt-sync-summary">{format!("VERIFIED: {} · INDETERMINATE: {} · INVALID chains: {} · rejected events: {} · unavailable relay requests: {} · discovery incomplete: {}",r.chains.iter().filter(|c|c.verification==ReceiptVerification::Verified).count(),r.chains.iter().filter(|c|c.verification==ReceiptVerification::Indeterminate).count(),r.chains.iter().filter(|c|c.verification==ReceiptVerification::Invalid).count(),r.rejected_events,r.relay_failures,r.incomplete)}</p>
                {r.chains.into_iter().filter(|c|c.verification!=ReceiptVerification::Verified).take(20).map(|c|view!{<p>{format!("{} · {:?}: {}",c.order_id,c.verification,c.reason)}</p>}).collect_view()}
            })}
            <div style="display:flex;gap:10px;flex-wrap:wrap">
                <button type="button" style="padding:9px 14px;border:1px solid oklch(0.5 0.13 210);background:transparent;color:oklch(0.8 0.13 210);font:800 11px ui-monospace,monospace;border-radius:4px" disabled=move||cfg!(feature="web")||receipts.is_none_or(|c|c.busy.get()||c.account.get().is_none()) on:click=move |_|{if let Some(c)=receipts {c.synchronize(false);}}>"SYNC RECEIPTS"</button>
                <button type="button" style="padding:9px 14px;border:1px solid oklch(0.5 0.13 210);background:transparent;color:oklch(0.8 0.13 210);font:800 11px ui-monospace,monospace;border-radius:4px" disabled=move||cfg!(feature="web")||receipts.is_none_or(|c|c.busy.get()||c.account.get().is_none()) on:click=move |_|{if let Some(c)=receipts {c.synchronize(true);}}>"RESCAN AVAILABLE HISTORY"</button>
            </div>
            <label style="display:grid;gap:8px;margin-top:16px">"SIGNED NOSTR EVENT OR JSON ARRAY — MAXIMUM 4 MIB / 2048 EVENTS"
                <textarea data-testid="receipt-import-input" rows="5" maxlength="4194304" disabled=cfg!(feature="web") prop:value=move||input.get() on:input=move|e|input.set(event_target_value(&e)) style="box-sizing:border-box;width:100%;min-width:0;padding:12px;background:#141417;border:1px solid oklch(0.32 0.01 60);color:#fff;font:400 12px ui-monospace,monospace;border-radius:4px"/>
            </label>
            <p>"Import works offline. Include predecessor receipts and required signed authorization events when available. Missing evidence is INDETERMINATE, not ownership; use Sync to retry missing relay or public LSP evidence. VERIFIED chains grant access only when their resolved status permits it."</p>
            <button data-testid="receipt-import-submit" type="button" style="padding:11px 20px;border:0;background:oklch(0.78 0.16 60);color:#0b0b0d;font:800 13px ui-monospace,monospace;border-radius:5px" disabled=move||cfg!(feature="web")||input.get().trim().is_empty()||receipts.is_none_or(|c|c.busy.get()||c.account.get().is_none()) on:click=move |_|{if let Some(c)=receipts {let json=input.get_untracked();input.set(String::new());c.import(json);}}>"VERIFY AND IMPORT"</button>
        </section>
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn entitlement_reconciliation_ignores_stale_success_errors_and_aba_without_clearing_new_work() {
        let owner = Owner::new();
        owner.with(|| {
            let c = ReceiptSyncContext::new();
            c.select_account(Some("alice".into()));
            let old = c.generation.get_untracked();
            for account in [Some("bob".into()), None, Some("alice".into())] {
                c.select_account(account);
            }
            let current = c.generation.get_untracked();
            c.grants_busy.set(true);
            let revision = c.revision.get_untracked();
            assert!(!c.finish_grants(
                old,
                Ok(crate::tauri_bridge::EntitlementSyncReport {
                    revoked: 1,
                    ..Default::default()
                })
            ));
            assert!(!c.finish_grants(old, Err("old error".into())));
            assert!(c.grants_busy.get_untracked());
            assert!(c.grants_report.get_untracked().is_none());
            assert!(c.grants_error.get_untracked().is_none());
            assert_eq!(c.revision.get_untracked(), revision);
            assert!(c.finish_grants(
                current,
                Ok(crate::tauri_bridge::EntitlementSyncReport {
                    revoked: 1,
                    ..Default::default()
                })
            ));
            assert!(!c.grants_busy.get_untracked());
            assert_eq!(c.revision.get_untracked(), revision + 1);
            assert_eq!(c.grants_report.get_untracked().expect("report").revoked, 1);
        });
    }
    #[test]
    fn stale_account_and_aba_results_cannot_change_receipts_or_revision() {
        let owner = Owner::new();
        owner.with(|| {
            let c = ReceiptSyncContext::new();
            c.select_account(Some("alice".into()));
            let old = c.generation.get_untracked();
            c.select_account(Some("bob".into()));
            c.select_account(Some("alice".into()));
            let revision = c.revision.get_untracked();
            assert!(!c.finish(old, Ok(ReceiptSyncReport::default())));
            assert!(c.report.get_untracked().is_none());
            assert_eq!(c.revision.get_untracked(), revision);
            let current = c.generation.get_untracked();
            assert!(c.finish(current, Ok(ReceiptSyncReport::default())));
            assert_eq!(c.revision.get_untracked(), revision + 1);
        });
    }
}

/// Fixed public identities, debug-only browser fixture. No signer or wallet.
#[cfg(debug_assertions)]
#[component]
pub fn ReceiptPortabilityPreview() -> impl IntoView {
    const BUYER: &str = "79be667ef9dcbbac55a06295ce870b07029bfcdb2dce28d959f2815b16f81798";
    let receipts = ReceiptSyncContext::new();
    provide_context(receipts);
    receipts.select_account(Some(BUYER.into()));
    #[cfg(all(target_arch = "wasm32", not(feature = "web")))]
    listen_ownership_events(receipts);
    Effect::new(move |_| receipts.synchronize(false));
    view! {
        <main style="max-width:1440px;margin:0 auto;padding:26px 28px;background:#0b0b0d">
            <p style="font:600 12px ui-monospace,monospace;color:oklch(0.8 0.14 195)">"RECEIPT PORTABILITY FIXTURE — MOCK IPC ONLY"</p>
            <p data-testid="receipt-parent-revision">{move||receipts.revision.get().to_string()}</p>
            <button data-testid="mock-receipt-account-switch" on:click=move |_|receipts.select_account(Some(if receipts.account.get_untracked().as_deref()==Some(BUYER){"c6047f9441ed7d6d3045406e95c07cd85a778e4b8cef3ca7abac09b95c709ee5".into()}else{BUYER.into()}))>"SWITCH MOCK ACCOUNT"</button>
            <ReceiptPortabilityPanel/>
        </main>
    }
}
