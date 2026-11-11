//! Refund actions are scoped to the active obligated identity by the backend.
use crate::{
    tauri_bridge::{
        invoke_pay_split_refund, invoke_prepare_split_refund, invoke_recipient_refund_history,
        invoke_resume_split_refund, RefundHistoryItem, RefundPaymentRequest, SplitRefundRequest,
    },
    AuthContext,
};
use leptos::prelude::*;
use wasm_bindgen_futures::spawn_local;
const BUTTON:&str="padding:10px 14px;border:1px solid oklch(0.5 0.13 210);background:transparent;color:oklch(0.8 0.14 195);font:800 11px ui-monospace,monospace;border-radius:4px";
const TEXT: &str = "font:400 12.5px/1.6 ui-monospace,monospace;color:oklch(0.68 0.01 60);margin:0";

#[cfg(any(target_arch = "wasm32", test))]
fn refresh_after_recovery(
    account: Option<&str>,
    loaded: &str,
    typed: &str,
    busy: bool,
    event: &serde_json::Value,
) -> bool {
    !busy
        && !loaded.is_empty()
        && loaded == typed.trim()
        && event["recovered"].as_u64().is_some_and(|count| count > 0)
        && account
            .and_then(|npub| nostr::PublicKey::parse(npub).ok())
            .is_some_and(|key| Some(key.to_hex().as_str()) == event["account"].as_str())
}
#[component]
fn RefundCard(
    row: RefundHistoryItem,
    server: String,
    account: String,
    generation: RwSignal<u64>,
    on_changed: Callback<()>,
) -> impl IntoView {
    let auth = use_context::<AuthContext>().expect("AuthContext");
    let expected_generation = generation.get_untracked();
    let confirmed = RwSignal::new(false);
    let busy = RwSignal::new(false);
    let message = RwSignal::new(String::new());
    let error = RwSignal::new(false);
    let action = {
        let row = row.clone();
        let server = server.clone();
        let account = account.clone();
        let auth = auth.clone();
        Callback::new(move |mode: &'static str| {
            if busy.get_untracked() || (mode == "pay" && !confirmed.get_untracked()) {
                return;
            }
            busy.set(true);
            message.set(
                if mode == "pay" {
                    "Payment in progress — original invoice only"
                } else {
                    "Recovery/metadata check in progress — no payment dispatch"
                }
                .into(),
            );
            error.set(false);
            let row = row.clone();
            let server = server.clone();
            let account = account.clone();
            let auth = auth.clone();
            spawn_local(async move {
                let result = if mode == "prepare" {
                    invoke_prepare_split_refund(SplitRefundRequest {
                        server_url: server,
                        quote_id: row.quote_id,
                        leg_id: row.leg_id,
                    })
                    .await
                    .map(|_| ())
                } else {
                    let request = RefundPaymentRequest {
                        server_url: server,
                        quote_id: row.quote_id,
                        leg_id: row.leg_id,
                        confirm_payment: mode == "pay",
                    };
                    if mode == "pay" {
                        invoke_pay_split_refund(request).await.map(|_| ())
                    } else {
                        invoke_resume_split_refund(request).await.map(|_| ())
                    }
                };
                if generation.get_untracked() != expected_generation
                    || auth.npub.get_untracked().as_deref() != Some(account.as_str())
                {
                    return;
                }
                busy.set(false);
                confirmed.set(false);
                match result {
                    Ok(()) => {
                        message.set("Original refund state persisted.".into());
                        on_changed.run(());
                    }
                    Err(value) => {
                        error.set(true);
                        message.set(value);
                        on_changed.run(());
                    }
                }
            });
        })
    };
    let verified = row.state == "verified";
    let awaiting = row.payment_state == "awaiting_payment";
    let unresolved = matches!(
        row.payment_state.as_str(),
        "uncertain" | "proof_checkpointed"
    );
    let amount = row.principal_msat;
    let invoice = row.invoice.clone();
    view! {
        <article data-testid="recipient-refund-card" style="clip-path:polygon(0 0,calc(100% - 12px) 0,100% 12px,100% 100%,12px 100%,0 calc(100% - 12px));border:1px solid oklch(0.32 0.01 60);background:#121215;padding:20px;display:grid;gap:14px;font-family:ui-monospace,monospace">
            <strong style="font:800 16px ui-monospace,monospace;color:#fff;overflow-wrap:anywhere">{format!("{} · {} msat",row.leg_id,amount)}</strong>
            <p style=TEXT>{format!("QUOTE {} · ORIGINAL DEADLINE (UNIX) {}",row.quote_id,row.due_at)}</p>
            <p data-testid="refund-state" style=if verified{"font:700 12px ui-monospace,monospace;color:oklch(0.8 0.14 195)"}else if row.state=="overdue"{"font:700 12px ui-monospace,monospace;color:oklch(0.65 0.18 25)"}else{"font:700 12px ui-monospace,monospace;color:oklch(0.8 0.16 60)"}>{format!("{} · {}",row.state.to_uppercase(),row.payment_state.replace('_'," ").to_uppercase())}</p>
            {invoice.map(|invoice|view!{<textarea data-testid="original-refund-invoice" aria-label="Original persisted refund invoice" readonly rows="3" style="width:100%;box-sizing:border-box;background:#141417;border:1px solid oklch(0.32 0.01 60);color:#fff;font:400 11px/1.5 ui-monospace,monospace;padding:10px;border-radius:4px">{invoice}</textarea>})}
            <Show when=move||!verified>
                <div style="display:flex;gap:12px;flex-wrap:wrap">
                    <Show when=move||!awaiting && !unresolved><button data-testid="prepare-refund" style=BUTTON disabled=move||busy.get() on:click=move |_|action.run("prepare")>"PREPARE ORIGINAL INVOICE — NO PAYMENT"</button></Show>
                    <Show when=move||unresolved><button data-testid="recover-refund" style=BUTTON disabled=move||busy.get() on:click=move |_|action.run("recover")>"LOOK UP ORIGINAL REFUND — NO PAYMENT"</button></Show>
                </div>
                <Show when=move||awaiting>
                    <label style=TEXT><input data-testid="confirm-refund-checkbox" type="checkbox" prop:checked=move||confirmed.get() on:change:target=move|event|confirmed.set(event.target().checked()) />{format!(" I authorize one payment of {amount} msat from this account's connected wallet.")}</label>
                    <button data-testid="pay-refund" style="padding:11px 20px;background:oklch(0.78 0.16 60);color:#0b0b0d;font:800 13px ui-monospace,monospace;border:0;border-radius:5px" disabled=move||busy.get() || !confirmed.get() on:click=move |_|action.run("pay")>"SEND REFUND PAYMENT"</button>
                </Show>
            </Show>
            <Show when=move||unresolved><p style=TEXT>"Outcome unresolved or proof awaiting confirmation. The original invoice and payment intent are retained. Do not create a replacement or repay."</p></Show>
            <p role="status" style=move||if error.get(){"font:600 12px/1.6 ui-monospace,monospace;color:oklch(0.65 0.18 25)"}else{TEXT}>{move||message.get()}</p>
        </article>
    }
}

