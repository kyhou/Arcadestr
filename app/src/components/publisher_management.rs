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
                message.set(Some("Couldn't start this change. Try again.".into()));
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
            message.set(Some(match result {Ok(event)=>format!("Done. The change was accepted and may take a moment to reach everyone. Select Check again before making another change. (event {})",event.id),Err(error)=>format!("{error}. If this stopped partway, use Finish publishing on this game instead of starting again. (operation {operation_id})")}));
        });
    });
    let render_auth = auth.clone();
    view! {
        <section class="v2-create-fields v2-publisher-access" aria-label="Revoke access" style="min-width:0;overflow-wrap:anywhere">
            <div class="v2-create-row-between">
                <h3 class="v2-create-subsection-title">"Upload servers"</h3>
                <button type="button" class="v2-btn-secondary" disabled=move || busy.get() on:click=move |_|refresh.update(|value|*value=value.wrapping_add(1))>"Check again"</button>
            </div>
            <p class="v2-create-help">"What this device has recorded about each server. This doesn't confirm a server is online right now."</p>
            <Show when=move ||servers.get_value().is_empty() && evidence.get().is_empty()><p class="v2-create-help">"This game has no upload servers."</p></Show>
            {move || {let mut urls=servers.get_value();for row in evidence.get() {if !urls.contains(&row.server_url){urls.push(row.server_url);}} urls.into_iter().map(|url| {
                let rows=evidence.get().into_iter().filter(|row|row.server_url==url).collect::<Vec<_>>();
                view! {<div class="v2-hub-role-row"><p class="v2-create-mono-value">{url}</p>
                    {if rows.is_empty(){view!{<p class="v2-create-help">"Listed for this game · nothing recorded on this device."</p>}.into_any()}else{rows.into_iter().map(|row|view!{<p class="v2-create-help">{format!("{} · permissions: {}{}",row.state.label(),row.capabilities.join(", "),row.diagnostic.map(|value|format!(" · last problem: {value}")).unwrap_or_default())}</p><details class="v2-create-disclosure"><summary class="v2-create-disclosure-summary">"Technical details"</summary><p class="v2-create-mono-value">{format!("Operation {} · service {}",row.operation_id,row.operator.unwrap_or_else(||"unknown".into()))}</p></details>}).collect_view().into_any()}}
                </div>}
            }).collect_view()}}

            <div>
                <label for="revoke-action" class="v2-create-field-label">"What do you want to do?"</label>
                <select id="revoke-action" aria-label="Credential action" class="v2-input" style="max-width:100%" prop:value=move ||if grant.get(){"grant"}else{"authority"} disabled=move || busy.get() on:change:target=move |event|{grant.set(event.target().value()=="grant");review.set(None);confirm.set(false);revision.update(|value|*value=value.wrapping_add(1));}>
                    <option value="authority">"Stop new copies from a service"</option>
                    <option value="grant">"Take away a player's access"</option>
                </select>
                <p class="v2-create-help">{move || if grant.get() {
                    "Only that player loses access. Their payment isn't refunded, and other purchases aren't affected."
                } else {
                    "Players who already have the game keep it. The service stops giving out new copies."
                }}</p>
            </div>
            <div>
                <label for="revoke-root" class="v2-create-field-label">{move || if grant.get() { "Access record ID" } else { "Service authorization ID" }}</label>
                <input id="revoke-root" aria-label="Credential root event ID" class="v2-input v2-create-mono-value" style="max-width:100%;box-sizing:border-box" prop:value=move ||root.get() disabled=move ||busy.get() on:input:target=move |event|{root.set(event.target().value());review.set(None);confirm.set(false);revision.update(|value|*value=value.wrapping_add(1));}/>
            </div>
            {authority_roots.into_iter().map(|id|{let display=id.chars().take(16).collect::<String>();view!{<button type="button" class="v2-btn-secondary" disabled=move ||busy.get() on:click=move |_|{root.set(id.clone());grant.set(false);review.set(None);confirm.set(false);revision.update(|value|*value=value.wrapping_add(1));}>{format!("Use this game's service authorization ({display}…)")}</button>}}).collect_view()}
            <button type="button" class="v2-btn-secondary" disabled=move ||busy.get() on:click=move |_|inspect.run(())>"Check"</button>
            {move ||review.get().map(|value|view!{
                <p class="v2-create-help">{if value.terminal { format!("{} · already revoked", value.subject) } else { format!("{} · active", value.subject) }}</p>
                <details class="v2-create-disclosure"><summary class="v2-create-disclosure-summary">"Technical details"</summary><p class="v2-create-mono-value">{format!("root {} · current {}",value.root_event_id,value.tip_event_id)}</p></details>
            })}
            <label class="v2-create-checkbox-row"><input type="checkbox" prop:checked=move ||confirm.get() disabled=move ||busy.get() || review.get().is_none() || attempted.get() on:change:target=move |event|confirm.set(event.target().checked())/>"I understand this can't be undone. It doesn't remove the game from the store."</label>
            <button type="button" class="v2-btn-danger" disabled=move ||busy.get() || attempted.get() || !confirm.get() || review.get().is_none_or(|value|value.terminal) || render_auth.npub.get()!=Some(publisher.get_value()) on:click=move |_|revoke.run(())>{move || if grant.get() { "Take away access" } else { "Stop new copies" }}</button>
            {move ||message.get().map(|text|view!{<p class="v2-create-inline-note" role="status">{text}</p>})}
        </section>
    }
}
