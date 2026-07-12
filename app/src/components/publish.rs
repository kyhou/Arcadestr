// ADP publish view component.

use leptos::prelude::*;
use wasm_bindgen_futures::spawn_local;

use crate::tauri_bridge::{
    invoke_check_adp_server, invoke_publish_adp_listing, listen_publish_progress,
    PublishAdpListingRequest, PublishProgressPayload,
};
use crate::AuthContext;

/// Validates ADP listing fields before publishing.
fn validate_listing(
    id: &str,
    title: &str,
    description: &str,
    server_url: &str,
    file_path: &str,
    version: &str,
    price_sats: u64,
    lud16: &str,
) -> Result<(), String> {
    if id.is_empty() {
        return Err("Listing ID is required".to_string());
    }
    if id.len() > 64 {
        return Err("Listing ID must be 64 characters or less".to_string());
    }
    if !id
        .chars()
        .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
    {
        return Err(
            "Listing ID can only contain lowercase letters, numbers, and hyphens".to_string(),
        );
    }
    if title.is_empty() {
        return Err("Title is required".to_string());
    }
    if title.len() > 100 {
        return Err("Title must be 100 characters or less".to_string());
    }
    if description.is_empty() {
        return Err("Description is required".to_string());
    }
    if description.len() > 2000 {
        return Err("Description must be 2000 characters or less".to_string());
    }
    if server_url.is_empty() {
        return Err("ADP server URL is required".to_string());
    }
    if !(server_url.starts_with("http://") || server_url.starts_with("https://")) {
        return Err("ADP server URL must start with http:// or https://".to_string());
    }
    if file_path.is_empty() {
        return Err("Build file path is required".to_string());
    }
    if version.is_empty() {
        return Err("Version is required".to_string());
    }
    if price_sats > 0 && lud16.is_empty() {
        return Err("Lightning address is required for priced ADP listings".to_string());
    }
    if !lud16.is_empty() {
        let parts: Vec<&str> = lud16.split('@').collect();
        if parts.len() != 2 || parts[0].is_empty() || parts[1].is_empty() {
            return Err("Lightning address must look like name@example.com".to_string());
        }
    }
    Ok(())
}

