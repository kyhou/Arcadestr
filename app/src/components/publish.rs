// ADP publish view component.

use leptos::prelude::*;
use nostr::nips::nip19::FromBech32;
use wasm_bindgen_futures::spawn_local;

use arcadestr_core::is_sha256_hex;

use crate::campaign_management::datetime_local_to_unix;
use crate::models::AcquisitionPolicy;
use crate::tauri_bridge::{
    invoke_check_adp_server, invoke_discover_adp_servers, invoke_hash_build_file,
    invoke_publish_adp_listing, invoke_resolve_adp_operator, invoke_select_build_file,
    listen_publish_progress, AdpServerAnnouncement, FulfillmentMode, HashBuildFileRequest,
    PublishAdpListingRequest, PublishProgressPayload, ResolveAdpOperatorRequest,
};
use crate::{AuthContext, GameListing};

#[derive(Debug, Clone, PartialEq, Eq)]
struct ServerEntry {
    url: String,
    label: String,
    reachability: ServerStatus,
    upload: ServerStatus,
    auto_operator: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ServerStatus {
    Idle,
    Pending,
    Ok,
    Failed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum AcquisitionKind {
    Gated,
    Public,
    TimedAccess,
}

impl AcquisitionKind {
    fn value(self) -> &'static str {
        match self {
            Self::Gated => "gated",
            Self::Public => "public",
            Self::TimedAccess => "timed-access",
        }
    }

    fn from_value(value: &str) -> Self {
        match value {
            "public" => Self::Public,
            "timed-access" => Self::TimedAccess,
            _ => Self::Gated,
        }
    }
}

fn acquisition_kind(policy: &AcquisitionPolicy) -> AcquisitionKind {
    match policy {
        AcquisitionPolicy::Gated => AcquisitionKind::Gated,
        AcquisitionPolicy::Public => AcquisitionKind::Public,
        AcquisitionPolicy::TimedAccess { .. } => AcquisitionKind::TimedAccess,
    }
}

fn acquisition_policy_from_form(
    kind: AcquisitionKind,
    starts_at: &str,
    ends_at: &str,
) -> Result<AcquisitionPolicy, String> {
    match kind {
        AcquisitionKind::Gated => Ok(AcquisitionPolicy::Gated),
        AcquisitionKind::Public => Ok(AcquisitionPolicy::Public),
        AcquisitionKind::TimedAccess => {
            if starts_at.trim().is_empty() {
                return Err("Choose when timed access starts".to_string());
            }
            if ends_at.trim().is_empty() {
                return Err("Choose when timed access ends".to_string());
            }
            let starts_at = datetime_local_to_unix(starts_at)
                .ok_or_else(|| "Timed access start is invalid".to_string())?;
            let ends_at = datetime_local_to_unix(ends_at)
                .ok_or_else(|| "Timed access end is invalid".to_string())?;
            if starts_at >= ends_at {
                return Err("Timed access must end after it starts".to_string());
            }
            Ok(AcquisitionPolicy::TimedAccess { starts_at, ends_at })
        }
    }
}

fn datetime_local_value(value: u64) -> String {
    let date = js_sys::Date::new(&(value as f64 * 1000.0).into());
    format!(
        "{:04}-{:02}-{:02}T{:02}:{:02}:{:02}",
        date.get_full_year(),
        date.get_month() + 1,
        date.get_date(),
        date.get_hours(),
        date.get_minutes(),
        date.get_seconds()
    )
}

impl ServerStatus {
    fn label(self) -> &'static str {
        match self {
            ServerStatus::Idle => "not checked",
            ServerStatus::Pending => "pending",
            ServerStatus::Ok => "ok",
            ServerStatus::Failed => "failed",
        }
    }

    fn class(self) -> &'static str {
        match self {
            ServerStatus::Idle => "text-on-surface-variant",
            ServerStatus::Pending => "text-secondary",
            ServerStatus::Ok => "text-secondary",
            ServerStatus::Failed => "text-error",
        }
    }
}

fn validate_listing(
    id: &str,
    title: &str,
    description: &str,
    price_sats: u64,
    lud16: &str,
    fulfillment_enabled: bool,
    servers: &[ServerEntry],
    _file_path: &Option<String>,
    file_hash: &Option<String>,
    version: &str,
    fulfillment_mode: &FulfillmentMode,
    operator_url: &str,
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
    if price_sats > 0 && lud16.is_empty() {
        return Err("Lightning address (lud16) is required for priced listings".to_string());
    }
    if !lud16.is_empty() {
        let parts: Vec<&str> = lud16.split('@').collect();
        if parts.len() != 2 || parts[0].is_empty() || parts[1].is_empty() {
            return Err("Lightning address must look like name@example.com".to_string());
        }
    }
    if fulfillment_enabled {
        if servers.is_empty() {
            return Err("Add at least one distribution server for fulfillment".to_string());
        }
        if let Some(bad_url) = servers
            .iter()
            .map(|server| server.url.as_str())
            .find(|url| !(url.starts_with("http://") || url.starts_with("https://")))
        {
            return Err(format!(
                "Server URL must start with http:// or https://: {bad_url}"
            ));
        }
        match file_hash.as_deref() {
            None => {
                return Err(
                    "Select a build file and wait for its hash before publishing fulfillment"
                        .to_string(),
                );
            }
            Some(hash) if !is_sha256_hex(hash) => {
                return Err(
                    "Existing SHA-256 metadata is invalid; select a replacement build file"
                        .to_string(),
                );
            }
            Some(_) => {}
        }
        if version.trim().is_empty() {
            return Err("Version is required for fulfillment".to_string());
        }
        match fulfillment_mode {
            FulfillmentMode::None => {
                return Err("Choose a fulfillment signing mode".to_string());
            }
            FulfillmentMode::Direct => {}
            FulfillmentMode::Delegate => {
                if operator_url.trim().is_empty() {
                    return Err("Operator URL is required for delegated fulfillment".to_string());
                }
                if !(operator_url.starts_with("http://") || operator_url.starts_with("https://")) {
                    return Err("Operator URL must start with http:// or https://".to_string());
                }
            }
        }
    }
    Ok(())
}

