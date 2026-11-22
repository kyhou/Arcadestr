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

/// Whether a retained operation belongs to the game with this identifier.
/// Listing and store page coordinates both end in the game's `d` tag.
fn operation_matches_game(coordinate: &str, d_tag: &str) -> bool {
    coordinate
        .splitn(3, ':')
        .nth(2)
        .is_some_and(|value| !value.is_empty() && value == d_tag)
}

/// Unfinished publishes the publisher can resume. On the dashboard it lists
/// every game; inside a game (`game_d_tag`) it only shows that game's.
#[component]
pub fn PublisherRecoveryPanel(
    /// Only show operations for this game.
    #[prop(optional, into)]
    game_d_tag: Option<Signal<Option<String>>>,
    /// Bump to reload after a publish attempt.
    #[prop(optional, into)]
    refresh_key: Option<Signal<u64>>,
    /// Runs after an operation finished successfully.
    #[prop(optional)]
    on_finished: Option<Callback<()>>,
    /// A build already selected in the editor, reused when its fingerprint
    /// matches the operation's, so the file needn't be chosen again.
    #[prop(optional, into)]
    current_build: Option<Signal<Option<BuildSelectionRef>>>,
) -> impl IntoView {
    let auth = use_context::<crate::AuthContext>().expect("AuthContext not provided");
    let scoped = game_d_tag.is_some();
    let in_scope = move |coordinate: &str| match game_d_tag {
        None => true,
        Some(d_tag) => d_tag
            .get()
            .is_some_and(|d_tag| operation_matches_game(coordinate, &d_tag)),
    };
    let state = RwSignal::new(None::<PublisherRecoveryState>);
    let message = RwSignal::new(None::<String>);
    let authorized = RwSignal::new(false);
    let uncertain = RwSignal::new(false);
    let busy = RwSignal::new(false);
    let selection = RwSignal::new(None::<BuildSelectionRef>);
    let refresh = RwSignal::new(0_u64);
    let query_auth = auth.clone();
    let loaded_epoch = StoredValue::new(None::<u64>);
    Effect::new(move |_| {
        refresh.track();
        if let Some(key) = refresh_key {
            key.track();
        }
        let epoch = query_auth.current_auth_flow();
        let publisher = query_auth.npub.get();
        // A reload keeps the last result visible; only another account clears it.
        let account_changed = loaded_epoch.get_value() != Some(epoch);
        loaded_epoch.set_value(Some(epoch));
        if account_changed {
            state.set(None);
            message.set(None);
        }
        selection.set(None);
        authorized.set(false);
        uncertain.set(false);
        busy.set(false);
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
                let selected = selected.selection.ok_or("No file was selected")?;
                let inspected = tauri_bridge::invoke_inspect_archive(InspectArchiveRequest {
                    request_id: super::publish::archive_request_id().map_err(archive_error)?,
                    selection_id: selected.selection_id.clone(),
                    expected_publisher_npub: publisher.clone(),
                    expected_format: None,
                })
                .await
                .map_err(archive_error)?;
                if inspected.summary.blocking_hazards {
                    return Err("This archive has problems and can't be used.".to_string());
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
                    message.set(Some(
                        "Build file selected. Resuming checks that it matches the original build."
                            .into(),
                    ));
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
            selection.get_untracked().or_else(|| {
                let expected = state.with_untracked(|state| {
                    state.as_ref().and_then(|state| {
                        state
                            .operations
                            .iter()
                            .find(|operation| operation.operation_id == operation_id)
                            .and_then(|operation| operation.artifact_hash.clone())
                    })
                });
                current_build
                    .and_then(|build| build.get_untracked())
                    .filter(|build| expected.as_deref() == Some(build.sha256.as_str()))
            })
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
                    refresh.update(|v| *v = v.wrapping_add(1));
                    message.set(Some(if scoped {
                        "Publishing finished.".into()
                    } else {
                        "Publish finished. Open the game to see its current state.".into()
                    }));
                    if let Some(on_finished) = on_finished {
                        on_finished.run(());
                    }
                }
                Err(error) => message.set(Some(super::publish::publish_error_message(&error))),
            }
        });
    });
    let pending = move || {
        state.with(|state| {
            state.as_ref().map_or(0, |state| {
                state
                    .operations
                    .iter()
                    .filter(|operation| {
                        operation.state != "complete" && in_scope(&operation.coordinate)
                    })
                    .count()
            })
        })
    };
    let has_work = move || message.get().is_some() || pending() > 0;
    view! {
        <Show when=has_work>
        <section class="v2-hub-card v2-publisher-recovery" aria-labelledby="publisher-recovery-title">
            <div class="v2-create-row-between">
                <h2 id="publisher-recovery-title" class="v2-create-section-title" data-testid="publisher-recovery-summary">{move || if scoped { scoped_summary(pending()) } else { recovery_summary(state.get().as_ref()) }}</h2>
                <button type="button" class="v2-btn-secondary" disabled=move || busy.get() on:click=move |_| refresh.update(|v| *v = v.wrapping_add(1))>"Check again"</button>
            </div>
            <p class="v2-create-help">{if scoped {
                "A publish of this game stopped before it finished. Finishing it re-sends exactly what was signed before and skips the steps that already worked. Nothing is retried automatically."
            } else {
                "These publishes stopped before they finished. Nothing is retried automatically. Resuming re-sends exactly what was signed before; it can't publish different content. To publish changes instead, open the game."
            }}</p>
            {move || state.get().map(|state| {
                let checkpoints = state.checkpoints;
                state.operations.into_iter().filter(|operation| operation.state != "complete" && in_scope(&operation.coordinate)).map(|operation| {
                    let id = operation.operation_id.clone();
                    let steps = checkpoints.iter().find(|(key, _)| key == &id).map(|(_, steps)| steps.clone()).unwrap_or_default();
                    let needs_archive = operation.artifact_hash.is_some();
                    view! {
                        <article class="v2-hub-role-row">
                            <p class="v2-hub-row-title">{operation_label(&operation.kind)}</p>
                            {needs_archive.then(|| view! {
                                <p class="v2-create-help">{if current_build.is_some() {
                                    "This publish included a build. If the build file chosen in Delivery & builds is the same one, it's used; otherwise select it again. A different file is rejected."
                                } else {
                                    "This publish included a build. Select the same build file again before resuming; a different file is rejected."
                                }}</p>
                            })}
                            <details class="v2-create-disclosure">
                                <summary class="v2-create-disclosure-summary">"Technical details"</summary>
                                <p class="v2-create-mono-value">{format!("{} · {}", operation.coordinate, operation.state)}</p>
                                <p class="v2-create-mono-value">{format!("Operation {} · build fingerprint {}", operation.operation_id, operation.artifact_hash.unwrap_or_else(|| "none".into()))}</p>
                                {steps.into_iter().map(|step| view! { <p class="v2-create-mono-value">{format!("{} · {} · {}{}", step.kind, step.target, checkpoint_label(false, &step.kind, &step.state), step.last_error.map(|error| format!(" · previous attempt: {error}")).unwrap_or_default())}</p> }).collect_view()}
                            </details>
                            <div class="v2-publisher-actions">
                                {needs_archive.then(|| view! {
                                    <button type="button" class="v2-btn-secondary" disabled=move || busy.get() on:click=move |_| select_archive.run(())>"Select build file"</button>
                                })}
                                <button type="button" class="v2-btn-primary" disabled=move || busy.get() || !authorized.get() on:click=move |_| resume.run((id.clone(), needs_archive))>{if scoped { "Finish publishing" } else { "Resume publish" }}</button>
                            </div>
                        </article>
                    }
                }).collect_view()
            })}
            <label class="v2-create-checkbox-row"><input type="checkbox" prop:checked=move || authorized.get() on:change:target=move |event| authorized.set(event.target().checked()) />"Resume with my active account."</label>
            <label class="v2-create-checkbox-row"><input type="checkbox" prop:checked=move || uncertain.get() on:change:target=move |event| uncertain.set(event.target().checked()) />"If it's unclear whether an upload finished, it's OK to send the same build again."</label>
            {move || message.get().map(|message| view! { <p class="v2-create-inline-note" role="status">{message}</p> })}
        </section>
        </Show>
    }
}

