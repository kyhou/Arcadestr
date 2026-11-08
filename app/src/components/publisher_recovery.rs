use crate::tauri_bridge::{
    self, BuildSelectionRef, HashSelectedArchiveRequest, InspectArchiveRequest,
    SelectArchiveRequest,
};

#[cfg(debug_assertions)]
#[component]
pub fn PublisherRecoveryPreview() -> impl IntoView {
    use nostr::nips::nip19::ToBech32;
    let auth = crate::AuthContext::new();
    let public = nostr::PublicKey::from_hex(
        "79be667ef9dcbbac55a06295ce870b07029bfcdb2dce28d959f2815b16f81798",
    )
    .ok();
    auth.npub.set(public.and_then(|key| key.to_bech32().ok()));
    auth.begin_auth_flow();
    provide_context(auth.clone());
    view! {
        <PublisherRecoveryPanel />
        <button type="button" data-testid="publisher-recovery-account-switch" on:click=move |_| { auth.begin_auth_flow(); auth.npub.set(None); }>"Mock account switch"</button>
    }
}
use arcadestr_core::publisher_recovery::PublisherRecoveryState;
use leptos::prelude::*;
use wasm_bindgen_futures::spawn_local;
fn archive_error(
    error: arcadestr_core::archive_inspection_contract::ArchiveSelectionError,
) -> String {
    super::publish::archive_error_message(&error)
}

