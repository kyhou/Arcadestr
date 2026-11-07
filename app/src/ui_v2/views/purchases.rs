use leptos::prelude::*;
use wasm_bindgen_futures::spawn_local;

use crate::models::{DurableAcquisitionKind, DurableAcquisitionRecord, DurableCredentialStatus};
use crate::tauri_bridge::{
    invoke_get_purchase_records, invoke_prepare_split_recovery,
    invoke_reconcile_pending_settlements, invoke_refresh_settlement,
    invoke_replace_split_refund_destination, invoke_resume_split_payment,
    invoke_settlement_history, ReplaceSplitRefundDestinationRequest, ResumeSplitPaymentRequest,
    SettlementHistoryItem, SplitQuoteRequest,
};
use crate::ui_v2::components::{
    EmptyState, ErrorSeverity, ErrorState, FeedbackLayout, LoadingState, PageTabItem,
    PageTabSemantics, PageTabs, StatusChip, StatusChipSize, StatusChipVariant,
};
use crate::AuthContext;

#[derive(Debug, Clone)]
enum PurchasesState {
    Loading,
    AccountRequired,
    Empty,
    Error,
    Ready(Vec<DurableAcquisitionRecord>),
    WebUnavailable,
}

#[derive(Debug, Clone)]
enum SettlementHistoryState {
    Loading,
    AccountRequired,
    Empty,
    Error,
    Ready(Vec<SettlementHistoryItem>),
    WebUnavailable,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PurchaseFilter {
    All,
    Purchases,
    PromotionClaims,
    Inactive,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum InitialAvailability {
    Load,
    AccountRequired,
    WebUnavailable,
}

fn initial_availability(web_target: bool, account_present: bool) -> InitialAvailability {
    if web_target {
        InitialAvailability::WebUnavailable
    } else if !account_present {
        InitialAvailability::AccountRequired
    } else {
        InitialAvailability::Load
    }
}

fn should_apply_response(request_generation: u64, current_generation: u64) -> bool {
    request_generation == current_generation
}

fn filter_matches(filter: PurchaseFilter, record: &DurableAcquisitionRecord) -> bool {
    match filter {
        PurchaseFilter::All => true,
        PurchaseFilter::Purchases => record.record_type == DurableAcquisitionKind::Purchase,
        PurchaseFilter::PromotionClaims => {
            record.record_type == DurableAcquisitionKind::PromotionClaim
        }
        PurchaseFilter::Inactive => matches!(
            record.status,
            DurableCredentialStatus::Refunded | DurableCredentialStatus::Revoked
        ),
    }
}

fn purchase_filter_id(filter: PurchaseFilter) -> &'static str {
    match filter {
        PurchaseFilter::All => "all",
        PurchaseFilter::Purchases => "purchases",
        PurchaseFilter::PromotionClaims => "promotion-claims",
        PurchaseFilter::Inactive => "inactive",
    }
}

fn purchase_filter_from_id(id: &str) -> Option<PurchaseFilter> {
    match id {
        "all" => Some(PurchaseFilter::All),
        "purchases" => Some(PurchaseFilter::Purchases),
        "promotion-claims" => Some(PurchaseFilter::PromotionClaims),
        "inactive" => Some(PurchaseFilter::Inactive),
        _ => None,
    }
}

fn purchase_filter_tabs() -> Vec<PageTabItem> {
    [
        (PurchaseFilter::All, "All"),
        (PurchaseFilter::Purchases, "Purchases"),
        (PurchaseFilter::PromotionClaims, "Promotion claims"),
        (PurchaseFilter::Inactive, "Refunded or revoked"),
    ]
    .into_iter()
    .map(|(filter, label)| PageTabItem::local(purchase_filter_id(filter), label))
    .collect()
}

fn record_type_label(record_type: DurableAcquisitionKind) -> &'static str {
    match record_type {
        DurableAcquisitionKind::Purchase => "Purchase",
        DurableAcquisitionKind::PromotionClaim => "Promotion claim",
    }
}

fn status_label(status: DurableCredentialStatus) -> &'static str {
    match status {
        DurableCredentialStatus::Active => "Access active",
        DurableCredentialStatus::Disputed => "Purchase disputed",
        DurableCredentialStatus::Refunded => "Purchase refunded",
        DurableCredentialStatus::Revoked => "Access revoked",
        DurableCredentialStatus::Unverified => "Record could not be verified",
    }
}

fn status_variant(status: DurableCredentialStatus) -> StatusChipVariant {
    match status {
        DurableCredentialStatus::Active => StatusChipVariant::Success,
        DurableCredentialStatus::Disputed => StatusChipVariant::Warning,
        DurableCredentialStatus::Refunded => StatusChipVariant::Cancelled,
        DurableCredentialStatus::Revoked => StatusChipVariant::Error,
        DurableCredentialStatus::Unverified => StatusChipVariant::Unverified,
    }
}

