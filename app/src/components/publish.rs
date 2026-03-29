// Publish view component

use leptos::prelude::*;
use wasm_bindgen_futures::spawn_local;

use crate::models::GameListing;
use crate::{invoke_publish_listing, AuthContext};

/// Validates listing fields before publishing.
/// Returns Ok(()) if all fields are valid, or Err with a user-friendly message.
fn validate_listing(
    id: &str,
    title: &str,
    description: &str,
    download_url: &str,
    lud16: &str,
) -> Result<(), String> {
    // Validate id: non-empty, only lowercase alphanumeric and hyphens, max 64 chars
    if id.is_empty() {
        return Err("Listing ID is required".to_string());
    }
    if id.len() > 64 {
        return Err("Listing ID must be 64 characters or less".to_string());
    }
    if !id.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-') {
        return Err("Listing ID can only contain lowercase letters, numbers, and hyphens".to_string());
    }

    // Validate title: non-empty, max 100 chars
    if title.is_empty() {
        return Err("Title is required".to_string());
    }
    if title.len() > 100 {
        return Err("Title must be 100 characters or less".to_string());
    }

    // Validate description: non-empty, max 2000 chars
    if description.is_empty() {
        return Err("Description is required".to_string());
    }
    if description.len() > 2000 {
        return Err("Description must be 2000 characters or less".to_string());
    }

    // Validate download_url: non-empty, must start with "https://"
    if download_url.is_empty() {
        return Err("Download URL is required".to_string());
    }
    if !download_url.starts_with("https://") {
        return Err("Download URL must start with https://".to_string());
    }

    // Validate lud16: if non-empty, must contain exactly one "@" with non-empty parts
    if !lud16.is_empty() {
        let parts: Vec<&str> = lud16.split('@').collect();
        if parts.len() != 2 {
            return Err("Lightning address must contain exactly one @ symbol".to_string());
        }
        if parts[0].is_empty() {
            return Err("Lightning address username cannot be empty".to_string());
        }
        if parts[1].is_empty() {
            return Err("Lightning address domain cannot be empty".to_string());
        }
    }

    Ok(())
}

