use crate::{
    tauri_bridge::{
        invoke_approve_purchase_plan, invoke_get_purchase_plan, invoke_resume_split_payment,
        ApprovePurchasePlanRequest, InvoiceNetwork, LegProgress, PurchasePlanRequest,
        PurchasePlanReview, ResumeSplitPaymentRequest,
    },
    AuthContext,
};
use arcadestr_core::purchase_contract::{
    UNKNOWN_ROUTING_FEES_ACKNOWLEDGEMENT, UNKNOWN_ROUTING_FEES_WARNING,
};
use leptos::prelude::*;
use wasm_bindgen_futures::spawn_local;

const PANEL:&str="padding:18px;border:1px solid oklch(0.32 0.01 60);background:#121215;clip-path:polygon(0 0,calc(100% - 12px) 0,100% 12px,100% 100%,12px 100%,0 calc(100% - 12px));font:400 12.5px/1.65 ui-monospace,monospace;color:oklch(0.68 0.01 60);overflow-wrap:anywhere";
const SECONDARY:&str="padding:9px 13px;border:1px solid oklch(0.5 0.13 210);background:transparent;color:oklch(0.8 0.14 195);font:800 11px ui-monospace,monospace;border-radius:4px";
const PRIMARY:&str="padding:11px 16px;border:0;background:oklch(0.78 0.16 60);color:#0b0b0d;font:800 12px ui-monospace,monospace;border-radius:5px";
const PRIMARY_DISABLED:&str="padding:11px 16px;border:1px solid rgba(255,255,255,.14);background:#1c1c20;color:oklch(0.5 0.01 60);font:800 12px ui-monospace,monospace;border-radius:5px;cursor:not-allowed";

/// Inline styles cannot target `:disabled`; derive the look from the same state.
pub(crate) fn primary_style(disabled: bool) -> &'static str {
    if disabled {
        PRIMARY_DISABLED
    } else {
        PRIMARY
    }
}
const FIELD:&str="padding:9px;border:1px solid rgba(255,255,255,.22);background:#141417;color:#fff;font:400 12px ui-monospace,monospace;border-radius:4px;width:100%;box-sizing:border-box";

pub fn msat_label(value: u64) -> String {
    if value % 1000 == 0 {
        format!("{} sats", value / 1000)
    } else {
        format!("{}.{:03} sats", value / 1000, value % 1000)
    }
}
pub fn progress_label(progress: &LegProgress) -> &'static str {
    match progress {
        LegProgress::Pending => "PENDING — NOT VERIFIED",
        LegProgress::Uncertain => "UNCERTAIN — NO REPAYMENT",
        LegProgress::Paid => "PAID — PROOF SUBMISSION PENDING",
        LegProgress::ProofAccepted => "PAYMENT PROOF ACCEPTED",
        LegProgress::FailedNotSent => "FAILED PURCHASE — LEG NOT SENT",
    }
}
fn buyer(auth: &Option<AuthContext>, tracked: bool) -> Option<String> {
    let auth = auth.as_ref()?;
    let npub = if tracked {
        auth.npub.get()
    } else {
        auth.npub.get_untracked()
    }?;
    super::blossom_media_upload::publisher_hex(&npub)
}
/// Routing-fee display for one leg. Missing reports are never shown as zero.
pub fn leg_fee_label(progress: &LegProgress, fees_paid_msat: Option<u64>) -> String {
    match (progress, fees_paid_msat) {
        (LegProgress::Paid | LegProgress::ProofAccepted, Some(fee)) => {
            format!("Routing fees paid: {}", msat_label(fee))
        }
        (LegProgress::Paid | LegProgress::ProofAccepted, None) => {
            "Routing fees not reported".into()
        }
        (LegProgress::Uncertain, _) => {
            "Payment outcome not yet known; routing fees not reported".into()
        }
        (LegProgress::Pending | LegProgress::FailedNotSent, _) => {
            "No payment sent for this part".into()
        }
    }
}