fn parse_csv_values(input: &str) -> Vec<String> {
    input
        .split(',')
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToString::to_string)
        .collect()
}

fn format_sha256(hash: &str) -> String {
    if !is_sha256_hex(hash) {
        return "Invalid SHA-256 metadata".to_string();
    }

    format!("{}...{}", &hash[..12], &hash[52..])
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

fn listing_spec(listing: &GameListing, key: &str) -> Option<String> {
    listing
        .specs
        .iter()
        .find(|(name, _)| name == key)
        .map(|(_, value)| value.clone())
}

fn listing_servers(listing: &GameListing) -> Vec<String> {
    listing
        .specs
        .iter()
        .filter(|(name, value)| name == "server" && !value.is_empty())
        .map(|(_, value)| value.clone())
        .collect()
}

fn listing_fulfillment_mode(listing: &GameListing) -> FulfillmentMode {
    let Some(fulfillment_pubkey) = listing_spec(listing, "fulfillment_pubkey") else {
        return FulfillmentMode::None;
    };
    match nostr::PublicKey::from_bech32(&listing.publisher_npub) {
        Ok(publisher) if publisher.to_hex() == fulfillment_pubkey => FulfillmentMode::Direct,
        Ok(_) => FulfillmentMode::Delegate,
        Err(_) => FulfillmentMode::None,
    }
}

fn listing_fulfillment_metadata(
    listing: &GameListing,
) -> (Option<String>, Option<u64>, Option<u64>) {
    (
        listing_spec(listing, "fulfillment_pubkey"),
        listing_spec(listing, "fulfillment_valid_from").and_then(|value| value.parse().ok()),
        listing_spec(listing, "fulfillment_revoked_at").and_then(|value| value.parse().ok()),
    )
}

fn initial_operator_url() -> String {
    String::new()
}

fn operator_resolution_request(listing: &GameListing) -> Option<ResolveAdpOperatorRequest> {
    if !matches!(listing_fulfillment_mode(listing), FulfillmentMode::Delegate) {
        return None;
    }
    let fulfillment_pubkey = listing_spec(listing, "fulfillment_pubkey")?;
    Some(ResolveAdpOperatorRequest {
        publisher_npub: listing.publisher_npub.clone(),
        fulfillment_pubkey,
        scope: listing.id.clone(),
    })
}

fn operator_prefill_update(
    current_operator_url: &str,
    resolution: Result<Option<String>, String>,
) -> Option<String> {
    if !current_operator_url.is_empty() {
        return None;
    }
    match resolution {
        Ok(Some(url)) => Some(url),
        Ok(None) | Err(_) => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const VALID_SHA256: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";

    #[test]
    fn format_sha256_abbreviates_valid_ascii_hex() {
        let hash = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";

        assert_eq!(format_sha256(hash), "0123456789ab...456789abcdef");
    }

    #[test]
    fn format_sha256_rejects_malformed_metadata() {
        for hash in [
            "",
            "abc123",
            "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcde",
            "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef0",
            "g123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef",
            "é123456789abcdef0123456789abcdef0123456789abcdef0123456789abcde",
        ] {
            assert_eq!(format_sha256(hash), "Invalid SHA-256 metadata");
        }
    }

    fn managed_listing(publisher_npub: String, fulfillment_pubkey: String) -> GameListing {
        GameListing {
            id: "managed-game".into(),
            source: crate::models::ListingSource::Nip99Listing,
            title: "Managed Game".into(),
            description: "Description".into(),
            images: Vec::new(),
            download_url: String::new(),
            price: 0.0,
            currency: "SATS".into(),
            price_sats: 0,
            quantity: None,
            tags: Vec::new(),
            specs: vec![
                ("server".into(), "http://localhost:9099".into()),
                ("version".into(), "1.4.2".into()),
                ("file_hash".into(), VALID_SHA256.into()),
                ("fulfillment_pubkey".into(), fulfillment_pubkey),
                ("fulfillment_valid_from".into(), "1710000000".into()),
                ("fulfillment_revoked_at".into(), "1710000999".into()),
            ],
            publisher_npub,
            stall_id: String::new(),
            stall_name: None,
            lud16: String::new(),
            event_id: Some("event-id".into()),
            created_at: 1,
            platforms: Vec::new(),
            nip94_event_id: None,
            acquisition: crate::models::AcquisitionPolicy::Gated,
            campaigns: Vec::new(),
            is_owned: false,
            #[cfg(debug_assertions)]
            nip99_raw_event_json: None,
        }
    }

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

    #[test]
    fn existing_fulfillment_hash_does_not_require_reselecting_build_file() {
        let result = validate_listing(
            "managed-game",
            "Managed Game",
            "Description",
            0,
            "",
            true,
            &[ServerEntry {
                url: "http://localhost:9099".into(),
                label: "Published server".into(),
                reachability: ServerStatus::Ok,
                upload: ServerStatus::Idle,
                auto_operator: false,
            }],
            &None,
            &Some(VALID_SHA256.into()),
            "1.4.2",
            &FulfillmentMode::Delegate,
            "http://localhost:9099",
        );

        assert_eq!(result, Ok(()));
    }

    #[test]
    fn malformed_existing_fulfillment_hash_requires_replacement_file() {
        let result = validate_listing(
            "managed-game",
            "Managed Game",
            "Description",
            0,
            "",
            true,
            &[ServerEntry {
                url: "http://localhost:9099".into(),
                label: "Published server".into(),
                reachability: ServerStatus::Ok,
                upload: ServerStatus::Idle,
                auto_operator: false,
            }],
            &None,
            &Some("abc123".into()),
            "1.4.2",
            &FulfillmentMode::Delegate,
            "http://localhost:9099",
        );

        assert_eq!(
            result,
            Err("Existing SHA-256 metadata is invalid; select a replacement build file".into())
        );
    }

    #[test]
    fn delegated_publication_defaults_are_recovered_from_listing_specs() {
        use nostr::nips::nip19::ToBech32;

        let publisher = nostr::Keys::generate();
        let delegate = nostr::Keys::generate();
        let listing = managed_listing(
            publisher
                .public_key()
                .to_bech32()
                .expect("publisher npub should encode"),
            delegate.public_key().to_hex(),
        );

        assert_eq!(listing_servers(&listing), vec!["http://localhost:9099"]);
        assert_eq!(listing_spec(&listing, "version").as_deref(), Some("1.4.2"));
        assert_eq!(
            listing_spec(&listing, "file_hash").as_deref(),
            Some(VALID_SHA256)
        );
        assert_eq!(
            listing_fulfillment_mode(&listing),
            FulfillmentMode::Delegate
        );
        assert_eq!(
            listing_fulfillment_metadata(&listing),
            (
                Some(delegate.public_key().to_hex()),
                Some(1_710_000_000),
                Some(1_710_000_999),
            )
        );
        assert_eq!(initial_operator_url(), "");

        let request = operator_resolution_request(&listing)
            .expect("delegated edit should request an exact local operator lookup");
        assert_eq!(request.publisher_npub, listing.publisher_npub);
        assert_eq!(request.fulfillment_pubkey, delegate.public_key().to_hex());
        assert_eq!(request.scope, "managed-game");
    }

    #[test]
    fn operator_prefill_only_applies_unique_success_to_empty_input() {
        assert_eq!(
            operator_prefill_update("", Ok(Some("https://operator.example.com".to_string())))
                .as_deref(),
            Some("https://operator.example.com")
        );
        assert_eq!(
            operator_prefill_update(
                "https://manual.example.com",
                Ok(Some("https://operator.example.com".to_string()))
            ),
            None
        );
        assert_eq!(operator_prefill_update("", Ok(None)), None);
        assert_eq!(
            operator_prefill_update("", Err("lookup failed".to_string())),
            None
        );
    }

    #[test]
    fn acquisition_kind_recovers_existing_policy() {
        assert_eq!(
            acquisition_kind(&AcquisitionPolicy::TimedAccess {
                starts_at: 100,
                ends_at: 200,
            }),
            AcquisitionKind::TimedAccess
        );
        assert_eq!(
            acquisition_kind(&AcquisitionPolicy::Public),
            AcquisitionKind::Public
        );
        assert_eq!(
            acquisition_kind(&AcquisitionPolicy::Gated),
            AcquisitionKind::Gated
        );
    }

    #[test]
    fn timed_acquisition_form_requires_ordered_dates() {
        let policy = acquisition_policy_from_form(
            AcquisitionKind::TimedAccess,
            "2026-07-18T12:30",
            "2026-07-18T13:30",
        )
        .expect("ordered dates should be accepted");
        assert!(matches!(
            policy,
            AcquisitionPolicy::TimedAccess { starts_at, ends_at } if starts_at < ends_at
        ));

        let error = acquisition_policy_from_form(
            AcquisitionKind::TimedAccess,
            "2026-07-18T13:30",
            "2026-07-18T12:30",
        )
        .expect_err("reversed dates should be rejected");
        assert_eq!(error, "Timed access must end after it starts");
    }
}

/// Publish view component - form for creating NIP-99 listings with optional ADP fulfillment.
#[component]
pub fn PublishView(#[prop(optional)] listing: Option<GameListing>) -> impl IntoView {
    let auth = use_context::<AuthContext>().expect("AuthContext not provided");
    let editing = listing.is_some();
    let published_servers = listing.as_ref().map(listing_servers).unwrap_or_default();
    let published_fulfillment_mode = listing
        .as_ref()
        .map(listing_fulfillment_mode)
        .unwrap_or(FulfillmentMode::None);
    let existing_file_hash = listing
        .as_ref()
        .and_then(|item| listing_spec(item, "file_hash"));
    let (
        existing_fulfillment_pubkey,
        existing_fulfillment_valid_from,
        existing_fulfillment_revoked_at,
    ) = listing
        .as_ref()
        .map(listing_fulfillment_metadata)
        .unwrap_or((None, None, None));
    let operator_resolution = listing.as_ref().and_then(operator_resolution_request);
    let initial_acquisition = listing
        .as_ref()
        .map(|item| item.acquisition.clone())
        .unwrap_or_default();

    let id = RwSignal::new(
        listing
            .as_ref()
            .map(|item| item.id.clone())
            .unwrap_or_default(),
    );
    let title = RwSignal::new(
        listing
            .as_ref()
            .map(|item| item.title.clone())
            .unwrap_or_default(),
    );
    let description = RwSignal::new(
        listing
            .as_ref()
            .map(|item| item.description.clone())
            .unwrap_or_default(),
    );
    let image_input = RwSignal::new(
        listing
            .as_ref()
            .map(|item| item.images.join(", "))
            .unwrap_or_default(),
    );
    let tag_input = RwSignal::new(
        listing
            .as_ref()
            .map(|item| item.tags.join(", "))
            .unwrap_or_default(),
    );
    let price_sats = RwSignal::new(
        listing
            .as_ref()
            .map(|item| item.price_sats)
            .unwrap_or_default(),
    );
    let platforms_input = RwSignal::new(
        listing
            .as_ref()
            .map(|item| item.platforms.join(", "))
            .unwrap_or_default(),
    );
    let lud16 = RwSignal::new(
        listing
            .as_ref()
            .map(|item| item.lud16.clone())
            .unwrap_or_default(),
    );
    let acquisition_kind = RwSignal::new(acquisition_kind(&initial_acquisition));
    let acquisition_starts_at = RwSignal::new(match &initial_acquisition {
        AcquisitionPolicy::TimedAccess { starts_at, .. } => datetime_local_value(*starts_at),
        _ => String::new(),
    });
    let acquisition_ends_at = RwSignal::new(match &initial_acquisition {
        AcquisitionPolicy::TimedAccess { ends_at, .. } => datetime_local_value(*ends_at),
        _ => String::new(),
    });
    let fulfillment_enabled =
        RwSignal::new(!matches!(published_fulfillment_mode, FulfillmentMode::None));
    let fulfillment_mode = RwSignal::new(published_fulfillment_mode);
    let discovered_servers = RwSignal::new(Vec::<AdpServerAnnouncement>::new());
    let discovery_error = RwSignal::new(None::<String>);
    let servers = RwSignal::new(
        published_servers
            .iter()
            .map(|url| ServerEntry {
                url: url.clone(),
                label: "Published server".into(),
                reachability: ServerStatus::Pending,
                upload: ServerStatus::Idle,
                auto_operator: false,
            })
            .collect::<Vec<_>>(),
    );
    let custom_server = RwSignal::new(String::new());
    let operator_url = RwSignal::new(initial_operator_url());
    let operator_auto_added = RwSignal::new(None::<String>);
    let file_path = RwSignal::new(None::<String>);
    let file_hash = RwSignal::new(existing_file_hash);
    let version = RwSignal::new(
        listing
            .as_ref()
            .and_then(|item| listing_spec(item, "version"))
            .unwrap_or_default(),
    );

    let is_publishing = RwSignal::new(false);
    let is_hashing = RwSignal::new(false);
    let success_message = RwSignal::new(None::<String>);
    let error_message = RwSignal::new(None::<String>);
    let progress_events = RwSignal::new(Vec::<PublishProgressPayload>::new());

    let add_server = move |url: String, label: String, auto_operator: bool| {
        let trimmed = url.trim().to_string();
        if trimmed.is_empty() {
            return;
        }
        servers.update(|entries| {
            if entries.iter().any(|entry| entry.url == trimmed) {
                return;
            }
            entries.push(ServerEntry {
                url: trimmed.clone(),
                label,
                reachability: ServerStatus::Pending,
                upload: ServerStatus::Idle,
                auto_operator,
            });
        });
        let servers_for_check = servers;
        spawn_local(async move {
            let status = match invoke_check_adp_server(trimmed.clone()).await {
                Ok(_) => ServerStatus::Ok,
                Err(_) => ServerStatus::Failed,
            };
            servers_for_check.update(|entries| {
                if let Some(entry) = entries.iter_mut().find(|entry| entry.url == trimmed) {
                    entry.reachability = status;
                }
            });
        });
    };

    let remove_server = move |url: String| {
        servers.update(|entries| entries.retain(|entry| entry.url != url));
    };

    let sync_operator_server = move |new_url: String| {
        if let Some(old_url) = operator_auto_added.get_untracked() {
            servers.update(|entries| {
                entries.retain(|entry| !(entry.auto_operator && entry.url == old_url))
            });
        }
        if new_url.trim().is_empty() {
            operator_auto_added.set(None);
            return;
        }
        operator_auto_added.set(Some(new_url.clone()));
        add_server(new_url, "Operator server".to_string(), true);
    };

    Effect::new(move |_| {
        spawn_local(async move {
            match invoke_discover_adp_servers().await {
                Ok(found) => {
                    discovered_servers.set(found);
                    discovery_error.set(None);
                }
                Err(err) => {
                    discovery_error.set(Some(format!("Couldn't reach relays for discovery: {err}")))
                }
            }
        });
    });

    if let Some(request) = operator_resolution {
        Effect::new(move |_| {
            let request = request.clone();
            spawn_local(async move {
                let resolution = invoke_resolve_adp_operator(request).await;
                if let Some(resolved) =
                    operator_prefill_update(&operator_url.get_untracked(), resolution)
                {
                    operator_url.set(resolved);
                }
            });
        });
    }

    let on_add_custom_server = move |_| {
        let url = custom_server.get();
        add_server(url.clone(), url.clone(), false);
        custom_server.set(String::new());
    };

    let on_select_file = move |_| {
        if is_hashing.get_untracked() || is_publishing.get_untracked() {
            return;
        }
        is_hashing.set(true);
        error_message.set(None);
        spawn_local(async move {
            match invoke_select_build_file().await {
                Ok(Some(path)) => {
                    file_path.set(Some(path.clone()));
                    file_hash.set(None);
                    match invoke_hash_build_file(HashBuildFileRequest { file_path: path }).await {
                        Ok(hash) => file_hash.set(Some(hash)),
                        Err(err) => error_message.set(Some(err)),
                    }
                }
                Ok(None) => {}
                Err(err) => error_message.set(Some(err)),
            }
            is_hashing.set(false);
        });
    };

    let on_submit = move |_| {
        if is_hashing.get_untracked() || is_publishing.get_untracked() {
            return;
        }
        if auth.npub.get().is_none() {
            error_message.set(Some("Not authenticated".to_string()));
            return;
        }

        let id_val = id.get();
        let title_val = title.get();
        let description_val = description.get();
        let lud16_val = lud16.get();
        let price_val = price_sats.get();
        let servers_val = servers.get();
        let file_path_val = file_path.get();
        let file_hash_val = file_hash.get();
        let version_val = version.get();
        let fulfillment_enabled_val = fulfillment_enabled.get();
        let fulfillment_mode_val = if fulfillment_enabled_val {
            fulfillment_mode.get()
        } else {
            FulfillmentMode::None
        };
        let operator_url_val = operator_url.get();
        let acquisition = match acquisition_policy_from_form(
            acquisition_kind.get(),
            &acquisition_starts_at.get(),
            &acquisition_ends_at.get(),
        ) {
            Ok(policy) => policy,
            Err(msg) => {
                error_message.set(Some(msg));
                return;
            }
        };

        if let Err(msg) = validate_listing(
            &id_val,
            &title_val,
            &description_val,
            price_val,
            &lud16_val,
            fulfillment_enabled_val,
            &servers_val,
            &file_path_val,
            &file_hash_val,
            &version_val,
            &fulfillment_mode_val,
            &operator_url_val,
        ) {
            error_message.set(Some(msg));
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
            tags: parse_csv_values(&tag_input.get()),
            images: parse_csv_values(&image_input.get()),
            fulfillment_mode: fulfillment_mode_val,
            operator_url: (!operator_url_val.trim().is_empty()).then_some(operator_url_val),
            servers: servers_val.into_iter().map(|entry| entry.url).collect(),
            file_path: file_path_val,
            existing_file_hash: file_hash_val,
            existing_fulfillment_pubkey: existing_fulfillment_pubkey.clone(),
            existing_fulfillment_valid_from,
            existing_fulfillment_revoked_at,
            version: (!version_val.trim().is_empty()).then_some(version_val),
            acquisition,
            platforms,
        };

        is_publishing.set(true);
        success_message.set(None);
        error_message.set(None);
        progress_events.set(Vec::new());
        servers.update(|entries| {
            for entry in entries {
                entry.upload = ServerStatus::Idle;
            }
        });

        spawn_local(async move {
            let listener_cleanup = listen_publish_progress(move |payload| {
                if payload.step == "upload" {
                    if let Some(server_url) = payload.server_url.clone() {
                        let status = match payload.status.as_str() {
                            "pending" => ServerStatus::Pending,
                            "ok" => ServerStatus::Ok,
                            "error" => ServerStatus::Failed,
                            _ => ServerStatus::Idle,
                        };
                        servers.update(|entries| {
                            if let Some(entry) =
                                entries.iter_mut().find(|entry| entry.url == server_url)
                            {
                                entry.upload = status;
                            }
                        });
                    }
                }
                progress_events.update(|events| events.push(payload));
            })
            .await
            .ok();

            let publish_result = invoke_publish_adp_listing(request).await;
            if let Some(cleanup) = listener_cleanup {
                cleanup();
            }
            match publish_result {
                Ok(result) => {
                    success_message.set(Some(format!("Published listing {}", result.event_id)));
                }
                Err(err) => error_message.set(Some(err)),
            }
            is_publishing.set(false);
        });
    };

    view! {
        <div class="max-w-6xl mx-auto px-8 py-10">
            <header class="mb-10 flex flex-col gap-4 lg:flex-row lg:items-end lg:justify-between">
                <div>
                    <h1 class="text-5xl font-extrabold font-headline tracking-tighter mb-2">{if editing { "Edit " } else { "Publish " }}<span class="text-primary italic">{if editing { "Publication" } else { "New Game" }}</span></h1>
                    <p class="text-on-surface-variant max-w-xl">"Create a Buy-only listing, or add the fulfillment tier for one-click install. Metadata is signed by your active Nostr signer."</p>
                </div>
                <button class="px-8 py-3 rounded-md bg-gradient-to-r from-primary to-primary-dim text-on-primary font-bold shadow-lg shadow-primary/20 active:scale-95 transition-all" on:click={on_submit} disabled={move || is_hashing.get() || is_publishing.get()}>
                    {move || if is_publishing.get() { "Publishing..." } else if editing { "Update publication" } else { "Publish to Nostr" }}
                </button>
            </header>

            <div class="grid grid-cols-12 gap-8">
                <div class="col-span-12 lg:col-span-8 space-y-8">
                    <section class="bg-surface-container-high/60 backdrop-blur-2xl border border-outline-variant/15 rounded-3xl p-8">
                        <h2 class="text-2xl font-bold font-headline mb-6">"Game Details"</h2>
                        <div class="space-y-5">
                            <div>
                                <label class="block text-xs font-bold uppercase tracking-widest text-primary mb-2">"Listing ID / Slug"</label>
                                <input class="w-full bg-surface-container-highest border-none rounded-md p-4 text-on-surface" placeholder="my-game-v1" prop:value={move || id.get()} on:input:target=move |ev| id.set(ev.target().value()) disabled={move || is_publishing.get()} />
                            </div>
                            <div>
                                <label class="block text-xs font-bold uppercase tracking-widest text-primary mb-2">"Title"</label>
                                <input class="w-full bg-surface-container-highest border-none rounded-md p-4 text-on-surface" placeholder="Neon Drifter" prop:value={move || title.get()} on:input:target=move |ev| title.set(ev.target().value()) disabled={move || is_publishing.get()} />
                            </div>
                            <div>
                                <label class="block text-xs font-bold uppercase tracking-widest text-primary mb-2">"Description"</label>
                                <textarea class="w-full bg-surface-container-highest border-none rounded-md p-4 text-on-surface" rows=5 placeholder="Tell players about your game..." prop:value={move || description.get()} on:input:target=move |ev| description.set(ev.target().value()) disabled={move || is_publishing.get()} />
                            </div>
                            <div class="grid md:grid-cols-2 gap-5">
                                <div>
                                    <label class="block text-xs font-bold uppercase tracking-widest text-primary mb-2">"Tags"</label>
                                    <input class="w-full bg-surface-container-highest border-none rounded-md p-4 text-on-surface" placeholder="arcade, multiplayer" prop:value={move || tag_input.get()} on:input:target=move |ev| tag_input.set(ev.target().value()) disabled={move || is_publishing.get()} />
                                </div>
                                <div>
                                    <label class="block text-xs font-bold uppercase tracking-widest text-primary mb-2">"Image URLs"</label>
                                    <input class="w-full bg-surface-container-highest border-none rounded-md p-4 text-on-surface" placeholder="https://..." prop:value={move || image_input.get()} on:input:target=move |ev| image_input.set(ev.target().value()) disabled={move || is_publishing.get()} />
                                </div>
                            </div>
                        </div>
                    </section>

                    <section class="bg-surface-container-high/60 backdrop-blur-2xl border border-outline-variant/15 rounded-3xl p-8">
                        <h2 class="text-2xl font-bold font-headline mb-2">"Distribution & Fulfillment"</h2>
                        <p class="text-sm text-on-surface-variant mb-6">"Without fulfillment fields this listing remains Buy-only. Enable fulfillment only when you want automated install/download."</p>
                        <label class="flex items-center gap-3 p-4 rounded-xl bg-surface-container/50 mb-6">
                            <input type="checkbox" checked={move || fulfillment_enabled.get()} on:change:target=move |ev| {
                                let enabled = ev.target().checked();
                                fulfillment_enabled.set(enabled);
                                if !enabled { fulfillment_mode.set(FulfillmentMode::None); }
                            } />
                            <span class="font-bold">"Enable automated install fulfillment"</span>
                        </label>

                        <Show when=move || fulfillment_enabled.get()>
                            <div class="space-y-6">
                                <div class="grid md:grid-cols-2 gap-4">
                                    <button class="rounded-xl bg-surface-container-highest p-4 text-left" on:click=move |_| fulfillment_mode.set(FulfillmentMode::Direct)>
                                        <span class="block font-bold text-secondary">"Sign fulfillment with my own key"</span>
                                        <span class="text-xs text-on-surface-variant">"Uses the authenticated signer pubkey. No provisioning event."</span>
                                    </button>
                                    <button class="rounded-xl bg-surface-container-highest p-4 text-left" on:click=move |_| fulfillment_mode.set(FulfillmentMode::Delegate)>
                                        <span class="block font-bold text-secondary">"Delegate to an operator"</span>
                                        <span class="text-xs text-on-surface-variant">"Calls /provision and publishes kind:30406."</span>
                                    </button>
                                </div>

                                <Show when=move || matches!(fulfillment_mode.get(), FulfillmentMode::Delegate)>
                                    <div class="rounded-2xl bg-surface-container/50 p-4 space-y-3">
                                        <label class="block text-xs font-bold uppercase tracking-widest text-secondary">"Operator URL"</label>
                                        <div class="flex gap-2">
                                            <input class="flex-1 bg-surface-container-highest border-none rounded-md p-3 text-on-surface" placeholder="https://operator.example.com" prop:value={move || operator_url.get()} on:input:target=move |ev| {
                                                let next = ev.target().value();
                                                operator_url.set(next.clone());
                                                if operator_auto_added.get_untracked().is_some() { sync_operator_server(next); }
                                            } />
                                            <select class="bg-surface-container-highest border-none rounded-md p-3 text-on-surface" on:change:target=move |ev| {
                                                let selected = ev.target().value();
                                                if !selected.is_empty() { operator_url.set(selected); }
                                            }>
                                                <option value="">"Copy from server"</option>
                                                {move || servers.get().into_iter().map(|server| {
                                                    let url = server.url.clone();
                                                    let text = url.clone();
                                                    view! { <option value={url}>{text}</option> }
                                                }).collect_view()}
                                            </select>
                                        </div>
                                        <label class="flex items-center gap-2 text-sm text-on-surface-variant">
                                            <input type="checkbox" checked={move || operator_auto_added.get().is_some()} on:change:target=move |ev| {
                                                if ev.target().checked() {
                                                    sync_operator_server(operator_url.get());
                                                } else if let Some(old_url) = operator_auto_added.get_untracked() {
                                                    servers.update(|entries| entries.retain(|entry| !(entry.auto_operator && entry.url == old_url)));
                                                    operator_auto_added.set(None);
                                                }
                                            } />
                                            "Also add this as a distribution server"
                                        </label>
                                    </div>
                                </Show>

                                <div class="rounded-2xl bg-surface-container/50 p-4 space-y-4">
                                    <div class="flex items-center justify-between gap-3">
                                        <h3 class="font-bold">"Discovered servers"</h3>
                                        <span class="text-xs text-on-surface-variant">"Live relay query; manual entry still works if discovery fails."</span>
                                    </div>
                                    {move || discovery_error.get().map(|msg| view! { <div class="rounded-xl border border-error/30 bg-error-container/30 px-4 py-3 text-sm font-medium text-error">{msg}</div> })}
                                    <div class="space-y-2">
                                        {move || discovered_servers.get().into_iter().map(|server| {
                                            let checked_url = server.url.clone();
                                            let label = server.name.clone().unwrap_or_else(|| server.url.clone());
                                            let url_for_checked = checked_url.clone();
                                            let url_for_change = checked_url.clone();
                                            let label_for_change = label.clone();
                                            let label_display = label.clone();
                                            view! {
                                                <label class="flex items-center justify-between gap-3 rounded-xl bg-surface-container-highest p-3">
                                                    <span><input type="checkbox" class="mr-3" checked={move || servers.get().iter().any(|entry| entry.url == url_for_checked)} on:change:target=move |ev| {
                                                        if ev.target().checked() { add_server(url_for_change.clone(), label_for_change.clone(), false); }
                                                        else { remove_server(url_for_change.clone()); }
                                                    } />{label_display}</span>
                                                    <span class="text-xs text-on-surface-variant">{server.supported_adp.unwrap_or_default()}</span>
                                                </label>
                                            }
                                        }).collect_view()}
                                    </div>
                                    <div class="flex gap-2">
                                        <input class="flex-1 bg-surface-container-highest border-none rounded-md p-3 text-on-surface" placeholder="Add custom server URL" prop:value={move || custom_server.get()} on:input:target=move |ev| custom_server.set(ev.target().value()) />
                                        <button class="px-4 py-2 rounded-md bg-secondary text-on-secondary font-bold" on:click={on_add_custom_server}>"Add"</button>
                                    </div>
                                    <div class="space-y-2">
                                        {move || servers.get().into_iter().map(|server| {
                                            let reachability = server.reachability;
                                            let upload = server.upload;
                                            let url = server.url.clone();
                                            view! {
                                                <div class="flex items-center justify-between gap-3 rounded-xl bg-surface-container-highest p-3">
                                                    <div>
                                                        <p class="text-sm font-bold">{server.label}</p>
                                                        <p class="text-xs text-on-surface-variant">{server.url}</p>
                                                    </div>
                                                    <div class="text-right text-xs">
                                                        <p class={reachability.class()}>{format!("reachability: {}", reachability.label())}</p>
                                                        <p class={upload.class()}>{format!("upload: {}", upload.label())}</p>
                                                    </div>
                                                    <button class="text-error text-sm" on:click=move |_| remove_server(url.clone())>"Remove"</button>
                                                </div>
                                            }
                                        }).collect_view()}
                                    </div>
                                </div>

                                <div class="grid md:grid-cols-2 gap-5">
                                    <div>
                                        <label class="block text-xs font-bold uppercase tracking-widest text-secondary mb-2">"Build File"</label>
                                        <button class="w-full rounded-md bg-surface-container-highest p-3 text-left" on:click={on_select_file} disabled={move || is_hashing.get() || is_publishing.get()}>
                                            {move || if is_hashing.get() { "Hashing...".to_string() } else { file_path.get().unwrap_or_else(|| "Select archive".to_string()) }}
                                        </button>
                                        <p class="text-xs text-on-surface-variant mt-2">{move || file_hash.get().map(|hash| format!("SHA-256: {}", format_sha256(&hash))).unwrap_or_else(|| "Hash appears after file selection.".to_string())}</p>
                                    </div>
                                    <div>
                                        <label class="block text-xs font-bold uppercase tracking-widest text-secondary mb-2">"Version"</label>
                                        <input class="w-full bg-surface-container-highest border-none rounded-md p-3 text-on-surface" placeholder="1.0.0" prop:value={move || version.get()} on:input:target=move |ev| version.set(ev.target().value()) />
                                    </div>
                                </div>
                            </div>
                        </Show>
                    </section>
                </div>

                <aside class="col-span-12 lg:col-span-4 space-y-8">
                    <section class="bg-surface-container-high/60 backdrop-blur-2xl border border-outline-variant/15 rounded-3xl p-6">
                        <h3 class="text-lg font-bold font-headline mb-5">"Pricing & Metadata"</h3>
                        <div class="space-y-5">
                            <div>
                                <label class="block text-[10px] font-bold uppercase tracking-widest text-secondary mb-2">"Pricing (sats)"</label>
                                <input class="w-full bg-surface-container-highest border-none rounded-md p-3 text-on-surface" type="number" min=0 prop:value={move || price_sats.get().to_string()} on:input:target=move |ev| { if let Ok(val) = ev.target().value().parse::<u64>() { price_sats.set(val); } } />
                            </div>
                            <div>
                                <label class="block text-[10px] font-bold uppercase tracking-widest text-secondary mb-2">"Lightning Address (lud16)"</label>
                                <input class="w-full bg-surface-container-highest border-none rounded-md p-3 text-on-surface" placeholder="you@example.com" prop:value={move || lud16.get()} on:input:target=move |ev| lud16.set(ev.target().value()) />
                            </div>
                            <div>
                                <label class="block text-[10px] font-bold uppercase tracking-widest text-secondary mb-2" for="acquisition-policy">"Acquisition policy"</label>
                                <select id="acquisition-policy" class="w-full bg-surface-container-highest border-none rounded-md p-3 text-on-surface" prop:value=move || acquisition_kind.get().value() on:change:target=move |ev| acquisition_kind.set(AcquisitionKind::from_value(&ev.target().value())) disabled=move || is_publishing.get()>
                                    <option value="gated">"Paid / gated"</option>
                                    <option value="public">"Public access"</option>
                                    <option value="timed-access">"Timed access"</option>
                                </select>
                                <p class="text-xs text-on-surface-variant mt-2">{move || match acquisition_kind.get() {
                                    AcquisitionKind::Gated => "Access requires a purchase or entitlement.",
                                    AcquisitionKind::Public => "Anyone can access the game without an entitlement.",
                                    AcquisitionKind::TimedAccess => "Anyone can access the game during the selected window.",
                                }}</p>
                            </div>
                            <Show when=move || matches!(acquisition_kind.get(), AcquisitionKind::TimedAccess)>
                                <div class="grid gap-3">
                                    <div>
                                        <label class="block text-[10px] font-bold uppercase tracking-widest text-secondary mb-2" for="acquisition-start">"Access starts"</label>
                                        <input id="acquisition-start" class="w-full bg-surface-container-highest border-none rounded-md p-3 text-on-surface" type="datetime-local" step="1" prop:value=move || acquisition_starts_at.get() on:input:target=move |ev| acquisition_starts_at.set(ev.target().value()) disabled=move || is_publishing.get() />
                                    </div>
                                    <div>
                                        <label class="block text-[10px] font-bold uppercase tracking-widest text-secondary mb-2" for="acquisition-end">"Access ends"</label>
                                        <input id="acquisition-end" class="w-full bg-surface-container-highest border-none rounded-md p-3 text-on-surface" type="datetime-local" step="1" prop:value=move || acquisition_ends_at.get() on:input:target=move |ev| acquisition_ends_at.set(ev.target().value()) disabled=move || is_publishing.get() />
                                    </div>
                                    <p class="text-xs text-on-surface-variant">"Times use your local timezone."</p>
                                </div>
                            </Show>
                            <div>
                                <label class="block text-[10px] font-bold uppercase tracking-widest text-secondary mb-2">"Platforms"</label>
                                <input class="w-full bg-surface-container-highest border-none rounded-md p-3 text-on-surface" placeholder="linux-x86_64, windows-x86_64" prop:value={move || platforms_input.get()} on:input:target=move |ev| platforms_input.set(ev.target().value()) />
                            </div>
                        </div>
                    </section>

                    <section class="bg-surface-container-high/60 backdrop-blur-2xl border border-outline-variant/15 rounded-3xl p-6">
                        <h3 class="text-lg font-bold font-headline mb-5">"Nostr Identity"</h3>
                        <p class="text-[10px] font-bold uppercase tracking-widest text-tertiary mb-2">"Authenticated signer"</p>
                        <div class="bg-surface-container-highest rounded-md p-3 text-xs font-mono text-on-surface break-all">{move || auth.npub.get().unwrap_or_else(|| "Not authenticated".to_string())}</div>
                        <p class="text-xs text-on-surface-variant mt-3">"This value is read-only and comes from the active NIP-46 session."</p>
                    </section>

                    <section class="bg-gradient-to-br from-surface-container-high to-surface-container-lowest border border-outline-variant/10 rounded-3xl p-6">
                        <p class="text-[10px] font-bold uppercase tracking-widest text-primary-dim mb-4">"Publish status"</p>
                        {move || error_message.get().map(|msg| view! { <div class="mb-4 rounded-xl border border-error/30 bg-error-container/30 px-4 py-3 text-sm font-medium text-error">{msg}</div> })}
                        {move || success_message.get().map(|msg| view! { <div class="mb-4 rounded-xl border border-secondary/30 bg-secondary-container/30 px-4 py-3 text-sm font-medium text-secondary">{msg}</div> })}
                        <ul class="space-y-2 text-xs text-on-surface-variant">
                            {move || progress_events.get().into_iter().map(|event| view! {
                                <li>{format!("{}{}: {}{}", event.step, event.server_url.map(|url| format!(" ({url})")).unwrap_or_default(), event.status, event.message.map(|m| format!(" - {m}")).unwrap_or_default())}</li>
                            }).collect_view()}
                        </ul>
                    </section>
                </aside>
            </div>
        </div>
    }
}