#[component]
pub fn RecipientRefunds() -> impl IntoView {
    let auth = use_context::<AuthContext>().expect("AuthContext");
    let server = RwSignal::new(String::new());
    let loaded_server = RwSignal::new(String::new());
    let rows = RwSignal::new(Vec::<RefundHistoryItem>::new());
    let generation = RwSignal::new(0u64);
    let busy = RwSignal::new(false);
    let message = RwSignal::new(
        if cfg!(feature = "web") {
            "Recipient refund discovery, recovery and payment require the desktop application."
        } else {
            "Choose a processor to load obligations owed by the active identity. Loading never sends a payment."
        }
        .to_owned(),
    );
    let recovery_events = RwSignal::new(if cfg!(feature = "web") {
        "unavailable"
    } else {
        "connecting"
    });
    let auth_effect = auth.clone();
    Effect::new(move || {
        let _ = auth_effect.npub.get();
        generation.update(|n| *n = n.wrapping_add(1));
        rows.set(vec![]);
        // A background event for a newly selected account must not authorize
        // reloading a processor that only the previous account selected.
        loaded_server.set(String::new());
        busy.set(false);
    });
    let load = {
        let auth = auth.clone();
        Callback::new(move |()| {
            if cfg!(feature = "web") {
                return;
            }
            let Some(account) = auth.npub.get_untracked() else {
                message.set("Connect the obligated recipient account first.".into());
                return;
            };
            let url = server.get_untracked().trim().to_owned();
            if url.is_empty() {
                message.set("Enter the original processor URL.".into());
                return;
            }
            generation.update(|n| *n = n.wrapping_add(1));
            let expected = generation.get_untracked();
            let auth = auth.clone();
            busy.set(true);
            message.set("Loading authenticated recipient obligations — no payment".into());
            spawn_local(async move {
                let response = invoke_recipient_refund_history(url.clone()).await;
                if generation.get_untracked() != expected
                    || auth.npub.get_untracked().as_deref() != Some(account.as_str())
                {
                    return;
                }
                busy.set(false);
                loaded_server.set(url);
                match response {
                    Ok(values) => {
                        message.set(if values.is_empty(){"No obligations are owed by this account at this processor."}else{"Only obligations owed by the active identity are shown. Payment requires a separate confirmation."}.into());
                        rows.set(values);
                    }
                    Err(error) => {
                        message.set(error);
                        rows.set(vec![]);
                    }
                }
            });
        })
    };
    #[cfg(all(target_arch = "wasm32", not(feature = "web")))]
    {
        use std::{
            cell::{Cell, RefCell},
            rc::Rc,
        };
        let alive = Rc::new(Cell::new(true));
        let cleanup: Rc<RefCell<Option<Box<dyn FnOnce()>>>> = Rc::new(RefCell::new(None));
        let cleanup_on_drop = send_wrapper::SendWrapper::new(cleanup.clone());
        let alive_on_drop = send_wrapper::SendWrapper::new(alive.clone());
        on_cleanup(move || {
            alive_on_drop.set(false);
            if let Some(unlisten) = cleanup_on_drop.borrow_mut().take() {
                unlisten();
            }
        });
        let event_auth = auth.clone();
        let event_alive = alive.clone();
        spawn_local(async move {
            let listener =
                crate::tauri_invoke::listen("settlement_recovery_updated", move |event| {
                    if !event_alive.get() {
                        return;
                    }
                    if refresh_after_recovery(
                        event_auth.npub.get_untracked().as_deref(),
                        &loaded_server.get_untracked(),
                        &server.get_untracked(),
                        busy.get_untracked(),
                        &event,
                    ) {
                        recovery_events.set("refreshing");
                        // Read authenticated metadata only; never prepare or pay here.
                        load.run(());
                    } else {
                        recovery_events.set("ignored");
                    }
                })
                .await;
            if let Ok(unlisten) = listener {
                if alive.get() {
                    recovery_events.set("ready");
                    *cleanup.borrow_mut() = Some(Box::new(unlisten));
                } else {
                    unlisten();
                }
            } else if alive.get() {
                recovery_events.set("unavailable");
            }
        });
    }
    view! {
        <section data-recovery-events=move||recovery_events.get() style="display:grid;gap:16px;margin:28px 0;max-width:1440px;font-family:ui-monospace,monospace">
            <div style="display:flex;gap:10px;align-items:center"><span style="font:800 12px ui-monospace,monospace;letter-spacing:1.5px;color:oklch(0.8 0.16 60)">"◆ RECIPIENT REFUNDS"</span><span style="flex:1;height:1px;background:repeating-linear-gradient(90deg,oklch(0.4 0.03 60 / .6) 0 4px,transparent 4px 8px)"></span></div>
            <div style="display:flex;gap:12px;flex-wrap:wrap"><input data-testid="refund-processor" aria-label="Original refund processor URL" placeholder="https://your-processor.example" disabled=cfg!(feature="web") prop:value=move||server.get() on:input:target=move|event|server.set(event.target().value()) style="flex:1;min-width:200px;padding:10px 12px;background:#141417;color:#fff;border:1px solid oklch(0.32 0.01 60);font:600 12px ui-monospace,monospace;border-radius:4px" /><button data-testid="load-refunds" style=BUTTON disabled=move||cfg!(feature="web")||busy.get() on:click=move |_|load.run(())>"LOAD OBLIGATIONS — NO PAYMENT"</button></div>
            <p role="status" style=TEXT>{move||message.get()}</p>
            {move||rows.get().into_iter().map(|row|view!{<RefundCard row=row server=loaded_server.get_untracked() account=auth.npub.get_untracked().unwrap_or_default() generation=generation on_changed=load />}).collect_view()}
        </section>
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use nostr::nips::nip19::ToBech32;
    #[test]
    fn background_recovery_refresh_is_account_scoped_and_never_reloads_an_unsent_processor() {
        let key = nostr::Keys::generate();
        let npub = key.public_key().to_bech32().expect("npub");
        let event = serde_json::json!({"account":key.public_key().to_hex(),"recovered":1});
        assert!(refresh_after_recovery(
            Some(&npub),
            "http://localhost:9099",
            "http://localhost:9099",
            false,
            &event
        ));
        assert!(!refresh_after_recovery(
            None,
            "http://localhost:9099",
            "http://localhost:9099",
            false,
            &event
        ));
        assert!(!refresh_after_recovery(
            Some("invalid"),
            "http://localhost:9099",
            "http://localhost:9099",
            false,
            &event
        ));
        let other = nostr::Keys::generate()
            .public_key()
            .to_bech32()
            .expect("other");
        assert!(!refresh_after_recovery(
            Some(&other),
            "http://localhost:9099",
            "http://localhost:9099",
            false,
            &event
        ));
        assert!(!refresh_after_recovery(
            Some(&npub),
            "http://localhost:9099",
            "http://localhost:9099",
            true,
            &event
        ));
        assert!(!refresh_after_recovery(
            Some(&npub),
            "http://localhost:9099",
            "http://localhost:19100",
            false,
            &event
        ));
        assert!(!refresh_after_recovery(Some(&npub), "", "", false, &event));
        assert!(!refresh_after_recovery(
            Some(&npub),
            "http://localhost:9099",
            "http://localhost:9099",
            false,
            &serde_json::json!({"account":key.public_key().to_hex(),"recovered":0,"blocked":1})
        ));
    }
}