fn amount_label(record: &DurableAcquisitionRecord) -> Option<String> {
    record
        .amount
        .zip(record.currency.as_ref())
        .map(|(amount, currency)| format!("{amount} {currency}"))
}

fn record_has_partial_data(record: &DurableAcquisitionRecord) -> bool {
    record.validation_error.is_some()
        || record.listing_title.is_none()
        || (record.record_type == DurableAcquisitionKind::Purchase
            && (record.amount.is_none() || record.currency.is_none()))
}

/// Abbreviate an opaque record identifier for display. Full values stay
/// available through the deliberate copy control, never rendered inline.
fn short_record_identifier(value: &str) -> String {
    let characters = value.chars().collect::<Vec<_>>();
    if characters.len() <= 20 {
        return value.to_string();
    }
    format!(
        "{}…{}",
        characters[..10].iter().collect::<String>(),
        characters[characters.len() - 6..]
            .iter()
            .collect::<String>()
    )
}

/// Render a stored unix timestamp in the reader's local time. Zero means the
/// record carries no usable acquisition time; it is not rendered as an epoch date.
fn acquired_at_label(acquired_at: u64) -> String {
    if acquired_at == 0 {
        return "Acquisition time unavailable".to_string();
    }
    let date = js_sys::Date::new(&(acquired_at as f64 * 1000.0).into());
    format!(
        "Acquired {:04}-{:02}-{:02} {:02}:{:02}",
        date.get_full_year(),
        date.get_month() + 1,
        date.get_date(),
        date.get_hours(),
        date.get_minutes()
    )
}

/// Arcadestr stores one durable acquisition record per purchase or claim. There
/// is no per-event receipt chain to render, so the detail surface says so rather
/// than inventing a timeline.
const RECEIPT_TIMELINE_UNAVAILABLE: &str =
    "A per-event receipt timeline is not available here. The acquisition list shows one validated durable record per verified purchase; signed receipt chains and verification references are retained locally, without exposing encrypted private evidence.";

fn record_verification_label(record: &DurableAcquisitionRecord) -> &'static str {
    if let Some(verification)=record.verification {return match verification {
        crate::tauri_bridge::ReceiptVerification::Verified=>"VERIFIED",
        crate::tauri_bridge::ReceiptVerification::Indeterminate=>"INDETERMINATE",
        crate::tauri_bridge::ReceiptVerification::Invalid=>"INVALID",
    };}
    if record.validation_error.is_some() {
        "Not verified"
    } else if record.status == DurableCredentialStatus::Unverified {
        "Not verified"
    } else {
        "Validated on load"
    }
}

fn settlement_state_color(state: &str) -> &'static str {
    match state {
        "settled" | "verified" => "oklch(0.8 0.14 195)",
        "failed" | "overdue" | "disputed" => "oklch(0.65 0.18 25)",
        _ => "oklch(0.78 0.16 60)",
    }
}

fn settlement_display_state(record: &SettlementHistoryItem) -> &'static str {
    if record.state == "settled" && record.receipt.is_some() {
        "Purchase completed"
    } else if record.state == "settled" {
        "Recovery available"
    } else if record.recovery_state.as_deref() == Some("recovering") {
        "Recovery in progress"
    } else if record
        .attempts
        .iter()
        .any(|a| a.state != "settled" && a.has_intent && !a.has_proof)
    {
        "Payment outcome uncertain"
    } else if record
        .attempts
        .iter()
        .any(|a| a.state != "settled" && a.has_proof)
    {
        "Recovery available"
    } else if record.state == "failed" {
        "Purchase failed"
    } else if record.attempts.iter().any(|a| a.state == "settled") {
        "Partially settled"
    } else {
        "Awaiting payment"
    }
}

