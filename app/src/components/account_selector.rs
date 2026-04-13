use crate::models::Nip49ImportRequest;
use crate::store::profiles::use_profile;
use crate::{AuthContext, StoredAccount};
use leptos::prelude::*;
use wasm_bindgen_futures::spawn_local;

const NIP49_HRP: &str = "ncryptsec";
const NIP49_VERSION_V2: u8 = 0x02;
const NIP49_MIN_PASSWORD_CHARS: usize = 8;
const BECH32_CHARSET: &str = "qpzry9x8gf2tvdw0s3jn54khce6mua7l";

#[derive(Clone, Copy, PartialEq, Eq)]
enum AddAccountTab {
    ExistingMethods,
    ImportFromBackup,
}

fn validate_nip49_password(password: &str) -> Result<(), String> {
    if password.trim().is_empty() || password.chars().count() < NIP49_MIN_PASSWORD_CHARS {
        return Err("NIP-49 password must be at least 8 characters.".to_string());
    }

    Ok(())
}

fn extract_nip49_version(ncryptsec: &str) -> Result<u8, String> {
    let normalized = ncryptsec.trim().to_ascii_lowercase();
    let (hrp, data_part) = normalized
        .split_once('1')
        .ok_or_else(|| "NIP-49 payload must start with 'ncryptsec1'.".to_string())?;

    if hrp != NIP49_HRP {
        return Err("NIP-49 payload must start with 'ncryptsec1'.".to_string());
    }

    if data_part.len() < 6 {
        return Err("NIP-49 payload is too short.".to_string());
    }

    let data = data_part
        .chars()
        .map(bech32_char_value)
        .collect::<Result<Vec<u8>, String>>()?;

    if !verify_bech32_checksum(hrp, &data) {
        return Err("NIP-49 bech32 payload encoding is invalid.".to_string());
    }

    let payload_five_bit = &data[..data.len() - 6];
    let payload = convert_bits(payload_five_bit, 5, 8, false)?;
    payload
        .first()
        .copied()
        .ok_or_else(|| "NIP-49 bech32 payload encoding is invalid.".to_string())
}

fn bech32_char_value(ch: char) -> Result<u8, String> {
    BECH32_CHARSET
        .chars()
        .position(|candidate| candidate == ch)
        .map(|value| value as u8)
        .ok_or_else(|| "NIP-49 bech32 payload encoding is invalid.".to_string())
}

fn verify_bech32_checksum(hrp: &str, data: &[u8]) -> bool {
    bech32_polymod(
        bech32_hrp_expand(hrp)
            .into_iter()
            .chain(data.iter().copied()),
    ) == 1
}

fn bech32_hrp_expand(hrp: &str) -> Vec<u8> {
    let mut expanded = Vec::with_capacity(hrp.len() * 2 + 1);
    expanded.extend(hrp.bytes().map(|byte| byte >> 5));
    expanded.push(0);
    expanded.extend(hrp.bytes().map(|byte| byte & 0x1f));
    expanded
}

fn bech32_polymod(values: impl IntoIterator<Item = u8>) -> u32 {
    const GENERATOR: [u32; 5] = [
        0x3b6a57b2,
        0x2650_8e6d,
        0x1ea1_19fa,
        0x3d42_33dd,
        0x2a14_62b3,
    ];

    let mut chk: u32 = 1;
    for value in values {
        let top = chk >> 25;
        chk = ((chk & 0x01ff_ffff) << 5) ^ (value as u32);

        for (index, generator) in GENERATOR.iter().enumerate() {
            if (top >> index) & 1 == 1 {
                chk ^= generator;
            }
        }
    }

    chk
}

