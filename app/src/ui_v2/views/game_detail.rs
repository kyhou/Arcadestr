use std::cell::RefCell;
use std::rc::Rc;

use leptos::prelude::*;
use send_wrapper::SendWrapper;
use wasm_bindgen_futures::spawn_local;

use crate::models::{
    npub_fallback_label, AcquisitionPolicy, GameDetailCommerce, GameDetailPresentation,
    GameListing, ListingSource, PlatformInfo, StorePageDetailState, StorePageListingRef,
    UserProfile,
};
use crate::store::try_use_profile_store;
use crate::tauri_bridge::{
    invoke_add_game_to_library, invoke_claim_entitlement, invoke_confirm_purchase,
    invoke_connect_nwc_wallet, invoke_discover_campaigns, invoke_enrich_store_page_detail,
    invoke_get_installed_games, invoke_get_listing_ownership, invoke_get_platform_info,
    invoke_install_game, invoke_is_game_in_library, invoke_pay_nwc_invoice,
    invoke_request_lnurl_invoice, listen_download_complete, listen_download_progress,
};
use crate::tauri_bridge::{
    CampaignPointerInput, ClaimEntitlementRequest, ConfirmPurchaseRequest, ConnectNwcWalletRequest,
    DiscoverCampaignsRequest, DiscoveredCampaign, DownloadProgressPayload, PayNwcInvoiceRequest,
    RequestLnurlInvoiceRequest,
};
use crate::ui_v2::components::{
    artwork_state_from_url, ArtworkRole, Dialog, DialogCloseAction, DialogCloseButtonPolicy,
    DialogClosePolicy, DialogCloseRequest, DialogDismissal, DialogSourcePolicy, DialogWidth,
    GameArtwork, StatusChip, StatusChipSize, StatusChipVariant, StorePageRichDetail,
};
use crate::ui_v2::views::browse_games::listing_categories;
use crate::ui_v2::views::use_fallback_cover;
use crate::{invoke_fetch_profile, AuthContext};

type DownloadCompleteCleanup = Rc<RefCell<Option<Box<dyn FnOnce()>>>>;
type DownloadProgressCleanup = Rc<RefCell<Option<Box<dyn FnOnce()>>>>;

#[derive(Clone, Debug, PartialEq, Eq)]
enum InstallFlowState {
    Preparing,
    Downloading { bytes: u64, total: Option<u64> },
    Finalizing,
    Completed,
    Failed(InstallFailurePresentation),
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct InstallFailurePresentation {
    title: &'static str,
    message: &'static str,
    retryable: bool,
}

/// The declared dismissal contract for the install dialog.
///
/// The current installer offers no cancellation, so while an attempt is active
/// nothing dismisses the dialog and the close control says why.
fn install_close_contract() -> (DialogClosePolicy, DialogDismissal) {
    (
        DialogClosePolicy::BlockedWhileBusy,
        DialogDismissal {
            escape: DialogSourcePolicy::Allowed,
            backdrop: DialogSourcePolicy::Ignored,
            close_button: DialogCloseButtonPolicy::DisabledWhileBusy,
        },
    )
}

impl InstallFlowState {
    /// Overline shown above the dialog title.
    fn kicker(&self) -> &'static str {
        match self {
            Self::Preparing | Self::Downloading { .. } => "Verified download",
            Self::Finalizing => "Integrity check",
            Self::Completed => "Complete",
            Self::Failed(_) => "Download failed",
        }
    }

    fn title(&self) -> &'static str {
        match self {
            Self::Preparing => "Preparing download",
            Self::Downloading { .. } => "Downloading artifact",
            Self::Finalizing => "Verifying and registering",
            Self::Completed => "Artifact verified and registered",
            Self::Failed(failure) => failure.title,
        }
    }
}

fn install_flow_from_progress(payload: &DownloadProgressPayload) -> InstallFlowState {
    if payload
        .total
        .is_some_and(|total| total > 0 && payload.bytes >= total)
    {
        InstallFlowState::Finalizing
    } else {
        InstallFlowState::Downloading {
            bytes: payload.bytes,
            total: payload.total,
        }
    }
}

fn download_progress_percent(bytes: u64, total: Option<u64>) -> Option<u8> {
    let total = total.filter(|total| *total > 0)?;
    Some(((bytes.min(total) as u128 * 100) / total as u128) as u8)
}

fn format_download_bytes(bytes: u64) -> String {
    const KIB: f64 = 1024.0;
    const MIB: f64 = KIB * 1024.0;
    const GIB: f64 = MIB * 1024.0;
    let bytes = bytes as f64;
    if bytes >= GIB {
        format!("{:.1} GiB", bytes / GIB)
    } else if bytes >= MIB {
        format!("{:.1} MiB", bytes / MIB)
    } else if bytes >= KIB {
        format!("{:.1} KiB", bytes / KIB)
    } else {
        format!("{} B", bytes as u64)
    }
}

