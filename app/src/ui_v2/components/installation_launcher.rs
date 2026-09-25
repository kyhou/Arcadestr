use std::sync::{
    atomic::{AtomicBool, AtomicU64, Ordering},
    Arc,
};

static NEXT_LAUNCHER_INSTANCE: AtomicU64 = AtomicU64::new(1);

thread_local! {
    static LAST_PREFERENCE_REVISION: std::cell::Cell<i64> = const { std::cell::Cell::new(0) };
}

use arcadestr_core::{
    distribution::LaunchType,
    launch_contract::{
        LaunchConfiguration, LaunchFallbackReason, LaunchOption, LaunchOutcome, LaunchSession,
        ProfileReferenceStatus,
    },
};
use leptos::prelude::*;
use wasm_bindgen_futures::spawn_local;

use crate::tauri_bridge::{
    invoke_get_launch_options, invoke_get_launch_status, invoke_launch_game,
    invoke_set_launch_profile_preference,
};
use crate::ui_v2::components::{
    Dialog, DialogCloseAction, DialogClosePolicy, DialogCloseRequest, DialogDismissal,
    DialogInitialFocus, DialogWidth,
};

fn play_candidates(options: &[LaunchOption]) -> Vec<LaunchOption> {
    options
        .iter()
        .filter(|option| option.is_play_candidate())
        .cloned()
        .collect()
}

fn saved_preference_value(configuration: &LaunchConfiguration) -> String {
    if configuration.saved_preference_status == ProfileReferenceStatus::Available {
        configuration
            .saved_preference_profile_id
            .clone()
            .unwrap_or_default()
    } else {
        String::new()
    }
}

fn automatic_profile_label(configuration: &LaunchConfiguration) -> String {
    let publisher_label = (configuration.publisher_default_status
        == ProfileReferenceStatus::Available)
        .then(|| configuration.publisher_default_profile_id.as_ref())
        .flatten()
        .and_then(|id| {
            configuration
                .options
                .iter()
                .find(|option| &option.profile_id == id)
        })
        .map(|option| option.display_name.as_str());
    if let Some(label) = publisher_label {
        return format!("Use publisher default ({label})");
    }
    let label = configuration
        .effective_profile_id
        .as_ref()
        .and_then(|id| {
            configuration
                .options
                .iter()
                .find(|option| &option.profile_id == id)
        })
        .map(|option| option.display_name.as_str())
        .unwrap_or("unavailable");
    format!("Use automatic default ({label})")
}

fn fallback_message(configuration: &LaunchConfiguration) -> Option<&'static str> {
    if configuration.fallback_reasons.iter().any(|reason| {
        matches!(
            reason,
            LaunchFallbackReason::SavedPreferenceInvalid
                | LaunchFallbackReason::SavedPreferenceMissing
                | LaunchFallbackReason::SavedPreferenceNonGame
                | LaunchFallbackReason::SavedPreferenceUnavailable
        )
    }) {
        Some("Your saved Play profile is unavailable. Using the best available profile without changing your preference.")
    } else if configuration.fallback_reasons.iter().any(|reason| {
        matches!(
            reason,
            LaunchFallbackReason::PublisherDefaultInvalid
                | LaunchFallbackReason::PublisherDefaultMissing
                | LaunchFallbackReason::PublisherDefaultNonGame
                | LaunchFallbackReason::PublisherDefaultUnavailable
        )
    }) {
        Some("The publisher's default profile is unavailable. Using the first available game profile.")
    } else {
        None
    }
}

fn effective_profile_label(configuration: &LaunchConfiguration) -> String {
    configuration
        .effective_profile_id
        .as_ref()
        .and_then(|id| {
            configuration
                .options
                .iter()
                .find(|option| &option.profile_id == id)
        })
        .map(|option| option.display_name.clone())
        .unwrap_or_else(|| "Unavailable".into())
}

fn has_properties(configuration: &LaunchConfiguration) -> bool {
    play_candidates(&configuration.options).len() > 1
        || configuration
            .options
            .iter()
            .any(|option| option.launch_type != LaunchType::Game)
        || configuration
            .options
            .iter()
            .any(|option| option.unavailable_reason.is_some())
}

fn next_properties_dialog_id() -> String {
    format!(
        "game-properties-{}",
        NEXT_LAUNCHER_INSTANCE.fetch_add(1, Ordering::Relaxed)
    )
}

