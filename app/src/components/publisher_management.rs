use crate::tauri_bridge::*;
use arcadestr_core::publisher_management::*;
use leptos::prelude::*;
use wasm_bindgen_futures::spawn_local;

#[cfg(debug_assertions)]
#[component]
pub fn PublisherManagementPreview() -> impl IntoView {
    use nostr::nips::nip19::ToBech32;
    let auth = crate::AuthContext::new();
    let public = nostr::PublicKey::from_hex(
        "79be667ef9dcbbac55a06295ce870b07029bfcdb2dce28d959f2815b16f81798",
    )
    .expect("public test fixture");
    let publisher = public.to_bech32().expect("npub fixture");
    auth.npub.set(Some(publisher.clone()));
    auth.begin_auth_flow();
    provide_context(auth.clone());
    view! {<PublisherManagementPanel publisher=publisher coordinate=format!("30402:{public}:fixture") servers=vec!["https://one.test".into(),"https://two.test".into()] />
    <button type="button" data-testid="management-account-switch" on:click=move |_|{auth.begin_auth_flow();auth.npub.set(None);}>"Mock account switch"</button>}
}

#[component]
pub fn PublisherManagementPanel(
    publisher: String,
    coordinate: String,
    servers: Vec<String>,
    #[prop(optional)] authority_roots: Vec<String>,
) -> impl IntoView {
    let auth = use_context::<crate::AuthContext>().expect("AuthContext not provided");
    let publisher = StoredValue::new(publisher);
    let coordinate = StoredValue::new(coordinate);
    let servers = StoredValue::new(servers);
    let evidence = RwSignal::new(Vec::<DistributionEvidence>::new());
    let root = RwSignal::new(String::new());
    let grant = RwSignal::new(false);
    let review = RwSignal::new(None::<CredentialReview>);
    let confirm = RwSignal::new(false);
    let busy = RwSignal::new(false);
    let attempted = RwSignal::new(false);
    let message = RwSignal::new(None::<String>);
    let revision = RwSignal::new(0u64);
    let refresh = RwSignal::new(0u64);
    let effect_auth = auth.clone();
    Effect::new(move |_| {
        let _ = refresh.get();
        let current = effect_auth.npub.get();
        let epoch = effect_auth.current_auth_flow();
        revision.update(|value| *value = value.wrapping_add(1));
        let generation = revision.get_untracked();
        evidence.set(Vec::new());
        review.set(None);
        confirm.set(false);
        busy.set(false);
        message.set(None);
        attempted.set(false);
        if current.as_deref() != Some(publisher.get_value().as_str()) {
            root.set(String::new());
            return;
        }
        let request_auth = effect_auth.clone();
        spawn_local(async move {
            let result =
                invoke_publisher_distribution_state(publisher.get_value(), coordinate.get_value())
                    .await;
            if !request_auth.is_current_auth_flow(epoch) || revision.get_untracked() != generation {
                return;
            }
            match result {
                Ok(rows) => evidence.set(rows),
                Err(error) => message.set(Some(error)),
            }
        });
    });
    let review_auth = auth.clone();
    let inspect = Callback::new(move |_: ()| {
        if busy.get_untracked() || review_auth.npub.get_untracked() != Some(publisher.get_value()) {
            return;
        }
        revision.update(|value| *value = value.wrapping_add(1));
        let generation = revision.get_untracked();
        let epoch = review_auth.current_auth_flow();
        let response_auth = review_auth.clone();
        let id = root.get_untracked();
        let action = if grant.get_untracked() {
            RevocationAction::RevokeGrant
        } else {
            RevocationAction::CancelAuthority
        };
        busy.set(true);
        review.set(None);
        confirm.set(false);
        message.set(None);
        spawn_local(async move {
            let result = invoke_review_publisher_credential(
                publisher.get_value(),
                coordinate.get_value(),
                id,
                action,
            )
            .await;
            if !response_auth.is_current_auth_flow(epoch) || revision.get_untracked() != generation
            {
                return;
            }
            busy.set(false);
            match result {
                Ok(value) => review.set(Some(value)),
                Err(error) => message.set(Some(error)),
            }
        });
    });
    let revoke_auth = auth.clone();
    let revoke = Callback::new(move |_: ()| {
        if busy.get_untracked()
            || attempted.get_untracked()
            || !confirm.get_untracked()
            || revoke_auth.npub.get_untracked() != Some(publisher.get_value())
        {
            return;
        }
        let Some(value) = review.get_untracked().filter(|value| !value.terminal) else {
            return;
        };
        let operation_id = match super::publish::archive_request_id() {
            Ok(id) => id.as_str().to_owned(),
            Err(_) => {
                message.set(Some("Cannot create a durable operation ID".into()));
                return;
            }
        };
        let epoch = revoke_auth.current_auth_flow();
        let response_auth = revoke_auth.clone();
        let generation = revision.get_untracked();
        let request = RevokePublisherCredentialRequest {
            expected_publisher_npub: publisher.get_value(),
            coordinate: coordinate.get_value(),
            root_event_id: value.root_event_id,
            expected_tip_event_id: value.tip_event_id,
            action: if grant.get_untracked() {
                RevocationAction::RevokeGrant
            } else {
                RevocationAction::CancelAuthority
            },
            operation_id: operation_id.clone(),
            confirm: true,
        };
        busy.set(true);
        attempted.set(true);
        confirm.set(false);
        spawn_local(async move {
            let result = invoke_revoke_publisher_credential(request).await;
            if !response_auth.is_current_auth_flow(epoch) || revision.get_untracked() != generation
            {
                return;
            }
            busy.set(false);
            message.set(Some(match result {Ok(event)=>format!("Selected relay acknowledged transition {}. This does not prove global propagation. Refresh credential evidence before further action.",event.id),Err(error)=>format!("Operation {operation_id}: {error}. If retained, reconcile this exact operation in Publisher recovery; do not start a replacement cancellation.")}));
        });
    });
    let render_auth = auth.clone();
    view! {
        <section style="max-width:1440px;margin:18px auto;padding:18px;background:#121215;border:1px solid oklch(0.32 0.01 60);clip-path:polygon(0 0,calc(100% - 12px) 0,100% 12px,100% 100%,12px 100%,0 calc(100% - 12px));font:400 12.5px/1.6 ui-monospace,monospace;color:#fff;overflow-wrap:anywhere">
            <h2 style="font:800 12px ui-monospace,monospace;color:oklch(0.8 0.16 60);letter-spacing:1.5px">"DISTRIBUTION & CREDENTIAL MANAGEMENT"</h2>
            <p>"Declared servers and historical operation evidence are not current download availability. Health checks do not prove registration or availability."</p>
            <Show when=move ||servers.get_value().is_empty() && evidence.get().is_empty()><p>"No declared servers or local distribution checkpoints. Select distribution servers in the listing editor to begin a reviewed publication."</p></Show>
            <button type="button" style="padding:10px;border:1px solid oklch(0.5 0.13 210);background:transparent;color:oklch(0.78 0.13 210);font-family:ui-monospace,monospace" disabled=move || busy.get() on:click=move |_|refresh.update(|value|*value=value.wrapping_add(1))>"REFRESH LOCAL EVIDENCE"</button>
            {move || {let mut urls=servers.get_value();for row in evidence.get() {if !urls.contains(&row.server_url){urls.push(row.server_url);}} urls.into_iter().map(|url| {
                let rows=evidence.get().into_iter().filter(|row|row.server_url==url).collect::<Vec<_>>();
                view! {<div style="padding:10px;border-top:1px solid oklch(0.32 0.01 60)"><strong>{url}</strong>
                    {if rows.is_empty(){view!{<p>"Declared · no local operation evidence; registration and availability unknown."</p>}.into_any()}else{rows.into_iter().map(|row|view!{<p>{format!("{} · operation {} · operator {} · intended capabilities {}{}",row.state.label(),row.operation_id,row.operator.unwrap_or_else(||"unknown".into()),row.capabilities.join(", "),row.diagnostic.map(|value|format!(" · previous attempt: {value}")).unwrap_or_default())}</p>}).collect_view().into_any()}}
                </div>}
            }).collect_view()}}
            <p>"Authority cancellation stops future issuance; receipts and grants issued before cancellation retain their historical validation rules. Individual grant revocation affects only that entitlement, not an independent purchase or another grant. Campaign cancellation stops claims, listing removal changes discovery, and operator offboarding is a separate unresolved workflow. None is performed implicitly here."</p>
            <label>"ACTION "<select aria-label="Credential action" prop:value=move ||if grant.get(){"grant"}else{"authority"} style="background:#141417;color:#fff;padding:10px;font-family:ui-monospace,monospace" disabled=move || busy.get() on:change:target=move |event|{grant.set(event.target().value()=="grant");review.set(None);confirm.set(false);revision.update(|value|*value=value.wrapping_add(1));}>
                <option value="authority">"CANCEL FULFILLMENT AUTHORITY"</option><option value="grant">"REVOKE ONE ENTITLEMENT GRANT"</option>
            </select></label>
            <label style="display:block">"CREDENTIAL ROOT EVENT ID"<input aria-label="Credential root event ID" style="display:block;width:100%;box-sizing:border-box;background:#141417;color:#fff;padding:10px;border:1px solid oklch(0.32 0.01 60);font-family:ui-monospace,monospace" prop:value=move ||root.get() disabled=move ||busy.get() on:input:target=move |event|{root.set(event.target().value());review.set(None);confirm.set(false);revision.update(|value|*value=value.wrapping_add(1));}/></label>
            {authority_roots.into_iter().map(|id|{let display=id.clone();view!{<button type="button" style="padding:8px;border:1px solid oklch(0.5 0.13 210);background:transparent;color:oklch(0.78 0.13 210);font-family:ui-monospace,monospace;overflow-wrap:anywhere;max-width:100%" disabled=move ||busy.get() on:click=move |_|{root.set(id.clone());grant.set(false);review.set(None);confirm.set(false);revision.update(|value|*value=value.wrapping_add(1));}>{format!("SELECT LISTED AUTHORITY {display}")}</button>}}).collect_view()}
            <button type="button" style="padding:10px;border:1px solid oklch(0.5 0.13 210);background:transparent;color:oklch(0.78 0.13 210);font-family:ui-monospace,monospace" disabled=move ||busy.get() on:click=move |_|inspect.run(())>"REVIEW CURRENT CREDENTIAL >"</button>
            {move ||review.get().map(|value|view!{<p>{format!("{} · root {} · current tip {} · terminal {}",value.subject,value.root_event_id,value.tip_event_id,value.terminal)}</p>})}
            <label style="display:block"><input type="checkbox" prop:checked=move ||confirm.get() disabled=move ||busy.get() || review.get().is_none() || attempted.get() on:change:target=move |event|confirm.set(event.target().checked())/>" I authorize this exact developer-signed transition for the reviewed credential. This is not listing removal or operator offboarding."</label>
            <button type="button" style="padding:10px;border:1px solid oklch(0.65 0.18 25);background:transparent;color:oklch(0.65 0.18 25);font-family:ui-monospace,monospace" disabled=move ||busy.get() || attempted.get() || !confirm.get() || review.get().is_none_or(|value|value.terminal) || render_auth.npub.get()!=Some(publisher.get_value()) on:click=move |_|revoke.run(())>"CONFIRM REVIEWED TRANSITION"</button>
            {move ||message.get().map(|text|view!{<p role="status">{text}</p>})}
        </section>
    }
}