fn convert_bits(data: &[u8], from: u32, to: u32, pad: bool) -> Result<Vec<u8>, String> {
    let mut acc: u32 = 0;
    let mut bits: u32 = 0;
    let mut ret = Vec::new();
    let maxv: u32 = (1 << to) - 1;
    let max_acc: u32 = (1 << (from + to - 1)) - 1;

    for value in data {
        let value_u32 = *value as u32;
        if value_u32 >> from != 0 {
            return Err("NIP-49 bech32 payload encoding is invalid.".to_string());
        }

        acc = ((acc << from) | value_u32) & max_acc;
        bits += from;

        while bits >= to {
            bits -= to;
            ret.push(((acc >> bits) & maxv) as u8);
        }
    }

    if pad {
        if bits > 0 {
            ret.push(((acc << (to - bits)) & maxv) as u8);
        }
    } else if bits >= from || ((acc << (to - bits)) & maxv) != 0 {
        return Err("NIP-49 bech32 payload encoding is invalid.".to_string());
    }

    Ok(ret)
}

/// Account selection/switching UI
/// Primary view for login - shows list of stored accounts
#[component]
pub fn AccountSelector(
    auth: AuthContext,
    #[prop(into)] on_switch: Callback<String>,
    #[prop(into)] on_delete: Callback<String>,
    #[prop(into)] on_add_account: Callback<()>,
) -> impl IntoView {
    let accounts = move || auth.accounts.get();

    let add_account_tab = RwSignal::new(AddAccountTab::ExistingMethods);

    let import_ncryptsec = RwSignal::new(String::new());
    let import_password = RwSignal::new(String::new());
    let import_profile_name = RwSignal::new(String::new());
    let import_error = RwSignal::new(None::<String>);
    let import_success = RwSignal::new(None::<String>);
    let import_in_flight = RwSignal::new(false);

    let ncryptsec_validation = Signal::derive(move || {
        let ncryptsec = import_ncryptsec.get();
        let ncryptsec = ncryptsec.trim().to_string();

        if ncryptsec.is_empty() {
            return None;
        }

        match extract_nip49_version(&ncryptsec) {
            Ok(NIP49_VERSION_V2) => None,
            Ok(version) => Some(format!(
                "Unsupported NIP-49 version 0x{version:02x}. Expected 0x02."
            )),
            Err(error) => Some(error),
        }
    });

    let password_validation = Signal::derive(move || {
        let password = import_password.get();

        if password.trim().is_empty() {
            return None;
        }

        validate_nip49_password(&password)
            .err()
            .map(|error| error.to_string())
    });

    let can_import_from_backup = Signal::derive(move || {
        !import_ncryptsec.get().trim().is_empty()
            && !import_password.get().trim().is_empty()
            && ncryptsec_validation.get().is_none()
            && password_validation.get().is_none()
            && !import_in_flight.get()
    });

    let on_import_from_backup = move |_| {
        import_error.set(None);
        import_success.set(None);

        if let Some(validation_error) = ncryptsec_validation.get() {
            import_error.set(Some(validation_error));
            return;
        }

        if let Some(validation_error) = password_validation.get() {
            import_error.set(Some(validation_error));
            return;
        }

        let ncryptsec = import_ncryptsec.get().trim().to_string();
        let password = import_password.get();
        let profile_name = import_profile_name.get().trim().to_string();

        if ncryptsec.is_empty() || password.trim().is_empty() {
            import_error.set(Some(
                "Both ncryptsec and password are required to import a backup.".to_string(),
            ));
            return;
        }

        import_in_flight.set(true);

        spawn_local(async move {
            let request = Nip49ImportRequest {
                ncryptsec,
                password,
            };

            #[cfg(not(feature = "web"))]
            let import_result = crate::tauri_invoke::invoke::<String>(
                "nip49_import",
                serde_json::json!({ "request": request }),
            )
            .await;

            #[cfg(feature = "web")]
            let import_result: Result<String, String> =
                Err("NIP-49 import is desktop-only and unavailable on web target.".to_string());

            match import_result {
                Ok(message) => {
                    let profile_note = if profile_name.is_empty() {
                        String::new()
                    } else {
                        format!(" Profile name: {}.", profile_name)
                    };

                    import_success.set(Some(format!(
                        "Backup import submitted. {}{}",
                        message, profile_note
                    )));
                    import_error.set(None);
                }
                Err(error) => {
                    import_error.set(Some(format!("Import failed: {error}")));
                    import_success.set(None);
                }
            }

            import_in_flight.set(false);
        });
    };

    view! {
        <div class="account-selector">
            // Removed duplicate header - parent component already has "Welcome Back"

            <div class="accounts-list">
                <For
                    each=accounts
                    key=|account| account.id.clone()
                    children=move |account: StoredAccount| {
                        // Clone values for use in closures
                        let account_id = account.id.clone();
                        let account_name = account.name.clone();
                        let account_npub = account.npub.clone();
                        let account_mode = account.signing_mode.clone();
                        let is_current = account.is_current;

                        // Profile data from StoredAccount (immediately available)
                        let stored_picture = account.picture.clone();
                        let stored_display_name = account.display_name.clone();
                        let stored_username = account.username.clone();

                        // Create reactive signal for profile - this will update when store changes
                        let profile_signal = use_profile(account_npub.clone());

                        view! {
                            <div
                                class={format!("account-card {}", if is_current { "active" } else { "" })}
                                on:click={
                                    let account_id = account_id.clone();
                                    move |_| {
                                        // Only switch if not currently active
                                        if !is_current {
                                            on_switch.run(account_id.clone());
                                        }
                                    }
                                }
                            >
                                <div class="account-avatar">
                                    {{
                                        let account_name_for_avatar = account_name.clone();
                                        let stored_pic = stored_picture.clone();
                                        let stored_disp_name = stored_display_name.clone();

                                        move || {
                                            // First try to get from reactive profile store (may have fresher data)
                                            match profile_signal.get() {
                                                Some(profile) => {
                                                    if let Some(picture) = profile.picture {
                                                        view! {
                                                            <img src={picture} class="account-avatar-img" alt="avatar" />
                                                        }.into_any()
                                                    } else {
                                                        let letter = profile.display().chars().next().unwrap_or('?');
                                                        view! {
                                                            <div class="avatar-placeholder">{letter}</div>
                                                        }.into_any()
                                                    }
                                                }
                                                None => {
                                                    // No profile in store, use stored data from StoredAccount
                                                    if let Some(picture) = stored_pic.clone() {
                                                        view! {
                                                            <img src={picture} class="account-avatar-img" alt="avatar" />
                                                        }.into_any()
                                                    } else {
                                                        // Fallback to first letter of display name or account name
                                                        let name_for_letter = stored_disp_name.clone()
                                                            .or_else(|| account_name_for_avatar.clone())
                                                            .unwrap_or_else(|| "?".to_string());
                                                        let letter = name_for_letter.chars().next().unwrap_or('?');
                                                        view! {
                                                            <div class="avatar-placeholder">{letter}</div>
                                                        }.into_any()
                                                    }
                                                }
                                            }
                                        }
                                    }}
                                </div>

                                <div class="account-info">
                                    <span class="account-name">
                                        {{
                                            let account_name_for_display = account_name.clone();
                                            let stored_disp_name = stored_display_name.clone();
                                            let stored_user = stored_username.clone();

                                            move || {
                                                // First try reactive profile store
                                                match profile_signal.get() {
                                                    Some(profile) => profile.display(),
                                                    None => {
                                                        // Use stored data: display_name > username > account name > npub
                                                        stored_disp_name.clone()
                                                            .or_else(|| stored_user.clone())
                                                            .or_else(|| account_name_for_display.clone())
                                                            .unwrap_or_else(|| "Unnamed Account".to_string())
                                                    }
                                                }
                                            }
                                        }}
                                    </span>
                                    <span class="account-npub">
                                        {format!("{}...{}", &account_npub[..8], &account_npub[account_npub.len()-8..])}
                                    </span>
                                    <span class="account-mode">
                                        {account_mode.clone()}
                                    </span>
                                </div>

                                <div class="account-actions">
                                    <Show
                                        when=move || is_current
                                        fallback={
                                            let account_id = account_id.clone();
                                            move || view! {
                                                <button
                                                    class="switch-btn"
                                                    on:click={
                                                        let account_id = account_id.clone();
                                                        move |e| {
                                                            e.stop_propagation(); // Prevent card click
                                                            on_switch.run(account_id.clone());
                                                        }
                                                    }
                                                >
                                                    "Connect"
                                                </button>
                                            }
                                        }
                                    >
                                        <span class="current-badge">"Current"</span>
                                    </Show>

                                    <button
                                        class="delete-btn"
                                        on:click={
                                            let account_id = account_id.clone();
                                            move |e| {
                                                e.stop_propagation(); // Prevent card click
                                                on_delete.run(account_id.clone());
                                            }
                                        }
                                        title="Delete account"
                                    >
                                        "×"
                                    </button>
                                </div>
                            </div>
                        }
                    }
                />
            </div>

            <div class="account-selector-footer">
                <div class="account-add-tabs" role="tablist" aria-label="Account import options">
                    <button
                        class="add-account-tab-btn"
                        class:active=move || add_account_tab.get() == AddAccountTab::ExistingMethods
                        on:click=move |_| add_account_tab.set(AddAccountTab::ExistingMethods)
                        role="tab"
                        aria-selected=move || add_account_tab.get() == AddAccountTab::ExistingMethods
                    >
                        "Add New Account"
                    </button>
                    <button
                        class="add-account-tab-btn"
                        class:active=move || add_account_tab.get() == AddAccountTab::ImportFromBackup
                        on:click=move |_| add_account_tab.set(AddAccountTab::ImportFromBackup)
                        role="tab"
                        aria-selected=move || add_account_tab.get() == AddAccountTab::ImportFromBackup
                    >
                        "Import from Backup"
                    </button>
                </div>

                <Show
                    when=move || add_account_tab.get() == AddAccountTab::ExistingMethods
                    fallback=move || {
                        view! {
                            <div class="nip49-import-tab">
                                <div class="input-group">
                                    <label for="nip49-import-ncryptsec">"ncryptsec"</label>
                                    <input
                                        id="nip49-import-ncryptsec"
                                        type="text"
                                        placeholder="ncryptsec1..."
                                        bind:value=import_ncryptsec
                                    />
                                    <Show when=move || ncryptsec_validation.get().is_some()>
                                        <p class="error-text">{move || ncryptsec_validation.get().unwrap_or_default()}</p>
                                    </Show>
                                </div>

                                <div class="input-group">
                                    <label for="nip49-import-password">"Password"</label>
                                    <input
                                        id="nip49-import-password"
                                        type="password"
                                        autocomplete="current-password"
                                        placeholder="Backup password"
                                        bind:value=import_password
                                    />
                                    <Show when=move || password_validation.get().is_some()>
                                        <p class="error-text">{move || password_validation.get().unwrap_or_default()}</p>
                                    </Show>
                                </div>

                                <div class="input-group">
                                    <label for="nip49-import-profile-name">"Profile Name (optional)"</label>
                                    <input
                                        id="nip49-import-profile-name"
                                        type="text"
                                        placeholder="My Gaming Account"
                                        bind:value=import_profile_name
                                    />
                                </div>

                                <Show when=move || import_error.get().is_some()>
                                    <p class="error-text">{move || import_error.get().unwrap_or_default()}</p>
                                </Show>

                                <Show when=move || import_success.get().is_some()>
                                    <p class="success-text">{move || import_success.get().unwrap_or_default()}</p>
                                </Show>

                                <button
                                    class="add-account-btn"
                                    on:click=on_import_from_backup
                                    disabled=move || !can_import_from_backup.get()
                                >
                                    {move || {
                                        if import_in_flight.get() {
                                            "Importing..."
                                        } else {
                                            "Import Backup"
                                        }
                                    }}
                                </button>
                            </div>
                        }
                    }
                >
                    <button class="add-account-btn" on:click=move |_| on_add_account.run(())>
                        "+ Add New Account"
                    </button>
                </Show>
            </div>
        </div>
    }
}
