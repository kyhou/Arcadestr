use crate::{
    tauri_bridge::{
        invoke_approve_purchase_plan, invoke_get_purchase_plan, invoke_resume_split_payment,
        ApprovePurchasePlanRequest, InvoiceNetwork, LegProgress, PurchasePlanRequest,
        PurchasePlanReview, ResumeSplitPaymentRequest,
    },
    AuthContext,
};
use leptos::prelude::*;
use wasm_bindgen_futures::spawn_local;

const PANEL:&str="padding:18px;border:1px solid oklch(0.32 0.01 60);background:#121215;clip-path:polygon(0 0,calc(100% - 12px) 0,100% 12px,100% 100%,12px 100%,0 calc(100% - 12px));font:400 12.5px/1.65 ui-monospace,monospace;color:oklch(0.68 0.01 60);overflow-wrap:anywhere";
const SECONDARY:&str="padding:9px 13px;border:1px solid oklch(0.5 0.13 210);background:transparent;color:oklch(0.8 0.14 195);font:800 11px ui-monospace,monospace;border-radius:4px";
const PRIMARY:&str="padding:11px 16px;border:0;background:oklch(0.78 0.16 60);color:#0b0b0d;font:800 12px ui-monospace,monospace;border-radius:5px";
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
pub fn approval_request(
    review: &PurchasePlanReview,
    account: &str,
    fee: &str,
    total: &str,
    network: Option<InvoiceNetwork>,
    confirmed: bool,
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
    let fee = fee
        .parse::<u64>()
        .map_err(|_| "Enter whole non-negative millisatoshis for the fee reserve")?;
    let total = total
        .parse::<u64>()
        .map_err(|_| "Enter whole non-negative millisatoshis for the maximum expenditure")?;
    if review
        .quote
        .gross_msat
        .checked_add(fee)
        .is_none_or(|v| v > total)
        || total > i64::MAX as u64
    {
        return Err("Principal plus the routing-fee reserve exceeds the approved maximum.".into());
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
        fee_reserve_msat: fee,
        total_msat: total,
        confirm_complete_plan: true,
        fee_policy: Default::default(),
        accept_unenforced_fee_risk: false,
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
    let wallet = use_context::<crate::ui_v2::nwc_wallet::NwcWalletContext>();
    let review = RwSignal::new(None::<PurchasePlanReview>);
    let error = RwSignal::new(None::<String>);
    let busy = RwSignal::new(false);
    let confirmed = RwSignal::new(false);
    let fee = RwSignal::new("50000".to_string());
    let total = RwSignal::new(String::new());
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
                    fee.set(
                        plan.approved_limits
                            .as_ref()
                            .map(|v| v.fee_reserve_msat)
                            .unwrap_or(50000)
                            .to_string(),
                    );
                    total.set(
                        plan.approved_limits
                            .as_ref()
                            .map(|v| v.total_msat)
                            .or_else(|| plan.quote.gross_msat.checked_add(50000))
                            .map(|v| v.to_string())
                            .unwrap_or_default(),
                    );
                    network.set(plan.network);
                    fee_risk.set(
                        plan.approved_fee_policy
                            == Some(
                                arcadestr_core::purchase_contract::RoutingFeePolicy::ReportedFees,
                            ),
                    );
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
        let mut request = match approval_request(
            &plan,
            &account,
            &fee.get_untracked(),
            &total.get_untracked(),
            network.get_untracked(),
            confirmed.get_untracked(),
        ) {
            Ok(v) => v,
            Err(e) => {
                error.set(Some(e));
                return;
            }
        };
        if fee_risk.get_untracked() {
            request.fee_policy = arcadestr_core::purchase_contract::RoutingFeePolicy::ReportedFees;
            request.accept_unenforced_fee_risk = true;
        }
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
            <p style="color:oklch(0.8 0.14 195)">"Local approval does not unlock the wallet or authorize live payment execution."</p>
            {move || error.get().map(|e|view!{<p role="alert" style="color:oklch(0.65 0.18 25)">{e}</p>})}
            {move || if busy.get(){Some(view!{<p role="status">"Checking the original plan or payment…"</p>})}else{None}}
            {move || review.get().map(|r|{
                let can_approve=r.terms_verified && r.wallet_connected && !r.halted && !r.purchase_completed && r.settlement_state=="active" && r.approved_limits.is_none();
                let immutable=r.approved_limits.is_some();
                let statuses=r.legs.clone();
                let operator_fees=r.quote.legs.iter().filter(|l|l.role==arcadestr_core::adp_settlement::PaymentRole::Operator).map(|l|l.amount_msat).sum::<u64>();
                let capabilities=r.wallet_capabilities.as_ref().map(|c|format!("Advertised: pay_invoice {}; lookup_invoice {}. {}",c.supports("pay_invoice"),c.supports("lookup_invoice"),c.network.as_deref().unwrap_or("network unavailable"))).unwrap_or_else(||"Wallet capabilities unavailable; no payment authority inferred.".into());
                view!{
                    <p style="color:#fff">{format!("Principal: {} · included operator fees: {}",msat_label(r.quote.gross_msat),msat_label(operator_fees))}</p>
                    <p>{format!("Maximum payment dispatches: {} — only original plan legs; no atomic batch settlement.",r.quote.legs.len())}</p>
                    <p>{format!("Buyer: {} · purchase: {}",r.quote.buyer_pubkey,r.quote.quote_id)}</p>
                    <p>{format!("Listing: {} · original quote: {}",r.quote.listing_event_id,r.quote_event_id)}</p>
                    <p data-testid="purchase-completion" style="color:oklch(0.8 0.14 195)">{if r.purchase_completed {"PURCHASE COMPLETED — VALIDATED RECEIPT STORED"}else{"PURCHASE NOT COMPLETED — ALL LEGS AND A VALIDATED RECEIPT ARE REQUIRED"}}</p>
                    <p>{if r.terms_verified {"Developer-accepted terms and deterministic split verified locally."}else{"Original listing/term evidence unavailable or unverified. Approval is disabled; read-only recovery remains available."}}</p>
                    <p>{capabilities}</p>
                    <p>{r.restriction}</p>
                    <p>"Aggregate receipts publicly link the buyer, listing, recipients, amounts and payment hashes. This reveals the revenue split and can correlate payments. Raw invoices, preimages and refund destinations remain encrypted."</p>
                    <p>{format!("Approved fee policy: {:?}",r.approved_fee_policy)}</p>
                    <p>"Routing fees reported after payment are not a wallet-enforced ceiling. Strict capped execution remains restricted when provider assurance is unavailable. Multiple Lightning payments are not atomic."</p>
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
                                <p>{status.as_ref().and_then(|s|s.fees_paid_msat).map(|v|format!("Verified routing fee: {}",msat_label(v))).unwrap_or_else(||"Routing fee not independently available.".into())}</p>
                                <button type="button" style=SECONDARY disabled=move||disabled||busy.get() on:click=move |_|{if let Some(id)=attempt.clone(){recover.run(id);}}>"CHECK ORIGINAL PAYMENT — NO REPAYMENT"</button>
                            </article>
                        }
                    }).collect_view()}
                    </div>
                    <div style="display:grid;grid-template-columns:repeat(auto-fit,minmax(180px,1fr));gap:12px">
                        <label>"MAX ROUTING FEES (MSAT)"<input data-testid="purchase-fee-limit" style=FIELD type="number" min="0" step="1" disabled=immutable prop:value=move||fee.get() on:input:target=move|e|{fee.set(e.target().value());confirmed.set(false);}/></label>
                        <label>"MAX EXPENDITURE (MSAT)"<input data-testid="purchase-total-limit" style=FIELD type="number" min="0" step="1" disabled=immutable prop:value=move||total.get() on:input:target=move|e|{total.set(e.target().value());confirmed.set(false);}/></label>
                        <label>"INVOICE NETWORK"<select style=FIELD disabled=immutable prop:value=move||network.get().map(|n|format!("{n:?}")).unwrap_or_default() on:change:target=move|e|{network.set(match e.target().value().as_str(){"Bitcoin"=>Some(InvoiceNetwork::Bitcoin),"Testnet"=>Some(InvoiceNetwork::Testnet),"Signet"=>Some(InvoiceNetwork::Signet),"Regtest"=>Some(InvoiceNetwork::Regtest),_=>None});confirmed.set(false);}>
                            <option value="">"Select explicitly"</option><option value="Bitcoin">"Bitcoin"</option><option value="Testnet">"Testnet"</option><option value="Signet">"Signet"</option><option value="Regtest">"Regtest"</option>
                        </select></label>
                    </div>
                    <p data-testid="purchase-approved-state">{if r.approved {"IMMUTABLE PLAN APPROVED — LIVE EXECUTION STILL LOCKED"}else if immutable {"APPROVAL IS BOUND TO THE ORIGINAL WALLET; RESTORE THAT CONNECTION"}else{"NO BUYER APPROVAL STORED"}}</p>
                    <label style="display:flex;align-items:flex-start;gap:8px;margin:14px 0"><input data-testid="purchase-fee-risk" type="checkbox" disabled=immutable prop:checked=move||fee_risk.get() on:change:target=move|e|{fee_risk.set(e.target().checked());confirmed.set(false);}/>"Optional reported-fee mode: I accept that this wallet may exceed the desired fee and total limits before reporting costs. Arcadestr can halt later payments, not enforce those limits externally. Leave unchecked for provider-ceiling mode. Neither choice authorizes a live test."</label>
                    <label style="display:flex;align-items:flex-start;gap:8px;margin:14px 0"><input data-testid="purchase-approval-check" type="checkbox" disabled=move||!can_approve||busy.get() prop:checked=move||confirmed.get() on:change:target=move|e|confirmed.set(e.target().checked())/>"I explicitly approve every recipient, amount, accepted operator fee and maximum expenditure shown. I understand that approval is application-local and the wallet fee guarantee and live execution remain restricted."</label>
                    <button data-testid="approve-purchase-plan" type="button" style=PRIMARY disabled=move||!can_approve||!confirmed.get()||busy.get() on:click=move |_|approve.run(())>"APPROVE IMMUTABLE PLAN"</button>
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

#[cfg(test)]
mod tests {
    use super::*;
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
    fn complete_plan_confirmation_and_exact_budget_are_required() {
        let r = review();
        assert!(approval_request(&r, "buyer", "50000", "250000", r.network, false).is_err());
        let request = approval_request(&r, "buyer", "50000", "250000", r.network, true)
            .expect("explicit approval");
        assert_eq!(request.fee_reserve_msat, 50000);
        assert_eq!(request.total_msat, 250000);
        assert_eq!(request.quote_event_id, "signed-quote");
        assert_eq!(request.commitment, "immutable-review");
        for (fee, total) in [
            ("50000", "249999"),
            ("-1", "250000"),
            ("0.5", "250000"),
            ("18446744073709551615", "250000"),
        ] {
            assert!(approval_request(&r, "buyer", fee, total, r.network, true).is_err());
        }
        assert!(approval_request(&r, "other-buyer", "50000", "250000", r.network, true).is_err());
        assert!(approval_request(
            &r,
            "buyer",
            "50000",
            "250000",
            Some(InvoiceNetwork::Bitcoin),
            true
        )
        .is_err());
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
            assert!(approval_request(&r, "buyer", "50000", "250000", r.network, true).is_err());
        }
    }
}