fn parse_platform_tags(input: &str) -> Result<Vec<String>, String> {
    input
        .split(',')
        .map(str::trim)
        .filter(|tag| !tag.is_empty())
        .map(|tag| {
            if tag.chars().any(char::is_whitespace) {
                return Err("Platform tags cannot contain whitespace".to_string());
            }
            if !tag.contains('-') {
                return Err("Platform tags must look like <os>-<arch>".to_string());
            }
            Ok(tag.to_string())
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_platform_tags_trims_values_and_discards_empty_entries() {
        let platforms = parse_platform_tags(" linux-x86_64, ,windows-x86_64, macos-aarch64 ")
            .expect("valid platform tags should parse");

        assert_eq!(
            platforms,
            vec!["linux-x86_64", "windows-x86_64", "macos-aarch64"]
        );
    }

    #[test]
    fn parse_platform_tags_rejects_whitespace_inside_tag() {
        let err = parse_platform_tags("linux x86_64")
            .expect_err("platform tags with whitespace should be rejected");

        assert_eq!(err, "Platform tags cannot contain whitespace");
    }

    #[test]
    fn parse_platform_tags_rejects_tags_without_os_arch_separator() {
        let err =
            parse_platform_tags("linux").expect_err("platform tags without '-' should be rejected");

        assert_eq!(err, "Platform tags must look like <os>-<arch>");
    }
}

/// Publish view component - form for creating ADP listings.
#[component]
pub fn PublishView() -> impl IntoView {
    let auth = use_context::<AuthContext>().expect("AuthContext not provided");

    let id = RwSignal::new(String::new());
    let title = RwSignal::new(String::new());
    let description = RwSignal::new(String::new());
    let price_sats = RwSignal::new(0u64);
    let platforms_input = RwSignal::new(String::new());
    let lud16 = RwSignal::new(String::new());
    let server_url = RwSignal::new(String::new());
    let file_path = RwSignal::new(String::new());
    let version = RwSignal::new(String::new());

    let is_publishing = RwSignal::new(false);
    let is_checking_server = RwSignal::new(false);
    let server_ok = RwSignal::new(false);
    let success_message = RwSignal::new(None::<String>);
    let error_message = RwSignal::new(None::<String>);
    let progress_events = RwSignal::new(Vec::<PublishProgressPayload>::new());

    let on_check_server = move |_| {
        let url = server_url.get();
        is_checking_server.set(true);
        server_ok.set(false);
        error_message.set(None);
        spawn_local(async move {
            match invoke_check_adp_server(url).await {
                Ok(info) => {
                    server_ok.set(true);
                    success_message.set(Some(format!("ADP server reachable: {}", info.pubkey)));
                }
                Err(err) => error_message.set(Some(err)),
            }
            is_checking_server.set(false);
        });
    };

    let on_submit = move |_| {
        if auth.npub.get().is_none() {
            error_message.set(Some("Not authenticated".to_string()));
            return;
        }

        let id_val = id.get();
        let title_val = title.get();
        let description_val = description.get();
        let server_url_val = server_url.get();
        let file_path_val = file_path.get();
        let version_val = version.get();
        let lud16_val = lud16.get();
        let price_val = price_sats.get();

        if let Err(msg) = validate_listing(
            &id_val,
            &title_val,
            &description_val,
            &server_url_val,
            &file_path_val,
            &version_val,
            price_val,
            &lud16_val,
        ) {
            error_message.set(Some(msg));
            return;
        }
        if !server_ok.get() {
            error_message.set(Some("Check the ADP server before publishing".to_string()));
            return;
        }

        let platforms = match parse_platform_tags(&platforms_input.get()) {
            Ok(platforms) => platforms,
            Err(msg) => {
                error_message.set(Some(msg));
                return;
            }
        };

        let request = PublishAdpListingRequest {
            d_tag: id_val,
            title: title_val,
            description: description_val,
            price_sats: price_val,
            lud16: (!lud16_val.is_empty()).then_some(lud16_val),
            server_url: server_url_val,
            file_path: file_path_val,
            version: version_val,
            platforms,
        };

        is_publishing.set(true);
        success_message.set(None);
        error_message.set(None);
        progress_events.set(Vec::new());

        spawn_local(async move {
            let _listener = listen_publish_progress(move |payload| {
                progress_events.update(|events| events.push(payload));
            })
            .await
            .ok();

            match invoke_publish_adp_listing(request).await {
                Ok(result) => {
                    success_message.set(Some(format!(
                        "Published ADP listing {} ({})",
                        result.upload.game_coordinate, result.upload.download_url
                    )));
                    id.set(String::new());
                    title.set(String::new());
                    description.set(String::new());
                    price_sats.set(0);
                    platforms_input.set(String::new());
                    lud16.set(String::new());
                    file_path.set(String::new());
                    version.set(String::new());
                    server_ok.set(false);
                }
                Err(err) => error_message.set(Some(err)),
            }
            is_publishing.set(false);
        });
    };

    view! {
        <div class="publish-container">
            <h2 class="publish-title">"Publish ADP Game"</h2>

            <div class="publish-form">
                <div class="form-group">
                    <label class="form-label">"Listing ID / Slug"</label>
                    <input class="form-input" type="text" placeholder="my-game-v1"
                        prop:value={move || id.get()} on:input:target=move |ev| id.set(ev.target().value()) disabled={move || is_publishing.get()} />
                </div>

                <div class="form-group">
                    <label class="form-label">"Title"</label>
                    <input class="form-input" type="text" placeholder="My Awesome Game"
                        prop:value={move || title.get()} on:input:target=move |ev| title.set(ev.target().value()) disabled={move || is_publishing.get()} />
                </div>

                <div class="form-group">
                    <label class="form-label">"Description"</label>
                    <textarea class="form-textarea" rows=4 placeholder="Describe your game..."
                        prop:value={move || description.get()} on:input:target=move |ev| description.set(ev.target().value()) disabled={move || is_publishing.get()} />
                </div>

                <div class="form-group">
                    <label class="form-label">"Price (satoshis) — 0 for free"</label>
                    <input class="form-input" type="number" min=0 prop:value={move || price_sats.get().to_string()}
                        on:input:target=move |ev| {
                            if let Ok(val) = ev.target().value().parse::<u64>() {
                                price_sats.set(val);
                            }
                        }
                        disabled={move || is_publishing.get()} />
                </div>

                <div class="form-group">
                    <label class="form-label">"ADP Server URL"</label>
                    <input class="form-input" type="text" placeholder="http://localhost:9099"
                        prop:value={move || server_url.get()} on:input:target=move |ev| { server_url.set(ev.target().value()); server_ok.set(false); }
                        disabled={move || is_publishing.get()} />
                    <button class="secondary-button" on:click={on_check_server} disabled={move || is_publishing.get() || is_checking_server.get()}>
                        {move || if server_ok.get() { "Server OK" } else if is_checking_server.get() { "Checking..." } else { "Check Server" }}
                    </button>
                </div>

                <div class="form-group">
                    <label class="form-label">"Build File Path"</label>
                    <input class="form-input" type="text" placeholder="/path/to/game.zip"
                        prop:value={move || file_path.get()} on:input:target=move |ev| file_path.set(ev.target().value()) disabled={move || is_publishing.get()} />
                </div>

                <div class="form-group">
                    <label class="form-label">"Version"</label>
                    <input class="form-input" type="text" placeholder="1.0.0"
                        prop:value={move || version.get()} on:input:target=move |ev| version.set(ev.target().value()) disabled={move || is_publishing.get()} />
                </div>

                <div class="form-group">
                    <label class="form-label">"Platforms (comma-separated)"</label>
                    <input class="form-input" type="text" placeholder="linux-x86_64, windows-x86_64"
                        prop:value={move || platforms_input.get()} on:input:target=move |ev| platforms_input.set(ev.target().value()) disabled={move || is_publishing.get()} />
                </div>

                <div class="form-group">
                    <label class="form-label">"Lightning Address (lud16)"</label>
                    <input class="form-input" type="text" placeholder="you@example.com"
                        prop:value={move || lud16.get()} on:input:target=move |ev| lud16.set(ev.target().value()) disabled={move || is_publishing.get()} />
                </div>

                <div class="publish-progress">
                    <h3>"Publish progress"</h3>
                    <ul>
                        {move || progress_events.get().into_iter().map(|event| view! {
                            <li>{format!("{}: {}{}", event.step, event.status, event.message.map(|m| format!(" — {m}")).unwrap_or_default())}</li>
                        }).collect_view()}
                    </ul>
                </div>

                <button class="publish-button" on:click={on_submit} disabled={move || is_publishing.get()}>
                    {move || if is_publishing.get() { "Publishing..." } else { "Publish ADP Listing" }}
                </button>

                {move || success_message.get().map(|msg| view! { <div class="success-message">{msg}</div> })}
                {move || error_message.get().map(|msg| view! { <div class="error-message">{msg}</div> })}
            </div>
        </div>
    }
}
