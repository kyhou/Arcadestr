use std::cell::RefCell;
use std::rc::Rc;

use leptos::prelude::*;
use send_wrapper::SendWrapper;
use wasm_bindgen_futures::spawn_local;

use crate::models::{
    npub_fallback_label, AcquisitionPolicy, GameListing, ListingSource, PlatformInfo, UserProfile,
};
use crate::store::try_use_profile_store;
use crate::tauri_bridge::{
    invoke_add_game_to_library, invoke_claim_entitlement, invoke_confirm_purchase,
    invoke_connect_nwc_wallet, invoke_discover_campaigns, invoke_get_installed_games,
    invoke_get_listing_ownership, invoke_get_platform_info, invoke_install_game,
    invoke_is_game_in_library, invoke_pay_nwc_invoice, invoke_request_lnurl_invoice,
    listen_download_complete,
};
use crate::tauri_bridge::{
    CampaignPointerInput, ClaimEntitlementRequest, ConfirmPurchaseRequest, ConnectNwcWalletRequest,
    DiscoverCampaignsRequest, DiscoveredCampaign, PayNwcInvoiceRequest, RequestLnurlInvoiceRequest,
};
use crate::ui_v2::views::browse_games::listing_categories;
use crate::ui_v2::views::{use_fallback_cover, FALLBACK_COVER};
use crate::{invoke_fetch_profile, AuthContext};