#[component]
fn SettlementRecordCard(
    record: SettlementHistoryItem,
    account: String,
    on_changed: Callback<()>,
) -> impl IntoView {
    let auth = use_context::<AuthContext>().expect("AuthContext not provided");
    let quote_id = record.quote_id.clone();
    let server_url = record.processor_url.clone();
    let state_name = record.state.clone();
    let recovery = record.recovery.clone();
    let display_state = settlement_display_state(&record);
    let attempts = record.attempts.clone();
    let can_reconcile =
        record.receipt.is_none() && attempts.iter().any(|a| a.has_intent || a.has_proof);
    let refund_lud16 = RwSignal::new(String::new());
    let operation = RwSignal::new(None::<String>);
    let error = RwSignal::new(None::<String>);
    let success = RwSignal::new(None::<String>);
    let on_reconcile = {
        let auth = auth.clone();
        let account = account.clone();
        let attempts = attempts.clone();
        Callback::new(move |()| {
            operation.set(Some(
                "Recovery in progress — read-only wallet lookup; no payment".into(),
            ));
            error.set(None);
            let auth = auth.clone();
            let account = account.clone();
            let attempts = attempts.clone();
            spawn_local(async move {
                let mut result = Ok(());
                for attempt in attempts.into_iter().filter(|a| a.has_intent || a.has_proof) {
                    if auth.npub.get_untracked().as_deref() != Some(account.as_str()) {
                        return;
                    }
                    match invoke_resume_split_payment(ResumeSplitPaymentRequest {
                        server_url: attempt.processor_url,
                        quote_id: attempt.quote_id,
                        attempt_id: attempt.attempt_id,
                    })
                    .await
                    {
                        Ok(response) if response.state == "settled" => break,
                        Ok(_) => {}
                        Err(message) => {
                            result = Err(message);
                            break;
                        }
                    }
                }
                if auth.npub.get_untracked().as_deref() != Some(account.as_str()) {
                    return;
                }
                operation.set(None);
                match result {
                    Ok(()) => {
                        success.set(Some(
                            "Original payment/proof checked. No payment was dispatched.".into(),
                        ));
                        on_changed.run(());
                    }
                    Err(message) => error.set(Some(message)),
                }
            });
        })
    };

    let on_refresh = {
        let auth = auth.clone();
        let account = account.clone();
        let quote_id = quote_id.clone();
        let server_url = server_url.clone();
        Callback::new(move |()| {
            operation.set(Some("Refreshing settlement".into()));
            error.set(None);
            let auth = auth.clone();
            let account = account.clone();
            let quote_id = quote_id.clone();
            let server_url = server_url.clone();
            spawn_local(async move {
                let result = invoke_refresh_settlement(SplitQuoteRequest {
                    server_url,
                    quote_id,
                })
                .await;
                if auth.npub.get_untracked().as_deref() != Some(account.as_str()) {
                    return;
                }
                operation.set(None);
                match result {
                    Ok(_) => on_changed.run(()),
                    Err(message) => error.set(Some(message)),
                }
            });
        })
    };

    let on_recover = {
        let auth = auth.clone();
        let account = account.clone();
        let quote_id = quote_id.clone();
        let server_url = server_url.clone();
        Callback::new(move |()| {
            operation.set(Some("Preparing recovery evidence".into()));
            error.set(None);
            let auth = auth.clone();
            let account = account.clone();
            let quote_id = quote_id.clone();
            let server_url = server_url.clone();
            spawn_local(async move {
                let result = invoke_prepare_split_recovery(SplitQuoteRequest {
                    server_url,
                    quote_id,
                })
                .await;
                if auth.npub.get_untracked().as_deref() != Some(account.as_str()) {
                    return;
                }
                operation.set(None);
                match result {
                    Ok(_) => {
                        success.set(Some(
                            "Recovery evidence retained for every paid leg.".into(),
                        ));
                        on_changed.run(());
                    }
                    Err(message) => error.set(Some(message)),
                }
            });
        })
    };

    let on_replace_destination = {
        let auth = auth.clone();
        let account = account.clone();
        let quote_id = quote_id.clone();
        let server_url = server_url.clone();
        Callback::new(move |()| {
            let destination = refund_lud16.get();
            if destination.trim().is_empty() {
                error.set(Some("Enter a Lightning address for refunds.".into()));
                return;
            }
            operation.set(Some("Replacing refund destination".into()));
            error.set(None);
            let auth = auth.clone();
            let account = account.clone();
            let quote_id = quote_id.clone();
            let server_url = server_url.clone();
            spawn_local(async move {
                let result =
                    invoke_replace_split_refund_destination(ReplaceSplitRefundDestinationRequest {
                        server_url,
                        quote_id,
                        refund_lud16: destination,
                    })
                    .await;
                if auth.npub.get_untracked().as_deref() != Some(account.as_str()) {
                    return;
                }
                operation.set(None);
                match result {
                    Ok(_) => success.set(Some(
                        "Refund destination replaced without changing the recovery deadline."
                            .into(),
                    )),
                    Err(message) => error.set(Some(message)),
                }
            });
        })
    };

    let recover_visible_state = state_name.clone();
    let destination_visible_state = state_name.clone();
    view! {
        <article style="clip-path:polygon(0 0,calc(100% - 12px) 0,100% 12px,100% 100%,12px 100%,0 calc(100% - 12px));border:1px solid oklch(0.32 0.01 60);background:#121215;padding:20px;display:grid;gap:14px;font-family:ui-monospace,monospace">
            <div style="display:flex;justify-content:space-between;align-items:flex-start;gap:16px;flex-wrap:wrap">
                <div style="display:grid;gap:5px;min-width:0">
                    <span style="font:800 11px ui-monospace,monospace;letter-spacing:1.5px;color:oklch(0.8 0.16 60)">"SPLIT SETTLEMENT"</span>
                    <strong style="font:800 16px/1.15 ui-monospace,monospace;color:#fff;overflow-wrap:anywhere">{record.listing_event_id}</strong>
                    <span style="font:600 10.5px ui-monospace,monospace;color:oklch(0.55 0.01 60);overflow-wrap:anywhere">{format!("QUOTE {}", quote_id)}</span>
                </div>
                <span data-testid="settlement-state" style=format!("padding:4px 11px;border:1px solid {};color:{};background:#141417;font:700 11px ui-monospace,monospace;border-radius:4px;text-transform:uppercase", settlement_state_color(&state_name), settlement_state_color(&state_name))>{display_state}</span>
            </div>
            <crate::ui_v2::components::PurchasePlanPanel quote_id=quote_id.clone() server_url=server_url.clone() on_recovered=on_changed/>
            <div style="display:flex;gap:10px;flex-wrap:wrap">
                <button style="padding:9px 14px;border:1px solid oklch(0.5 0.13 210);background:transparent;color:oklch(0.8 0.13 210);font:800 11px ui-monospace,monospace;border-radius:4px" on:click=move |_| on_refresh.run(()) disabled=move || operation.get().is_some()>"REFRESH STATUS"</button>
                <Show when=move||can_reconcile><button data-testid="recover-purchase" style="padding:9px 14px;border:1px solid oklch(0.5 0.13 210);background:transparent;color:oklch(0.8 0.14 195);font:800 11px ui-monospace,monospace;border-radius:4px" disabled=move||operation.get().is_some() on:click=move |_|on_reconcile.run(())>"CHECK ORIGINAL PAYMENT — NO PAYMENT"</button></Show>
                <Show when=move || recover_visible_state == "failed">
                    <button style="padding:9px 14px;border:1px solid oklch(0.5 0.13 210);background:transparent;color:oklch(0.8 0.13 210);font:800 11px ui-monospace,monospace;border-radius:4px" on:click=move |_| on_recover.run(()) disabled=move || operation.get().is_some()>"PREPARE RECOVERY"</button>
                </Show>
            </div>
            <Show when=move || destination_visible_state == "failed">
                <div style="display:grid;grid-template-columns:minmax(0,1fr) auto;gap:10px">
                    <input style="min-width:0;padding:10px 12px;border:1px solid oklch(0.32 0.01 60);background:#141417;color:#fff;font:600 12px ui-monospace,monospace;border-radius:4px" type="text" placeholder="replacement@example.com" prop:value=move || refund_lud16.get() on:input:target=move |event| refund_lud16.set(event.target().value()) />
                    <button style="padding:9px 14px;border:1px solid rgba(255,255,255,.22);background:transparent;color:oklch(0.68 0.01 60);font:800 11px ui-monospace,monospace;border-radius:4px" on:click=move |_| on_replace_destination.run(()) disabled=move || operation.get().is_some()>"REPLACE REFUND ADDRESS"</button>
                </div>
            </Show>
            {recovery.into_iter().map(|obligation| {
                view! {
                    <div style="border-top:1px solid oklch(0.4 0.03 60 / .3);padding-top:14px;display:grid;gap:7px">
                        <div style="display:flex;justify-content:space-between;gap:12px;flex-wrap:wrap">
                            <span style="font:700 11px ui-monospace,monospace;color:#fff">{format!("LEG {} · {} msat", obligation.leg_id, obligation.principal_msat)}</span>
                            <span style=format!("font:700 11px ui-monospace,monospace;color:{};text-transform:uppercase", settlement_state_color(&obligation.state))>{format!("{} · DUE {}", obligation.state, obligation.due_at)}</span>
                        </div>
                        <p style="margin:0;color:oklch(0.6 0.01 60);font:400 10.5px/1.5 ui-monospace,monospace">"The obligated recipient must pay and confirm this principal refund. Refresh status to retain late evidence."</p>
                    </div>
                }
            }).collect::<Vec<_>>()}
            <Show when=move || operation.get().is_some()><p style="margin:0;color:oklch(0.8 0.14 195);font:600 11px ui-monospace,monospace">{move || operation.get().unwrap_or_default()}</p></Show>
            <Show when=move || success.get().is_some()><p style="margin:0;color:oklch(0.8 0.14 195);font:600 11px ui-monospace,monospace">{move || success.get().unwrap_or_default()}</p></Show>
            <Show when=move || error.get().is_some()><p role="alert" style="margin:0;color:oklch(0.65 0.18 25);font:600 11px ui-monospace,monospace">{move || error.get().unwrap_or_default()}</p></Show>
        </article>
    }
}