/// The approved fee policy in buyer language; no estimate is ever derived.
pub fn fee_policy_label(review: &PurchasePlanReview) -> Option<String> {
    use arcadestr_core::purchase_contract::RoutingFeePolicy;
    Some(match review.approved_fee_policy? {
        RoutingFeePolicy::UnknownFeesAcknowledged => {
            "Approved: Lightning routing fees are set by your wallet and were not known in advance (acknowledged).".into()
        }
        RoutingFeePolicy::ReportedFees => format!(
            "Approved under the earlier reported-fee policy: reserve {} (accounting only, not a wallet-enforced cap).",
            review
                .approved_limits
                .as_ref()
                .and_then(|l| l.fee_reserve_msat)
                .map(msat_label)
                .unwrap_or_else(|| "unavailable".into())
        ),
        RoutingFeePolicy::ProviderCeiling => {
            "Approved: strict wallet-enforced routing-fee ceiling.".into()
        }
    })
}

/// Builds the warning-only approval. There is no fee estimate, reserve or
/// maximum total; the buyer must explicitly acknowledge unknown routing fees.
pub fn approval_request(
    review: &PurchasePlanReview,
    account: &str,
    network: Option<InvoiceNetwork>,
    confirmed: bool,
    fees_acknowledged: bool,
) -> Result<ApprovePurchasePlanRequest, String> {
    if !confirmed
        || !review.terms_verified
        || !review.wallet_connected
        || review.halted
        || review.purchase_completed
        || review.settlement_state != "active"
        || review.approved_limits.is_some()
        || review.quote.buyer_pubkey != account
    {
        return Err("Review the verified plan as its original buyer before approving.".into());
    }
    if !fees_acknowledged {
        return Err(
            "Acknowledge that additional Lightning fees may apply before approving.".into(),
        );
    }
    if review.network.is_some() && review.network != network {
        return Err("Selected network differs from the original invoice.".into());
    }
    Ok(ApprovePurchasePlanRequest {
        expected_account_pubkey: account.into(),
        quote_id: review.quote.quote_id.clone(),
        quote_event_id: review.quote_event_id.clone(),
        commitment: review.commitment.clone(),
        network: network.ok_or("Select the invoice network explicitly")?,
        fee_reserve_msat: None,
        total_msat: None,
        confirm_complete_plan: true,
        fee_policy: arcadestr_core::purchase_contract::RoutingFeePolicy::UnknownFeesAcknowledged,
        accept_unenforced_fee_risk: true,
    })
}

