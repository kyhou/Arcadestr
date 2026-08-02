use leptos::prelude::*;
use wasm_bindgen_futures::spawn_local;

use crate::models::{DurableAcquisitionKind, DurableAcquisitionRecord, DurableCredentialStatus};
use crate::tauri_bridge::invoke_get_purchase_records;
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

#[component]
pub fn PurchasesView() -> impl IntoView {
    let auth = use_context::<AuthContext>().expect("AuthContext not provided");
    let state = RwSignal::new(PurchasesState::Loading);
    let filter = RwSignal::new(PurchaseFilter::All);
    let request_generation = RwSignal::new(0_u64);
    let selected_filter = Signal::derive(move || purchase_filter_id(filter.get()).to_string());
    let select_filter = Callback::new(move |id: String| {
        if let Some(next) = purchase_filter_from_id(&id) {
            filter.set(next);
        }
    });

    #[cfg(feature = "web")]
    state.set(PurchasesState::WebUnavailable);

    #[cfg(not(feature = "web"))]
    Effect::new(move |_| {
        let account = auth.npub.get().unwrap_or_default();
        let generation = request_generation.get_untracked().wrapping_add(1);
        request_generation.set(generation);

        match initial_availability(false, !account.is_empty()) {
            InitialAvailability::AccountRequired => {
                state.set(PurchasesState::AccountRequired);
                return;
            }
            InitialAvailability::Load => state.set(PurchasesState::Loading),
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

    view! {
        <section class="v2-purchases">
            <header class="v2-purchases-hero v2-panel-glass">
                <p class="v2-store-kicker">"Account records"</p>
                <h1 class="v2-display">"Purchases and access"</h1>
                <p>"Durable purchases and promotion claims recorded for your active account."</p>
            </header>

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
                <span class="material-symbols-outlined">{if record.record_type == DurableAcquisitionKind::Purchase { "receipt_long" } else { "redeem" }}</span>
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
                <span>{format!("Acquired: {}", record.acquired_at)}</span>
            </div>
            <details class="v2-purchase-technical">
                <summary>"Record details"</summary>
                <dl>
                    <div><dt>"Record ID"</dt><dd>{record.record_id}</dd></div>
                    {record.campaign_id.map(|campaign| view! { <div><dt>"Promotion ID"</dt><dd>{campaign}</dd></div> })}
                </dl>
            </details>
        </article>
    }
}

#[cfg(test)]
mod tests {
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
}