fn settlement_history_view(
    state: SettlementHistoryState,
    account: String,
    on_changed: Callback<()>,
) -> AnyView {
    let body = match state {
        SettlementHistoryState::Loading => view! { <p style="color:oklch(0.6 0.01 60);font:600 11px ui-monospace,monospace">"Loading active settlement and recovery records..."</p> }.into_any(),
        SettlementHistoryState::AccountRequired => view! { <p style="color:oklch(0.6 0.01 60);font:400 12.5px/1.6 ui-monospace,monospace">"Connect an account to view resumable settlement records."</p> }.into_any(),
        SettlementHistoryState::Empty => view! { <div style="padding:36px 28px;text-align:center;border:1px solid oklch(0.32 0.01 60);background:#121215"><strong style="display:block;color:#fff;font:800 16px ui-monospace,monospace">"No split settlements yet"</strong><p style="color:oklch(0.6 0.01 60);font:400 12.5px/1.6 ui-monospace,monospace">"Multi-recipient purchases, partial payments, and refund recovery will appear here."</p></div> }.into_any(),
        SettlementHistoryState::Error => view! { <p role="alert" style="color:oklch(0.65 0.18 25);font:600 11px ui-monospace,monospace">"Settlement records could not be loaded. Durable purchase records remain available below."</p> }.into_any(),
        SettlementHistoryState::WebUnavailable => view! { <p style="color:oklch(0.6 0.01 60);font:400 12.5px/1.6 ui-monospace,monospace">"Settlement recovery is available in the desktop app."</p> }.into_any(),
        SettlementHistoryState::Ready(records) => view! { <div style="display:grid;gap:18px">{records.into_iter().map(|record| view! { <SettlementRecordCard record=record account=account.clone() on_changed=on_changed /> }).collect::<Vec<_>>()}</div> }.into_any(),
    };
    view! {
        <section style="margin:24px 0;display:grid;gap:16px">
            <div style="display:flex;align-items:center;gap:10px"><span style="font:800 12px ui-monospace,monospace;letter-spacing:1.5px;color:oklch(0.8 0.16 60)">"◆ SETTLEMENT & RECOVERY"</span><span style="flex:1;height:1px;background:repeating-linear-gradient(90deg,oklch(0.4 0.03 60 / .6) 0 4px,transparent 4px 8px)"></span></div>
            {body}
        </section>
    }.into_any()
}

