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
    listen_publish_progress, AdpServerAnnouncement, CampaignPointerInput, FulfillmentMode,
    HashBuildFileRequest, PublishAdpListingRequest, PublishProgressPayload,
    ResolveAdpOperatorRequest,
};
use crate::ui_v2::views::{use_fallback_cover, valid_cover_url};
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

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum PublishStage {
    Details,
    Pricing,
    Builds,
    Review,
}

impl PublishStage {
    const ALL: [Self; 4] = [Self::Details, Self::Pricing, Self::Builds, Self::Review];

    fn index(self) -> usize {
        match self {
            Self::Details => 0,
            Self::Pricing => 1,
            Self::Builds => 2,
            Self::Review => 3,
        }
    }

    fn title(self) -> &'static str {
        match self {
            Self::Details => "Game details",
            Self::Pricing => "Pricing and access",
            Self::Builds => "Builds and distribution",
            Self::Review => "Review and publish",
        }
    }

    fn next(self) -> Option<Self> {
        Self::ALL.get(self.index() + 1).copied()
    }

    fn previous(self) -> Option<Self> {
        self.index()
            .checked_sub(1)
            .and_then(|index| Self::ALL.get(index).copied())
    }
}

#[cfg(test)]
const SUPPORTS_DRAFTS: bool = false;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PublicationOutcome {
    Idle,
    Publishing,
    Complete,
    Partial,
    Failed,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct PublicationState {
    outcome: PublicationOutcome,
    listing_published: bool,
    message: Option<String>,
}

impl Default for PublicationState {
    fn default() -> Self {
        Self {
            outcome: PublicationOutcome::Idle,
            listing_published: false,
            message: None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ReadinessChecklist {
    metadata: bool,
    pricing_and_access: bool,
    distribution: bool,
    authorization: bool,
    warnings: Vec<String>,
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

fn paid_price(paid_enabled: bool, input: &str, lud16: &str) -> Result<u64, String> {
    if !paid_enabled {
        return Ok(0);
    }
    let price = input
        .trim()
        .parse::<u64>()
        .map_err(|_| "Enter a valid whole-number price in sats".to_string())?;
    if price == 0 {
        return Err("Paid purchase requires a price greater than zero sats".to_string());
    }
    if lud16.trim().is_empty() {
        return Err("Lightning address is required when paid purchase is enabled".to_string());
    }
    let mut address_parts = lud16.trim().split('@');
    let user = address_parts.next().unwrap_or_default();
    let domain = address_parts.next().unwrap_or_default();
    if user.is_empty() || domain.is_empty() || address_parts.next().is_some() {
        return Err("Lightning address must look like name@example.com".to_string());
    }
    Ok(price)
}

fn is_http_url(value: &str) -> bool {
    url::Url::parse(value).is_ok_and(|parsed| {
        matches!(parsed.scheme(), "http" | "https")
            && parsed.host_str().is_some()
            && !value
                .strip_prefix("https://")
                .or_else(|| value.strip_prefix("http://"))
                .is_some_and(|remainder| remainder.starts_with('/'))
    })
}

fn validate_http_urls(input: &str, label: &str) -> Result<Vec<String>, String> {
    let values = parse_csv_values(input);
    for value in &values {
        if !is_http_url(value) {
            return Err(format!("{label} is not a valid HTTP(S) URL: {value}"));
        }
    }
    Ok(values)
}

fn publication_progress(
    mut state: PublicationState,
    event: &PublishProgressPayload,
) -> PublicationState {
    if state.outcome != PublicationOutcome::Partial {
        state.outcome = PublicationOutcome::Publishing;
    }
    if event.step == "publish-listing" && event.status == "ok" {
        state.listing_published = true;
    }
    if state.listing_published && event.step == "upload" && event.status == "error" {
        state.outcome = PublicationOutcome::Partial;
        state.message = Some(
            "The game page exists on the network, but automated installation is incomplete because an upload failed. Targeted upload retry is not supported by the current backend."
                .to_string(),
        );
    }
    state
}

fn publication_completed(
    mut state: PublicationState,
    result: Result<(String, bool), String>,
) -> PublicationState {
    match result {
        Ok((_event_id, true)) => {
            state.listing_published = true;
            state.outcome = PublicationOutcome::Partial;
            state.message = Some(
                "The game page exists on the network, but automated installation is incomplete because an upload failed. Targeted upload retry is not supported by the current backend."
                    .to_string(),
            );
        }
        Ok((event_id, false)) => {
            state.listing_published = true;
            state.outcome = PublicationOutcome::Complete;
            state.message = Some(format!("Network publication completed: {event_id}"));
        }
        Err(error) if state.listing_published => {
            state.outcome = PublicationOutcome::Partial;
            state.message = Some(format!(
                "The game page exists on the network, but the remaining publication work failed: {error}. Targeted upload retry is not supported by the current backend."
            ));
        }
        Err(error) => {
            state.outcome = PublicationOutcome::Failed;
            state.message = Some(error);
        }
    }
    state
}

fn progress_label(event: &PublishProgressPayload) -> String {
    let stage = match event.step.as_str() {
        "hash-file" => "Build verification",
        "check-operator" => "Distribution provider check",
        "provision" => "Publishing authorization",
        "publish-listing" => "Game page publication",
        "confirm-propagation" => "Network publication confirmation",
        "upload" => "Build upload",
        _ => "Publication",
    };
    let status = match event.status.as_str() {
        "pending" => "in progress",
        "ok" => "completed",
        "error" => "failed",
        other => other,
    };
    format!(
        "{stage}{}: {status}{}",
        event
            .server_url
            .as_ref()
            .map(|url| format!(" for {url}"))
            .unwrap_or_default(),
        event
            .message
            .as_ref()
            .map(|message| format!(" — {message}"))
            .unwrap_or_default()
    )
}

fn publication_account_matches(initiating_npub: &str, active_npub: Option<&str>) -> bool {
    active_npub == Some(initiating_npub)
}

fn publication_account_allowed(editing_publisher: Option<&str>, active_npub: &str) -> bool {
    editing_publisher.is_none_or(|publisher| publisher == active_npub)
}

fn can_dispatch(is_hashing: bool, is_publishing: bool) -> bool {
    !is_hashing && !is_publishing
}

fn when_fulfillment_enabled<T>(enabled: bool, value: T) -> Option<T> {
    enabled.then_some(value)
}

fn advance_stage(
    current: PublishStage,
    validation: Result<(), String>,
) -> Result<PublishStage, String> {
    validation?;
    current
        .next()
        .ok_or_else(|| "Already at the review stage".to_string())
}

fn file_selection_changed(new_path: Option<String>) -> (Option<String>, Option<String>) {
    match new_path {
        Some(path) => (Some(path), None),
        None => (None, None),
    }
}

fn build_capability_message() -> &'static str {
    #[cfg(feature = "web")]
    {
        "Build file selection, hashing, and network publication require the desktop app."
    }
    #[cfg(not(feature = "web"))]
    {
        "This desktop app can select, hash, and publish build files."
    }
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
    let id = id.trim();
    let title = title.trim();
    let description = description.trim();
    let lud16 = lud16.trim();
    if id.is_empty() {
        return Err("Game page identifier is required".to_string());
    }
    if id.len() > 64 {
        return Err("Game page identifier must be 64 characters or less".to_string());
    }
    if !id
        .chars()
        .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
    {
        return Err(
            "Game page identifier can only contain lowercase letters, numbers, and hyphens"
                .to_string(),
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
            .find(|url| !is_http_url(url))
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
                if !is_http_url(operator_url) {
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
    let tags = input
        .split(',')
        .map(str::trim)
        .filter(|tag| !tag.is_empty())
        .map(|tag| {
            if tag.chars().any(char::is_whitespace) {
                return Err("Platform tags cannot contain whitespace".to_string());
            }
            let mut parts = tag.split('-');
            let os = parts.next().unwrap_or_default();
            let arch = parts.next().unwrap_or_default();
            if os.is_empty()
                || arch.is_empty()
                || parts.next().is_some()
                || !os.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
                || !arch.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
            {
                return Err("Platform tags must look like <os>-<arch>".to_string());
            }
            Ok(tag.to_string())
        })
        .collect::<Result<Vec<_>, _>>()?;
    let mut unique = std::collections::HashSet::new();
    if let Some(duplicate) = tags.iter().find(|tag| !unique.insert(tag.as_str())) {
        return Err(format!("Duplicate platform tag: {duplicate}"));
    }
    Ok(tags)
}

#[allow(clippy::too_many_arguments)]
fn readiness_checklist(
    id: &str,
    title: &str,
    description: &str,
    image_input: &str,
    paid_enabled: bool,
    price_input: &str,
    lud16: &str,
    acquisition: Result<AcquisitionPolicy, String>,
    platforms_input: &str,
    fulfillment_enabled: bool,
    servers: &[ServerEntry],
    file_hash: Option<&str>,
    version: &str,
    fulfillment_mode: &FulfillmentMode,
    operator_url: &str,
) -> ReadinessChecklist {
    let metadata = validate_listing(
        id,
        title,
        description,
        0,
        "",
        false,
        &[],
        &None,
        &None,
        "",
        &FulfillmentMode::None,
        "",
    )
    .is_ok()
        && validate_http_urls(image_input, "Image URL").is_ok();
    let pricing_and_access =
        paid_price(paid_enabled, price_input, lud16).is_ok() && acquisition.is_ok();
    let platforms_ok = parse_platform_tags(platforms_input).is_ok();
    let distribution = platforms_ok
        && (!fulfillment_enabled
            || (!servers.is_empty()
                && file_hash.is_some_and(is_sha256_hex)
                && !version.trim().is_empty()
                && !matches!(fulfillment_mode, FulfillmentMode::None)));
    let authorization = !fulfillment_enabled
        || matches!(fulfillment_mode, FulfillmentMode::Direct)
        || (matches!(fulfillment_mode, FulfillmentMode::Delegate)
            && !operator_url.trim().is_empty());
    let mut warnings = Vec::new();
    if !fulfillment_enabled {
        warnings.push("No build will be uploaded; this publishes game-page metadata only.".into());
    }
    if servers
        .iter()
        .any(|server| server.reachability == ServerStatus::Failed)
    {
        warnings.push("At least one distribution provider could not be reached.".into());
    }
    ReadinessChecklist {
        metadata,
        pricing_and_access,
        distribution,
        authorization,
        warnings,
    }
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

    #[test]
    fn stage_navigation_validates_forward_and_keeps_form_values_external() {
        let title = String::from("Preserved game");
        assert_eq!(
            advance_stage(PublishStage::Details, Err("missing".into())),
            Err("missing".into())
        );
        assert_eq!(
            advance_stage(PublishStage::Details, Ok(())),
            Ok(PublishStage::Pricing)
        );
        assert_eq!(title, "Preserved game");
    }

    #[test]
    fn drafts_are_not_supported() {
        assert!(!SUPPORTS_DRAFTS);
        assert!(!include_str!("publish.rs").contains(concat!("Save", " Draft")));
        assert!(!include_str!("publish.rs").contains(concat!("Save", " draft")));
    }

    #[test]
    fn paid_purchase_requires_positive_parseable_sats_and_lud16() {
        assert!(paid_price(true, "abc", "seller@example.com").is_err());
        assert!(paid_price(true, "0", "seller@example.com").is_err());
        assert!(paid_price(true, "25", "").is_err());
        assert_eq!(paid_price(true, "25", "seller@example.com"), Ok(25));
        assert_eq!(paid_price(false, "invalid", ""), Ok(0));
    }

    #[test]
    fn zero_price_does_not_imply_public_and_public_is_explicit() {
        let gated = acquisition_policy_from_form(AcquisitionKind::Gated, "", "")
            .expect("gated access should be explicit");
        assert_eq!(paid_price(false, "0", ""), Ok(0));
        assert!(matches!(gated, AcquisitionPolicy::Gated));
        assert!(matches!(
            acquisition_policy_from_form(AcquisitionKind::Public, "", ""),
            Ok(AcquisitionPolicy::Public)
        ));
    }

    #[test]
    fn duplicate_and_malformed_platform_tags_are_rejected() {
        assert_eq!(
            parse_platform_tags("linux-x86_64,linux-x86_64"),
            Err("Duplicate platform tag: linux-x86_64".into())
        );
        assert!(parse_platform_tags("linux-x86_64-extra").is_err());
        assert!(parse_platform_tags("-x86_64").is_err());
    }

    #[test]
    fn image_urls_must_be_http_or_https() {
        assert!(validate_http_urls(
            "https://example.com/a.png, http://example.com/b.png",
            "Image URL"
        )
        .is_ok());
        assert!(validate_http_urls("file:///tmp/a.png", "Image URL").is_err());
        assert!(validate_http_urls("https:///missing-host.png", "Image URL").is_err());
    }

    #[test]
    fn selecting_a_different_file_invalidates_the_hash() {
        let (path, hash) = file_selection_changed(Some("/tmp/new.zip".into()));
        assert_eq!(path.as_deref(), Some("/tmp/new.zip"));
        assert_eq!(hash, None);
    }

    #[test]
    fn readiness_reports_provider_reachability_and_existing_hash_reuse() {
        let servers = vec![ServerEntry {
            url: "https://dist.example.com".into(),
            label: "Provider".into(),
            reachability: ServerStatus::Failed,
            upload: ServerStatus::Idle,
            auto_operator: false,
        }];
        let checklist = readiness_checklist(
            "game",
            "Game",
            "Description",
            "https://example.com/cover.png",
            false,
            "0",
            "",
            Ok(AcquisitionPolicy::Gated),
            "linux-x86_64",
            true,
            &servers,
            Some(VALID_SHA256),
            "1.0.0",
            &FulfillmentMode::Direct,
            "",
        );
        assert!(checklist.metadata);
        assert!(checklist.pricing_and_access);
        assert!(checklist.distribution, "a valid existing hash is reusable");
        assert!(checklist.authorization);
        assert!(checklist
            .warnings
            .iter()
            .any(|warning| warning.contains("could not be reached")));
    }

    #[test]
    fn publication_reducer_never_marks_upload_failure_complete() {
        let published = publication_progress(
            PublicationState {
                outcome: PublicationOutcome::Publishing,
                ..Default::default()
            },
            &PublishProgressPayload {
                step: "publish-listing".into(),
                status: "ok".into(),
                server_url: None,
                message: None,
            },
        );
        let partial = publication_progress(
            published,
            &PublishProgressPayload {
                step: "upload".into(),
                status: "error".into(),
                server_url: Some("https://dist.example.com".into()),
                message: Some("offline".into()),
            },
        );
        assert_eq!(partial.outcome, PublicationOutcome::Partial);
        assert!(partial.listing_published);
        assert!(partial
            .message
            .as_deref()
            .is_some_and(|message| message.contains("game page exists")));

        let command_failure = publication_completed(partial, Err("upload failed".into()));
        assert_eq!(command_failure.outcome, PublicationOutcome::Partial);
        assert_ne!(command_failure.outcome, PublicationOutcome::Complete);
    }

    #[test]
    fn stale_account_and_duplicate_dispatch_guards_are_explicit() {
        assert!(publication_account_matches("npub1a", Some("npub1a")));
        assert!(!publication_account_matches("npub1a", Some("npub1b")));
        assert!(!publication_account_matches("npub1a", None));
        assert!(publication_account_allowed(Some("npub1a"), "npub1a"));
        assert!(!publication_account_allowed(Some("npub1a"), "npub1b"));
        assert!(publication_account_allowed(None, "npub1b"));
        assert!(can_dispatch(false, false));
        assert!(!can_dispatch(true, false));
        assert!(!can_dispatch(false, true));
    }

    #[test]
    fn disabling_fulfillment_omits_hidden_configuration() {
        assert_eq!(when_fulfillment_enabled(false, Some("hash")), None);
        assert_eq!(when_fulfillment_enabled(false, vec!["server"]), None);
        assert_eq!(
            when_fulfillment_enabled(true, Some("hash")),
            Some(Some("hash"))
        );
    }

    #[test]
    fn capability_message_matches_the_build_target() {
        #[cfg(feature = "web")]
        assert!(build_capability_message().contains("require the desktop app"));
        #[cfg(not(feature = "web"))]
        assert!(build_capability_message().contains("desktop app can"));
    }
}

/// Publish view component - form for creating NIP-99 listings with optional ADP fulfillment.
#[component]
pub fn PublishView(#[prop(optional)] listing: Option<GameListing>) -> impl IntoView {
    let auth = use_context::<AuthContext>().expect("AuthContext not provided");
    let editing = listing.is_some();
    let editing_publisher = listing.as_ref().map(|item| item.publisher_npub.clone());
    let existing_event_id = listing.as_ref().and_then(|item| item.event_id.clone());
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
    let existing_campaigns = listing
        .as_ref()
        .map(|item| {
            item.campaigns
                .iter()
                .map(|pointer| CampaignPointerInput {
                    root_event_id: pointer.root_event_id.clone(),
                    relay_hint: pointer.relay_hint.clone(),
                })
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    let existing_nip94_event_id = listing
        .as_ref()
        .and_then(|item| item.nip94_event_id.clone());
    let existing_fulfillment_locked = existing_fulfillment_pubkey.is_some();

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
    let initial_price = listing
        .as_ref()
        .map(|item| item.price_sats)
        .unwrap_or_default();
    let paid_enabled = RwSignal::new(initial_price > 0);
    let price_input = RwSignal::new(initial_price.to_string());
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

    let current_stage = RwSignal::new(PublishStage::Details);
    let stage_error = RwSignal::new(None::<String>);

    let is_publishing = RwSignal::new(false);
    let is_hashing = RwSignal::new(false);
    let error_message = RwSignal::new(None::<String>);
    let progress_events = RwSignal::new(Vec::<PublishProgressPayload>::new());
    let publication_state = RwSignal::new(PublicationState::default());
    let initiating_account = RwSignal::new(None::<String>);
    let publication_account_stale = RwSignal::new(false);

    Effect::new(move |_| {
        if let Some(expected) = initiating_account.get() {
            if !publication_account_matches(&expected, auth.npub.get().as_deref()) {
                publication_account_stale.set(true);
            }
        }
    });

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
        add_server(new_url, "Distribution provider".to_string(), true);
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

    Effect::new(move |_| {
        for url in published_servers.clone() {
            spawn_local(async move {
                let status = match invoke_check_adp_server(url.clone()).await {
                    Ok(_) => ServerStatus::Ok,
                    Err(_) => ServerStatus::Failed,
                };
                servers.update(|entries| {
                    if let Some(entry) = entries.iter_mut().find(|entry| entry.url == url) {
                        entry.reachability = status;
                    }
                });
            });
        }
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
        if !can_dispatch(is_hashing.get_untracked(), is_publishing.get_untracked()) {
            return;
        }
        is_hashing.set(true);
        error_message.set(None);
        spawn_local(async move {
            match invoke_select_build_file().await {
                Ok(Some(path)) => {
                    let (next_path, next_hash) = file_selection_changed(Some(path.clone()));
                    file_path.set(next_path);
                    file_hash.set(next_hash);
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

    let on_next = move |_| {
        let stage = current_stage.get_untracked();
        let validation = match stage {
            PublishStage::Details => {
                let id_value = id.get_untracked();
                let title_value = title.get_untracked();
                let description_value = description.get_untracked();
                validate_listing(
                    &id_value,
                    &title_value,
                    &description_value,
                    0,
                    "",
                    false,
                    &[],
                    &None,
                    &None,
                    "",
                    &FulfillmentMode::None,
                    "",
                )
                .and_then(|_| {
                    validate_http_urls(&image_input.get_untracked(), "Image URL").map(|_| ())
                })
            }
            PublishStage::Pricing => paid_price(
                paid_enabled.get_untracked(),
                &price_input.get_untracked(),
                &lud16.get_untracked(),
            )
            .and_then(|_| {
                acquisition_policy_from_form(
                    acquisition_kind.get_untracked(),
                    &acquisition_starts_at.get_untracked(),
                    &acquisition_ends_at.get_untracked(),
                )
                .map(|_| ())
            }),
            PublishStage::Builds => {
                let platforms = parse_platform_tags(&platforms_input.get_untracked()).map(|_| ());
                platforms.and_then(|_| {
                    if !fulfillment_enabled.get_untracked() {
                        return Ok(());
                    }
                    validate_listing(
                        &id.get_untracked(),
                        &title.get_untracked(),
                        &description.get_untracked(),
                        0,
                        "",
                        true,
                        &servers.get_untracked(),
                        &file_path.get_untracked(),
                        &file_hash.get_untracked(),
                        &version.get_untracked(),
                        &fulfillment_mode.get_untracked(),
                        &operator_url.get_untracked(),
                    )
                })
            }
            PublishStage::Review => Err("Already at the review stage".to_string()),
        };
        match advance_stage(stage, validation) {
            Ok(next) => {
                current_stage.set(next);
                stage_error.set(None);
            }
            Err(error) => stage_error.set(Some(error)),
        }
    };

    let existing_campaigns_for_submit = existing_campaigns.clone();
    let existing_nip94_for_submit = existing_nip94_event_id.clone();
    let existing_fulfillment_key_for_submit = existing_fulfillment_pubkey.clone();
    let editing_publisher_for_submit = editing_publisher.clone();
    let existing_event_id_for_submit = existing_event_id.clone();
    let on_submit = Callback::new(move |()| {
        if !can_dispatch(is_hashing.get_untracked(), is_publishing.get_untracked()) {
            return;
        }
        let Some(initiating_npub) = auth.npub.get() else {
            error_message.set(Some(
                "Sign in before starting network publication".to_string(),
            ));
            return;
        };
        if !publication_account_allowed(editing_publisher_for_submit.as_deref(), &initiating_npub) {
            error_message.set(Some(
                "Switch to the developer account that published this Game page before updating it."
                    .to_string(),
            ));
            return;
        }

        let id_val = id.get().trim().to_string();
        let title_val = title.get().trim().to_string();
        let description_val = description.get().trim().to_string();
        let lud16_val = lud16.get().trim().to_string();
        let price_val = match paid_price(paid_enabled.get(), &price_input.get(), &lud16_val) {
            Ok(price) => price,
            Err(msg) => {
                error_message.set(Some(msg));
                return;
            }
        };
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
        let images = match validate_http_urls(&image_input.get(), "Image URL") {
            Ok(images) => images,
            Err(msg) => {
                error_message.set(Some(msg));
                return;
            }
        };

        let request = PublishAdpListingRequest {
            expected_publisher_npub: initiating_npub.clone(),
            existing_event_id: existing_event_id_for_submit.clone(),
            d_tag: id_val,
            title: title_val,
            description: description_val,
            price_sats: price_val,
            lud16: (!lud16_val.is_empty()).then_some(lud16_val),
            tags: parse_csv_values(&tag_input.get()),
            images,
            fulfillment_mode: fulfillment_mode_val,
            operator_url: when_fulfillment_enabled(
                fulfillment_enabled_val,
                (!operator_url_val.trim().is_empty()).then_some(operator_url_val),
            )
            .flatten(),
            servers: when_fulfillment_enabled(
                fulfillment_enabled_val,
                servers_val.into_iter().map(|entry| entry.url).collect(),
            )
            .unwrap_or_default(),
            file_path: when_fulfillment_enabled(fulfillment_enabled_val, file_path_val).flatten(),
            existing_file_hash: when_fulfillment_enabled(fulfillment_enabled_val, file_hash_val)
                .flatten(),
            existing_fulfillment_pubkey: when_fulfillment_enabled(
                fulfillment_enabled_val,
                existing_fulfillment_key_for_submit.clone(),
            )
            .flatten(),
            existing_fulfillment_valid_from: when_fulfillment_enabled(
                fulfillment_enabled_val,
                existing_fulfillment_valid_from,
            )
            .flatten(),
            existing_fulfillment_revoked_at: when_fulfillment_enabled(
                fulfillment_enabled_val,
                existing_fulfillment_revoked_at,
            )
            .flatten(),
            version: when_fulfillment_enabled(
                fulfillment_enabled_val,
                (!version_val.trim().is_empty()).then_some(version_val),
            )
            .flatten(),
            acquisition,
            platforms,
            campaigns: existing_campaigns_for_submit.clone(),
            nip94_event_id: existing_nip94_for_submit.clone(),
        };

        is_publishing.set(true);
        initiating_account.set(Some(initiating_npub.clone()));
        publication_account_stale.set(false);
        error_message.set(None);
        progress_events.set(Vec::new());
        publication_state.set(PublicationState {
            outcome: PublicationOutcome::Publishing,
            ..PublicationState::default()
        });
        servers.update(|entries| {
            for entry in entries {
                entry.upload = ServerStatus::Idle;
            }
        });

        spawn_local(async move {
            let progress_account = initiating_npub.clone();
            let listener_cleanup = listen_publish_progress(move |payload| {
                if publication_account_stale.get_untracked()
                    || !publication_account_matches(
                        &progress_account,
                        auth.npub.get_untracked().as_deref(),
                    )
                {
                    return;
                }
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
                publication_state
                    .update(|state| *state = publication_progress(state.clone(), &payload));
                progress_events.update(|events| events.push(payload));
            })
            .await;
            let listener_cleanup = match listener_cleanup {
                Ok(cleanup) => cleanup,
                Err(error) => {
                    publication_state.set(PublicationState {
                        outcome: PublicationOutcome::Failed,
                        listing_published: false,
                        message: Some(format!(
                            "Publication did not start because progress monitoring is unavailable: {error}"
                        )),
                    });
                    is_publishing.set(false);
                    initiating_account.set(None);
                    return;
                }
            };

            let publish_result = invoke_publish_adp_listing(request).await;
            listener_cleanup();
            if publication_account_stale.get_untracked()
                || !publication_account_matches(
                    &initiating_npub,
                    auth.npub.get_untracked().as_deref(),
                )
            {
                let current = publication_state.get_untracked();
                publication_state.set(PublicationState {
                    outcome: if current.listing_published {
                        PublicationOutcome::Partial
                    } else {
                        PublicationOutcome::Failed
                    },
                    listing_published: current.listing_published,
                    message: Some(if current.listing_published {
                        "The Game page was published before the active account changed, but later results were ignored. Switch to the initiating account and refresh Published games to reconcile uploads."
                            .into()
                    } else {
                        "The active account changed during publication. The stale response was ignored; check Published games with the initiating account before retrying."
                            .into()
                    }),
                });
                is_publishing.set(false);
                initiating_account.set(None);
                return;
            }
            match publish_result {
                Ok(result) => {
                    let uploads_failed = result.uploads.iter().any(|upload| upload.status != "ok");
                    publication_state.update(|state| {
                        *state = publication_completed(
                            state.clone(),
                            Ok((result.event_id, uploads_failed)),
                        )
                    });
                }
                Err(err) => publication_state
                    .update(|state| *state = publication_completed(state.clone(), Err(err))),
            }
            is_publishing.set(false);
            initiating_account.set(None);
        });
    });

    let checklist = move || {
        readiness_checklist(
            &id.get(),
            &title.get(),
            &description.get(),
            &image_input.get(),
            paid_enabled.get(),
            &price_input.get(),
            &lud16.get(),
            acquisition_policy_from_form(
                acquisition_kind.get(),
                &acquisition_starts_at.get(),
                &acquisition_ends_at.get(),
            ),
            &platforms_input.get(),
            fulfillment_enabled.get(),
            &servers.get(),
            file_hash.get().as_deref(),
            &version.get(),
            &fulfillment_mode.get(),
            &operator_url.get(),
        )
    };

    view! {
        <main class="v2-publish-wizard max-w-6xl mx-auto px-8 py-10">
            <header class="mb-8">
                <p class="text-xs font-bold uppercase tracking-widest text-primary mb-2">"Publishing workflow"</p>
                <h1 class="text-5xl font-extrabold font-headline tracking-tighter mb-2">{if editing { "Update your " } else { "Publish a " }}<span class="text-primary italic">"game page"</span></h1>
                <p class="text-on-surface-variant max-w-2xl">"Complete each stage, review the result, then authorize publication from your active account."</p>
            </header>

            <nav class="v2-publish-steps mb-8" aria-label="Publishing stages">
                <ol class="grid gap-3 md:grid-cols-4">
                    {PublishStage::ALL.into_iter().map(|stage| view! {
                        <li><button type="button" class="w-full rounded-xl bg-surface-container-high p-4 text-left disabled:opacity-40"
                            aria-current=move || (current_stage.get() == stage).then_some("step")
                            disabled=move || { stage > current_stage.get() || is_publishing.get() }
                            on:click=move |_| { current_stage.set(stage); stage_error.set(None); }>
                            <span class="block text-[10px] font-bold uppercase tracking-widest text-secondary">{format!("Stage {}", stage.index() + 1)}</span>
                            <span class="font-bold">{stage.title()}</span>
                        </button></li>
                    }).collect_view()}
                </ol>
            </nav>
            {move || stage_error.get().map(|msg| view! { <div id="publish-stage-error" role="alert" class="mb-6 rounded-xl border border-error/30 bg-error-container/30 px-4 py-3 text-sm font-medium text-error">{msg}</div> })}

            <div class="grid grid-cols-12 gap-8">
                <div class=move || if matches!(current_stage.get(), PublishStage::Details | PublishStage::Builds) { "col-span-12 lg:col-span-8 space-y-8" } else { "hidden" }>
                    <Show when=move || current_stage.get() == PublishStage::Details>
                    <section class="v2-publish-stage bg-surface-container-high/60 backdrop-blur-2xl border border-outline-variant/15 rounded-3xl p-8" aria-labelledby="publish-details-title">
                        <h2 id="publish-details-title" class="text-2xl font-bold font-headline mb-6">"Game details"</h2>
                        <div class="space-y-5">
                            <div>
                                <label for="publish-id" class="block text-xs font-bold uppercase tracking-widest text-primary mb-2">"Game page identifier (required)"</label>
                                <input id="publish-id" required=true aria-describedby="publish-id-help" class="w-full bg-surface-container-highest border-none rounded-md p-4 text-on-surface" placeholder="my-game-v1" prop:value={move || id.get()} on:input:target=move |ev| id.set(ev.target().value()) readonly=editing disabled={move || is_publishing.get()} />
                                <p id="publish-id-help" class="text-xs text-on-surface-variant mt-2">{if editing { "This permanent identifier is locked while updating the existing Game page." } else { "This permanent identifier becomes the Game page coordinate. Protocol detail: it is the listing d tag." }}</p>
                            </div>
                            <div>
                                <label for="publish-title" class="block text-xs font-bold uppercase tracking-widest text-primary mb-2">"Title (required)"</label>
                                <input id="publish-title" required=true class="w-full bg-surface-container-highest border-none rounded-md p-4 text-on-surface" placeholder="Neon Drifter" prop:value={move || title.get()} on:input:target=move |ev| title.set(ev.target().value()) disabled={move || is_publishing.get()} />
                            </div>
                            <div>
                                <label for="publish-description" class="block text-xs font-bold uppercase tracking-widest text-primary mb-2">"Description (required)"</label>
                                <textarea id="publish-description" required=true class="w-full bg-surface-container-highest border-none rounded-md p-4 text-on-surface" rows=5 placeholder="Tell players about your game..." prop:value={move || description.get()} on:input:target=move |ev| description.set(ev.target().value()) disabled={move || is_publishing.get()} />
                            </div>
                            <div class="grid md:grid-cols-2 gap-5">
                                <div>
                                    <label for="publish-tags" class="block text-xs font-bold uppercase tracking-widest text-primary mb-2">"Tags"</label>
                                    <input id="publish-tags" class="w-full bg-surface-container-highest border-none rounded-md p-4 text-on-surface" placeholder="arcade, multiplayer" prop:value={move || tag_input.get()} on:input:target=move |ev| tag_input.set(ev.target().value()) disabled={move || is_publishing.get()} />
                                </div>
                                <div>
                                    <label for="publish-images" class="block text-xs font-bold uppercase tracking-widest text-primary mb-2">"Image URLs"</label>
                                    <input id="publish-images" aria-describedby="publish-images-help" class="w-full bg-surface-container-highest border-none rounded-md p-4 text-on-surface" placeholder="https://..." prop:value={move || image_input.get()} on:input:target=move |ev| image_input.set(ev.target().value()) disabled={move || is_publishing.get()} />
                                    <p id="publish-images-help" class="text-xs text-on-surface-variant mt-2">"Hosted HTTP(S) URLs only. The first image is the cover; additional URLs are screenshots."</p>
                                    {move || valid_cover_url(&parse_csv_values(&image_input.get())).map(|url| view! {
                                        <img class="mt-4 h-40 w-28 rounded-xl object-cover" src=url alt="Game cover preview" on:error=use_fallback_cover />
                                    })}
                                </div>
                            </div>
                        </div>
                    </section>
                    </Show>

                    <Show when=move || current_stage.get() == PublishStage::Builds>
                    <section class="v2-publish-stage bg-surface-container-high/60 backdrop-blur-2xl border border-outline-variant/15 rounded-3xl p-8">
                        <h2 class="text-2xl font-bold font-headline mb-2">"Builds and distribution"</h2>
                        <p class="text-sm text-on-surface-variant mb-6">"Enable automated installation only when you have a build and distribution provider. Otherwise this remains a metadata-only game page."</p>
                        <p class="text-xs text-on-surface-variant mb-6">{build_capability_message()}</p>
                        <div class="mb-6">
                            <label for="publish-platforms" class="block text-xs font-bold uppercase tracking-widest text-secondary mb-2">"Platforms"</label>
                            <input id="publish-platforms" aria-describedby="publish-platforms-help" class="w-full bg-surface-container-highest border-none rounded-md p-3 text-on-surface" placeholder="linux-x86_64, windows-x86_64" prop:value={move || platforms_input.get()} on:input:target=move |ev| platforms_input.set(ev.target().value()) />
                            <p id="publish-platforms-help" class="text-xs text-on-surface-variant mt-2">"Unique tags in <os>-<arch> format."</p>
                        </div>
                        <label class="flex items-center gap-3 p-4 rounded-xl bg-surface-container/50 mb-6">
                            <input type="checkbox" checked={move || fulfillment_enabled.get()} disabled=move || is_publishing.get() || existing_fulfillment_locked on:change:target=move |ev| {
                                let enabled = ev.target().checked();
                                fulfillment_enabled.set(enabled);
                                if !enabled { fulfillment_mode.set(FulfillmentMode::None); }
                            } />
                            <span class="font-bold">"Enable automated installation"</span>
                        </label>
                        {existing_fulfillment_locked.then(|| view! { <p class="mb-6 text-xs text-on-surface-variant">"This existing publishing authorization cannot be removed by an ordinary Game page update. Keep the current mode to reuse its key, or deliberately choose another mode to replace the authorization."</p> })}

                        <Show when=move || fulfillment_enabled.get()>
                            <div class="space-y-6">
                                <div class="grid md:grid-cols-2 gap-4">
                                    <button type="button" aria-pressed=move || fulfillment_mode.get() == FulfillmentMode::Direct class="rounded-xl bg-surface-container-highest p-4 text-left" on:click=move |_| fulfillment_mode.set(FulfillmentMode::Direct)>
                                        <span class="block font-bold text-secondary">"Use my active account"</span>
                                        <span class="text-xs text-on-surface-variant">"Publishing authorization is signed directly. Protocol detail: no provisioning event."</span>
                                    </button>
                                    <button type="button" aria-pressed=move || fulfillment_mode.get() == FulfillmentMode::Delegate class="rounded-xl bg-surface-container-highest p-4 text-left" on:click=move |_| fulfillment_mode.set(FulfillmentMode::Delegate)>
                                        <span class="block font-bold text-secondary">"Authorize a distribution provider"</span>
                                        <span class="text-xs text-on-surface-variant">"Protocol detail: provisions a key and publishes kind 30406 authorization."</span>
                                    </button>
                                </div>

                                <Show when=move || matches!(fulfillment_mode.get(), FulfillmentMode::Delegate)>
                                    <div class="rounded-2xl bg-surface-container/50 p-4 space-y-3">
                                        <label for="publish-provider-url" class="block text-xs font-bold uppercase tracking-widest text-secondary">"Distribution provider URL"</label>
                                        <div class="flex gap-2">
                                            <input id="publish-provider-url" class="flex-1 bg-surface-container-highest border-none rounded-md p-3 text-on-surface" placeholder="https://provider.example.com" prop:value={move || operator_url.get()} on:input:target=move |ev| {
                                                let next = ev.target().value();
                                                operator_url.set(next.clone());
                                                if operator_auto_added.get_untracked().is_some() { sync_operator_server(next); }
                                            } />
                                            <select aria-label="Copy provider URL from a selected server" class="bg-surface-container-highest border-none rounded-md p-3 text-on-surface" on:change:target=move |ev| {
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
                                         <h3 class="font-bold">"Discovered distribution providers"</h3>
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
                                        <label for="publish-custom-server" class="sr-only">"Custom distribution server URL"</label>
                                        <input id="publish-custom-server" class="flex-1 bg-surface-container-highest border-none rounded-md p-3 text-on-surface" placeholder="Add custom server URL" prop:value={move || custom_server.get()} on:input:target=move |ev| custom_server.set(ev.target().value()) />
                                        <button type="button" class="px-4 py-2 rounded-md bg-secondary text-on-secondary font-bold" on:click={on_add_custom_server}>"Add"</button>
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
                                                         <p class={reachability.class()}>{format!("Provider check: {}", reachability.label())}</p>
                                                         <p class={upload.class()}>{format!("Build upload: {}", upload.label())}</p>
                                                    </div>
                                                    <button type="button" class="text-error text-sm" on:click=move |_| remove_server(url.clone())>"Remove"</button>
                                                </div>
                                            }
                                        }).collect_view()}
                                    </div>
                                </div>

                                <div class="grid md:grid-cols-2 gap-5">
                                    <div>
                                        <span id="publish-build-label" class="block text-xs font-bold uppercase tracking-widest text-secondary mb-2">"Build file (required for automated installation)"</span>
                                        <button type="button" aria-labelledby="publish-build-label" aria-describedby="publish-hash-status" class="w-full rounded-md bg-surface-container-highest p-3 text-left" on:click={on_select_file} disabled={move || is_hashing.get() || is_publishing.get()}>
                                            {move || if is_hashing.get() { "Hashing…".to_string() } else { file_path.get().unwrap_or_else(|| if file_hash.get().is_some() { "Using existing build — select replacement".to_string() } else { "Select archive".to_string() }) }}
                                        </button>
                                        <p id="publish-hash-status" aria-live="polite" class="text-xs text-on-surface-variant mt-2">{move || if is_hashing.get() { "Computing the read-only SHA-256 hash…".to_string() } else { file_hash.get().map(|hash| format!("Read-only SHA-256: {}", format_sha256(&hash))).unwrap_or_else(|| "Hash appears after file selection.".to_string()) }}</p>
                                    </div>
                                    <div>
                                        <label for="publish-version" class="block text-xs font-bold uppercase tracking-widest text-secondary mb-2">"Version (required)"</label>
                                        <input id="publish-version" required=true class="w-full bg-surface-container-highest border-none rounded-md p-3 text-on-surface" placeholder="1.0.0" prop:value={move || version.get()} on:input:target=move |ev| version.set(ev.target().value()) />
                                    </div>
                                </div>
                            </div>
                        </Show>
                    </section>
                    </Show>
                </div>

                <aside class=move || if matches!(current_stage.get(), PublishStage::Pricing | PublishStage::Review) { "col-span-12 space-y-8" } else { "col-span-12 lg:col-span-4 space-y-8" }>
                    <Show when=move || current_stage.get() == PublishStage::Pricing>
                    <section class="v2-publish-stage bg-surface-container-high/60 backdrop-blur-2xl border border-outline-variant/15 rounded-3xl p-6">
                        <h2 class="text-2xl font-bold font-headline mb-2">"Pricing and access"</h2>
                        <p class="text-sm text-on-surface-variant mb-5">"Paid purchase and current access are independent. A zero price never makes access public."</p>
                        <div class="space-y-5">
                            <label class="flex items-center gap-3 rounded-xl bg-surface-container/50 p-4">
                                <input type="checkbox" checked={move || paid_enabled.get()} on:change:target=move |ev| paid_enabled.set(ev.target().checked()) disabled=move || is_publishing.get() />
                                <span><strong>"Enable paid purchase"</strong><span class="block text-xs text-on-surface-variant">"Disabled sends a price of 0 sats."</span></span>
                            </label>
                            <Show when=move || paid_enabled.get()>
                            <div>
                                <label for="publish-price" class="block text-[10px] font-bold uppercase tracking-widest text-secondary mb-2">"Price in sats (required)"</label>
                                <input id="publish-price" required=true class="w-full bg-surface-container-highest border-none rounded-md p-3 text-on-surface" type="number" min=1 step=1 prop:value={move || price_input.get()} on:input:target=move |ev| price_input.set(ev.target().value()) />
                            </div>
                            <div>
                                <label for="publish-lud16" class="block text-[10px] font-bold uppercase tracking-widest text-secondary mb-2">"Lightning address (required)"</label>
                                <input id="publish-lud16" required=true class="w-full bg-surface-container-highest border-none rounded-md p-3 text-on-surface" placeholder="you@example.com" prop:value={move || lud16.get()} on:input:target=move |ev| lud16.set(ev.target().value()) />
                            </div>
                            </Show>
                            <div>
                                <label class="block text-[10px] font-bold uppercase tracking-widest text-secondary mb-2" for="acquisition-policy">"Current access"</label>
                                <select id="acquisition-policy" class="w-full bg-surface-container-highest border-none rounded-md p-3 text-on-surface" prop:value=move || acquisition_kind.get().value() on:change:target=move |ev| acquisition_kind.set(AcquisitionKind::from_value(&ev.target().value())) disabled=move || is_publishing.get()>
                                    <option value="gated">"Gated"</option>
                                    <option value="public">"Public"</option>
                                    <option value="timed-access">"Timed"</option>
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
                                        <label class="block text-[10px] font-bold uppercase tracking-widest text-secondary mb-2" for="acquisition-start">"Access starts (required)"</label>
                                        <input id="acquisition-start" required=true class="w-full bg-surface-container-highest border-none rounded-md p-3 text-on-surface" type="datetime-local" step="1" prop:value=move || acquisition_starts_at.get() on:input:target=move |ev| acquisition_starts_at.set(ev.target().value()) disabled=move || is_publishing.get() />
                                    </div>
                                    <div>
                                        <label class="block text-[10px] font-bold uppercase tracking-widest text-secondary mb-2" for="acquisition-end">"Access ends (required)"</label>
                                        <input id="acquisition-end" required=true class="w-full bg-surface-container-highest border-none rounded-md p-3 text-on-surface" type="datetime-local" step="1" prop:value=move || acquisition_ends_at.get() on:input:target=move |ev| acquisition_ends_at.set(ev.target().value()) disabled=move || is_publishing.get() />
                                    </div>
                                    <p class="text-xs text-on-surface-variant">"Times use your local timezone."</p>
                                </div>
                            </Show>
                        </div>
                    </section>
                    </Show>

                    <Show when=move || current_stage.get() == PublishStage::Review>
                    <section class="v2-publish-stage bg-surface-container-high/60 backdrop-blur-2xl border border-outline-variant/15 rounded-3xl p-6">
                        <h2 class="text-2xl font-bold font-headline mb-2">"Review and publish"</h2>
                        <p class="text-sm text-on-surface-variant mb-5">"Review the game page and active-account authorization before network publication."</p>
                        <p class="text-[10px] font-bold uppercase tracking-widest text-tertiary mb-2">"Publishing authorization"</p>
                        <div class="bg-surface-container-highest rounded-md p-3 text-xs font-mono text-on-surface break-all">{move || auth.npub.get().unwrap_or_else(|| "Not authenticated".to_string())}</div>
                        <p class="text-xs text-on-surface-variant mt-3">"Read-only active account. Protocol detail: the signer publishes the Nostr events."</p>
                        <dl class="mt-5 space-y-3 text-sm">
                            <div><dt class="font-bold">"Identifier"</dt><dd class="text-on-surface-variant break-all">{move || id.get()}</dd></div>
                            <div><dt class="font-bold">"Metadata"</dt><dd class="text-on-surface-variant">{move || format!("{}; {} tags; {} images", title.get(), parse_csv_values(&tag_input.get()).len(), parse_csv_values(&image_input.get()).len())}</dd></div>
                            <div><dt class="font-bold">"Description"</dt><dd class="text-on-surface-variant whitespace-pre-wrap">{move || description.get()}</dd></div>
                            <div><dt class="font-bold">"Paid purchase"</dt><dd class="text-on-surface-variant">{move || if paid_enabled.get() { format!("{} sats via {}", price_input.get(), lud16.get()) } else { "Disabled (0 sats)".to_string() }}</dd></div>
                            <div><dt class="font-bold">"Current access"</dt><dd class="text-on-surface-variant">{move || match acquisition_kind.get() { AcquisitionKind::Gated => "Gated".to_string(), AcquisitionKind::Public => "Public".to_string(), AcquisitionKind::TimedAccess => format!("Timed from {} to {} (local)", acquisition_starts_at.get(), acquisition_ends_at.get()) }}</dd></div>
                            <div><dt class="font-bold">"Platforms / version"</dt><dd class="text-on-surface-variant">{move || format!("{} / {}", platforms_input.get(), version.get())}</dd></div>
                            <div><dt class="font-bold">"Distribution provider/server"</dt><dd class="text-on-surface-variant">{move || if fulfillment_enabled.get() { format!("{} server(s); {}", servers.get().len(), operator_url.get()) } else { "Metadata only".to_string() }}</dd></div>
                            <div><dt class="font-bold">"Build file / hash"</dt><dd class="text-on-surface-variant break-all">{move || format!("{} / {}", file_path.get().unwrap_or_else(|| "existing or none".into()), file_hash.get().map(|hash| format_sha256(&hash)).unwrap_or_else(|| "none".into()))}</dd></div>
                            <div><dt class="font-bold">"Authorization"</dt><dd class="text-on-surface-variant">{move || match fulfillment_mode.get() { FulfillmentMode::None => "Game page only", FulfillmentMode::Direct => "Active account", FulfillmentMode::Delegate => "Distribution provider" }}</dd></div>
                            <div><dt class="font-bold">"Promotion links"</dt><dd class="text-on-surface-variant">{format!("{} preserved", existing_campaigns.len())}</dd></div>
                            <div><dt class="font-bold">"Publishing authorization key"</dt><dd class="text-on-surface-variant break-all">{existing_fulfillment_pubkey.clone().unwrap_or_else(|| "Prepared during publication when required".into())}</dd></div>
                        </dl>
                        <h3 class="font-bold mt-6 mb-3">"Readiness checklist"</h3>
                        <ul class="space-y-2 text-sm">
                            <li>{move || if checklist().metadata { "✓ Game details" } else { "✕ Game details need attention" }}</li>
                            <li>{move || if checklist().pricing_and_access { "✓ Pricing and access" } else { "✕ Pricing or access needs attention" }}</li>
                            <li>{move || if checklist().distribution { "✓ Builds and distribution" } else { "✕ Builds or distribution needs attention" }}</li>
                            <li>{move || if checklist().authorization { "✓ Publishing authorization" } else { "✕ Publishing authorization needs attention" }}</li>
                        </ul>
                        <ul class="mt-3 text-xs text-on-surface-variant">{move || checklist().warnings.into_iter().map(|warning| view! { <li>{format!("Warning: {warning}")}</li> }).collect_view()}</ul>
                    </section>
                    </Show>

                    <section class="bg-gradient-to-br from-surface-container-high to-surface-container-lowest border border-outline-variant/10 rounded-3xl p-6" aria-labelledby="publication-status-title">
                        <h2 id="publication-status-title" class="text-lg font-bold font-headline mb-4">"Publication status"</h2>
                        {move || error_message.get().map(|msg| view! { <div class="mb-4 rounded-xl border border-error/30 bg-error-container/30 px-4 py-3 text-sm font-medium text-error">{msg}</div> })}
                        <div aria-live="polite" aria-atomic="true">
                        {move || publication_state.get().message.map(|msg| {
                            let class = match publication_state.get().outcome { PublicationOutcome::Complete => "text-secondary", PublicationOutcome::Partial | PublicationOutcome::Failed => "text-error", _ => "text-on-surface-variant" };
                            view! { <p class={class}>{msg}</p> }
                        })}
                        <ul class="mt-3 space-y-2 text-xs text-on-surface-variant">
                            {move || progress_events.get().into_iter().map(|event| view! {
                                <li>{progress_label(&event)}</li>
                            }).collect_view()}
                        </ul>
                        </div>
                    </section>
                </aside>
            </div>

            <div class="mt-6 flex flex-wrap items-center justify-between gap-3">
                <button type="button" class="px-6 py-3 rounded-md bg-surface-container-highest font-bold disabled:opacity-40" disabled=move || current_stage.get() == PublishStage::Details || is_publishing.get() on:click=move |_| { if let Some(previous) = current_stage.get_untracked().previous() { current_stage.set(previous); stage_error.set(None); } }>"Back"</button>
                <Show when=move || current_stage.get() != PublishStage::Review>
                    <button type="button" class="px-8 py-3 rounded-md bg-primary text-on-primary font-bold" on:click=on_next disabled=move || is_publishing.get()>"Continue"</button>
                </Show>
                <Show when=move || current_stage.get() == PublishStage::Review>
                    <button type="button" class="px-8 py-3 rounded-md bg-gradient-to-r from-primary to-primary-dim text-on-primary font-bold" on:click=move |_| on_submit.run(()) disabled=move || is_hashing.get() || is_publishing.get()>
                        {move || if is_publishing.get() { "Publishing to network…".to_string() } else { match (editing, fulfillment_enabled.get()) { (true, true) => "Update game page and distribution", (true, false) => "Update game page", (false, true) => "Publish game page and build", (false, false) => "Publish game page" }.to_string() }}
                    </button>
                </Show>
            </div>
        </main>
    }
}