/// Reused on checkout and purchase history. Approval stores consent only; every
/// recovery action checks an existing payment and cannot initiate a new one.
#[component]
pub fn PurchasePlanPanel(
    quote_id: String,
    server_url: String,
    #[prop(optional)] on_review: Option<Callback<Option<PurchasePlanReview>>>,
    #[prop(optional)] on_recovered: Option<Callback<()>>,
    #[prop(optional)] external_refresh: Option<RwSignal<u64>>,
) -> impl IntoView {
    let auth = use_context::<AuthContext>();
    let payment_activation = crate::ui_v2::payment_activation::use_payment_activation();
    let wallet = use_context::<crate::ui_v2::nwc_wallet::NwcWalletContext>();
    let review = RwSignal::new(None::<PurchasePlanReview>);
    let error = RwSignal::new(None::<String>);
    let busy = RwSignal::new(false);
    let confirmed = RwSignal::new(false);
    let network = RwSignal::new(None::<InvoiceNetwork>);
    let refresh = RwSignal::new(0_u64);
    let generation = RwSignal::new(0_u64);
    let fee_risk = RwSignal::new(false);
    let load_auth = auth.clone();
    let load_quote = quote_id.clone();
    Effect::new(move |_| {
        let _ = refresh.get();
        if let Some(epoch) = external_refresh {
            let _ = epoch.get();
        }
        let wallet_busy = wallet.is_some_and(|w| {
            let _ = w.metadata.get();
            let _ = w.loaded.get();
            w.busy.get()
        });
        let account = buyer(&load_auth, true);
        let auth_epoch = load_auth
            .as_ref()
            .map(|a| a.current_auth_flow())
            .unwrap_or(0);
        generation.update(|v| *v = v.wrapping_add(1));
        let epoch = generation.get_untracked();
        review.set(None);
        if let Some(callback) = on_review {
            callback.run(None);
        }
        confirmed.set(false);
        fee_risk.set(false);
        error.set(None);
        if wallet_busy {
            busy.set(true);
            return;
        }
        let Some(account) = account else {
            error.set(Some(
                "Connect the original buyer to review this settlement.".into(),
            ));
            busy.set(false);
            return;
        };
        busy.set(true);
        let auth = load_auth.clone();
        let quote_id = load_quote.clone();
        spawn_local(async move {
            let result = invoke_get_purchase_plan(PurchasePlanRequest {
                expected_account_pubkey: account.clone(),
                quote_id,
            })
            .await;
            if generation.try_get_untracked() != Some(epoch)
                || buyer(&auth, false).as_deref() != Some(account.as_str())
                || !auth
                    .as_ref()
                    .is_some_and(|a| a.is_current_auth_flow(auth_epoch))
            {
                return;
            }
            busy.set(false);
            match result {
                Ok(plan) => {
                    if plan.quote.buyer_pubkey != account {
                        error.set(Some("Buyer changed; settlement review discarded.".into()));
                        return;
                    }
                    network.set(plan.network);
                    // Reflect only a durable acknowledgement; never preselect.
                    fee_risk.set(plan.approved_fee_notice.is_some());
                    if let Some(callback) = on_review {
                        callback.run(Some(plan.clone()));
                    }
                    review.set(Some(plan));
                }
                Err(e) => error.set(Some(e)),
            }
        });
    });
    let approve_auth = auth.clone();
    let approve = Callback::new(move |()| {
        if busy.get_untracked() {
            return;
        }
        let Some(account) = buyer(&approve_auth, false) else {
            return;
        };
        let Some(plan) = review.get_untracked() else {
            return;
        };
        let request = match approval_request(
            &plan,
            &account,
            network.get_untracked(),
            confirmed.get_untracked(),
            fee_risk.get_untracked(),
        ) {
            Ok(v) => v,
            Err(e) => {
                error.set(Some(e));
                return;
            }
        };
        let epoch = generation.get_untracked();
        let auth_epoch = approve_auth
            .as_ref()
            .map(|a| a.current_auth_flow())
            .unwrap_or(0);
        let auth = approve_auth.clone();
        busy.set(true);
        error.set(None);
        spawn_local(async move {
            let result = invoke_approve_purchase_plan(request).await;
            if generation.try_get_untracked() != Some(epoch)
                || buyer(&auth, false).as_deref() != Some(account.as_str())
                || !auth
                    .as_ref()
                    .is_some_and(|a| a.is_current_auth_flow(auth_epoch))
            {
                return;
            }
            busy.set(false);
            confirmed.set(false);
            match result {
                Ok(plan) => {
                    if let Some(callback) = on_review {
                        callback.run(Some(plan.clone()));
                    }
                    review.set(Some(plan));
                }
                Err(e) => error.set(Some(e)),
            }
        });
    });
    let recover_auth = auth.clone();
    let recovery_quote = quote_id.clone();
    let recover = Callback::new(move |attempt: String| {
        if busy.get_untracked() {
            return;
        }
        let Some(account) = buyer(&recover_auth, false) else {
            return;
        };
        let epoch = generation.get_untracked();
        let auth_epoch = recover_auth
            .as_ref()
            .map(|a| a.current_auth_flow())
            .unwrap_or(0);
        let auth = recover_auth.clone();
        let quote_id = recovery_quote.clone();
        let server_url = server_url.clone();
        busy.set(true);
        error.set(None);
        spawn_local(async move {
            let result = invoke_resume_split_payment(ResumeSplitPaymentRequest {
                server_url,
                quote_id,
                attempt_id: attempt,
            })
            .await;
            if generation.try_get_untracked() != Some(epoch)
                || buyer(&auth, false).as_deref() != Some(account.as_str())
                || !auth
                    .as_ref()
                    .is_some_and(|a| a.is_current_auth_flow(auth_epoch))
            {
                return;
            }
            busy.set(false);
            if let Err(e) = result {
                error.set(Some(e));
            }
            // Keep an inconclusive error visible; explicit Refresh re-reads state.
            if error.get_untracked().is_none() {
                refresh.update(|v| *v = v.wrapping_add(1));
                if let Some(callback) = on_recovered {
                    callback.run(());
                }
            }
        });
    });
    view! {
        <section style=PANEL data-testid="purchase-plan-panel" aria-label="Settlement plan and recovery">
            <h3 style="margin:0 0 12px;font:800 12px ui-monospace,monospace;letter-spacing:1.5px;color:oklch(0.8 0.16 60)">"◆ SETTLEMENT PLAN"</h3>
            <p style="color:oklch(0.8 0.14 195)">"Approving the plan does not send a payment or unlock the wallet."</p>
            {move || error.get().map(|e|view!{<p role="alert" style="color:oklch(0.65 0.18 25)">{e}</p>})}
            {move || if busy.get(){Some(view!{<p role="status">"Checking the original plan or payment…"</p>})}else{None}}
            {move || review.get().map(|r|{
                let can_approve=r.terms_verified && r.wallet_connected && !r.halted && !r.purchase_completed && r.settlement_state=="active" && r.approved_limits.is_none();
                let immutable=r.approved_limits.is_some();
                let approve_disabled=move||!can_approve||!fee_risk.get()||!confirmed.get()||busy.get()||!payment_activation.buyer_enabled()||!payment_activation.reported_fees_allowed();
                let state_label=plan_state_label(&r, payment_activation.buyer_enabled());
                let statuses=r.legs.clone();
                let operator_fees=r.quote.legs.iter().filter(|l|l.role==arcadestr_core::adp_settlement::PaymentRole::Operator).map(|l|l.amount_msat).sum::<u64>();
                let capabilities=r.wallet_capabilities.as_ref().map(|c|format!("Advertised: pay_invoice {}; lookup_invoice {}. {}",c.supports("pay_invoice"),c.supports("lookup_invoice"),c.network.as_deref().unwrap_or("network unavailable"))).unwrap_or_else(||"Wallet capabilities unavailable; no payment authority inferred.".into());
                view!{
                    <p style="color:#fff" data-testid="purchase-game-price">{format!("Game price: {} · included operator fees: {}",msat_label(r.quote.gross_msat),msat_label(operator_fees))}</p>
                    <p>"Recipients receive exactly the amounts below. Lightning routing fees are charged by your wallet in addition to the game price."</p>
                    <p>{format!("Maximum payment dispatches: {} — only original plan legs; no atomic batch settlement.",r.quote.legs.len())}</p>
                    <p>{format!("Buyer: {} · purchase: {}",r.quote.buyer_pubkey,r.quote.quote_id)}</p>
                    <p>{format!("Listing: {} · original quote: {}",r.quote.listing_event_id,r.quote_event_id)}</p>
                    <p data-testid="purchase-completion" style="color:oklch(0.8 0.14 195)">{if r.purchase_completed {"PURCHASE COMPLETED — VALIDATED RECEIPT STORED"}else{"PURCHASE NOT COMPLETED — ALL LEGS AND A VALIDATED RECEIPT ARE REQUIRED"}}</p>
                    <p>{if r.terms_verified {"Developer-accepted terms and deterministic split verified locally."}else{"Original listing/term evidence unavailable or unverified. Approval is disabled; read-only recovery remains available."}}</p>
                    <p>{capabilities}</p>
                    <p>"Aggregate receipts publicly link the buyer, listing, recipients, amounts and payment hashes. This reveals the revenue split and can correlate payments. Raw invoices, preimages and refund destinations remain encrypted."</p>
                    {fee_policy_label(&r).map(|label|view!{<p data-testid="purchase-fee-policy">{label}</p>})}
                    <p>"Routing fees go to the Lightning network, not to the publisher or the ADP operator. Refunds return the game price only, not network fees. Multiple Lightning payments are not atomic."</p>
                    <div style="display:grid;gap:10px;margin:14px 0">
                    {r.quote.legs.into_iter().map(|leg|{
                        let status=statuses.iter().find(|s|s.leg_id==leg.leg_id).cloned();
                        let label=status.as_ref().map(|s|progress_label(&s.progress)).unwrap_or("PENDING — NOT VERIFIED");
                        let payee=status.as_ref().and_then(|s|s.invoice_payee.clone()).unwrap_or_else(||"Not yet disclosed; only the original invoice can be bound.".into());
                        let attempt=status.as_ref().filter(|s|s.can_reconcile).and_then(|s|s.attempt_id.clone());let disabled=attempt.is_none();
                        view!{
                            <article style="padding:12px;border:1px solid rgba(255,255,255,.1);background:#141417" data-testid="purchase-plan-leg">
                                <strong style="color:#fff">{format!("{} — {}",leg.leg_id,msat_label(leg.amount_msat))}</strong>
                                <p>{format!("Recipient: {} · destination: {}",leg.recipient_pubkey,leg.destination)}</p>
                                <p>{format!("Accepted terms: {}",leg.accepted_terms_id.unwrap_or_else(||"Developer listing policy".into()))}</p>
                                <p>{format!("Original invoice signing key: {payee}")}</p>
                                <p data-testid="purchase-leg-progress" style="color:oklch(0.8 0.16 60)">{label}</p>
                                <p data-testid="purchase-leg-fee">{status.as_ref().map(|s|leg_fee_label(&s.progress,s.fees_paid_msat)).unwrap_or_else(||leg_fee_label(&LegProgress::Pending,None))}</p>
                                <button type="button" style=SECONDARY disabled=move||disabled||busy.get() on:click=move |_|{if let Some(id)=attempt.clone(){recover.run(id);}}>"CHECK ORIGINAL PAYMENT — NO REPAYMENT"</button>
                            </article>
                        }
                    }).collect_view()}
                    </div>
                    <div style="display:grid;grid-template-columns:repeat(auto-fit,minmax(180px,1fr));gap:12px">
                        <label>"INVOICE NETWORK"<select style=FIELD disabled=immutable prop:value=move||network.get().map(|n|format!("{n:?}")).unwrap_or_default() on:change:target=move|e|{network.set(match e.target().value().as_str(){"Bitcoin"=>Some(InvoiceNetwork::Bitcoin),"Testnet"=>Some(InvoiceNetwork::Testnet),"Signet"=>Some(InvoiceNetwork::Signet),"Regtest"=>Some(InvoiceNetwork::Regtest),_=>None});confirmed.set(false);}>
                            <option value="">"Select explicitly"</option><option value="Bitcoin">"Bitcoin"</option><option value="Testnet">"Testnet"</option><option value="Signet">"Signet"</option><option value="Regtest">"Regtest"</option>
                        </select></label>
                    </div>
                    <p data-testid="buyer-payment-policy">{move || payment_activation.buyer_label()}</p>
                    <p data-testid="purchase-approved-state">{state_label}</p>
                    <p role="note" data-testid="purchase-fee-warning" style="padding:10px 12px;border:1px solid oklch(0.6 0.15 60);color:oklch(0.85 0.12 70)">{UNKNOWN_ROUTING_FEES_WARNING}</p>
                    <label style="display:flex;align-items:flex-start;gap:8px;margin:14px 0"><input data-testid="purchase-fee-risk" type="checkbox" disabled=move||immutable||busy.get()||!payment_activation.reported_fees_allowed() prop:checked=move||fee_risk.get() on:change:target=move|e|{fee_risk.set(e.target().checked());confirmed.set(false);}/>{UNKNOWN_ROUTING_FEES_ACKNOWLEDGEMENT}</label>
                    <p>{move || (!immutable && !payment_activation.reported_fees_allowed()).then_some("This build accepts only wallets that enforce a routing-fee ceiling. Standard NWC wallets cannot be used for approval.")}</p>
                    <label style="display:flex;align-items:flex-start;gap:8px;margin:14px 0"><input data-testid="purchase-approval-check" type="checkbox" disabled=move||!can_approve||!fee_risk.get()||busy.get()||!payment_activation.buyer_enabled() prop:checked=move||confirmed.get() on:change:target=move|e|confirmed.set(e.target().checked())/>"I approve every recipient, amount, and operator fee shown. Approval alone does not send a payment."</label>
                    <button data-testid="approve-purchase-plan" type="button" style=move||primary_style(approve_disabled()) disabled=approve_disabled on:click=move |_|approve.run(())>"APPROVE IMMUTABLE PLAN"</button>
                }
            })}
            <button type="button" style=SECONDARY disabled=move||busy.get() on:click=move |_|refresh.update(|v|*v=v.wrapping_add(1))>"REFRESH ORIGINAL STATE"</button>
        </section>
    }
}