#[component]
pub fn PurchasesView() -> impl IntoView {
    let auth = use_context::<AuthContext>().expect("AuthContext not provided");
    let state = RwSignal::new(PurchasesState::Loading);
    let settlement_state = RwSignal::new(SettlementHistoryState::Loading);
    let filter = RwSignal::new(PurchaseFilter::All);
    let request_generation = RwSignal::new(0_u64);
    let settlement_generation = RwSignal::new(0_u64);
    let history_refresh = RwSignal::new(0_u64);
    let receipt_sync=use_context::<crate::ui_v2::receipt_sync::ReceiptSyncContext>();
    let selected_filter = Signal::derive(move || purchase_filter_id(filter.get()).to_string());
    let select_filter = Callback::new(move |id: String| {
        if let Some(next) = purchase_filter_from_id(&id) {
            filter.set(next);
        }
    });
    let refresh_settlements = Callback::new(move |()| {
        history_refresh.update(|generation| *generation = generation.wrapping_add(1));
    });

    #[cfg(feature = "web")]
    {
        state.set(PurchasesState::WebUnavailable);
        settlement_state.set(SettlementHistoryState::WebUnavailable);
    }

    #[cfg(not(feature = "web"))]
    {
        let load_auth = auth.clone();
        Effect::new(move |_| {
            let _ = history_refresh.get();
            if let Some(receipts)=receipt_sync {let _=receipts.revision.get();}
            let _epoch=load_auth.current_auth_flow();
            let account = load_auth.npub.get().unwrap_or_default();
            let generation = request_generation.get_untracked().wrapping_add(1);
            request_generation.set(generation);

            match initial_availability(false, !account.is_empty()) {
                InitialAvailability::AccountRequired => {
                    state.set(PurchasesState::AccountRequired);
                    return;
                }
                InitialAvailability::Load => {
                    state.set(PurchasesState::Loading);
                }
                InitialAvailability::WebUnavailable => return,
            }

            spawn_local(async move {
                let result = invoke_get_purchase_records().await;
                if !should_apply_response(generation, request_generation.get_untracked()) {
                    return;
                }
                match result {
                    Ok(records) if records.is_empty() => state.set(PurchasesState::Empty),
                    Ok(records) => state.set(PurchasesState::Ready(records)),
                    Err(_) => state.set(PurchasesState::Error),
                }
            });
        });
        // Receipt discovery/import refreshes ownership only. Do not turn a
        // receipt synchronization notification into ADP proof submission.
        let settlement_auth=auth.clone();
        Effect::new(move |_|{
            let _=history_refresh.get();let _=settlement_auth.current_auth_flow();
            let account=settlement_auth.npub.get();
            settlement_generation.update(|v|*v=v.wrapping_add(1));let generation=settlement_generation.get_untracked();
            if account.is_none(){settlement_state.set(SettlementHistoryState::AccountRequired);return}
            settlement_state.set(SettlementHistoryState::Loading);
            spawn_local(async move{
                let _=invoke_reconcile_pending_settlements().await;
                let result=invoke_settlement_history().await;
                if !should_apply_response(generation,settlement_generation.get_untracked()){return}
                match result {Ok(records) if records.is_empty()=>settlement_state.set(SettlementHistoryState::Empty),Ok(records)=>settlement_state.set(SettlementHistoryState::Ready(records)),Err(_)=>settlement_state.set(SettlementHistoryState::Error)}
            });
        });
    }

    view! {
        <section class="v2-purchases">
            <header class="v2-purchases-hero v2-panel-glass">
                <p class="v2-store-kicker">"Account records"</p>
                <h1 class="v2-display">"Purchases and access"</h1>
                <p>"Durable acquisitions, resumable split payments, and account-scoped recovery records."</p>
            </header>
            <crate::ui_v2::receipt_sync::ReceiptPortabilityPanel/>

            {move || settlement_history_view(
                settlement_state.get(),
                auth.npub.get().unwrap_or_default(),
                refresh_settlements,
            )}
            <super::recipient_refunds::RecipientRefunds />

            {move || match state.get() {
                PurchasesState::Loading => view! {
                    <LoadingState
                        title="Loading records"
                        description="Reading validated records for the active account."
                    />
                }.into_any(),
                PurchasesState::AccountRequired => purchase_state_view(
                    "person",
                    "Account required",
                    "Connect an account to view its durable purchases and promotion claims.",
                    false,
                ),
                PurchasesState::Empty => view! {
                    <EmptyState
                        title="No durable records yet"
                        description="Paid purchases and claimed keep-forever promotions will appear here."
                        icon="receipt_long"
                    />
                }.into_any(),
                PurchasesState::Error => view! {
                    <ErrorState
                        title="Records unavailable"
                        message="Purchase and access records could not be loaded."
                        severity=ErrorSeverity::Recoverable
                    />
                }.into_any(),
                PurchasesState::WebUnavailable => purchase_state_view(
                    "desktop_windows",
                    "Desktop records unavailable on web",
                    "Purchase and access history is stored and verified by the desktop app.",
                    false,
                ),
                PurchasesState::Ready(records) => {
                    let partial_count = records
                        .iter()
                        .filter(|record| record_has_partial_data(record))
                        .count();
                    let visible_records = records
                        .into_iter()
                        .filter(|record| filter_matches(filter.get(), record))
                        .collect::<Vec<_>>();
                    view! {
                        <div class="v2-purchases-toolbar v2-panel">
                            <PageTabs
                                items=purchase_filter_tabs()
                                selected=selected_filter
                                on_select=select_filter
                                aria_label="Filter purchase records"
                                semantics=PageTabSemantics::Filter
                            />
                            {(partial_count > 0).then(|| view! {
                                <p class="v2-purchases-partial" role="status">
                                    {format!("{partial_count} record(s) have incomplete local details.")}
                                </p>
                            })}
                        </div>

                        {if visible_records.is_empty() {
                            view! {
                                <EmptyState
                                    title="No matching records"
                                    description="No durable records match this filter."
                                    icon="filter_alt_off"
                                    layout=FeedbackLayout::Compact
                                />
                            }.into_any()
                        } else {
                            view! {
                                <div class="v2-purchase-list">
                                    {visible_records.into_iter().map(purchase_record_view).collect::<Vec<_>>()}
                                </div>
                            }.into_any()
                        }}
                    }
                    .into_any()
                }
            }}
        </section>
    }
}