type DownloadCompleteCleanup = Rc<RefCell<Option<Box<dyn FnOnce()>>>>;

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
            DetailOperation::Installing => "Installing...",
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
            label: if matches!(listing.acquisition, AcquisitionPolicy::Public) {
                "Play Game"
            } else if owned {
                "Install"
            } else {
                "Download while available"
            },
            explanation: if matches!(listing.acquisition, AcquisitionPolicy::Public) {
                "Adds the game to your library and starts the download."
            } else if owned {
                "Durable ownership is confirmed for the active account."
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
        DetailOperation::Installing => "Downloading and installing the game...",
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

#[component]
pub fn GameDetailView(listing: GameListing, on_back: Callback<()>) -> impl IntoView {
    let media = valid_image_urls(&listing.images);
    let hero_image = media
        .first()
        .cloned()
        .unwrap_or_else(|| FALLBACK_COVER.to_string());
    let gallery_images = media.into_iter().skip(1).collect::<Vec<_>>();
    let categories = listing_categories(&listing)
        .into_iter()
        .map(|category| category.label)
        .collect::<Vec<_>>();
    let kicker = categories
        .first()
        .cloned()
        .unwrap_or_else(|| "Game".to_string());
    let release_label = format_timestamp(listing.created_at);
    let protocol_label = match listing.source {
        ListingSource::Nip15Product => "NIP-15",
        ListingSource::Nip99Listing => "NIP-99",
        ListingSource::Legacy => "NIP-01",
    };
    let technical = technical_fields(&listing);
    let publisher_npub = listing.publisher_npub.clone();
    let seller_lud16 = listing.lud16.clone();
    let has_lightning = !seller_lud16.trim().is_empty();
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
    let operation = RwSignal::new(DetailOperation::Idle);
    let campaigns: RwSignal<Vec<DiscoveredCampaign>> = RwSignal::new(Vec::new());
    let campaign_loading: RwSignal<bool> = RwSignal::new(true);
    let campaign_error: RwSignal<Option<String>> = RwSignal::new(None);
    let campaign_refresh = RwSignal::new(0_u64);
    let pending_claim: RwSignal<Option<DiscoveredCampaign>> = RwSignal::new(None);
    let platform_info: RwSignal<Option<PlatformInfo>> = RwSignal::new(None);
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

    Effect::new(move |_| {
        spawn_local(async move {
            if let Ok(info) = invoke_get_platform_info().await {
                platform_info.set(Some(info));
            }
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
            let coordinate = coordinate.clone();
            spawn_local(async move {
                if let Ok(installed_games) = invoke_get_installed_games().await {
                    install_complete.set(
                        installed_games
                            .iter()
                            .any(|game| game.game_coordinate == coordinate),
                    );
                }
            });
        });
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
            operation.set(DetailOperation::Installing);
            buy_error.set(None);
            success_message.set(None);
            let listing = listing.clone();
            let coordinate = coordinate.clone();
            let auth_for_response = play_auth.clone();
            spawn_local(async move {
                if matches!(listing.acquisition, AcquisitionPolicy::Public) {
                    let Some(coordinate) = coordinate else {
                        buy_error.set(Some("Game listing coordinate is invalid.".to_string()));
                        operation.set(DetailOperation::Idle);
                        return;
                    };
                    if let Err(error) = invoke_add_game_to_library(coordinate).await {
                        buy_error.set(Some(format!("Could not add game to library: {error}")));
                        operation.set(DetailOperation::Idle);
                        return;
                    }
                    if auth_for_response.npub.get_untracked() != initiating_account {
                        operation.set(DetailOperation::Idle);
                        return;
                    }
                    library_added.set(true);
                }
                match invoke_install_game(&listing).await {
                    Ok(()) => {
                        install_complete.set(true);
                        success_message.set(Some("Installation completed.".to_string()));
                        operation.set(DetailOperation::Idle);
                    }
                    Err(err) => {
                        buy_error.set(Some(format!("Install failed: {err}")));
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
                {
                    install_complete.set(true);
                    success_message.set(Some("Installation completed.".to_string()));
                    operation.set(DetailOperation::Idle);
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
            select_primary_action(
                &listing,
                purchase_confirmed.get(),
                install_complete.get(),
                ownership_loading.get(),
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
            | PrimaryAction::Busy(_)
            | PrimaryAction::Installed
            | PrimaryAction::Incompatible
            | PrimaryAction::UnsupportedWeb
            | PrimaryAction::TimedUpcoming
            | PrimaryAction::TimedExpired
            | PrimaryAction::SignIn
            | PrimaryAction::NoPaymentAddress
            | PrimaryAction::Unavailable => {}
        })
    };
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
    let listing_for_compatibility = listing.clone();
    let listing_for_seller = listing.clone();

    view! {
        <section class="v2-detail-wrap">
            <button class="v2-btn-ghost v2-detail-back" on:click=move |_| on_back.run(())>
                <span class="material-symbols-outlined">"arrow_back"</span>
                "Back"
            </button>

            <header class="v2-panel-glass v2-detail-hero">
                <div class="v2-detail-hero-copy">
                    <p class="v2-store-kicker">{kicker}</p>
                    <h1 class="v2-display v2-detail-title">{title.clone()}</h1>
                    <p class="v2-hero-description">{description.clone()}</p>
                    <div class="v2-detail-tags">
                        {categories.iter().map(|category| {
                            view! { <span class="v2-chip">{category.clone()}</span> }
                        }).collect::<Vec<_>>()}
                    </div>
                </div>
                <div class="v2-detail-cover-frame">
                    <img src=hero_image alt=format!("{title} cover") on:error=use_fallback_cover />
                </div>
                <aside class="v2-detail-buy-panel v2-panel">
                    <p class="v2-store-kicker">"Access"</p>
                    <div class="v2-detail-price">
                        {move || if purchase_confirmed.get() {
                            "Owned".to_string()
                        } else {
                            unowned_access_label.clone()
                        }}
                    </div>
                    <button
                        class="v2-btn-primary"
                        on:click=move |_| on_primary.run(())
                        disabled=move || !decision_for_disabled().enabled
                    >
                        {move || decision_for_label().label}
                    </button>
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
                    <p class="v2-social-meta">{move || decision_for_explanation().explanation}</p>
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
                        <span>{move || match listing_compatibility(&listing_for_compatibility, platform_info.get().as_ref()) {
                            DetailCompatibility::Compatible => "Compatible with this device",
                            DetailCompatibility::Incompatible => "Not available for this device",
                            DetailCompatibility::Unknown => "Device compatibility unavailable",
                        }}</span>
                    </div>
                </aside>
            </header>

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

            <div class="v2-detail-grid">
                <main class="v2-detail-main-column">
                    <section class="v2-panel-glass v2-detail-description-block">
                        <p class="v2-store-kicker">"About this game"</p>
                        <h2>{title.clone()}</h2>
                        <p>{description}</p>
                    </section>

                    <section class="v2-panel-glass v2-detail-description-block">
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

                    <section class="v2-panel-glass v2-detail-description-block">
                        <h2>"Technical details"</h2>
                        <div class="v2-spec-grid">
                            {technical.iter().flat_map(|(label, value)| vec![
                                view! { <span>{label.clone()}</span> }.into_any(),
                                view! { <span class="v2-detail-technical-value">{value.clone()}</span> }.into_any(),
                            ]).collect::<Vec<_>>()}
                        </div>
                    </section>
                </main>

                <aside class="v2-panel v2-detail-seller-card">
                    <p class="v2-store-kicker">"Published by"</p>
                    {move || if profile_loading.get() {
                        view! { <p class="v2-social-meta">"Loading seller profile..."</p> }.into_any()
                    } else {
                        let profile = seller_profile.get();
                        let display = seller_display(profile.as_ref(), &listing_for_seller);
                        let avatar = profile
                            .as_ref()
                            .and_then(|item| item.picture.as_ref())
                            .and_then(|url| valid_image_urls(std::slice::from_ref(url)).into_iter().next())
                            .unwrap_or_default();
                        let about = profile.as_ref().and_then(|item| item.about.clone());
                        let nip05 = profile.as_ref().and_then(|item| item.nip05.clone());
                        let lightning = profile.as_ref().and_then(|item| item.lud16.clone())
                            .filter(|value| !value.trim().is_empty())
                            .or_else(|| has_lightning.then(|| seller_lud16.clone()));
                        view! {
                            <div>
                                <div class="v2-detail-seller-identity">
                                    {if avatar.is_empty() {
                                        view! { <div class="v2-detail-seller-avatar">{display.chars().next().unwrap_or('?').to_string()}</div> }.into_any()
                                    } else {
                                        view! { <img class="v2-detail-seller-avatar" src=avatar alt="Seller avatar" on:error=use_fallback_cover /> }.into_any()
                                    }}
                                    <div>
                                        <h3>{display}</h3>
                                        <p class="v2-social-meta">{truncate_chars(&publisher_npub, 28)}</p>
                                    </div>
                                </div>
                                {about.filter(|value| !value.trim().is_empty()).map(|value| view! {
                                    <p class="v2-detail-seller-about">{truncate_chars(&value, 240)}</p>
                                })}
                                {nip05.map(|value| view! { <p class="v2-social-meta">{value}</p> })}
                                {lightning.map(|value| view! { <p class="v2-social-meta">{format!("Lightning: {value}")}</p> })}
                                {if profile_error.get() {
                                    view! { <p class="v2-social-meta">"Relay profile unavailable; showing listing identity."</p> }.into_any()
                                } else {
                                    view! { <></> }.into_any()
                                }}
                            </div>
                        }.into_any()
                    }}
                </aside>
            </div>
        </section>
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
    fn public_game_actions_add_to_library_or_play() {
        let public = listing(AcquisitionPolicy::Public);
        let result = select_primary_action(
            &public,
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
        assert_eq!(result.label, "Play Game");
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
}