/// Debug-only, credential-free browser fixture. It cannot select a real account
/// or quote; tests inject an allowlisted mock IPC bridge before mounting WASM.
#[cfg(debug_assertions)]
#[component]
pub fn PurchasePlanPreview() -> impl IntoView {
    use nostr::nips::nip19::ToBech32;
    let auth = AuthContext::new();
    if let Ok(key) = nostr::PublicKey::from_hex(
        "79be667ef9dcbbac55a06295ce870b07029bfcdb2dce28d959f2815b16f81798",
    ) {
        auth.npub.set(key.to_bech32().ok());
    }
    provide_context(auth);
    let wallet = crate::ui_v2::nwc_wallet::NwcWalletContext::new();
    wallet.loaded.set(true);
    provide_context(wallet);
    let parent_review = RwSignal::new(None::<PurchasePlanReview>);
    let recovered = RwSignal::new(0_u64);
    view! {
        <main style="max-width:1440px;margin:0 auto;padding:24px;box-sizing:border-box;background:#0b0b0d">
            <p style="font:600 12px ui-monospace,monospace;color:oklch(0.8 0.14 195)">"LOCAL UI FIXTURE — MOCK IPC ONLY — NO WALLET OR PAYMENT"</p>
            <p data-testid="plan-parent-approved">{move || parent_review.get().is_some_and(|p|p.approved).to_string()}</p>
            <p data-testid="plan-parent-dispatch-ready">{move || parent_review.get().is_some_and(|p|p.dispatch_ready(101)).to_string()}</p>
            <p data-testid="plan-parent-recovered">{move || recovered.get().to_string()}</p>
            <button data-testid="mock-wallet-change" style=SECONDARY on:click=move |_|wallet.metadata.set(Some(crate::tauri_bridge::ConnectNwcWalletResponse{wallet_pubkey:"mock-wallet".into(),relays:Vec::new(),lud16:None}))>"SIMULATE WALLET CHANGE — MOCK ONLY"</button>
            <PurchasePlanPanel quote_id="mock-purchase".to_string() server_url="http://127.0.0.1:1".to_string()
                on_review=Callback::new(move |review|parent_review.set(review))
                on_recovered=Callback::new(move |()|recovered.update(|v|*v+=1))/>
        </main>
    }
}