/// Unfinished operations need publisher action; completed ones are history.
fn incomplete_operations(state: &PublisherRecoveryState) -> usize {
    state
        .operations
        .iter()
        .filter(|operation| operation.state != "complete")
        .count()
}

fn recovery_summary(state: Option<&PublisherRecoveryState>) -> String {
    match state.map(incomplete_operations) {
        Some(0) | None => "Unfinished publishing".into(),
        Some(1) => "Unfinished publishing · 1 to resolve".into(),
        Some(count) => format!("Unfinished publishing · {count} to resolve"),
    }
}

fn scoped_summary(pending: usize) -> String {
    match pending {
        0 | 1 => "Publishing didn't finish".into(),
        count => format!("{count} publishes didn't finish"),
    }
}

fn operation_label(kind: &str) -> String {
    match kind {
        "listing" => "Game publish".into(),
        "store_page" => "Store page update".into(),
        "campaign" => "Promotion publish".into(),
        kind if kind.starts_with("revoke") || kind.starts_with("cancel") => "Access change".into(),
        other => other.replace('_', " "),
    }
}

/// Recovery wording for one journal checkpoint. `prepared` rows are retained
/// intents: only on an unfinished operation can their remote outcome be
/// unknown, and `distribution` rows are declarations whose evidence comes from
/// the provision/upload steps.
fn checkpoint_label<'a>(operation_complete: bool, kind: &str, state: &'a str) -> &'a str {
    match state {
        "prepared" if operation_complete => "recorded · operation complete",
        "prepared" if kind == "distribution" => "declared; see provision/upload steps",
        "prepared" => "unacknowledged / outcome unknown",
        other => other,
    }
}

