use leptos::prelude::*;
use wasm_bindgen_futures::spawn_local;

/// Backup creation and restoration UI
#[component]
pub fn BackupManager() -> impl IntoView {
    let backup_string = RwSignal::new(String::new());
    let restore_input = RwSignal::new(String::new());
    let status = RwSignal::new(None::<String>);
    let is_loading = RwSignal::new(false);

    let create_backup = move |_| {
        spawn_local(async move {
            is_loading.set(true);
            status.set(Some("Creating backup...".to_string()));

            match crate::invoke_create_backup().await {
                Ok(result) => {
                    if let Some(backup) = result.get("backup").and_then(|b| b.as_str()) {
                        backup_string.set(backup.to_string());
                        status.set(Some(
                            "✅ Backup created! Save this string securely.".to_string(),
                        ));
                    } else {
                        status.set(Some("❌ Failed to get backup data".to_string()));
                    }
                }
                Err(e) => {
                    status.set(Some(format!("❌ Error: {}", e)));
                }
            }
            is_loading.set(false);
        });
    };

    let restore_backup = move |_| {
        let backup_data = restore_input.get();
        if backup_data.is_empty() {
            status.set(Some("❌ Please paste a backup string".to_string()));
            return;
        }

        spawn_local(async move {
            is_loading.set(true);
            status.set(Some("Restoring accounts...".to_string()));

            match crate::invoke_restore_backup(backup_data).await {
                Ok(result) => {
                    let count = result
                        .get("restored_count")
                        .and_then(|c| c.as_u64())
                        .unwrap_or(0);
                    status.set(Some(format!("✅ Restored {} accounts", count)));
                    restore_input.set(String::new());
                }
                Err(e) => {
                    status.set(Some(format!("❌ Error: {}", e)));
                }
            }
            is_loading.set(false);
        });
    };

    view! {
        <div class="backup-manager">
            <h3>"Backup & Restore"</h3>

            <div class="backup-section">
                <h4>"Create Backup"</h4>
                <p class="info">
                    "Create an encrypted backup of all your accounts. "
                    "Save this string securely - it can restore your accounts on any device."
                </p>

                <button
                    on:click=create_backup
                    disabled=is_loading
                    class="primary"
                >
                    "Generate Backup"
                </button>

                <Show when=move || !backup_string.get().is_empty()>
                    <div class="backup-output">
                        <label>"Your encrypted backup:"</label>
                        <textarea
                            readonly
                            prop:value=backup_string
                            rows="6"
                            class="backup-string"
                        />
                        <p class="warning">
                            "⚠️ Save this somewhere secure! This is the only way to recover your accounts."
                        </p>
                    </div>
                </Show>
            </div>

            <div class="restore-section">
                <h4>"Restore from Backup"</h4>
                <p class="info">
                    "Paste a backup string to restore your accounts."
                </p>

                <textarea
                    placeholder="Paste backup string here..."
                    bind:value=restore_input
                    rows="6"
                    class="restore-input"
                />

                <button
                    on:click=restore_backup
                    disabled=is_loading
                    class="secondary"
                >
                    "Restore Accounts"
                </button>
            </div>

            <Show when=move || status.get().is_some()>
                <div class="status-message">
                    {status.get().unwrap()}
                </div>
            </Show>
        </div>
    }
}