fn install_failure_presentation(error: &str) -> InstallFailurePresentation {
    let normalized = error.to_lowercase();
    if normalized.contains("hash")
        || normalized.contains("integrity")
        || normalized.contains("mismatch")
        || normalized.contains("quarantine")
    {
        InstallFailurePresentation {
            title: "Integrity verification failed",
            message: "The downloaded artifact was rejected and was not registered as installed.",
            retryable: true,
        }
    } else if normalized.contains("auth")
        || normalized.contains("ownership")
        || normalized.contains("entitlement")
        || normalized.contains("unauthorized")
    {
        InstallFailurePresentation {
            title: "Download authorization failed",
            message: "Current access could not be verified. No installation was recorded.",
            retryable: true,
        }
    } else if normalized.contains("platform") || normalized.contains("unsupported") {
        InstallFailurePresentation {
            title: "Artifact is not compatible",
            message: "The available artifact does not support this device.",
            retryable: false,
        }
    } else {
        InstallFailurePresentation {
            title: "Download could not finish",
            message:
                "The artifact was not verified or registered. Retry when the source is available.",
            retryable: true,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum DetailOperation {
    #[default]
    Idle,
    RequestingInvoice,
    ConnectingWallet,
    PayingWallet,
    ConfirmingPurchase,
    Claiming,
    AddingToLibrary,
    Installing,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum PrimaryAction {
    CheckingOwnership,
    CheckingInstallation,
    Buy,
    Claim,
    Install,
    Installed,
    Incompatible,
    UnsupportedWeb,
    TimedUpcoming,
    TimedExpired,
    SignIn,
    NoPaymentAddress,
    Unavailable,
    LocalStateUnavailable,
    Busy(DetailOperation),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum DetailCompatibility {
    Compatible,
    Incompatible,
    Unknown,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct PrimaryActionDecision {
    action: PrimaryAction,
    label: &'static str,
    explanation: &'static str,
    enabled: bool,
}

fn operation_blocks_dispatch(operation: DetailOperation) -> bool {
    operation != DetailOperation::Idle
}

fn account_response_is_current(active_account: Option<&str>, initiating_account: &str) -> bool {
    active_account == Some(initiating_account)
}

fn active_campaign(campaigns: &[DiscoveredCampaign]) -> Option<DiscoveredCampaign> {
    campaigns
        .iter()
        .find(|campaign| campaign.classification == "active")
        .cloned()
}

fn select_primary_action(
    listing: &GameListing,
    owned: bool,
    installed: bool,
    ownership_loading: bool,
    installation_loading: bool,
    installation_error: bool,
    campaigns: &[DiscoveredCampaign],
    campaign_pending: bool,
    compatibility: DetailCompatibility,
    operation: DetailOperation,
    standalone_web: bool,
    authenticated: bool,
    now: u64,
) -> PrimaryActionDecision {
    if operation_blocks_dispatch(operation) {
        let label = match operation {
            DetailOperation::RequestingInvoice => "Requesting invoice...",
            DetailOperation::ConnectingWallet => "Connecting wallet...",
            DetailOperation::PayingWallet => "Paying invoice...",
            DetailOperation::ConfirmingPurchase => "Recording ownership...",
            DetailOperation::Claiming => "Claiming access...",
            DetailOperation::AddingToLibrary => "Adding to library...",
            DetailOperation::Installing => "Preparing download...",
            DetailOperation::Idle => "Working...",
        };
        return PrimaryActionDecision {
            action: PrimaryAction::Busy(operation),
            label,
            explanation: "Please wait for the current operation to finish.",
            enabled: false,
        };
    }
    if ownership_loading {
        return PrimaryActionDecision {
            action: PrimaryAction::CheckingOwnership,
            label: "Checking ownership...",
            explanation: "Checking durable access for the active account.",
            enabled: false,
        };
    }
    if installed {
        return PrimaryActionDecision {
            action: PrimaryAction::Installed,
            label: "Installed",
            explanation: "The game is installed on this device.",
            enabled: false,
        };
    }
    if installation_loading {
        return PrimaryActionDecision {
            action: PrimaryAction::CheckingInstallation,
            label: "Checking this device...",
            explanation: "Reading the local installation registry before enabling a download.",
            enabled: false,
        };
    }
    if installation_error {
        return PrimaryActionDecision {
            action: PrimaryAction::LocalStateUnavailable,
            label: "Installation status unavailable",
            explanation: "Retry the local registry lookup before downloading this artifact.",
            enabled: false,
        };
    }
    if standalone_web {
        return PrimaryActionDecision {
            action: PrimaryAction::UnsupportedWeb,
            label: "Desktop app required",
            explanation:
                "Purchases, claims, and native installation are available in the desktop app.",
            enabled: false,
        };
    }
    if compatibility == DetailCompatibility::Incompatible {
        return PrimaryActionDecision {
            action: PrimaryAction::Incompatible,
            label: "Not compatible",
            explanation: "This build does not support the detected platform.",
            enabled: false,
        };
    }
    if compatibility == DetailCompatibility::Unknown {
        return PrimaryActionDecision {
            action: PrimaryAction::Incompatible,
            label: "Compatibility unknown",
            explanation: "Platform detection must complete before this build can be installed.",
            enabled: false,
        };
    }
    if !owned && campaign_pending {
        return PrimaryActionDecision {
            action: PrimaryAction::Unavailable,
            label: "Checking offers...",
            explanation: "Checking signed campaign events before choosing an access path.",
            enabled: false,
        };
    }
    if !owned
        && matches!(listing.acquisition, AcquisitionPolicy::Gated)
        && listing.has_declared_price()
        && listing.price_sats == 0
    {
        return PrimaryActionDecision {
            action: PrimaryAction::Unavailable,
            label: "Purchase unavailable",
            explanation: "This checkout supports SATS-priced listings only.",
            enabled: false,
        };
    }

    let desired = if owned {
        PrimaryAction::Install
    } else if active_campaign(campaigns).is_some() {
        PrimaryAction::Claim
    } else {
        match listing.acquisition {
            AcquisitionPolicy::Public => PrimaryAction::Install,
            AcquisitionPolicy::TimedAccess { starts_at, .. } if now < starts_at => {
                PrimaryAction::TimedUpcoming
            }
            AcquisitionPolicy::TimedAccess { ends_at, .. } if now >= ends_at => {
                PrimaryAction::TimedExpired
            }
            AcquisitionPolicy::TimedAccess { .. } => PrimaryAction::Install,
            AcquisitionPolicy::Gated
                if listing.price_sats > 0 && listing.lud16.trim().is_empty() =>
            {
                PrimaryAction::NoPaymentAddress
            }
            AcquisitionPolicy::Gated if listing.price_sats > 0 => PrimaryAction::Buy,
            AcquisitionPolicy::Gated => PrimaryAction::Unavailable,
        }
    };

    if !authenticated
        && matches!(
            desired,
            PrimaryAction::Buy | PrimaryAction::Claim | PrimaryAction::Install
        )
    {
        return PrimaryActionDecision {
            action: PrimaryAction::SignIn,
            label: "Sign in required",
            explanation: "Sign in before purchasing, claiming, or installing this game.",
            enabled: false,
        };
    }

    match desired {
        PrimaryAction::Buy => PrimaryActionDecision {
            action: desired,
            label: "Buy with Lightning",
            explanation: "Payment is confirmed before durable ownership is recorded.",
            enabled: true,
        },
        PrimaryAction::Claim => PrimaryActionDecision {
            action: desired,
            label: "Claim and keep",
            explanation:
                "Claim during the active promotion to add permanent access to this account.",
            enabled: true,
        },
        PrimaryAction::Install => PrimaryActionDecision {
            action: desired,
            label: if matches!(listing.acquisition, AcquisitionPolicy::TimedAccess { .. }) && !owned
            {
                "Download while available"
            } else {
                "Download"
            },
            explanation: if matches!(listing.acquisition, AcquisitionPolicy::Public) {
                "Downloads, verifies, and registers the current artifact on this device."
            } else if owned {
                "Ownership is confirmed before the artifact is downloaded and verified."
            } else {
                "Access depends on the listing's current public or timed policy."
            },
            enabled: true,
        },
        PrimaryAction::TimedUpcoming => PrimaryActionDecision {
            action: desired,
            label: "Access not started",
            explanation: "Timed access begins at the publisher's configured start time.",
            enabled: false,
        },
        PrimaryAction::TimedExpired => PrimaryActionDecision {
            action: desired,
            label: "Access ended",
            explanation: "The timed-access interval has expired and did not create ownership.",
            enabled: false,
        },
        PrimaryAction::NoPaymentAddress => PrimaryActionDecision {
            action: desired,
            label: "Purchase unavailable",
            explanation: "The seller has not provided a Lightning payment address.",
            enabled: false,
        },
        PrimaryAction::Unavailable => PrimaryActionDecision {
            action: desired,
            label: "Access unavailable",
            explanation: "This listing has no current purchase or ungated access path.",
            enabled: false,
        },
        action => PrimaryActionDecision {
            action,
            label: "Unavailable",
            explanation: "No action is currently available.",
            enabled: false,
        },
    }
}

fn current_unix_secs() -> u64 {
    #[cfg(target_arch = "wasm32")]
    {
        (js_sys::Date::now() / 1000.0) as u64
    }

    #[cfg(not(target_arch = "wasm32"))]
    {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|duration| duration.as_secs())
            .unwrap_or_default()
    }
}

fn game_coordinate(listing: &GameListing) -> Option<String> {
    use nostr::nips::nip19::FromBech32;

    nostr::PublicKey::from_bech32(&listing.publisher_npub)
        .ok()
        .map(|publisher| format!("30402:{}:{}", publisher.to_hex(), listing.id))
}

fn show_add_to_library(
    listing: &GameListing,
    authenticated: bool,
    standalone_web: bool,
    has_coordinate: bool,
    loading: bool,
    saved: bool,
    owned: bool,
) -> bool {
    matches!(listing.acquisition, AcquisitionPolicy::Public)
        && authenticated
        && !standalone_web
        && has_coordinate
        && !loading
        && !saved
        && !owned
}

fn format_date(ts: u64) -> String {
    #[cfg(target_arch = "wasm32")]
    {
        let date = js_sys::Date::new(&(ts as f64 * 1000.0).into());
        let year = date.get_full_year();
        let month = date.get_month() + 1;
        let day = date.get_date();
        format!("{month:02}/{day:02}/{year}")
    }

    #[cfg(not(target_arch = "wasm32"))]
    {
        let years_since_epoch = ts / 31_556_952;
        let year = 1970 + years_since_epoch;
        year.to_string()
    }
}

fn format_timestamp(ts: u64) -> String {
    format!("Release date: {}", format_date(ts))
}

fn valid_image_urls(images: &[String]) -> Vec<String> {
    let mut valid = Vec::new();
    for candidate in images {
        let trimmed = candidate.trim();
        let Some(parsed) = url::Url::parse(trimmed).ok() else {
            continue;
        };
        if !matches!(parsed.scheme(), "http" | "https") {
            continue;
        }
        let Some(host) = parsed.host_str() else {
            continue;
        };
        if host == "example"
            || host.ends_with(".example")
            || host == "example.com"
            || host.ends_with(".example.com")
        {
            continue;
        }
        if !valid.iter().any(|url| url == trimmed) {
            valid.push(trimmed.to_string());
        }
    }
    valid
}

fn seller_display(profile: Option<&UserProfile>, listing: &GameListing) -> String {
    profile
        .map(UserProfile::display)
        .filter(|display| !display.trim().is_empty())
        .or_else(|| {
            listing
                .stall_name
                .clone()
                .filter(|name| !name.trim().is_empty())
        })
        .unwrap_or_else(|| npub_fallback_label(&listing.publisher_npub))
}

fn listing_compatibility(
    listing: &GameListing,
    platform: Option<&PlatformInfo>,
) -> DetailCompatibility {
    if listing.platforms.is_empty() {
        DetailCompatibility::Compatible
    } else if let Some(platform) = platform {
        if listing.platforms.iter().any(|tag| tag == &platform.tag()) {
            DetailCompatibility::Compatible
        } else {
            DetailCompatibility::Incompatible
        }
    } else {
        DetailCompatibility::Unknown
    }
}

fn technical_fields(listing: &GameListing) -> Vec<(String, String)> {
    let mut fields = Vec::new();
    if !listing.platforms.is_empty() {
        fields.push((
            "Supported platforms".to_string(),
            listing.platforms.join(", "),
        ));
    }
    for (key, label) in [
        ("version", "Version"),
        ("sha256", "File hash"),
        ("hash", "File hash"),
        ("server", "Distribution provider"),
    ] {
        if let Some((_, value)) = listing.specs.iter().find(|(spec_key, value)| {
            spec_key.eq_ignore_ascii_case(key) && !value.trim().is_empty()
        }) {
            if !fields.iter().any(|(existing, _)| existing == label) {
                fields.push((label.to_string(), value.clone()));
            }
        }
    }
    if let Some(event_id) = listing
        .event_id
        .as_ref()
        .filter(|value| !value.trim().is_empty())
    {
        fields.push(("Listing event".to_string(), event_id.clone()));
    }
    if let Some(event_id) = listing
        .nip94_event_id
        .as_ref()
        .filter(|value| !value.trim().is_empty())
    {
        fields.push(("File metadata event".to_string(), event_id.clone()));
    }
    fields.push(("Listing identifier".to_string(), listing.id.clone()));
    fields.push((
        "Published".to_string(),
        format_timestamp(listing.created_at),
    ));
    fields
}

fn operation_status(operation: DetailOperation) -> &'static str {
    match operation {
        DetailOperation::Idle => "Ready",
        DetailOperation::RequestingInvoice => "Requesting a Lightning invoice...",
        DetailOperation::ConnectingWallet => "Connecting to the wallet...",
        DetailOperation::PayingWallet => "Sending the Lightning payment...",
        DetailOperation::ConfirmingPurchase => "Confirming payment and recording ownership...",
        DetailOperation::Claiming => "Claiming permanent access...",
        DetailOperation::AddingToLibrary => "Adding the game to your library...",
        DetailOperation::Installing => "Downloading and verifying the game artifact...",
    }
}

fn truncate_chars(value: &str, max_chars: usize) -> String {
    let mut chars = value.chars();
    let truncated = chars.by_ref().take(max_chars).collect::<String>();
    if chars.next().is_some() {
        format!("{truncated}...")
    } else {
        truncated
    }
}

fn adp_server_url_from_download_url(download_url: &str) -> Option<String> {
    let marker = "/game/";
    download_url
        .find(marker)
        .and_then(|index| normalize_adp_server_url(&download_url[..index]))
}

fn normalize_adp_server_url(value: &str) -> Option<String> {
    let value = value.trim().trim_end_matches('/');
    let parsed = url::Url::parse(value).ok()?;
    matches!(parsed.scheme(), "http" | "https")
        .then(|| parsed.host_str())
        .flatten()
        .map(|_| value.to_string())
}

fn adp_server_url(specs: &[(String, String)], download_url: &str) -> Option<String> {
    specs
        .iter()
        .filter(|(key, _)| key == "server")
        .find_map(|(_, url)| normalize_adp_server_url(url))
        .or_else(|| adp_server_url_from_download_url(download_url))
}

fn detail_commerce(listing: &GameListing) -> Option<GameDetailCommerce> {
    Some(GameDetailCommerce {
        listing_coordinate: game_coordinate(listing)?,
        price_sats: listing.price_sats,
        acquisition: listing.acquisition.clone(),
        owned: listing.is_owned,
        platforms: listing.platforms.clone(),
        version: listing
            .specs
            .iter()
            .find(|(key, _)| key == "version")
            .map(|(_, value)| value.clone()),
        distribution_available: listing.specs.iter().any(|(key, _)| key == "server"),
        file_hash: listing
            .specs
            .iter()
            .find(|(key, _)| matches!(key.as_str(), "sha256" | "hash"))
            .map(|(_, value)| value.clone()),
    })
}

fn detail_response_is_current(
    current_generation: u64,
    requested_generation: u64,
    response_generation: u64,
    coordinate: &str,
    event_id: &str,
    presentation: &GameDetailPresentation,
) -> bool {
    current_generation == requested_generation
        && response_generation == requested_generation
        && presentation.listing_coordinate == coordinate
        && presentation.listing_event_id == event_id
}

#[component]
pub fn GameDetailView(listing: GameListing, on_back: Callback<()>) -> impl IntoView {
    let media = valid_image_urls(&listing.images);
    let hero_image = media.first().cloned();
    let gallery_images = media.into_iter().skip(1).collect::<Vec<_>>();
    let categories = listing_categories(&listing)
        .into_iter()
        .map(|category| category.label)
        .collect::<Vec<_>>();
    let release_label = format_timestamp(listing.created_at);
    let protocol_label = match listing.source {
        ListingSource::Nip15Product => "NIP-15",
        ListingSource::Nip99Listing => "NIP-99",
        ListingSource::Legacy => "NIP-01",
    };
    let technical = technical_fields(&listing);
    let publisher_npub = listing.publisher_npub.clone();
    let title = listing.title.clone();
    let description = listing.description.clone();

    // Buy flow state.
    let invoice: RwSignal<Option<String>> = RwSignal::new(None);
    let buy_error: RwSignal<Option<String>> = RwSignal::new(None);
    let success_message: RwSignal<Option<String>> = RwSignal::new(None);
    let show_invoice: RwSignal<bool> = RwSignal::new(false);
    let nwc_connection_input = RwSignal::new(String::new());
    let nwc_connected: RwSignal<bool> = RwSignal::new(false);
    let manual_preimage = RwSignal::new(String::new());
    let purchase_confirmed: RwSignal<bool> = RwSignal::new(listing.is_owned);
    let ownership_loading: RwSignal<bool> = RwSignal::new(!cfg!(feature = "web"));
    let library_added: RwSignal<bool> = RwSignal::new(false);
    let library_loading: RwSignal<bool> = RwSignal::new(!cfg!(feature = "web"));
    let install_complete: RwSignal<bool> = RwSignal::new(false);
    let install_state_loading: RwSignal<bool> = RwSignal::new(!cfg!(feature = "web"));
    let install_state_error = RwSignal::new(false);
    let install_state_refresh = RwSignal::new(0_u64);
    let operation = RwSignal::new(DetailOperation::Idle);
    let install_flow = RwSignal::new(None::<InstallFlowState>);
    let install_attempt_active = RwSignal::new(false);
    let install_attempt_account = RwSignal::new(None::<String>);
    let campaigns: RwSignal<Vec<DiscoveredCampaign>> = RwSignal::new(Vec::new());
    let campaign_loading: RwSignal<bool> = RwSignal::new(true);
    let campaign_error: RwSignal<Option<String>> = RwSignal::new(None);
    let campaign_refresh = RwSignal::new(0_u64);
    let pending_claim: RwSignal<Option<DiscoveredCampaign>> = RwSignal::new(None);
    let platform_info: RwSignal<Option<PlatformInfo>> = RwSignal::new(None);
    let platform_loading = RwSignal::new(!cfg!(feature = "web"));
    let platform_error = RwSignal::new(false);
    let platform_refresh = RwSignal::new(0_u64);
    let decision_time = RwSignal::new(current_unix_secs());

    #[cfg(target_arch = "wasm32")]
    {
        let clock = SendWrapper::new(gloo_timers::callback::Interval::new(30_000, move || {
            decision_time.set(current_unix_secs());
        }));
        on_cleanup(move || drop(clock));
    }

    // Seller profile state.
    let seller_profile: RwSignal<Option<UserProfile>> = RwSignal::new(None);
    let profile_loading: RwSignal<bool> = RwSignal::new(true);
    let profile_error: RwSignal<bool> = RwSignal::new(false);

    let auth = use_context::<AuthContext>().expect("AuthContext not provided");
    let detail_presentation = RwSignal::new(None::<GameDetailPresentation>);
    let detail_generation = RwSignal::new(0_u64);
    let listing_event_current = RwSignal::new(cfg!(feature = "web"));
    let listing_validation_loading = RwSignal::new(!cfg!(feature = "web"));
    let listing_validation_error = RwSignal::new(false);
    let listing_validation_refresh = RwSignal::new(0_u64);
    let detail_coordinate = game_coordinate(&listing);
    let detail_event_id = listing.event_id.clone();
    let _commerce = detail_commerce(&listing);

    let install_auth = auth.clone();
    Effect::new(move |_| {
        let active_account = install_auth.npub.get();
        if install_attempt_active.get_untracked()
            && install_attempt_account.get_untracked() != active_account
        {
            install_attempt_active.set(false);
            install_flow.set(None);
            operation.set(DetailOperation::Idle);
        }
    });

    Effect::new(move |_| {
        let _account = auth.npub.get();
        let _ = listing_validation_refresh.get();
        detail_generation.update(|value| *value = value.wrapping_add(1));
        listing_event_current.set(cfg!(feature = "web"));
        listing_validation_loading.set(!cfg!(feature = "web"));
        listing_validation_error.set(false);
        let requested_generation = detail_generation.get_untracked();
        let (Some(coordinate), Some(event_id)) =
            (detail_coordinate.clone(), detail_event_id.clone())
        else {
            detail_presentation.set(None);
            listing_validation_loading.set(false);
            listing_validation_error.set(true);
            return;
        };
        spawn_local(async move {
            let response = invoke_enrich_store_page_detail(
                requested_generation,
                StorePageListingRef {
                    listing_coordinate: coordinate.clone(),
                    listing_event_id: event_id.clone(),
                },
            )
            .await;
            let Ok(response) = response else {
                if detail_generation.get_untracked() == requested_generation {
                    listing_validation_loading.set(false);
                    listing_validation_error.set(true);
                }
                return;
            };
            if detail_generation.get_untracked() != requested_generation
                || response.generation != requested_generation
            {
                return;
            }
            listing_event_current.set(response.listing_event_current);
            listing_validation_error.set(!response.listing_event_current);
            if let Some(cached) = response.cached {
                if detail_response_is_current(
                    detail_generation.get_untracked(),
                    requested_generation,
                    response.generation,
                    &coordinate,
                    &event_id,
                    &cached,
                ) {
                    detail_presentation.set(Some(cached));
                }
            }
            match response.refreshed {
                StorePageDetailState::Enriched(presentation)
                    if detail_response_is_current(
                        detail_generation.get_untracked(),
                        requested_generation,
                        response.generation,
                        &coordinate,
                        &event_id,
                        &presentation,
                    ) =>
                {
                    detail_presentation.set(Some(presentation));
                }
                StorePageDetailState::Unavailable => {}
                StorePageDetailState::Enriched(_) => {}
                StorePageDetailState::NotAssociated
                | StorePageDetailState::NotFound
                | StorePageDetailState::Invalid
                | StorePageDetailState::Unsupported => detail_presentation.set(None),
            }
            listing_validation_loading.set(false);
        });
    });
    on_cleanup(move || detail_generation.update(|value| *value = value.wrapping_add(1)));

    Effect::new(move |_| {
        let requested_refresh = platform_refresh.get();
        platform_loading.set(!cfg!(feature = "web"));
        platform_error.set(false);
        spawn_local(async move {
            let result = invoke_get_platform_info().await;
            if platform_refresh.get_untracked() != requested_refresh {
                return;
            }
            match result {
                Ok(info) => platform_info.set(Some(info)),
                Err(_) => platform_error.set(true),
            }
            platform_loading.set(false);
        });
    });

    let ownership_publisher = listing.publisher_npub.clone();
    let ownership_listing_id = listing.id.clone();
    let ownership_auth = auth.clone();
    Effect::new(move |_| {
        let requested_account = ownership_auth.npub.get();
        purchase_confirmed.set(false);
        buy_error.set(None);

        let Some(requested_account) = requested_account else {
            ownership_loading.set(false);
            return;
        };

        ownership_loading.set(true);
        let publisher_npub = ownership_publisher.clone();
        let listing_id = ownership_listing_id.clone();
        let auth_for_response = ownership_auth.clone();
        spawn_local(async move {
            let result =
                invoke_get_listing_ownership(requested_account.clone(), publisher_npub, listing_id)
                    .await;
            if auth_for_response.npub.get_untracked().as_deref() != Some(requested_account.as_str())
            {
                return;
            }

            match result {
                Ok(is_owned) => purchase_confirmed.set(is_owned),
                Err(error) => buy_error.set(Some(format!("Ownership lookup failed: {error}"))),
            }
            ownership_loading.set(false);
        });
    });

    let install_game_coordinate = game_coordinate(&listing);
    let library_coordinate = install_game_coordinate.clone();
    let library_auth = auth.clone();
    Effect::new(move |_| {
        let requested_account = library_auth.npub.get();
        library_added.set(false);

        let (Some(requested_account), Some(coordinate)) =
            (requested_account, library_coordinate.clone())
        else {
            library_loading.set(false);
            return;
        };

        library_loading.set(true);
        let auth_for_response = library_auth.clone();
        spawn_local(async move {
            let result = invoke_is_game_in_library(coordinate).await;
            if auth_for_response.npub.get_untracked().as_deref() != Some(requested_account.as_str())
            {
                return;
            }
            match result {
                Ok(in_library) => library_added.set(in_library),
                Err(error) => buy_error.set(Some(format!("Library lookup failed: {error}"))),
            }
            library_loading.set(false);
        });
    });

    if let Some(coordinate) = install_game_coordinate.clone() {
        Effect::new(move |_| {
            let requested_refresh = install_state_refresh.get();
            let coordinate = coordinate.clone();
            install_state_loading.set(true);
            install_state_error.set(false);
            spawn_local(async move {
                let result = invoke_get_installed_games().await;
                if install_state_refresh.get_untracked() != requested_refresh {
                    return;
                }
                match result {
                    Ok(installed_games) => install_complete.set(
                        installed_games
                            .iter()
                            .any(|game| game.game_coordinate == coordinate),
                    ),
                    Err(_) => install_state_error.set(true),
                }
                install_state_loading.set(false);
            });
        });
    } else {
        install_state_loading.set(false);
    }

    let on_buy = {
        let listing = listing.clone();

        Callback::new(move |()| {
            if operation_blocks_dispatch(operation.get_untracked()) {
                return;
            }
            if auth.npub.get().is_none() {
                buy_error.set(Some("Not authenticated".to_string()));
                return;
            }
            if listing.price_sats == 0 {
                buy_error.set(Some("Free downloads are handled by Install.".to_string()));
                return;
            }
            if listing.lud16.trim().is_empty() {
                buy_error.set(Some("No Lightning address".to_string()));
                return;
            }

            operation.set(DetailOperation::RequestingInvoice);
            buy_error.set(None);
            success_message.set(None);
            show_invoice.set(false);

            let lud16 = listing.lud16.clone();
            let amount_sats = listing.price_sats;
            spawn_local(async move {
                match invoke_request_lnurl_invoice(RequestLnurlInvoiceRequest {
                    lud16,
                    amount_sats,
                })
                .await
                {
                    Ok(response) => {
                        invoice.set(Some(response.bolt11));
                        show_invoice.set(true);
                        operation.set(DetailOperation::Idle);
                    }
                    Err(e) => {
                        buy_error.set(Some(e));
                        operation.set(DetailOperation::Idle);
                    }
                }
            });
        })
    };

    let on_copy_invoice = Callback::new(move |()| {
        #[cfg(target_arch = "wasm32")]
        {
            if let Some(bolt11) = invoice.get() {
                if let Some(window) = leptos::web_sys::window() {
                    let _ = window.navigator().clipboard().write_text(&bolt11);
                }
            }
        }

        #[cfg(not(target_arch = "wasm32"))]
        {
            let _ = invoice;
        }
    });

    let on_open_wallet = Callback::new(move |()| {
        #[cfg(target_arch = "wasm32")]
        {
            if let Some(bolt11) = invoice.get() {
                let lightning_uri = format!("lightning:{}", bolt11);
                if let Some(win) = leptos::web_sys::window() {
                    let _ = win.location().set_href(&lightning_uri);
                }
            }
        }

        #[cfg(not(target_arch = "wasm32"))]
        {
            let _ = invoice;
        }
    });

    let confirm_auth = auth.clone();
    let confirm_with_preimage = {
        let listing = listing.clone();
        let confirm_auth = confirm_auth.clone();
        move |preimage: String| {
            let Some(initiating_account) = confirm_auth.npub.get() else {
                buy_error.set(Some("Not authenticated".to_string()));
                return;
            };
            let Some(bolt11) = invoice.get() else {
                buy_error.set(Some(
                    "Request an invoice before confirming purchase".to_string(),
                ));
                return;
            };
            let publisher_npub = listing.publisher_npub.clone();
            let listing_id = listing.id.clone();
            let server_url = match adp_server_url(&listing.specs, &listing.download_url) {
                Some(server_url) => server_url,
                None => {
                    buy_error.set(Some(
                        "Listing does not include an ADP server URL".to_string(),
                    ));
                    return;
                }
            };

            operation.set(DetailOperation::ConfirmingPurchase);
            buy_error.set(None);
            success_message.set(None);
            let auth_for_response = confirm_auth.clone();
            spawn_local(async move {
                let request = ConfirmPurchaseRequest {
                    publisher_npub,
                    listing_id,
                    server_url,
                    bolt11,
                    preimage,
                };
                let result = invoke_confirm_purchase(request).await;
                if !account_response_is_current(
                    auth_for_response.npub.get_untracked().as_deref(),
                    &initiating_account,
                ) {
                    operation.set(DetailOperation::Idle);
                    return;
                }
                match result {
                    Ok(_) => {
                        purchase_confirmed.set(true);
                        success_message.set(Some(
                            "Payment confirmed. Durable ownership is ready for installation."
                                .to_string(),
                        ));
                        operation.set(DetailOperation::Idle);
                    }
                    Err(err) => {
                        let message = if err.contains("402") {
                            "Payment proof was rejected. Check the invoice preimage.".to_string()
                        } else if err.contains("409") {
                            purchase_confirmed.set(true);
                            "Purchase already confirmed. You can install this game.".to_string()
                        } else if err.contains("404") {
                            "This game is not hosted on the selected ADP server.".to_string()
                        } else if err.contains("500") {
                            "The ADP server is not authorized to fulfill this listing.".to_string()
                        } else {
                            err
                        };
                        buy_error.set(Some(message));
                        operation.set(DetailOperation::Idle);
                    }
                }
            });
        }
    };

    let on_connect_nwc = Callback::new(move |()| {
        if operation_blocks_dispatch(operation.get_untracked()) {
            return;
        }
        let connection_string = nwc_connection_input.get();
        if connection_string.trim().is_empty() {
            buy_error.set(Some(
                "Paste a Nostr Wallet Connect string first".to_string(),
            ));
            return;
        }
        operation.set(DetailOperation::ConnectingWallet);
        buy_error.set(None);
        spawn_local(async move {
            match invoke_connect_nwc_wallet(ConnectNwcWalletRequest { connection_string }).await {
                Ok(_) => {
                    nwc_connected.set(true);
                    operation.set(DetailOperation::Idle);
                }
                Err(err) => {
                    buy_error.set(Some(err));
                    operation.set(DetailOperation::Idle);
                }
            }
        });
    });

    let on_pay_nwc = {
        let confirm_with_preimage = confirm_with_preimage.clone();
        let pay_auth = auth.clone();
        Callback::new(move |()| {
            if operation_blocks_dispatch(operation.get_untracked()) {
                return;
            }
            let Some(bolt11) = invoice.get() else {
                buy_error.set(Some("Request an invoice before paying".to_string()));
                return;
            };
            let Some(initiating_account) = pay_auth.npub.get() else {
                buy_error.set(Some("Not authenticated".to_string()));
                return;
            };
            operation.set(DetailOperation::PayingWallet);
            buy_error.set(None);
            let confirm_with_preimage = confirm_with_preimage.clone();
            let auth_for_response = pay_auth.clone();
            spawn_local(async move {
                match invoke_pay_nwc_invoice(PayNwcInvoiceRequest { bolt11 }).await {
                    Ok(result) => {
                        if !account_response_is_current(
                            auth_for_response.npub.get_untracked().as_deref(),
                            &initiating_account,
                        ) {
                            operation.set(DetailOperation::Idle);
                            return;
                        }
                        manual_preimage.set(result.preimage.clone());
                        confirm_with_preimage(result.preimage);
                    }
                    Err(err) => {
                        buy_error.set(Some(err));
                        operation.set(DetailOperation::Idle);
                    }
                }
            });
        })
    };

    let on_confirm_manual = {
        let confirm_with_preimage = confirm_with_preimage.clone();
        Callback::new(move |()| {
            if operation_blocks_dispatch(operation.get_untracked()) {
                return;
            }
            let preimage = manual_preimage.get();
            if preimage.trim().is_empty() {
                buy_error.set(Some("Paste the payment preimage first".to_string()));
                return;
            }
            confirm_with_preimage(preimage);
        })
    };

    let on_download = {
        let listing = listing.clone();
        let coordinate = install_game_coordinate.clone();
        let play_auth = auth.clone();
        Callback::new(move |()| {
            if operation_blocks_dispatch(operation.get_untracked()) {
                return;
            }
            let initiating_account = play_auth.npub.get_untracked();
            install_attempt_account.set(initiating_account.clone());
            install_attempt_active.set(true);
            install_flow.set(Some(InstallFlowState::Preparing));
            operation.set(DetailOperation::Installing);
            buy_error.set(None);
            success_message.set(None);
            let listing = listing.clone();
            let coordinate = coordinate.clone();
            let auth_for_response = play_auth.clone();
            spawn_local(async move {
                if matches!(listing.acquisition, AcquisitionPolicy::Public) {
                    let Some(coordinate) = coordinate else {
                        let failure = InstallFailurePresentation {
                            title: "Download unavailable",
                            message: "The signed listing coordinate is invalid, so no artifact was downloaded.",
                            retryable: false,
                        };
                        buy_error.set(Some(failure.message.to_string()));
                        install_flow.set(Some(InstallFlowState::Failed(failure)));
                        install_attempt_active.set(false);
                        operation.set(DetailOperation::Idle);
                        return;
                    };
                    if let Err(error) = invoke_add_game_to_library(coordinate).await {
                        let failure = install_failure_presentation(&error);
                        buy_error.set(Some(failure.message.to_string()));
                        install_flow.set(Some(InstallFlowState::Failed(failure)));
                        install_attempt_active.set(false);
                        operation.set(DetailOperation::Idle);
                        return;
                    }
                    if auth_for_response.npub.get_untracked() != initiating_account {
                        install_attempt_active.set(false);
                        install_flow.set(None);
                        operation.set(DetailOperation::Idle);
                        return;
                    }
                    library_added.set(true);
                }
                match invoke_install_game(&listing).await {
                    Ok(()) => {
                        install_complete.set(true);
                        install_state_error.set(false);
                        install_state_loading.set(false);
                        if auth_for_response.npub.get_untracked() != initiating_account {
                            return;
                        }
                        if !install_attempt_active.get_untracked() {
                            return;
                        }
                        install_attempt_active.set(false);
                        install_flow.set(Some(InstallFlowState::Completed));
                        success_message.set(Some(
                            "Artifact verified and registered on this device.".to_string(),
                        ));
                        operation.set(DetailOperation::Idle);
                    }
                    Err(err) => {
                        if !install_attempt_active.get_untracked()
                            || auth_for_response.npub.get_untracked() != initiating_account
                        {
                            return;
                        }
                        let failure = install_failure_presentation(&err);
                        install_attempt_active.set(false);
                        install_flow.set(Some(InstallFlowState::Failed(failure.clone())));
                        buy_error.set(Some(failure.message.to_string()));
                        operation.set(DetailOperation::Idle);
                    }
                }
            });
        })
    };

    let on_add_to_library = {
        let coordinate = install_game_coordinate.clone();
        let add_auth = auth.clone();
        Callback::new(move |()| {
            if operation_blocks_dispatch(operation.get_untracked()) || library_added.get_untracked()
            {
                return;
            }
            let Some(coordinate) = coordinate.clone() else {
                buy_error.set(Some("Game listing coordinate is invalid.".to_string()));
                return;
            };
            let initiating_account = add_auth.npub.get_untracked();
            operation.set(DetailOperation::AddingToLibrary);
            buy_error.set(None);
            success_message.set(None);
            let auth_for_response = add_auth.clone();
            spawn_local(async move {
                let result = invoke_add_game_to_library(coordinate).await;
                if auth_for_response.npub.get_untracked() != initiating_account {
                    operation.set(DetailOperation::Idle);
                    return;
                }
                match result {
                    Ok(()) => {
                        library_added.set(true);
                        success_message.set(Some("Game added to your library.".to_string()));
                    }
                    Err(error) => {
                        buy_error.set(Some(format!("Could not add game to library: {error}")));
                    }
                }
                operation.set(DetailOperation::Idle);
            });
        })
    };

    let expected_install_coordinate = install_game_coordinate.clone();
    let download_complete_cleanup: DownloadCompleteCleanup = Rc::new(RefCell::new(None));
    let download_complete_disposed = Rc::new(RefCell::new(false));
    let download_complete_registration_started = Rc::new(RefCell::new(false));
    let download_complete_cleanup_for_effect = Rc::clone(&download_complete_cleanup);
    let download_complete_disposed_for_effect = Rc::clone(&download_complete_disposed);
    let download_complete_registration_started_for_effect =
        Rc::clone(&download_complete_registration_started);
    Effect::new(move |_| {
        if *download_complete_registration_started_for_effect.borrow() {
            return;
        }
        *download_complete_registration_started_for_effect.borrow_mut() = true;

        let expected_install_coordinate = expected_install_coordinate.clone();
        let cleanup_handle = Rc::clone(&download_complete_cleanup_for_effect);
        let disposed = Rc::clone(&download_complete_disposed_for_effect);
        spawn_local(async move {
            if let Ok(listener) = listen_download_complete(move |payload| {
                if expected_install_coordinate.as_deref() == Some(payload.game_coordinate.as_str())
                    && install_attempt_active.get_untracked()
                {
                    install_flow.set(Some(InstallFlowState::Finalizing));
                }
            })
            .await
            {
                if *disposed.borrow() {
                    listener();
                } else {
                    *cleanup_handle.borrow_mut() = Some(listener);
                }
            }
        });
    });

    let download_complete_cleanup_for_teardown =
        SendWrapper::new(Rc::clone(&download_complete_cleanup));
    let download_complete_disposed_for_teardown =
        SendWrapper::new(Rc::clone(&download_complete_disposed));
    on_cleanup(move || {
        *download_complete_disposed_for_teardown.borrow_mut() = true;
        if let Some(cleanup) = download_complete_cleanup_for_teardown.borrow_mut().take() {
            cleanup();
        }
    });

    let expected_progress_coordinate = install_game_coordinate.clone();
    let download_progress_cleanup: DownloadProgressCleanup = Rc::new(RefCell::new(None));
    let download_progress_disposed = Rc::new(RefCell::new(false));
    let download_progress_registration_started = Rc::new(RefCell::new(false));
    let download_progress_cleanup_for_effect = Rc::clone(&download_progress_cleanup);
    let download_progress_disposed_for_effect = Rc::clone(&download_progress_disposed);
    let download_progress_registration_started_for_effect =
        Rc::clone(&download_progress_registration_started);
    Effect::new(move |_| {
        if *download_progress_registration_started_for_effect.borrow() {
            return;
        }
        *download_progress_registration_started_for_effect.borrow_mut() = true;

        let expected_coordinate = expected_progress_coordinate.clone();
        let cleanup_handle = Rc::clone(&download_progress_cleanup_for_effect);
        let disposed = Rc::clone(&download_progress_disposed_for_effect);
        spawn_local(async move {
            if let Ok(listener) = listen_download_progress(move |payload| {
                if expected_coordinate.as_deref() == Some(payload.game_coordinate.as_str())
                    && install_attempt_active.get_untracked()
                {
                    install_flow.set(Some(install_flow_from_progress(&payload)));
                }
            })
            .await
            {
                if *disposed.borrow() {
                    listener();
                } else {
                    *cleanup_handle.borrow_mut() = Some(listener);
                }
            }
        });
    });

    let download_progress_cleanup_for_teardown =
        SendWrapper::new(Rc::clone(&download_progress_cleanup));
    let download_progress_disposed_for_teardown =
        SendWrapper::new(Rc::clone(&download_progress_disposed));
    on_cleanup(move || {
        *download_progress_disposed_for_teardown.borrow_mut() = true;
        if let Some(cleanup) = download_progress_cleanup_for_teardown.borrow_mut().take() {
            cleanup();
        }
    });

    let campaign_publisher = listing.publisher_npub.clone();
    let campaign_listing_id = listing.id.clone();
    let campaign_pointers = listing
        .campaigns
        .iter()
        .map(|pointer| CampaignPointerInput {
            root_event_id: pointer.root_event_id.clone(),
            relay_hint: pointer.relay_hint.clone(),
        })
        .collect::<Vec<_>>();
    Effect::new(move |_| {
        let _ = campaign_refresh.get();
        let request = DiscoverCampaignsRequest {
            publisher_npub: campaign_publisher.clone(),
            listing_id: campaign_listing_id.clone(),
            pointers: campaign_pointers.clone(),
        };
        spawn_local(async move {
            campaign_loading.set(true);
            campaign_error.set(None);
            match invoke_discover_campaigns(request).await {
                Ok(discovered) => campaigns.set(discovered),
                Err(error) => campaign_error.set(Some(error)),
            }
            campaign_loading.set(false);
        });
    });

    let claim_listing = listing.clone();
    let claim_auth = auth.clone();
    let on_claim = Callback::new(move |campaign: DiscoveredCampaign| {
        if operation_blocks_dispatch(operation.get_untracked()) {
            return;
        }
        let Some(initiating_account) = claim_auth.npub.get() else {
            buy_error.set(Some("Not authenticated".to_string()));
            return;
        };
        let Some(server_url) = adp_server_url(&claim_listing.specs, &claim_listing.download_url)
        else {
            buy_error.set(Some(
                "Listing does not include an ADP server URL".to_string(),
            ));
            return;
        };
        operation.set(DetailOperation::Claiming);
        buy_error.set(None);
        success_message.set(None);
        pending_claim.set(None);
        let request = ClaimEntitlementRequest {
            publisher_npub: claim_listing.publisher_npub.clone(),
            listing_id: claim_listing.id.clone(),
            campaign_event_id: campaign.root_event_id,
            server_url,
        };
        let auth_for_response = claim_auth.clone();
        spawn_local(async move {
            let result = invoke_claim_entitlement(request).await;
            if auth_for_response.npub.get_untracked().as_deref()
                != Some(initiating_account.as_str())
            {
                operation.set(DetailOperation::Idle);
                return;
            }
            match result {
                Ok(response) => {
                    purchase_confirmed.set(true);
                    success_message.set(Some(if response.already_claimed {
                        "Permanent access was already present for this account.".to_string()
                    } else {
                        "Permanent access was added to this account.".to_string()
                    }));
                }
                Err(error) => buy_error.set(Some(error)),
            }
            operation.set(DetailOperation::Idle);
        });
    });

    let publisher_npub_for_fetch = publisher_npub.clone();
    let profile_store_for_fetch = try_use_profile_store();

    Effect::new(move |_| {
        let npub = publisher_npub_for_fetch.clone();
        let store = profile_store_for_fetch.clone();
        spawn_local(async move {
            profile_loading.set(true);
            profile_error.set(false);

            let cached = store.as_ref().and_then(|s| s.get_untracked(&npub));
            if let Some(profile) = cached {
                seller_profile.set(Some(profile));
                profile_loading.set(false);
                return;
            }

            match invoke_fetch_profile(npub, None).await {
                Ok(profile) => {
                    if let Some(s) = &store {
                        s.put(profile.clone());
                    }
                    seller_profile.set(Some(profile));
                }
                Err(_) => {
                    seller_profile.set(None);
                    profile_error.set(true);
                }
            }
            profile_loading.set(false);
        });
    });

    let decision_auth = auth.clone();
    let primary_decision = {
        let listing = listing.clone();
        let decision_auth = decision_auth.clone();
        move || {
            let discovered = campaigns.get();
            if listing_validation_error.get() {
                return PrimaryActionDecision {
                    action: PrimaryAction::Unavailable,
                    label: "Listing validation unavailable",
                    explanation: "Retry validation before acquiring or downloading this listing.",
                    enabled: false,
                };
            }
            select_primary_action(
                &listing,
                purchase_confirmed.get(),
                install_complete.get(),
                ownership_loading.get() || listing_validation_loading.get(),
                install_state_loading.get(),
                install_state_error.get(),
                &discovered,
                campaign_loading.get() || campaign_error.get().is_some(),
                listing_compatibility(&listing, platform_info.get().as_ref()),
                operation.get(),
                cfg!(all(target_arch = "wasm32", feature = "web")),
                decision_auth.npub.get().is_some(),
                decision_time.get(),
            )
        }
    };
    let on_primary = {
        let decision = primary_decision.clone();
        let on_buy = on_buy.clone();
        let on_download = on_download.clone();
        Callback::new(move |()| match decision().action {
            PrimaryAction::Buy => on_buy.run(()),
            PrimaryAction::Install => on_download.run(()),
            PrimaryAction::Claim => {
                let discovered = campaigns.get_untracked();
                pending_claim.set(active_campaign(&discovered));
            }
            PrimaryAction::CheckingOwnership
            | PrimaryAction::CheckingInstallation
            | PrimaryAction::Busy(_)
            | PrimaryAction::Installed
            | PrimaryAction::Incompatible
            | PrimaryAction::UnsupportedWeb
            | PrimaryAction::TimedUpcoming
            | PrimaryAction::TimedExpired
            | PrimaryAction::SignIn
            | PrimaryAction::NoPaymentAddress
            | PrimaryAction::Unavailable
            | PrimaryAction::LocalStateUnavailable => {}
        })
    };
    let retry_primary = on_primary.clone();
    let decision_for_retry = primary_decision.clone();
    let decision_for_label = primary_decision.clone();
    let decision_for_disabled = primary_decision.clone();
    let decision_for_explanation = primary_decision.clone();
    let unowned_access_label = match &listing.acquisition {
        AcquisitionPolicy::Public => "Public access".to_string(),
        AcquisitionPolicy::TimedAccess { .. } => "Timed access".to_string(),
        AcquisitionPolicy::Gated if listing.price_sats > 0 => {
            format!("{} sats", listing.price_sats)
        }
        AcquisitionPolicy::Gated if listing.has_declared_price() => {
            format!("{} {}", listing.price, listing.currency)
        }
        AcquisitionPolicy::Gated => "Restricted".to_string(),
    };
    let (access_variant, access_icon) = match listing.acquisition {
        AcquisitionPolicy::Public => (StatusChipVariant::Public, Some("public")),
        AcquisitionPolicy::TimedAccess { .. } => (StatusChipVariant::TimedAccess, Some("schedule")),
        AcquisitionPolicy::Gated if listing.has_declared_price() => {
            (StatusChipVariant::Active, Some("bolt"))
        }
        AcquisitionPolicy::Gated => (StatusChipVariant::Gated, Some("lock")),
    };
    let listing_platforms = listing.platforms.clone();
    let listing_version = listing
        .specs
        .iter()
        .find(|(key, value)| key == "version" && !value.trim().is_empty())
        .map(|(_, value)| value.clone());
    let listing_release_date = format_date(listing.created_at);
    let platform_detection_required = !listing.platforms.is_empty();
    let listing_for_compatibility = listing.clone();
    let listing_for_seller = listing.clone();
    let listing_title_for_header = title.clone();
    let listing_description_for_header = description.clone();
    let listing_hero_for_header = hero_image.clone();
    let artwork_title = title.clone();

    view! {
        <section class="v2-detail-wrap">
            <button class="v2-btn-ghost v2-detail-back" on:click=move |_| on_back.run(())>
                <span aria-hidden="true">"<"</span>
                "Back to browse"
            </button>

            <div class="v2-detail-layout">
                <main class="v2-detail-main-column">
                    <header class="v2-detail-hero">
                        {move || {
                            let artwork_url = detail_presentation
                                .get()
                                .and_then(|page| {
                                    page.media
                                        .iter()
                                        .find(|item| item.role == "hero")
                                        .or_else(|| page.media.iter().find(|item| item.role == "capsule"))
                                        .map(|item| item.url.clone())
                                })
                                .or_else(|| listing_hero_for_header.clone());
                            view! {
                                <GameArtwork
                                    title=artwork_title.clone()
                                    state=artwork_state_from_url(artwork_url)
                                    role=ArtworkRole::Hero
                                />
                            }
                        }}
                        <div class="v2-detail-hero-shade" aria-hidden="true"></div>
                        <h1 class="v2-detail-title">{move || detail_presentation.get().and_then(|page| page.title).unwrap_or_else(|| listing_title_for_header.clone())}</h1>
                    </header>

                    <section class="v2-detail-section v2-detail-about" aria-labelledby="detail-about-title">
                        <p class="v2-store-kicker">"About"</p>
                        <h2 id="detail-about-title" class="sr-only">"About this game"</h2>
                        <p>{move || detail_presentation.get().and_then(|page| page.summary).filter(|value| !value.trim().is_empty()).unwrap_or_else(|| listing_description_for_header.clone())}</p>
                        <div class="v2-detail-tags">
                            {move || {
                                let values = detail_presentation
                                    .get()
                                    .map(|page| page.genres.into_iter().chain(page.features).take(8).collect::<Vec<_>>())
                                    .filter(|values| !values.is_empty())
                                    .unwrap_or_else(|| categories.clone());
                                values.into_iter().map(|category| view! { <span class="v2-chip">{category}</span> }).collect_view()
                            }}
                        </div>
                    </section>

                    <section class="v2-detail-section" aria-labelledby="detail-compatibility-title">
                        <p id="detail-compatibility-title" class="v2-store-kicker">"Compatibility"</p>
                        <div class="v2-detail-compatibility">
                            {if listing_platforms.is_empty() {
                                view! { <span class="v2-chip">"No platform restriction declared"</span> }.into_any()
                            } else {
                                listing_platforms.iter().map(|platform| view! { <span class="v2-chip">{platform.clone()}</span> }).collect_view().into_any()
                            }}
                            <span class="v2-detail-compatibility-state" role="status" aria-live="polite">
                                {move || match listing_compatibility(&listing_for_compatibility, platform_info.get().as_ref()) {
                                    DetailCompatibility::Compatible => "Compatible with this device",
                                    DetailCompatibility::Incompatible => "Not available for this device",
                                    DetailCompatibility::Unknown if platform_error.get() => "Device compatibility unavailable",
                                    DetailCompatibility::Unknown => "Checking this device",
                                }}
                            </span>
                        </div>
                    </section>

                    <section class="v2-detail-section" aria-labelledby="detail-release-title">
                        <p id="detail-release-title" class="v2-store-kicker">"Release info"</p>
                        <p class="v2-detail-release-info">
                            {move || {
                                let mut values = vec![detail_presentation.get().and_then(|page| page.release_date).filter(|value| !value.trim().is_empty()).unwrap_or_else(|| listing_release_date.clone())];
                                if let Some(version) = listing_version.clone() {
                                    values.push(format!("Version {version}"));
                                }
                                values.join(" · ")
                            }}
                        </p>
                    </section>

                    <section class="v2-detail-section" aria-labelledby="detail-developer-title">
                        <p id="detail-developer-title" class="v2-store-kicker">"Developer"</p>
                        {move || if profile_loading.get() {
                            view! { <p class="v2-social-meta">"Loading publisher profile..."</p> }.into_any()
                        } else {
                            let profile = seller_profile.get();
                            let store_identity = detail_presentation.get().and_then(|page| page.developer.or(page.publisher)).filter(|value| !value.trim().is_empty());
                            let display = store_identity.unwrap_or_else(|| seller_display(profile.as_ref(), &listing_for_seller));
                            let avatar = profile
                                .as_ref()
                                .and_then(|item| item.picture.as_ref())
                                .and_then(|url| valid_image_urls(std::slice::from_ref(url)).into_iter().next())
                                .unwrap_or_default();
                            let nip05 = profile.as_ref().and_then(|item| item.nip05.clone());
                            view! {
                                <div class="v2-detail-seller-identity">
                                    {if avatar.is_empty() {
                                        view! { <div class="v2-detail-seller-avatar">{display.chars().next().unwrap_or('?').to_string()}</div> }.into_any()
                                    } else {
                                        view! { <img class="v2-detail-seller-avatar" src=avatar alt="Publisher avatar" on:error=use_fallback_cover /> }.into_any()
                                    }}
                                    <div>
                                        <h3>{display}</h3>
                                        {nip05.map(|value| view! { <p class="v2-detail-publisher-verification">{value}</p> })}
                                        <p class="v2-social-meta">{truncate_chars(&publisher_npub, 28)}</p>
                                    </div>
                                </div>
                                {if profile_error.get() {
                                    view! { <p class="v2-social-meta">"Relay profile unavailable; showing the published listing identity."</p> }.into_any()
                                } else {
                                    view! { <></> }.into_any()
                                }}
                            }.into_any()
                        }}
                    </section>

                    {if !gallery_images.is_empty() {
                        view! {
                            <section class="v2-detail-media" aria-label="Game media">
                                {gallery_images.iter().map(|url| view! {
                                    <img src=url.clone() alt=format!("{title} screenshot") loading="lazy" on:error=use_fallback_cover />
                                }).collect::<Vec<_>>()}
                            </section>
                        }.into_any()
                    } else {
                        view! { <></> }.into_any()
                    }}

                    {move || detail_presentation.get().map(|presentation| view! { <StorePageRichDetail presentation=presentation /> })}

                    <section class="v2-detail-section v2-detail-description-block">
                        <div class="v2-section-header"><h2>"Access campaigns"</h2></div>
                        {move || if campaign_loading.get() {
                            view! { <p class="v2-social-meta">"Checking signed campaign events..."</p> }.into_any()
                        } else if let Some(error) = campaign_error.get() {
                            view! {
                                <div class="v2-detail-alert v2-detail-alert-error">
                                    <p>{format!("Campaign lookup failed: {error}")}</p>
                                    <button class="v2-btn-secondary" on:click=move |_| campaign_refresh.update(|generation| *generation = generation.wrapping_add(1))>
                                        "Retry campaign lookup"
                                    </button>
                                </div>
                            }.into_any()
                        } else if campaigns.get().is_empty() {
                            view! { <p class="v2-social-meta">"No entitlement campaigns are attached to this listing."</p> }.into_any()
                        } else {
                            let cards = campaigns.get().into_iter().map(|campaign| view! {
                                <article class="v2-detail-campaign-card">
                                    <div>
                                        <strong>{campaign.campaign_id}</strong>
                                        <p class="v2-social-meta">
                                            {format!("{} to {}", format_date(campaign.starts_at), format_date(campaign.ends_at))}
                                        </p>
                                    </div>
                                    <span class="v2-chip">{campaign.classification}</span>
                                </article>
                            }).collect::<Vec<_>>();
                            view! { <div class="v2-detail-campaign-list">{cards}</div> }.into_any()
                        }}
                    </section>

                    <section class="v2-detail-section v2-detail-description-block">
                        <h2>"Technical details"</h2>
                        <div class="v2-spec-grid">
                            {technical.iter().flat_map(|(label, value)| vec![
                                view! { <span>{label.clone()}</span> }.into_any(),
                                view! { <span class="v2-detail-technical-value">{value.clone()}</span> }.into_any(),
                            ]).collect::<Vec<_>>()}
                        </div>
                    </section>
                </main>

                <aside class="v2-detail-sidebar">
                    <section class="v2-detail-buy-panel" aria-labelledby="detail-access-title">
                    <p class="v2-store-kicker">"Access"</p>
                    <h2 id="detail-access-title" class="sr-only">"Access options"</h2>
                    <div class="v2-detail-access-states">
                        <StatusChip
                            label=unowned_access_label
                            variant=access_variant
                            icon=access_icon
                            size=StatusChipSize::Standard
                        />
                        {move || if install_complete.get() {
                            view! { <StatusChip label="Installed" variant=StatusChipVariant::Installed icon=Some("download_done") size=StatusChipSize::Compact /> }.into_any()
                        } else if purchase_confirmed.get() {
                            view! { <StatusChip label="Owned" variant=StatusChipVariant::Owned icon=Some("verified_user") size=StatusChipSize::Compact /> }.into_any()
                        } else if active_campaign(&campaigns.get()).is_some() {
                            view! { <StatusChip label="Free claim available" variant=StatusChipVariant::Success icon=Some("redeem") size=StatusChipSize::Compact /> }.into_any()
                        } else {
                            view! { <></> }.into_any()
                        }}
                    </div>
                    <p class="v2-detail-access-note">{move || decision_for_explanation().explanation}</p>
                    <button
                        class="v2-btn-primary"
                        on:click=move |_| on_primary.run(())
                        disabled=move || !decision_for_disabled().enabled
                    >
                        {move || decision_for_label().label}
                    </button>
                    <Show when=move || install_state_error.get()>
                        <button class="v2-btn-secondary" disabled=move || install_state_loading.get() on:click=move |_| install_state_refresh.update(|generation| *generation = generation.wrapping_add(1))>
                            {move || if install_state_loading.get() { "Checking..." } else { "Retry device status" }}
                        </button>
                    </Show>
                    <Show when=move || listing_validation_error.get()>
                        <button class="v2-btn-secondary" disabled=move || listing_validation_loading.get() on:click=move |_| listing_validation_refresh.update(|generation| *generation = generation.wrapping_add(1))>
                            {move || if listing_validation_loading.get() { "Validating..." } else { "Retry listing validation" }}
                        </button>
                    </Show>
                    <Show when=move || platform_detection_required && platform_error.get() && !cfg!(feature = "web")>
                        <button class="v2-btn-secondary" disabled=move || platform_loading.get() on:click=move |_| platform_refresh.update(|generation| *generation = generation.wrapping_add(1))>
                            {move || if platform_loading.get() { "Checking..." } else { "Retry compatibility check" }}
                        </button>
                    </Show>
                    <Show when=move || {
                        show_add_to_library(
                            &listing,
                            auth.npub.get().is_some(),
                            cfg!(all(target_arch = "wasm32", feature = "web")),
                            install_game_coordinate.is_some(),
                            library_loading.get() || ownership_loading.get(),
                            library_added.get(),
                            purchase_confirmed.get(),
                        )
                    }>
                        <button
                            class="v2-btn-secondary"
                            on:click=move |_| on_add_to_library.run(())
                            disabled=move || operation_blocks_dispatch(operation.get())
                        >
                            "Add to Library"
                        </button>
                    </Show>
                    {move || if operation.get() != DetailOperation::Idle {
                        view! { <p class="v2-detail-status">{operation_status(operation.get())}</p> }.into_any()
                    } else {
                        view! { <></> }.into_any()
                    }}
                    {move || buy_error.get().map(|error| view! {
                        <p class="v2-detail-alert v2-detail-alert-error">{error}</p>
                    })}
                    {move || success_message.get().map(|message| view! {
                        <p class="v2-detail-alert v2-detail-alert-success">{message}</p>
                    })}

                    {move || pending_claim.get().map(|campaign| {
                        let claim_campaign = campaign.clone();
                        let on_claim = on_claim.clone();
                        view! {
                            <div class="v2-detail-confirm">
                                <strong>"Confirm permanent access claim"</strong>
                                <p class="v2-social-meta">
                                    {format!("Campaign {} is active until {}.", campaign.campaign_id, format_date(campaign.ends_at))}
                                </p>
                                <button class="v2-btn-primary" on:click=move |_| on_claim.run(claim_campaign.clone())>
                                    "Confirm claim"
                                </button>
                                <button class="v2-btn-ghost" on:click=move |_| pending_claim.set(None)>
                                    "Cancel"
                                </button>
                            </div>
                        }
                    })}

                    {move || if show_invoice.get() && !purchase_confirmed.get() {
                        view! {
                            <div class="v2-detail-invoice">
                                <p class="v2-social-meta">"Lightning invoice"</p>
                                <code>{move || invoice.get().unwrap_or_default()}</code>
                                <div class="v2-detail-action-row">
                                    <button class="v2-btn-secondary" on:click=move |_| on_copy_invoice.run(())>"Copy"</button>
                                    <button class="v2-btn-secondary" on:click=move |_| on_open_wallet.run(())>"Open wallet"</button>
                                </div>
                                <input
                                    class="v2-input"
                                    type="password"
                                    aria-label="Nostr Wallet Connect connection string"
                                    placeholder="nostr+walletconnect://..."
                                    prop:value=move || nwc_connection_input.get()
                                    on:input:target=move |event| nwc_connection_input.set(event.target().value())
                                />
                                <button class="v2-btn-secondary" on:click=move |_| on_connect_nwc.run(())>
                                    {move || if nwc_connected.get() { "Wallet connected" } else { "Connect NWC wallet" }}
                                </button>
                                <button
                                    class="v2-btn-primary"
                                    on:click=move |_| on_pay_nwc.run(())
                                    disabled=move || !nwc_connected.get() || operation_blocks_dispatch(operation.get())
                                >
                                    "Pay and confirm"
                                </button>
                                <input
                                    class="v2-input"
                                    type="text"
                                    aria-label="Payment preimage"
                                    placeholder="Payment preimage"
                                    prop:value=move || manual_preimage.get()
                                    on:input:target=move |event| manual_preimage.set(event.target().value())
                                />
                                <button class="v2-btn-secondary" on:click=move |_| on_confirm_manual.run(())>
                                    "Confirm manual payment"
                                </button>
                            </div>
                        }.into_any()
                    } else {
                        view! { <></> }.into_any()
                    }}

                    <div class="v2-detail-buy-meta">
                        <span>{release_label.clone()}</span>
                        <span>{format!("Protocol: {protocol_label}")}</span>
                    </div>
                    </section>

                    <Show when=move || purchase_confirmed.get()>
                        <section class="v2-detail-ownership-panel" aria-labelledby="detail-ownership-title">
                            <p id="detail-ownership-title" class="v2-store-kicker">"Ownership status"</p>
                            <p>"Permanent access is recorded for the active account."</p>
                        </section>
                    </Show>
                </aside>
            </div>

            <Dialog
                id="install-dialog"
                open=Signal::derive(move || install_flow.get().is_some())
                title=Signal::derive(move || install_flow.get().map(|flow| flow.title().to_string()).unwrap_or_default())
                kicker=Signal::derive(move || install_flow.get().map(|flow| flow.kicker().to_string()).unwrap_or_default())
                title_live=true
                width=DialogWidth::Standard
                // An install attempt cannot be cancelled by the current
                // installer, so the dialog refuses dismissal while one runs
                // rather than leaving the operation orphaned.
                policy=install_close_contract().0
                dismissal=install_close_contract().1
                busy=Signal::derive(move || install_attempt_active.get())
                close_label="Close download status"
                close_blocked_hint="Cancellation is unavailable in the current installer, so this window stays open until the active operation finishes."
                on_close=UnsyncCallback::new(move |request: DialogCloseRequest| {
                    // While an attempt is active this resolves to Ignore, so
                    // Escape and the close control do nothing at all.
                    if request.action == DialogCloseAction::Dismiss {
                        install_flow.set(None);
                    }
                })
                actions={
                    let retry_primary = StoredValue::new_local(retry_primary.clone());
                    let decision_for_retry = StoredValue::new_local(decision_for_retry.clone());
                    move || view! {
                        {move || install_flow.get().map(|flow| {
                            let retry_primary = retry_primary.get_value();
                            let retry_allowed =
                                (decision_for_retry.get_value())().action == PrimaryAction::Install;
                            match flow {
                                InstallFlowState::Completed => view! {
                                    <button class="v2-btn-primary" on:click=move |_| install_flow.set(None)>"Close"</button>
                                }.into_any(),
                                InstallFlowState::Failed(failure) => view! {
                                    // Retry and close stay distinct actions.
                                    {(failure.retryable && retry_allowed).then(|| view! {
                                        <button class="v2-btn-primary" disabled=move || operation_blocks_dispatch(operation.get()) on:click=move |_| retry_primary.run(())>"Retry download"</button>
                                    })}
                                    <button class="v2-btn-secondary" disabled=move || operation_blocks_dispatch(operation.get()) on:click=move |_| install_flow.set(None)>"Close"</button>
                                }.into_any(),
                                _ => view! { <></> }.into_any(),
                            }
                        })}
                    }
                }
            >
                {move || install_flow.get().map(|flow| {
                    match flow {
                        InstallFlowState::Preparing => view! {
                            <p>"Resolving current access and the signed artifact source."</p>
                            <div class="v2-install-progress v2-install-progress-indeterminate" role="progressbar" aria-label="Preparing download"></div>
                            <p class="arc-dialog-note">"This window stays open while the active operation is running."</p>
                        }.into_any(),
                        InstallFlowState::Downloading { bytes, total } => {
                            let percent = download_progress_percent(bytes, total);
                            let transferred = total.map(|total| format!("{} of {}", format_download_bytes(bytes.min(total)), format_download_bytes(total)));
                            view! {
                                <p>"Only transferred bytes reported by the desktop downloader are shown."</p>
                                {if let Some(percent) = percent {
                                    view! {
                                        <div class="v2-install-progress" role="progressbar" aria-label="Artifact download progress" aria-valuemin="0" aria-valuemax="100" aria-valuenow=percent.to_string()>
                                            <span style=format!("width: {percent}%")></span>
                                        </div>
                                        <div class="v2-install-progress-copy"><strong>{format!("{percent}%")}</strong><span>{transferred.unwrap_or_default()}</span></div>
                                    }.into_any()
                                } else {
                                    view! {
                                        <div class="v2-install-progress v2-install-progress-indeterminate" role="progressbar" aria-label="Artifact download progress"></div>
                                        <div class="v2-install-progress-copy"><strong>"Downloading"</strong><span>"Total size unavailable"</span></div>
                                    }.into_any()
                                }}
                                <p class="arc-dialog-note">"Cancellation is unavailable in the current installer."</p>
                            }.into_any()
                        }
                        InstallFlowState::Finalizing => view! {
                            <p>"The download finished. Arcadestr is verifying the exact artifact hash before recording it on this device."</p>
                            <div class="v2-install-progress v2-install-progress-indeterminate" role="progressbar" aria-label="Verifying artifact integrity"></div>
                            <p class="arc-dialog-note">"The artifact is not marked installed until verification and registry persistence finish."</p>
                        }.into_any(),
                        InstallFlowState::Completed => view! {
                            <p role="status">"The local artifact is recorded on this device. Arcadestr does not yet launch or extract game packages."</p>
                        }.into_any(),
                        InstallFlowState::Failed(failure) => view! {
                            <p role="alert">{failure.message}</p>
                        }.into_any(),
                    }
                })}
            </Dialog>
        </section>
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_active_install_attempt_cannot_be_dismissed() {
        use crate::ui_v2::components::{resolve_close, DialogCloseAction, DialogCloseSource};

        let (policy, dismissal) = install_close_contract();
        for source in [
            DialogCloseSource::Escape,
            DialogCloseSource::Backdrop,
            DialogCloseSource::CloseButton,
            DialogCloseSource::Cancel,
        ] {
            assert_eq!(
                resolve_close(policy, dismissal, true, source),
                DialogCloseAction::Ignore,
                "{source:?} must not interrupt an active install"
            );
        }
    }

    #[test]
    fn a_finished_install_dialog_can_be_closed() {
        use crate::ui_v2::components::{resolve_close, DialogCloseAction, DialogCloseSource};

        let (policy, dismissal) = install_close_contract();
        assert_eq!(
            resolve_close(policy, dismissal, false, DialogCloseSource::Escape),
            DialogCloseAction::Dismiss
        );
        assert_eq!(
            resolve_close(policy, dismissal, false, DialogCloseSource::CloseButton),
            DialogCloseAction::Dismiss
        );
    }

    #[test]
    fn the_install_dialog_never_offers_an_unsupported_cancellation() {
        use crate::ui_v2::components::{resolve_close, DialogCloseAction, DialogCloseSource};

        let (policy, dismissal) = install_close_contract();
        // No confirmation prompt: there is no installer cancellation to
        // confirm, and offering one would be a lie.
        assert_ne!(
            resolve_close(policy, dismissal, true, DialogCloseSource::Escape),
            DialogCloseAction::RequestConfirmation
        );
        let source = include_str!("game_detail.rs");
        assert!(source.contains("Cancellation is unavailable in the current installer"));
    }

    #[test]
    fn retry_and_close_remain_distinct_install_actions() {
        let source = include_str!("game_detail.rs");
        assert!(source.contains(r#">"Retry download"</button>"#));
        assert!(source.contains(r#">"Close"</button>"#));
    }

    fn listing(acquisition: AcquisitionPolicy) -> GameListing {
        GameListing {
            id: "game-id".to_string(),
            source: ListingSource::Nip99Listing,
            title: "Game".to_string(),
            description: "Description".to_string(),
            images: Vec::new(),
            download_url: String::new(),
            price: 1000.0,
            currency: "SATS".to_string(),
            price_sats: 1000,
            quantity: None,
            tags: Vec::new(),
            specs: Vec::new(),
            publisher_npub: "npub1publisher".to_string(),
            stall_id: String::new(),
            stall_name: None,
            lud16: "seller@example.org".to_string(),
            event_id: None,
            created_at: 100,
            platforms: Vec::new(),
            nip94_event_id: None,
            acquisition,
            campaigns: Vec::new(),
            is_owned: false,
            #[cfg(debug_assertions)]
            nip99_raw_event_json: None,
        }
    }

    fn campaign(classification: &str) -> DiscoveredCampaign {
        DiscoveredCampaign {
            root_event_id: "root".to_string(),
            campaign_id: "launch".to_string(),
            starts_at: 50,
            ends_at: 150,
            classification: classification.to_string(),
            event_id: Some("current".to_string()),
            predecessor_event_id: None,
            mode: "claim".to_string(),
        }
    }

    fn decision(
        listing: &GameListing,
        owned: bool,
        installed: bool,
        campaigns: &[DiscoveredCampaign],
        campaign_loading: bool,
        compatibility: DetailCompatibility,
        operation: DetailOperation,
        standalone_web: bool,
        now: u64,
    ) -> PrimaryActionDecision {
        select_primary_action(
            listing,
            owned,
            installed,
            false,
            false,
            false,
            campaigns,
            campaign_loading,
            compatibility,
            operation,
            standalone_web,
            true,
            now,
        )
    }

    #[test]
    fn adp_server_url_prefers_listing_server_metadata() {
        let specs = vec![("server".to_string(), "http://localhost:9099/".to_string())];

        assert_eq!(
            adp_server_url(&specs, ""),
            Some("http://localhost:9099".to_string())
        );
    }

    #[test]
    fn adp_server_url_falls_back_to_legacy_download_url() {
        assert_eq!(
            adp_server_url(&[], "https://dist.example.com/game/publisher/game-v1"),
            Some("https://dist.example.com".to_string())
        );
    }

    #[test]
    fn adp_server_url_skips_malformed_server_metadata() {
        let specs = vec![
            ("server".to_string(), "https://?missing-host".to_string()),
            (
                "server".to_string(),
                "https://dist.example.com/".to_string(),
            ),
        ];

        assert_eq!(
            adp_server_url(&specs, ""),
            Some("https://dist.example.com".to_string())
        );
        assert_eq!(
            adp_server_url(
                &[("server".to_string(), "https://?missing-host".to_string(),)],
                "https://legacy.example.com/game/publisher/game-v1",
            ),
            Some("https://legacy.example.com".to_string())
        );
    }

    #[test]
    fn durable_ownership_precedes_campaign_and_purchase() {
        let listing = listing(AcquisitionPolicy::Gated);
        let campaigns = vec![campaign("active")];

        assert_eq!(
            decision(
                &listing,
                true,
                false,
                &campaigns,
                false,
                DetailCompatibility::Compatible,
                DetailOperation::Idle,
                false,
                100,
            )
            .action,
            PrimaryAction::Install
        );
    }

    #[test]
    fn active_campaign_precedes_paid_purchase() {
        let listing = listing(AcquisitionPolicy::Gated);

        assert_eq!(
            decision(
                &listing,
                false,
                false,
                &[campaign("active")],
                false,
                DetailCompatibility::Compatible,
                DetailOperation::Idle,
                false,
                100,
            )
            .action,
            PrimaryAction::Claim
        );
    }

    #[test]
    fn campaign_lookup_blocks_purchase_until_precedence_is_known() {
        let listing = listing(AcquisitionPolicy::Gated);
        let result = decision(
            &listing,
            false,
            false,
            &[],
            true,
            DetailCompatibility::Compatible,
            DetailOperation::Idle,
            false,
            100,
        );

        assert_eq!(result.action, PrimaryAction::Unavailable);
        assert!(!result.enabled);
    }

    #[test]
    fn timed_access_respects_both_boundaries_without_creating_ownership() {
        let listing = listing(AcquisitionPolicy::TimedAccess {
            starts_at: 100,
            ends_at: 200,
        });
        let action_at = |now| {
            decision(
                &listing,
                false,
                false,
                &[],
                false,
                DetailCompatibility::Compatible,
                DetailOperation::Idle,
                false,
                now,
            )
            .action
        };

        assert_eq!(action_at(99), PrimaryAction::TimedUpcoming);
        assert_eq!(action_at(100), PrimaryAction::Install);
        assert_eq!(action_at(199), PrimaryAction::Install);
        assert_eq!(action_at(200), PrimaryAction::TimedExpired);
    }

    #[test]
    fn install_and_compatibility_states_block_other_actions() {
        let listing = listing(AcquisitionPolicy::Public);
        assert_eq!(
            decision(
                &listing,
                false,
                true,
                &[],
                false,
                DetailCompatibility::Compatible,
                DetailOperation::Idle,
                false,
                100,
            )
            .action,
            PrimaryAction::Installed
        );
        assert_eq!(
            decision(
                &listing,
                false,
                false,
                &[],
                false,
                DetailCompatibility::Incompatible,
                DetailOperation::Idle,
                false,
                100,
            )
            .action,
            PrimaryAction::Incompatible
        );
    }

    #[test]
    fn gated_listing_requires_price_and_payment_address() {
        let mut listing = listing(AcquisitionPolicy::Gated);
        listing.lud16.clear();
        assert_eq!(
            decision(
                &listing,
                false,
                false,
                &[],
                false,
                DetailCompatibility::Compatible,
                DetailOperation::Idle,
                false,
                100,
            )
            .action,
            PrimaryAction::NoPaymentAddress
        );

        listing.price_sats = 0;
        assert_eq!(
            decision(
                &listing,
                false,
                false,
                &[],
                false,
                DetailCompatibility::Compatible,
                DetailOperation::Idle,
                false,
                100,
            )
            .action,
            PrimaryAction::Unavailable
        );

        listing.price = 9.99;
        listing.currency = "USD".to_string();
        let unsupported_currency = decision(
            &listing,
            false,
            false,
            &[],
            false,
            DetailCompatibility::Compatible,
            DetailOperation::Idle,
            false,
            100,
        );
        assert_eq!(unsupported_currency.action, PrimaryAction::Unavailable);
        assert_eq!(unsupported_currency.label, "Purchase unavailable");
    }

    #[test]
    fn payment_response_must_match_initiating_account() {
        assert!(account_response_is_current(Some("npub-a"), "npub-a"));
        assert!(!account_response_is_current(Some("npub-b"), "npub-a"));
        assert!(!account_response_is_current(None, "npub-a"));
    }

    #[test]
    fn standalone_web_and_busy_operations_are_non_interactive() {
        let listing = listing(AcquisitionPolicy::Public);
        assert_eq!(
            decision(
                &listing,
                false,
                false,
                &[],
                false,
                DetailCompatibility::Compatible,
                DetailOperation::Idle,
                true,
                100,
            )
            .action,
            PrimaryAction::UnsupportedWeb
        );
        assert_eq!(
            decision(
                &listing,
                false,
                false,
                &[],
                false,
                DetailCompatibility::Compatible,
                DetailOperation::Installing,
                false,
                100,
            )
            .action,
            PrimaryAction::Busy(DetailOperation::Installing)
        );
    }

    #[test]
    fn native_acquisition_requires_an_authenticated_account() {
        let listing = listing(AcquisitionPolicy::Public);
        let result = select_primary_action(
            &listing,
            false,
            false,
            false,
            false,
            false,
            &[],
            false,
            DetailCompatibility::Compatible,
            DetailOperation::Idle,
            false,
            false,
            100,
        );

        assert_eq!(result.action, PrimaryAction::SignIn);
        assert!(!result.enabled);
    }

    #[test]
    fn local_installation_lookup_blocks_duplicate_downloads_until_resolved() {
        let listing = listing(AcquisitionPolicy::Public);
        let loading = select_primary_action(
            &listing,
            false,
            false,
            false,
            true,
            false,
            &[],
            false,
            DetailCompatibility::Compatible,
            DetailOperation::Idle,
            false,
            true,
            100,
        );
        assert_eq!(loading.action, PrimaryAction::CheckingInstallation);
        assert!(!loading.enabled);

        let failed = select_primary_action(
            &listing,
            false,
            false,
            false,
            false,
            true,
            &[],
            false,
            DetailCompatibility::Compatible,
            DetailOperation::Idle,
            false,
            true,
            100,
        );
        assert_eq!(failed.action, PrimaryAction::LocalStateUnavailable);
        assert!(!failed.enabled);
    }

    #[test]
    fn public_game_actions_add_to_library_or_download() {
        let public = listing(AcquisitionPolicy::Public);
        let result = select_primary_action(
            &public,
            false,
            false,
            false,
            false,
            false,
            &[],
            false,
            DetailCompatibility::Compatible,
            DetailOperation::Idle,
            false,
            true,
            100,
        );

        assert_eq!(result.action, PrimaryAction::Install);
        assert_eq!(result.label, "Download");
        assert!(show_add_to_library(
            &public, true, false, true, false, false, false
        ));
        assert!(!show_add_to_library(
            &public, true, false, true, false, true, false
        ));
        assert!(!show_add_to_library(
            &public, true, false, true, false, false, true
        ));

        let gated = listing(AcquisitionPolicy::Gated);
        assert!(!show_add_to_library(
            &gated, true, false, true, false, false, false
        ));
    }

    #[test]
    fn download_progress_is_determinate_only_with_a_reliable_total() {
        assert_eq!(download_progress_percent(25, Some(100)), Some(25));
        assert_eq!(download_progress_percent(150, Some(100)), Some(100));
        assert_eq!(download_progress_percent(25, None), None);
        assert_eq!(download_progress_percent(25, Some(0)), None);

        let downloading = install_flow_from_progress(&DownloadProgressPayload {
            game_coordinate: "30402:publisher:game".into(),
            bytes: 25,
            total: Some(100),
        });
        assert_eq!(
            downloading,
            InstallFlowState::Downloading {
                bytes: 25,
                total: Some(100)
            }
        );
    }

    #[test]
    fn completed_download_waits_for_integrity_and_registry_completion() {
        let phase = install_flow_from_progress(&DownloadProgressPayload {
            game_coordinate: "30402:publisher:game".into(),
            bytes: 100,
            total: Some(100),
        });
        assert_eq!(phase, InstallFlowState::Finalizing);
        assert_ne!(phase, InstallFlowState::Completed);
    }

    #[test]
    fn install_failures_use_safe_actionable_categories() {
        let integrity = install_failure_presentation(
            "hash mismatch at /private/home/player/downloads/game.bin; quarantined",
        );
        assert_eq!(integrity.title, "Integrity verification failed");
        assert!(integrity.retryable);
        assert!(!integrity.message.contains("/private/"));

        let auth = install_failure_presentation("ownership authorization rejected");
        assert_eq!(auth.title, "Download authorization failed");
        assert!(auth.retryable);
        assert!(auth.message.contains("No installation was recorded"));

        let unsupported = install_failure_presentation("unsupported platform");
        assert!(!unsupported.retryable);
    }

    #[test]
    fn media_filter_rejects_placeholders_invalid_schemes_and_duplicates() {
        let urls = valid_image_urls(&[
            "https://cdn.arcadestr.test/cover.webp".to_string(),
            "https://cdn.arcadestr.test/cover.webp".to_string(),
            "https://example.com/mock.png".to_string(),
            "javascript:alert(1)".to_string(),
            "not a url".to_string(),
        ]);

        assert_eq!(urls, vec!["https://cdn.arcadestr.test/cover.webp"]);
    }

    #[test]
    fn game_detail_store_page_response_requires_current_navigation_and_listing_event() {
        let presentation = GameDetailPresentation {
            listing_coordinate: "30402:publisher:game".into(),
            listing_event_id: "event-2".into(),
            store_page_coordinate: "30407:publisher:page".into(),
            event_id: "page-event".into(),
            title: None,
            summary: None,
            description_html: None,
            media: Vec::new(),
            sections: Vec::new(),
            genres: Vec::new(),
            features: Vec::new(),
            languages: Vec::new(),
            requirements: Vec::new(),
            accessibility: Vec::new(),
            links: Default::default(),
            developer: None,
            publisher: None,
            release_date: None,
        };
        assert!(detail_response_is_current(
            4,
            4,
            4,
            "30402:publisher:game",
            "event-2",
            &presentation,
        ));
        assert!(!detail_response_is_current(
            5,
            4,
            4,
            "30402:publisher:game",
            "event-2",
            &presentation,
        ));
        assert!(!detail_response_is_current(
            4,
            4,
            4,
            "30402:publisher:game",
            "event-1",
            &presentation,
        ));
    }

    #[test]
    fn game_detail_commerce_remains_listing_derived() {
        use nostr::nips::nip19::ToBech32;

        let mut listing = listing(AcquisitionPolicy::Gated);
        listing.publisher_npub = nostr::Keys::generate()
            .public_key()
            .to_bech32()
            .expect("npub");
        listing.price_sats = 42_000;
        listing.platforms = vec!["linux-x86_64".into()];
        listing.specs = vec![
            ("version".into(), "2.0.0".into()),
            ("server".into(), "https://dist.example.org".into()),
            ("sha256".into(), "aa".repeat(32)),
        ];
        let commerce = detail_commerce(&listing).expect("canonical listing");
        assert_eq!(commerce.price_sats, 42_000);
        assert_eq!(commerce.platforms, vec!["linux-x86_64"]);
        assert_eq!(commerce.version.as_deref(), Some("2.0.0"));
        assert!(commerce.distribution_available);
        assert_eq!(
            commerce.file_hash.as_deref(),
            Some("aa".repeat(32).as_str())
        );
    }
}