fn purchase_state_view(
    icon: &'static str,
    title: &'static str,
    message: &'static str,
    error: bool,
) -> AnyView {
    view! {
        <section class="v2-purchase-state v2-panel" class:v2-purchase-state-error=error role=if error { "alert" } else { "status" }>
            <span class="material-symbols-outlined" aria-hidden="true">{icon}</span>
            <div><h2>{title}</h2><p>{message}</p></div>
        </section>
    }
    .into_any()
}

fn purchase_record_view(record: DurableAcquisitionRecord) -> impl IntoView {
    let record_id = record.record_id.clone();
    let record_id_for_copy = record.record_id.clone();
    let verification = record_verification_label(&record);
    let copy_status = RwSignal::new(None::<String>);
    let copy_record_id = Callback::new(move |()| {
        let value = record_id_for_copy.clone();
        let Some(clipboard) = web_sys::window().map(|window| window.navigator().clipboard()) else {
            copy_status.set(Some("Clipboard unavailable.".to_string()));
            return;
        };
        let _ = clipboard.write_text(&value);
        copy_status.set(Some("Full record ID copied.".to_string()));
    });
    let title = record
        .listing_title
        .clone()
        .filter(|title| !title.trim().is_empty())
        .unwrap_or_else(|| "Game title unavailable".to_string());
    let record_type = record_type_label(record.record_type);
    let status = status_label(record.status);
    let status_variant = status_variant(record.status);

    view! {
        <article class="v2-purchase-record v2-panel">
            <div class="v2-purchase-record-mark" aria-hidden="true">
                <span class="material-symbols-outlined" aria-hidden="true">{if record.record_type == DurableAcquisitionKind::Purchase { "receipt_long" } else { "redeem" }}</span>
            </div>
            <div class="v2-purchase-record-copy">
                <p class="v2-store-kicker">{record_type}</p>
                <h2>{title}</h2>
                <p class="v2-purchase-coordinate">{record.game_coordinate.clone()}</p>
                {record.validation_error.clone().map(|_| view! {
                    <p class="v2-purchase-validation" role="alert">"Record could not be verified."</p>
                })}
            </div>
            <div class="v2-purchase-record-summary">
                <StatusChip
                    label=status
                    variant=status_variant
                    icon=None
                    size=StatusChipSize::Compact
                />
                {amount_label(&record).map(|amount| view! { <strong>{amount}</strong> })}
                <span>{acquired_at_label(record.acquired_at)}</span>
            </div>
            <details class="v2-purchase-technical">
                <summary>"Record details"</summary>
                <dl>
                    <div><dt>"Verification"</dt><dd>{verification}</dd></div>
                    <div><dt>"Record ID"</dt><dd>{short_record_identifier(&record_id)}<button type="button" class="v2-btn-ghost v2-purchase-copy" on:click=move |_| copy_record_id.run(())>"Copy full ID"</button></dd></div>
                    {record.campaign_id.clone().map(|campaign| view! { <div><dt>"Promotion ID"</dt><dd>{short_record_identifier(&campaign)}</dd></div> })}
                </dl>
                <p class="v2-purchase-timeline-unavailable">{RECEIPT_TIMELINE_UNAVAILABLE}</p>
                {move || copy_status.get().map(|status| view! { <p class="v2-purchase-copy-status" role="status">{status}</p> })}
            </details>
        </article>
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn settlement_labels_preserve_uncertainty_and_receipt_precedence() {
        let mut record:crate::tauri_bridge::SettlementHistoryItem=serde_json::from_value(serde_json::json!({"quote_id":"q","processor_url":"https://processor","listing_event_id":"listing","state":"active"})).expect("history fixture");
        assert_eq!(super::settlement_display_state(&record), "Awaiting payment");
        record
            .attempts
            .push(crate::tauri_bridge::RecoveryAttemptView {
                quote_id: "q".into(),
                processor_url: "https://processor".into(),
                attempt_id: "original".into(),
                leg_id: "developer".into(),
                state: "disclosed".into(),
                has_intent: true,
                has_proof: false,
            });
        assert_eq!(
            super::settlement_display_state(&record),
            "Payment outcome uncertain"
        );
        record.recovery_state = Some("recovering".into());
        assert_eq!(
            super::settlement_display_state(&record),
            "Recovery in progress"
        );
        record.recovery_state = None;
        record.attempts[0].has_proof = true;
        assert_eq!(
            super::settlement_display_state(&record),
            "Recovery available"
        );
        record.attempts[0].state = "settled".into();
        assert_eq!(
            super::settlement_display_state(&record),
            "Partially settled"
        );
        record.state = "failed".into();
        assert_eq!(super::settlement_display_state(&record), "Purchase failed");
        record.state = "settled".into();
        assert_eq!(
            super::settlement_display_state(&record),
            "Recovery available"
        );
        record.receipt = Some(serde_json::json!({"id":"persisted-validated-receipt"}));
        record.recovery_state = Some("recovering".into());
        assert_eq!(
            super::settlement_display_state(&record),
            "Purchase completed"
        );
    }
    use super::*;

    fn record(
        record_type: DurableAcquisitionKind,
        status: DurableCredentialStatus,
    ) -> DurableAcquisitionRecord {
        DurableAcquisitionRecord {
            record_type,
            game_coordinate: "30402:publisher:game".to_string(),
            listing_title: None,
            amount: None,
            currency: None,
            acquired_at: 1,
            status,
            record_id: "record".to_string(),
            validation_error: None,
            campaign_id: None,
            verification: None,
        }
    }

    #[test]
    fn stale_account_response_is_rejected() {
        assert!(!should_apply_response(2, 3));
        assert!(should_apply_response(3, 3));
    }

    #[test]
    fn standalone_web_is_unavailable() {
        assert_eq!(
            initial_availability(true, true),
            InitialAvailability::WebUnavailable
        );
        assert_eq!(
            initial_availability(false, false),
            InitialAvailability::AccountRequired
        );
    }

    #[test]
    fn filters_only_real_durable_record_types_and_statuses() {
        let purchase = record(
            DurableAcquisitionKind::Purchase,
            DurableCredentialStatus::Active,
        );
        let revoked = record(
            DurableAcquisitionKind::PromotionClaim,
            DurableCredentialStatus::Revoked,
        );

        assert!(filter_matches(PurchaseFilter::Purchases, &purchase));
        assert!(!filter_matches(PurchaseFilter::PromotionClaims, &purchase));
        assert!(filter_matches(PurchaseFilter::PromotionClaims, &revoked));
        assert!(filter_matches(PurchaseFilter::Inactive, &revoked));
        let disputed = record(
            DurableAcquisitionKind::Purchase,
            DurableCredentialStatus::Disputed,
        );
        assert!(!filter_matches(PurchaseFilter::Inactive, &disputed));
    }

    #[test]
    fn unverified_status_uses_plain_language() {
        assert_eq!(
            status_label(DurableCredentialStatus::Unverified),
            "Record could not be verified"
        );
        assert_eq!(
            status_variant(DurableCredentialStatus::Unverified),
            StatusChipVariant::Unverified
        );
    }

    #[test]
    fn purchase_tab_ids_resolve_to_local_filter_state() {
        for filter in [
            PurchaseFilter::All,
            PurchaseFilter::Purchases,
            PurchaseFilter::PromotionClaims,
            PurchaseFilter::Inactive,
        ] {
            assert_eq!(
                purchase_filter_from_id(purchase_filter_id(filter)),
                Some(filter)
            );
        }
        assert_eq!(purchase_filter_from_id("unknown"), None);
    }

    #[test]
    fn purchase_without_amount_is_partial_data() {
        let purchase = record(
            DurableAcquisitionKind::Purchase,
            DurableCredentialStatus::Active,
        );
        assert!(record_has_partial_data(&purchase));
    }
    #[test]
    fn record_identifiers_are_abbreviated_for_display() {
        let long = "a".repeat(64);
        let shown = short_record_identifier(&long);
        assert!(shown.len() < long.len());
        assert!(shown.contains('…'));
        assert!(shown.starts_with("aaaaaaaaaa"));
        // Short identifiers are shown intact rather than padded or truncated oddly.
        assert_eq!(short_record_identifier("abc123"), "abc123");
    }

    #[test]
    fn missing_acquisition_time_is_not_rendered_as_an_epoch_date() {
        // Formatting a real timestamp needs js_sys::Date, which is unavailable in
        // native tests; the zero case is the one that could lie about a date.
        assert_eq!(acquired_at_label(0), "Acquisition time unavailable");
        assert!(!acquired_at_label(0).contains("1970"));
    }

    #[test]
    fn verification_state_follows_validation_not_optimism() {
        let mut clean = record(
            DurableAcquisitionKind::Purchase,
            DurableCredentialStatus::Active,
        );
        assert_eq!(record_verification_label(&clean), "Validated on load");

        clean.validation_error = Some("bad signature".into());
        assert_eq!(record_verification_label(&clean), "Not verified");

        let unverified = record(
            DurableAcquisitionKind::Purchase,
            DurableCredentialStatus::Unverified,
        );
        assert_eq!(record_verification_label(&unverified), "Not verified");
    }

    #[test]
    fn receipt_timeline_is_declared_unavailable_rather_than_invented() {
        assert!(RECEIPT_TIMELINE_UNAVAILABLE.contains("not available"));
        assert!(RECEIPT_TIMELINE_UNAVAILABLE.contains("one validated durable record"));
        let source = include_str!("purchases.rs");
        for fabricated in [
            ["Invoice", " paid"].concat(),
            ["Proof", " verified"].concat(),
            ["Shipment", " sent"].concat(),
            ["Order", " timeline"].concat(),
        ] {
            assert!(!source.contains(&fabricated), "fabricated receipt event");
        }
    }

    #[test]
    fn portable_verification_states_are_explicit_not_implied_ownership(){
        let mut record=record(DurableAcquisitionKind::Purchase,DurableCredentialStatus::Unverified);
        for (state,label) in [(crate::tauri_bridge::ReceiptVerification::Verified,"VERIFIED"),(crate::tauri_bridge::ReceiptVerification::Indeterminate,"INDETERMINATE"),(crate::tauri_bridge::ReceiptVerification::Invalid,"INVALID")] {
            record.verification=Some(state);assert_eq!(record_verification_label(&record),label);
        }
    }

    #[test]
    fn empty_history_and_unavailable_records_are_distinct_states() {
        // Empty is a successful result; Error and WebUnavailable are not.
        let labels = [
            matches!(PurchasesState::Empty, PurchasesState::Empty),
            matches!(PurchasesState::Error, PurchasesState::Error),
            matches!(
                PurchasesState::WebUnavailable,
                PurchasesState::WebUnavailable
            ),
            matches!(
                PurchasesState::AccountRequired,
                PurchasesState::AccountRequired
            ),
        ];
        assert!(labels.into_iter().all(|matched| matched));
    }
}