/// The plan's actual state, most terminal fact first. Approval never implies
/// payment, and nothing short of a stored validated receipt reads as complete.
pub(crate) fn plan_state_label(r: &PurchasePlanReview, buyer_enabled: bool) -> &'static str {
    match r.settlement_state.as_str() {
        _ if r.purchase_completed => "PURCHASE COMPLETED — VALIDATED RECEIPT STORED",
        "failed" => "PURCHASE FAILED — NO FURTHER PAYMENTS; SETTLED LEGS ARE REFUND OBLIGATIONS",
        "superseded" => "CLOSED WITHOUT PAYMENT — QUOTE EXPIRED OR LISTING CHANGED",
        "settled" => "PAYMENTS SETTLED — RECEIPT NOT YET VALIDATED; RECOVER, DO NOT PAY AGAIN",
        _ if r.halted => "FURTHER PAYMENTS HALTED — RECOVER THE ORIGINAL PURCHASE",
        _ if r.approved && !buyer_enabled => {
            "PLAN APPROVED — BUYER PAYMENTS ARE DISABLED IN THIS BUILD"
        }
        _ if r.approved => "PLAN APPROVED — EACH PAYMENT REQUIRES YOUR EXPLICIT ACTION",
        _ if r.approved_limits.is_some() => {
            "APPROVAL IS BOUND TO THE ORIGINAL WALLET; RESTORE THAT CONNECTION"
        }
        _ if !buyer_enabled => "AWAITING APPROVAL — BUYER PAYMENTS ARE DISABLED IN THIS BUILD",
        _ => "AWAITING YOUR APPROVAL — NO PAYMENT HAS BEEN SENT",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn plan_state_reflects_terminal_state_before_approval() {
        let mut r = review();
        assert_eq!(
            plan_state_label(&r, true),
            "AWAITING YOUR APPROVAL — NO PAYMENT HAS BEEN SENT"
        );
        assert!(plan_state_label(&r, false).contains("DISABLED IN THIS BUILD"));
        r.approved = true;
        assert!(plan_state_label(&r, true).starts_with("PLAN APPROVED — EACH PAYMENT"));
        assert!(plan_state_label(&r, false).contains("DISABLED"));
        for (state, prefix) in [
            ("failed", "PURCHASE FAILED"),
            ("superseded", "CLOSED WITHOUT PAYMENT"),
            ("settled", "PAYMENTS SETTLED — RECEIPT NOT"),
        ] {
            r.settlement_state = state.into();
            assert!(plan_state_label(&r, true).starts_with(prefix), "{state}");
            assert!(!plan_state_label(&r, true).contains("LOCKED"));
        }
        r.purchase_completed = true;
        assert!(plan_state_label(&r, true).starts_with("PURCHASE COMPLETED"));
    }

    #[test]
    fn amounts_are_exact_and_partial_progress_is_not_ownership() {
        assert_eq!(msat_label(100000), "100 sats");
        assert_eq!(msat_label(100001), "100.001 sats");
        assert!(progress_label(&LegProgress::Paid).contains("PENDING"));
        assert!(progress_label(&LegProgress::Uncertain).contains("NO REPAYMENT"));
    }
    fn review() -> PurchasePlanReview {
        serde_json::from_value(serde_json::json!({
            "quote":{"version":1,"quote_id":"purchase","buyer_pubkey":"buyer","listing_event_id":"listing","listing_coordinate":"30402:developer:test","processor_pubkey":"operator","gross_msat":200000,"issued_at":100,"payment_expires_at":1900,"confirmation_expires_at":86500,"refund_destination_commitment":"refund","legs":[{"leg_id":"developer","role":"developer","recipient_pubkey":"developer","destination":"bob@example.test","amount_msat":100000},{"leg_id":"operator","role":"operator","recipient_pubkey":"operator","destination":"carol@example.test","amount_msat":100000,"accepted_terms_id":"accepted-root"}],"state":"active"},
            "quote_event_id":"signed-quote","commitment":"immutable-review","network":"Regtest","terms_verified":true,"approved":false,"approved_limits":null,"halted":false,"purchase_completed":false,"settlement_state":"active","wallet_connected":true,"wallet_capabilities":null,"legs":[],"restriction":"Live execution locked"
        })).expect("UI-only plan fixture")
    }
    #[test]
    fn approval_requires_explicit_fee_acknowledgement_and_invents_no_fee_limits() {
        use arcadestr_core::purchase_contract::RoutingFeePolicy;
        let r = review();
        // Neither consent may be implied.
        assert!(approval_request(&r, "buyer", r.network, false, true).is_err());
        assert!(approval_request(&r, "buyer", r.network, true, false).is_err());
        let request =
            approval_request(&r, "buyer", r.network, true, true).expect("explicit approval");
        assert_eq!(
            request.fee_policy,
            RoutingFeePolicy::UnknownFeesAcknowledged
        );
        assert!(request.accept_unenforced_fee_risk);
        assert_eq!(request.fee_reserve_msat, None);
        assert_eq!(request.total_msat, None);
        assert!(request.confirm_complete_plan);
        assert_eq!(request.quote_event_id, "signed-quote");
        assert_eq!(request.commitment, "immutable-review");
        assert!(approval_request(&r, "other-buyer", r.network, true, true).is_err());
        assert!(approval_request(&r, "buyer", Some(InvoiceNetwork::Bitcoin), true, true).is_err());
        // The wire request serializes no reserve or maximum total.
        let json = serde_json::to_value(&request).expect("json");
        assert!(json["fee_reserve_msat"].is_null() && json["total_msat"].is_null());
        assert_eq!(json["fee_policy"], "unknown_fees_acknowledged");
    }

    #[test]
    fn disabled_primary_action_is_visibly_distinct() {
        assert_eq!(primary_style(false), PRIMARY);
        let disabled = primary_style(true);
        assert_ne!(disabled, PRIMARY);
        assert!(disabled.contains("cursor:not-allowed"));
        assert!(
            !disabled.contains("oklch(0.78 0.16 60)"),
            "no active accent"
        );
    }

    #[test]
    fn fee_labels_report_actual_or_unknown_fees_never_zero_or_estimates() {
        use arcadestr_core::purchase_contract::RoutingFeePolicy;
        assert_eq!(
            leg_fee_label(&LegProgress::Paid, Some(1500)),
            "Routing fees paid: 1.500 sats"
        );
        assert_eq!(
            leg_fee_label(&LegProgress::ProofAccepted, Some(0)),
            "Routing fees paid: 0 sats"
        );
        for progress in [LegProgress::Paid, LegProgress::ProofAccepted] {
            assert_eq!(leg_fee_label(&progress, None), "Routing fees not reported");
        }
        assert!(!leg_fee_label(&LegProgress::Uncertain, None).contains("0 sats"));
        assert!(leg_fee_label(&LegProgress::Pending, None).contains("No payment"));
        let mut r = review();
        assert_eq!(fee_policy_label(&r), None);
        r.approved_fee_policy = Some(RoutingFeePolicy::UnknownFeesAcknowledged);
        let label = fee_policy_label(&r).expect("label");
        assert!(label.contains("not known in advance"));
        assert!(!label.contains("sats"), "no estimate or reserve is shown");
        r.approved_fee_policy = Some(RoutingFeePolicy::ReportedFees);
        r.approved_limits = Some(arcadestr_core::purchase_contract::PurchaseLimits {
            principal_msat: 200000,
            fee_reserve_msat: Some(50000),
            total_msat: Some(250000),
            maximum_dispatches: 2,
        });
        assert!(fee_policy_label(&r).expect("legacy").contains("50 sats"));
        assert!(UNKNOWN_ROUTING_FEES_WARNING.contains("may exceed the game price"));
        assert_eq!(
            UNKNOWN_ROUTING_FEES_ACKNOWLEDGEMENT,
            "I understand that additional Lightning fees may apply."
        );
    }

    #[test]
    fn checkout_requires_current_approval_capability_and_resolved_prior_legs() {
        use arcadestr_core::purchase_contract::{PurchaseLegStatus, WalletCapabilities};
        let mut r = review();
        r.legs = r
            .quote
            .legs
            .iter()
            .map(|leg| PurchaseLegStatus {
                leg_id: leg.leg_id.clone(),
                progress: LegProgress::Pending,
                attempt_id: None,
                payment_hash: None,
                invoice_payee: None,
                fees_paid_msat: None,
                can_reconcile: false,
            })
            .collect();
        assert!(!r.dispatch_ready(101));
        r.approved = true;
        assert!(r.dispatch_ready(101));
        r.wallet_capabilities = Some(WalletCapabilities {
            methods: vec!["get_info".into(), "lookup_invoice".into()],
            network: Some("regtest".into()),
        });
        assert!(!r.dispatch_ready(101));
        r.wallet_capabilities
            .as_mut()
            .expect("capabilities")
            .methods
            .push("pay_invoice".into());
        assert!(r.dispatch_ready(101));
        for progress in [
            LegProgress::Uncertain,
            LegProgress::Paid,
            LegProgress::FailedNotSent,
        ] {
            r.legs[0].progress = progress;
            assert!(!r.dispatch_ready(101));
        }
        r.legs[0].progress = LegProgress::ProofAccepted;
        assert!(r.dispatch_ready(101));
        assert!(!r.dispatch_ready(r.quote.payment_expires_at));
        r.wallet_connected = false;
        assert!(!r.dispatch_ready(101));
    }
    #[test]
    fn unavailable_changed_or_already_approved_state_cannot_be_approved_again() {
        for mutation in 0..5 {
            let mut r = review();
            match mutation {
                0 => r.terms_verified = false,
                1 => r.wallet_connected = false,
                2 => r.halted = true,
                3 => r.settlement_state = "failed".into(),
                _ => r.purchase_completed = true,
            };
            assert!(approval_request(&r, "buyer", r.network, true, true).is_err());
        }
    }
}