fn preference_revision(persisted: Option<i64>) -> i64 {
    let now;
    #[cfg(target_arch = "wasm32")]
    {
        now = js_sys::Date::now() as i64;
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|duration| i64::try_from(duration.as_millis()).unwrap_or(i64::MAX))
            .unwrap_or(0);
    }
    LAST_PREFERENCE_REVISION.with(|last| {
        let next = now
            .max(persisted.unwrap_or(0))
            .max(last.get())
            .saturating_add(1);
        last.set(next);
        next
    })
}

fn session_label(session: &LaunchSession) -> String {
    match &session.outcome {
        None => "Running".into(),
        Some(LaunchOutcome::Exited {
            code: Some(code), ..
        }) => format!("Exited (code {code})"),
        Some(LaunchOutcome::Exited {
            signal: Some(signal),
            ..
        }) => format!("Exited (signal {signal})"),
        Some(LaunchOutcome::Exited { .. }) => "Exited".into(),
        Some(LaunchOutcome::Detached) => "Detached from client".into(),
    }
}

/// Launch controls depend solely on the local installation ID. Ownership,
/// current listing availability and relay state are deliberately not inputs.
#[component]
pub fn InstallationLauncher(installation_id: String) -> impl IntoView {
    let id = StoredValue::new(installation_id);
    let configuration = RwSignal::new(None::<LaunchConfiguration>);
    let loaded = RwSignal::new(false);
    let session = RwSignal::new(None::<LaunchSession>);
    let busy = RwSignal::new(false);
    let preference_busy = RwSignal::new(false);
    let preference_selection = RwSignal::new(String::new());
    let properties_open = RwSignal::new(false);
    let launcher_error = RwSignal::new(None::<String>);
    let modal_error = RwSignal::new(None::<String>);
    let status_error = RwSignal::new(None::<String>);
    let poll_busy = RwSignal::new(false);
    let generation = RwSignal::new(0_u64);
    let alive = Arc::new(AtomicBool::new(true));
    let cleanup_alive = alive.clone();
    on_cleanup(move || cleanup_alive.store(false, Ordering::Relaxed));

    let alive_for_load = alive.clone();
    Effect::new(move |_| {
        let alive = alive_for_load.clone();
        spawn_local(async move {
            let result = invoke_get_launch_options(&id.get_value()).await;
            if !alive.load(Ordering::Relaxed) {
                return;
            }
            match result {
                Ok(value) => {
                    preference_selection.set(saved_preference_value(&value));
                    configuration.set(Some(value));
                }
                Err(value) => launcher_error.set(Some(value.to_string())),
            }
            loaded.set(true);
        });
    });

    let alive_for_poll = alive.clone();
    let poll = Callback::new(move |()| {
        if busy.get_untracked() || poll_busy.get_untracked() {
            return;
        }
        poll_busy.set(true);
        let requested_generation = generation.get_untracked();
        let alive = alive_for_poll.clone();
        spawn_local(async move {
            let result = invoke_get_launch_status(&id.get_value()).await;
            if !alive.load(Ordering::Relaxed) {
                return;
            }
            poll_busy.set(false);
            if generation.get_untracked() != requested_generation {
                return;
            }
            match result {
                Ok(value) => {
                    session.set(value);
                    status_error.set(None);
                }
                Err(value) => status_error.set(Some(value.to_string())),
            }
        });
    });
    Effect::new(move |_| poll.run(()));
    #[cfg(target_arch = "wasm32")]
    {
        let interval =
            send_wrapper::SendWrapper::new(gloo_timers::callback::Interval::new(750, move || {
                poll.run(())
            }));
        on_cleanup(move || drop(interval));
    }

    let alive_for_launch = alive.clone();
    let launch = Callback::new(move |profile_id: Option<String>| {
        if busy.get_untracked() {
            return;
        }
        busy.set(true);
        generation.update(|value| *value += 1);
        let modal_launch = profile_id.is_some();
        if modal_launch {
            modal_error.set(None);
        } else {
            launcher_error.set(None);
        }
        let alive = alive_for_launch.clone();
        spawn_local(async move {
            let result = invoke_launch_game(&id.get_value(), profile_id.as_deref()).await;
            if !alive.load(Ordering::Relaxed) {
                return;
            }
            match result {
                Ok(value) => {
                    session.set(Some(value));
                    status_error.set(None);
                }
                Err(value) => {
                    if modal_launch {
                        modal_error.set(Some(value.to_string()));
                    } else {
                        launcher_error.set(Some(value.to_string()));
                    }
                }
            }
            busy.set(false);
        });
    });
    let alive_for_preference = alive.clone();
    let save_preference = Callback::new(move |profile_id: Option<String>| {
        if preference_busy.get_untracked() {
            return;
        }
        preference_busy.set(true);
        modal_error.set(None);
        let updated_at = preference_revision(
            configuration
                .get_untracked()
                .and_then(|value| value.preference_updated_at),
        );
        let alive = alive_for_preference.clone();
        spawn_local(async move {
            let result = invoke_set_launch_profile_preference(
                &id.get_value(),
                profile_id.as_deref(),
                updated_at,
            )
            .await;
            if !alive.load(Ordering::Relaxed) {
                return;
            }
            match result {
                Ok(value) => {
                    preference_selection.set(saved_preference_value(&value));
                    configuration.set(Some(value));
                }
                Err(value) => {
                    preference_selection.set(
                        configuration
                            .get_untracked()
                            .as_ref()
                            .map(saved_preference_value)
                            .unwrap_or_default(),
                    );
                    modal_error.set(Some(value.to_string()));
                }
            }
            preference_busy.set(false);
        });
    });
    let running = Memo::new(move |_| session.get().is_some_and(|value| value.is_running()));
    let disabled = move || busy.get() || preference_busy.get() || running.get();
    let dialog_id = next_properties_dialog_id();

    view! {
        <div class="arc-installation-launcher" aria-label="Installed game launch controls">
            <Show when=move || !loaded.get()><span role="status">"Loading launch profiles…"</span></Show>
            <Show when=move || configuration.get().is_some_and(|value| !play_candidates(&value.options).is_empty())>
                <button class="v2-btn-primary"
                    disabled=move || disabled() || configuration.get().and_then(|value| value.effective_profile_id).is_none()
                    on:click=move |_| launch.run(None)>
                    {move || if busy.get() { "Starting…" } else if running.get() { "Game Running" } else { "Play Game" }}
                </button>
                <Show when=move || configuration.get().as_ref().and_then(fallback_message).is_some()>
                    <p class="arc-library-row-warning" role="status">{move || configuration.get().as_ref().and_then(fallback_message)}</p>
                </Show>
            </Show>
            <Show when=move || loaded.get() && configuration.get().is_some_and(|value| play_candidates(&value.options).is_empty())>
                <span class="arc-library-row-warning">"No available game launch profile"</span>
            </Show>
            <Show when=move || configuration.get().as_ref().is_some_and(has_properties)>
                <button type="button" class="v2-btn-secondary arc-game-properties-button"
                    aria-haspopup="dialog"
                    on:click=move |_| {
                        modal_error.set(None);
                        properties_open.set(true);
                    }>
                    <span class="material-symbols-outlined" aria-hidden="true">"settings"</span>
                    "Properties"
                </button>
            </Show>
            <div role="status" aria-live="polite">{move || session.get().as_ref().map(session_label)}</div>
            <Show when=move || launcher_error.get().is_some()><p class="arc-library-row-warning" role="alert">{move || launcher_error.get()}</p></Show>
            <Show when=move || status_error.get().is_some()><p role="status">{move || status_error.get().map(|error| format!("Running state unavailable: {error}"))}</p></Show>

            <Dialog
                id=dialog_id
                open=Signal::derive(move || properties_open.get())
                title="Game Properties"
                kicker="Installed game"
                description="Advanced launch preferences for this installation. Normal play uses the correct publisher profile automatically."
                width=DialogWidth::Standard
                policy=DialogClosePolicy::BlockedWhileBusy
                dismissal=DialogDismissal::busy_aware()
                busy=Signal::derive(move || preference_busy.get())
                initial_focus=DialogInitialFocus::CloseButton
                close_label="Close game properties"
                close_blocked_hint="Wait for the launch preference to finish saving"
                on_close=UnsyncCallback::new(move |request: DialogCloseRequest| {
                    if request.action == DialogCloseAction::Dismiss {
                        properties_open.set(false);
                    }
                })
                actions=move || view! {
                    <button type="button" class="v2-btn-secondary" disabled=move || preference_busy.get()
                        on:click=move |_| properties_open.set(false)>"Close"</button>
                }
            >
                <div class="arc-game-properties">
                    <div class="arc-game-properties-nav">
                        <span>"General"</span>
                    </div>
                    <div class="arc-game-properties-content">
                        <section>
                            <div class="arc-game-properties-heading">
                                <div>
                                    <h3>"Launch Options"</h3>
                                    <p>"Arcadestr automatically chooses the publisher profile for this device. Override it only when needed."</p>
                                </div>
                                <span class="arc-game-properties-effective">{move || configuration.get().as_ref().map(effective_profile_label).unwrap_or_else(|| "Unavailable".into())}</span>
                            </div>
                            <Show when=move || configuration.get().is_some_and(|value| play_candidates(&value.options).len() > 1)>
                                <label class="arc-game-properties-field">
                                    <span>"Preferred Play profile"</span>
                                    <select prop:value=move || preference_selection.get()
                                        disabled=disabled
                                        on:change=move |event| {
                                            let value = event_target_value(&event);
                                            preference_selection.set(value.clone());
                                            save_preference.run((!value.is_empty()).then_some(value));
                                        }>
                                        <option value="">{move || configuration.get().as_ref().map(automatic_profile_label).unwrap_or_else(|| "Use automatic default".into())}</option>
                                        {move || configuration.get().map(|value| play_candidates(&value.options).into_iter().map(|option| {
                                            let is_publisher_default = value.publisher_default_profile_id.as_deref() == Some(option.profile_id.as_str());
                                            let label = if is_publisher_default {
                                                format!("{} (publisher default)", option.display_name)
                                            } else {
                                                option.display_name
                                            };
                                            view! { <option value=option.profile_id>{label}</option> }
                                        }).collect::<Vec<_>>())}
                                    </select>
                                </label>
                            </Show>
                            <Show when=move || preference_busy.get()>
                                <p class="arc-dialog-status" role="status">"Saving launch preference…"</p>
                            </Show>
                            <Show when=move || modal_error.get().is_some()>
                                <p class="arc-dialog-alert" role="alert">{move || modal_error.get()}</p>
                            </Show>
                        </section>
                        <Show when=move || configuration.get().is_some_and(|value| value.options.iter().any(|option| option.launch_type != LaunchType::Game))>
                            <section>
                                <div class="arc-game-properties-heading">
                                    <div><h3>"Tools"</h3><p>"Run publisher-provided configuration utilities and other non-game profiles."</p></div>
                                </div>
                                <div class="arc-game-properties-tools">
                                    {move || configuration.get().map(|value| value.options.into_iter().filter(|option| option.launch_type != LaunchType::Game).map(|option| {
                                        let unavailable = option.unavailable_reason.is_some();
                                        let profile = option.profile_id.clone();
                                        view! {
                                            <button type="button" class="v2-btn-secondary" disabled=move || unavailable || disabled()
                                                on:click=move |_| launch.run(Some(profile.clone()))>{format!("{} ({:?})", option.display_name, option.launch_type)}</button>
                                        }
                                    }).collect::<Vec<_>>())}
                                </div>
                            </section>
                        </Show>
                        <Show when=move || configuration.get().is_some_and(|value| value.options.iter().any(|option| option.unavailable_reason.is_some()))>
                            <section>
                                <div class="arc-game-properties-heading">
                                    <div><h3>"Unavailable Profiles"</h3><p>"These publisher profiles cannot run from the current installation."</p></div>
                                </div>
                                <ul class="arc-game-properties-unavailable">{move || configuration.get().map(|value| value.options.into_iter().filter_map(|option| option.unavailable_reason.map(|reason| view! {
                                    <li><strong>{option.display_name}</strong><span>{reason}</span></li>
                                })).collect::<Vec<_>>())}</ul>
                            </section>
                        </Show>
                        <div class="arc-game-properties-status" role="status" aria-live="polite">
                            {move || if busy.get() {
                                Some("Starting launch profile…".to_string())
                            } else {
                                session.get().as_ref().map(session_label)
                            }}
                        </div>
                        <Show when=move || status_error.get().is_some()>
                            <p class="arc-dialog-status" role="status">{move || status_error.get().map(|error| format!("Running state unavailable: {error}"))}</p>
                        </Show>
                    </div>
                </div>
            </Dialog>
        </div>
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn option(id: &str, launch_type: LaunchType, reason: Option<&str>) -> LaunchOption {
        LaunchOption {
            profile_id: id.into(),
            display_name: id.into(),
            launch_type,
            unavailable_reason: reason.map(str::to_owned),
        }
    }

    fn configuration(options: Vec<LaunchOption>) -> LaunchConfiguration {
        LaunchConfiguration {
            options,
            publisher_default_profile_id: Some("game".into()),
            saved_preference_profile_id: None,
            preference_updated_at: None,
            publisher_default_status: ProfileReferenceStatus::Available,
            saved_preference_status: ProfileReferenceStatus::NotConfigured,
            effective_profile_id: Some("game".into()),
            resolution: arcadestr_core::launch_contract::LaunchResolutionStatus::Selected(
                arcadestr_core::launch_contract::LaunchProfileSource::PublisherDefault,
            ),
            fallback_reasons: Vec::new(),
        }
    }

    #[test]
    fn play_excludes_tools_and_unavailable_games() {
        let mut options = vec![
            option("config", LaunchType::Config, None),
            option("missing", LaunchType::Game, Some("Executable is missing")),
        ];
        assert!(play_candidates(&options).is_empty());
        options.push(option("game", LaunchType::Game, None));
        options.push(option("vulkan", LaunchType::Game, None));
        assert_eq!(
            play_candidates(&options)
                .into_iter()
                .map(|option| option.profile_id)
                .collect::<Vec<_>>(),
            vec!["game", "vulkan"]
        );
        assert_eq!(
            options[1].unavailable_reason.as_deref(),
            Some("Executable is missing")
        );
    }

    #[test]
    fn properties_appear_only_for_advanced_launch_controls() {
        let one_game = configuration(vec![option("game", LaunchType::Game, None)]);
        assert!(!has_properties(&one_game));
        assert_eq!(effective_profile_label(&one_game), "game");

        let multiple_games = configuration(vec![
            option("game", LaunchType::Game, None),
            option("vulkan", LaunchType::Game, None),
        ]);
        assert!(has_properties(&multiple_games));

        let tool = configuration(vec![
            option("game", LaunchType::Game, None),
            option("config", LaunchType::Config, None),
        ]);
        assert!(has_properties(&tool));

        let unavailable = configuration(vec![
            option("game", LaunchType::Game, None),
            option("missing", LaunchType::Game, Some("Executable is missing")),
        ]);
        assert!(has_properties(&unavailable));
    }

    #[test]
    fn properties_dialog_ids_are_unique_per_launcher_instance() {
        assert_ne!(next_properties_dialog_id(), next_properties_dialog_id());
    }

    #[test]
    fn saved_preference_and_fallback_copy_follow_backend_status() {
        let mut value = configuration(vec![option("game", LaunchType::Game, None)]);
        assert_eq!(saved_preference_value(&value), "");
        assert_eq!(
            automatic_profile_label(&value),
            "Use publisher default (game)"
        );
        assert_eq!(fallback_message(&value), None);

        value.saved_preference_profile_id = Some("vulkan".into());
        value.saved_preference_status = ProfileReferenceStatus::Unavailable;
        value.fallback_reasons = vec![LaunchFallbackReason::SavedPreferenceUnavailable];
        assert_eq!(saved_preference_value(&value), "");
        assert!(fallback_message(&value)
            .expect("fallback message")
            .contains("without changing your preference"));

        value.publisher_default_status = ProfileReferenceStatus::Unavailable;
        assert_eq!(
            automatic_profile_label(&value),
            "Use automatic default (game)"
        );
    }

    #[test]
    fn preference_revisions_are_monotonic_and_honor_persisted_state() {
        let first = preference_revision(None);
        let second = preference_revision(None);
        assert!(second > first);
        let persisted = second.saturating_add(100);
        assert!(preference_revision(Some(persisted)) > persisted);
    }

    #[test]
    fn nonzero_exit_is_not_called_a_crash() {
        let session = LaunchSession {
            session_id: "session".into(),
            installation_id: "installation".into(),
            profile_id: "game".into(),
            pid: 1,
            started_at: 1,
            ended_at: Some(2),
            outcome: Some(LaunchOutcome::Exited {
                code: Some(7),
                signal: None,
            }),
        };
        assert_eq!(session_label(&session), "Exited (code 7)");
        assert!(!session.is_running());
    }
}