#[component]
pub fn PublisherRecoveryPanel() -> impl IntoView {
    let auth = use_context::<crate::AuthContext>().expect("AuthContext not provided");
    let state = RwSignal::new(None::<PublisherRecoveryState>);
    let message = RwSignal::new(None::<String>);
    let authorized = RwSignal::new(false);
    let uncertain = RwSignal::new(false);
    let busy = RwSignal::new(false);
    let selection = RwSignal::new(None::<BuildSelectionRef>);
    let refresh = RwSignal::new(0_u64);
    let query_auth = auth.clone();
    Effect::new(move |_| {
        refresh.track();
        let epoch = query_auth.current_auth_flow();
        let publisher = query_auth.npub.get();
        state.set(None);
        selection.set(None);
        authorized.set(false);
        uncertain.set(false);
        busy.set(false);
        message.set(None);
        let Some(publisher) = publisher else { return };
        let response_auth = query_auth.clone();
        spawn_local(async move {
            let result = tauri_bridge::invoke_publisher_recovery_state(publisher).await;
            if !response_auth.is_current_auth_flow(epoch) {
                return;
            }
            match result {
                Ok(result) => state.set(Some(result)),
                Err(error) => message.set(Some(error)),
            }
        });
    });
    let picker_auth = auth.clone();
    let select_archive = Callback::new(move |_: ()| {
        if busy.get_untracked() {
            return;
        }
        let Some(publisher) = picker_auth.npub.get_untracked() else {
            return;
        };
        let epoch = picker_auth.current_auth_flow();
        let response_auth = picker_auth.clone();
        busy.set(true);
        selection.set(None);
        spawn_local(async move {
            let result = async {
                let selected = tauri_bridge::invoke_select_archive_build(SelectArchiveRequest {
                    request_id: super::publish::archive_request_id().map_err(archive_error)?,
                    expected_publisher_npub: publisher.clone(),
                })
                .await
                .map_err(archive_error)?;
                let selected = selected.selection.ok_or("Archive selection cancelled")?;
                let inspected = tauri_bridge::invoke_inspect_archive(InspectArchiveRequest {
                    request_id: super::publish::archive_request_id().map_err(archive_error)?,
                    selection_id: selected.selection_id.clone(),
                    expected_publisher_npub: publisher.clone(),
                    expected_format: None,
                })
                .await
                .map_err(archive_error)?;
                if inspected.summary.blocking_hazards {
                    return Err("Archive inspection found blocking hazards".to_string());
                }
                let hash = tauri_bridge::invoke_hash_selected_archive(HashSelectedArchiveRequest {
                    request_id: super::publish::archive_request_id().map_err(archive_error)?,
                    selection_id: selected.selection_id,
                    expected_publisher_npub: publisher,
                })
                .await
                .map_err(archive_error)?;
                Ok(BuildSelectionRef {
                    selection_id: hash.selection_id,
                    sha256: hash.sha256,
                })
            }
            .await;
            if !response_auth.is_current_auth_flow(epoch) {
                return;
            }
            busy.set(false);
            match result {
                Ok(build) => {
                    message.set(Some(format!(
                        "Reselected SHA-256: {}. Recovery rejects a different pinned digest.",
                        build.sha256.as_str()
                    )));
                    selection.set(Some(build));
                }
                Err(error) => message.set(Some(error)),
            }
        });
    });
    let resume_auth = auth.clone();
    let resume = Callback::new(move |(operation_id, needs_archive): (String, bool)| {
        if !authorized.get_untracked() || busy.get_untracked() {
            return;
        }
        let Some(publisher) = resume_auth.npub.get_untracked() else {
            return;
        };
        let epoch = resume_auth.current_auth_flow();
        let response_auth = resume_auth.clone();
        let allow_uncertain = uncertain.get_untracked();
        let build = if needs_archive {
            selection.get_untracked()
        } else {
            None
        };
        busy.set(true);
        authorized.set(false);
        spawn_local(async move {
            let result = tauri_bridge::invoke_resume_publisher_operation(
                publisher,
                operation_id,
                allow_uncertain,
                build,
            )
            .await;
            if !response_auth.is_current_auth_flow(epoch) {
                return;
            }
            busy.set(false);
            match result {
                Ok(_) => {
                    message.set(Some("Operation reconciled through its original publication flow. Refresh the game to inspect current network state.".into()));
                    refresh.update(|v| *v = v.wrapping_add(1));
                }
                Err(error) => message.set(Some(error)),
            }
        });
    });
    view! {
        <details style="margin:14px 0;padding:16px;background:#121215;border:1px solid oklch(0.32 0.01 60);font:400 12.5px/1.6 ui-monospace,monospace;color:#fff;overflow-wrap:anywhere">
            <summary style="color:oklch(0.8 0.16 60);font-weight:800">"PUBLISHER DRAFTS & RECOVERY"</summary>
            <p>"Nothing resumes automatically. Restore drafts in their editors. Review checkpoints before authorizing an exact-event retry. Server availability is not inferred from a prior upload acknowledgement."</p>
            <button type="button" style="padding:10px;background:transparent;color:oklch(0.8 0.13 210);border:1px solid oklch(0.5 0.13 210);font-family:ui-monospace,monospace" disabled=move || busy.get() on:click=move |_| refresh.update(|v| *v = v.wrapping_add(1))>"REFRESH LOCAL RECOVERY"</button>
            <button type="button" style="padding:10px;background:transparent;color:oklch(0.8 0.13 210);border:1px solid oklch(0.5 0.13 210);font-family:ui-monospace,monospace" disabled=move || busy.get() on:click=move |_| select_archive.run(())>"RESELECT PINNED ARCHIVE"</button>
            <label style="display:block"><input type="checkbox" prop:checked=move || authorized.get() on:change:target=move |event| authorized.set(event.target().checked()) />" I authorize resuming the selected original operation using the active publisher account."</label>
            <label style="display:block"><input type="checkbox" prop:checked=move || uncertain.get() on:change:target=move |event| uncertain.set(event.target().checked()) />" If provisioning/upload outcome is unknown, I explicitly authorize a same-scope, same-artifact retry. The remote action may already have completed."</label>
            {move || state.get().map(|state| {
                let checkpoints = state.checkpoints;
                view! {
                    <p>{format!("{} saved draft(s) · {} retained operation(s)", state.drafts.len(), state.operations.len())}</p>
                    {state.drafts.into_iter().map(|draft| view! { <p>{format!("Local draft: {} · revision {} · restore in its editor", draft.draft_id, draft.revision)}</p> }).collect_view()}
                    {state.operations.into_iter().map(|operation| {
                        let id = operation.operation_id.clone();
                        let steps = checkpoints.iter().find(|(key, _)| key == &id).map(|(_, steps)| steps.clone()).unwrap_or_default();
                        let complete = operation.state == "complete";
                        let needs_archive = operation.artifact_hash.is_some();
                        view! {
                            <article style="padding:12px;margin-top:12px;border:1px solid oklch(0.32 0.01 60);background:#141417;clip-path:polygon(0 0,calc(100% - 12px) 0,100% 12px,100% 100%,12px 100%,0 calc(100% - 12px))">
                                <p>{format!("{} · {} · {}", operation.kind, operation.coordinate, operation.state)}</p>
                                <p>{format!("Operation {} · pinned hash {}", operation.operation_id, operation.artifact_hash.unwrap_or_else(|| "No new archive".into()))}</p>
                                {steps.into_iter().map(|step| view! { <p>{format!("{} · {} · {}{}", step.kind, step.target, if step.state == "prepared" { "unacknowledged / outcome unknown" } else { step.state.as_str() }, step.last_error.map(|error| format!(" · previous attempt: {error}")).unwrap_or_default())}</p> }).collect_view()}
                                <button type="button" style="padding:10px;background:transparent;color:oklch(0.8 0.13 210);border:1px solid oklch(0.5 0.13 210);font-family:ui-monospace,monospace" disabled=move || complete || busy.get() || !authorized.get() on:click=move |_| resume.run((id.clone(), needs_archive))>"RECONCILE & RESUME ORIGINAL"</button>
                            </article>
                        }
                    }).collect_view()}
                }
            })}
            {move || message.get().map(|message| view! { <p role="status">{message}</p> })}
        </details>
    }
}