/// Publish view component - form for creating new listings.
#[component]
pub fn PublishView() -> impl IntoView {
    let auth = use_context::<AuthContext>().expect("AuthContext not provided");

    // Form field signals
    let id = RwSignal::new(String::new());
    let title = RwSignal::new(String::new());
    let description = RwSignal::new(String::new());
    let price_sats = RwSignal::new(0u64);
    let download_url = RwSignal::new(String::new());
    let tags_input = RwSignal::new(String::new());
    let lud16 = RwSignal::new(String::new());

    // Status signals
    let is_publishing = RwSignal::new(false);
    let success_message = RwSignal::new(None::<String>);
    let error_message = RwSignal::new(None::<String>);

    let on_submit = move |_| {
        // Get current npub
        let npub = match auth.npub.get() {
            Some(n) => n,
            None => {
                error_message.set(Some("Not authenticated".to_string()));
                return;
            }
        };

        // Get field values for validation
        let id_val = id.get();
        let title_val = title.get();
        let description_val = description.get();
        let download_url_val = download_url.get();
        let lud16_val = lud16.get();

        // Validate fields before proceeding
        if let Err(msg) = validate_listing(&id_val, &title_val, &description_val, &download_url_val, &lud16_val) {
            error_message.set(Some(msg));
            return;
        }

        // Parse tags
        let tags: Vec<String> = tags_input
            .get()
            .split(',')
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .collect();

        // Get current timestamp
        #[cfg(target_arch = "wasm32")]
        let created_at = (js_sys::Date::now() as u64) / 1000;
        #[cfg(not(target_arch = "wasm32"))]
        let created_at = 0u64; // Fallback for native

        let listing = GameListing {
            id: id_val,
            title: title_val,
            description: description_val,
            price_sats: price_sats.get(),
            download_url: download_url_val,
            publisher_npub: npub,
            created_at,
            tags,
            event_id: None, // Will be set by the backend after publishing
            lud16: lud16_val,
        };

        is_publishing.set(true);
        success_message.set(None);
        error_message.set(None);

        spawn_local(async move {
            match invoke_publish_listing(listing).await {
                Ok(event_id) => {
                    success_message.set(Some(format!("✅ Published! Event ID: {}", event_id)));
                    // Clear form
                    id.set(String::new());
                    title.set(String::new());
                    description.set(String::new());
                    price_sats.set(0);
                    download_url.set(String::new());
                    tags_input.set(String::new());
                    lud16.set(String::new());
                    is_publishing.set(false);
                }
                Err(e) => {
                    error_message.set(Some(e));
                    is_publishing.set(false);
                }
            }
        });
    };

    view! {
        <div class="publish-container">
            <h2 class="publish-title">"Publish Game"</h2>

            <div class="publish-form">
                <div class="form-group">
                    <label class="form-label">"Listing ID / Slug"</label>
                    <input
                        class="form-input"
                        type="text"
                        placeholder="my-game-v1"
                        prop:value={move || id.get()}
                        on:input:target=move |ev| id.set(ev.target().value())
                        disabled={move || is_publishing.get()}
                    />
                </div>

                <div class="form-group">
                    <label class="form-label">"Title"</label>
                    <input
                        class="form-input"
                        type="text"
                        placeholder="My Awesome Game"
                        prop:value={move || title.get()}
                        on:input:target=move |ev| title.set(ev.target().value())
                        disabled={move || is_publishing.get()}
                    />
                </div>

                <div class="form-group">
                    <label class="form-label">"Description"</label>
                    <textarea
                        class="form-textarea"
                        rows=4
                        placeholder="Describe your game..."
                        prop:value={move || description.get()}
                        on:input:target=move |ev| description.set(ev.target().value())
                        disabled={move || is_publishing.get()}
                    />
                </div>

                <div class="form-group">
                    <label class="form-label">"Price (satoshis) — 0 for free"</label>
                    <input
                        class="form-input"
                        type="number"
                        min=0
                        prop:value={move || price_sats.get().to_string()}
                        on:input:target=move |ev| {
                            if let Ok(val) = ev.target().value().parse::<u64>() {
                                price_sats.set(val);
                            }
                        }
                        disabled={move || is_publishing.get()}
                    />
                </div>

                <div class="form-group">
                    <label class="form-label">"Download URL"</label>
                    <input
                        class="form-input"
                        type="text"
                        placeholder="https://..."
                        prop:value={move || download_url.get()}
                        on:input:target=move |ev| download_url.set(ev.target().value())
                        disabled={move || is_publishing.get()}
                    />
                </div>

                <div class="form-group">
                    <label class="form-label">"Tags (comma-separated)"</label>
                    <input
                        class="form-input"
                        type="text"
                        placeholder="rpg, pixel-art"
                        prop:value={move || tags_input.get()}
                        on:input:target=move |ev| tags_input.set(ev.target().value())
                        disabled={move || is_publishing.get()}
                    />
                </div>

                <div class="form-group">
                    <label class="form-label">"Lightning Address (lud16) — for receiving payments"</label>
                    <input
                        class="form-input"
                        type="text"
                        placeholder="you@walletofsatoshi.com"
                        prop:value={move || lud16.get()}
                        on:input:target=move |ev| lud16.set(ev.target().value())
                        disabled={move || is_publishing.get()}
                    />
                </div>

                <button
                    class="publish-button"
                    on:click={on_submit}
                    disabled={move || is_publishing.get()}
                >
                    {move || if is_publishing.get() {
                        "Publishing...".to_string()
                    } else {
                        "Publish Listing".to_string()
                    }}
                </button>

                {move || {
                    success_message.get().map(|msg| view! {
                            <div class="success-message">{msg}</div>
                        })
                }}

                {move || {
                    error_message.get().map(|err| view! {
                            <div class="error-message">{err}</div>
                        })
                }}
            </div>
        </div>
    }
}