#[cfg(test)]
mod checkpoint_label_tests {
    use super::checkpoint_label;

    #[test]
    fn incomplete_operations_are_announced_and_completed_ones_are_not() {
        use super::*;
        let operation = |state: &str| arcadestr_core::publisher_recovery::PublisherOperation {
            operation_id: state.into(),
            kind: "listing".into(),
            coordinate: "c".into(),
            intent: serde_json::Value::Null,
            artifact_hash: None,
            state: state.into(),
            result: None,
        };
        let mut state = PublisherRecoveryState {
            drafts: vec![],
            operations: vec![operation("complete")],
            checkpoints: vec![],
        };
        assert_eq!(recovery_summary(Some(&state)), "Unfinished publishing");
        state.operations.push(operation("incomplete"));
        assert_eq!(incomplete_operations(&state), 1);
        assert_eq!(
            recovery_summary(Some(&state)),
            "Unfinished publishing · 1 to resolve"
        );
    }

    #[test]
    fn game_scoped_recovery_only_matches_that_game() {
        use super::operation_matches_game;
        assert!(operation_matches_game("30402:abc:neon", "neon"));
        assert!(operation_matches_game("30078:abc:neon", "neon"));
        assert!(!operation_matches_game("30402:abc:neon-2", "neon"));
        assert!(!operation_matches_game("30402:abc:", ""));
        assert!(!operation_matches_game("malformed", "malformed"));
        // Legacy identifiers may contain colons; everything after the second belongs to the tag.
        assert!(operation_matches_game("30402:abc:old:id", "old:id"));
    }

    #[test]
    fn operation_kinds_use_plain_names() {
        assert_eq!(super::operation_label("listing"), "Game publish");
        assert_eq!(super::operation_label("revoke_grant"), "Access change");
        assert_eq!(super::operation_label("new_kind"), "new kind");
    }

    #[test]
    fn completed_operations_never_report_unknown_outcomes() {
        for kind in ["distribution", "revoke_grant", "upload", "provision"] {
            assert_eq!(
                checkpoint_label(true, kind, "prepared"),
                "recorded · operation complete"
            );
        }
        assert_eq!(
            checkpoint_label(true, "relay", "acknowledged"),
            "acknowledged"
        );
    }

    #[test]
    fn unfinished_operations_keep_unknown_outcomes_visible() {
        assert_eq!(
            checkpoint_label(false, "upload", "prepared"),
            "unacknowledged / outcome unknown"
        );
        assert_eq!(
            checkpoint_label(false, "provision", "prepared"),
            "unacknowledged / outcome unknown"
        );
        assert_eq!(
            checkpoint_label(false, "distribution", "prepared"),
            "declared; see provision/upload steps"
        );
        assert_eq!(checkpoint_label(false, "upload", "failed"), "failed");
    }
}
