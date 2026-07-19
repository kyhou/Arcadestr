use std::cell::RefCell;
use std::rc::Rc;

use leptos::prelude::*;
use send_wrapper::SendWrapper;
use wasm_bindgen_futures::spawn_local;

use crate::components::{BadgeEarnedModal, ProfileRow};
use crate::models::{
    AcquisitionPolicy, BadgeAward, BadgeDefinition, EarnedBadgeSummary, GameListing, ListingSource,
    UserProfile,
};
use crate::store::try_use_profile_store;
use crate::tauri_bridge::{
    invoke_claim_entitlement, invoke_confirm_purchase, invoke_connect_nwc_wallet,
    invoke_discover_campaigns, invoke_install_game, invoke_pay_nwc_invoice,
    invoke_request_lnurl_invoice, listen_download_complete,
};
use crate::tauri_bridge::{
    CampaignPointerInput, ClaimEntitlementRequest, ConfirmPurchaseRequest, ConnectNwcWalletRequest,
    DiscoverCampaignsRequest, DiscoveredCampaign, PayNwcInvoiceRequest, RequestLnurlInvoiceRequest,
};
use crate::{invoke_fetch_profile, AuthContext};

type DownloadCompleteCleanup = Rc<RefCell<Option<Box<dyn FnOnce()>>>>;

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

fn format_timestamp(ts: u64) -> String {
    #[cfg(target_arch = "wasm32")]
    {
        let date = js_sys::Date::new(&(ts as f64 * 1000.0).into());
        let year = date.get_full_year();
        let month = date.get_month() + 1;
        let day = date.get_date();
        format!("Release Date: {month:02}/{day:02}/{year}")
    }

    #[cfg(not(target_arch = "wasm32"))]
    {
        let years_since_epoch = ts / 31_556_952;
        let year = 1970 + years_since_epoch;
        format!("Release Date: {year}")
    }
}

fn safe_css_url(url: &str) -> String {
    let trimmed = url.trim();
    let supported_scheme = trimmed.starts_with("https://")
        || trimmed.starts_with("http://")
        || trimmed.starts_with("data:image/");
    let has_css_breakout = trimmed
        .chars()
        .any(|ch| matches!(ch, '\'' | '"' | ')' | ';' | '\\'));

    if supported_scheme && !has_css_breakout {
        trimmed.to_string()
    } else {
        String::new()
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

fn hero_buy_panel_metadata(
    stall_name: Option<&str>,
    release_label: &str,
    protocol_label: &str,
) -> Vec<String> {
    vec![
        format!("Publisher: {}", stall_name.unwrap_or("Independent")),
        release_label.to_string(),
        format!("Protocol: {protocol_label}"),
    ]
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
    let earned_badge_preview = RwSignal::new(None::<EarnedBadgeSummary>);

    let close_badge_modal = {
        let earned_badge_preview = earned_badge_preview;
        Callback::new(move |_| {
            earned_badge_preview.set(None);
        })
    };

    // Follow-up: wire to kind-8 relay subscription when badge issuance lands.
    let on_invoice_created = {
        let earned_badge_preview = earned_badge_preview;
        Callback::new(move |_| {
            #[cfg(debug_assertions)]
            {
                earned_badge_preview.set(Some(debug_badge_preview()));
            }

            #[cfg(not(debug_assertions))]
            {
                let _ = earned_badge_preview;
            }
        })
    };

    let hero_image = listing
        .images
        .first()
        .map(|url| safe_css_url(url))
        .unwrap_or_default();
    let hero_style = format!(
        "background-image: linear-gradient(to top, rgba(10,14,20,0.88), rgba(10,14,20,0.45)), url('{hero_image}'); background-size: cover; background-position: center;"
    );
    let kicker = listing
        .tags
        .first()
        .cloned()
        .unwrap_or_else(|| "Game".to_string());
    let direct_access = listing.acquisition.allows_access_at(current_unix_secs());
    let price_label = match &listing.acquisition {
        AcquisitionPolicy::Public => "Public access".to_string(),
        AcquisitionPolicy::TimedAccess { .. } if direct_access => "Timed access".to_string(),
        _ if listing.price_sats > 0 => format!("{} Sats", listing.price_sats),
        _ => "Gated".to_string(),
    };
    let buy_button_label = match &listing.acquisition {
        AcquisitionPolicy::Public => "Public install".to_string(),
        AcquisitionPolicy::TimedAccess { .. } if direct_access => {
            "Install during access window".to_string()
        }
        _ if listing.price_sats > 0 => {
            format!("Buy with Lightning - {} sats", listing.price_sats)
        }
        _ => "Ownership required".to_string(),
    };
    let release_label = format_timestamp(listing.created_at);
    let protocol_label = match listing.source {
        ListingSource::Nip15Product => "NIP-15",
        ListingSource::Nip99Listing => "NIP-99",
        ListingSource::Legacy => "NIP-01",
    };
    let gallery_images = listing.images.iter().skip(1).cloned().collect::<Vec<_>>();
    let tags = listing.tags.clone();
    let specs = listing.specs.clone();
    let publisher_npub = listing.publisher_npub.clone();
    let seller_lud16 = listing.lud16.clone();
    let has_lightning = !seller_lud16.trim().is_empty();
    let title = listing.title.clone();
    let description = listing.description.clone();
    let hero_metadata = hero_buy_panel_metadata(
        listing.stall_name.as_deref(),
        &release_label,
        protocol_label,
    );

    // Buy flow state.
    let invoice: RwSignal<Option<String>> = RwSignal::new(None);
    let buy_loading: RwSignal<bool> = RwSignal::new(false);
    let buy_error: RwSignal<Option<String>> = RwSignal::new(None);
    let show_invoice: RwSignal<bool> = RwSignal::new(false);
    let nwc_connection_input = RwSignal::new(String::new());
    let nwc_connected: RwSignal<bool> = RwSignal::new(false);
    let manual_preimage = RwSignal::new(String::new());
    let purchase_confirmed: RwSignal<bool> = RwSignal::new(listing.is_owned);
    let install_loading: RwSignal<bool> = RwSignal::new(false);
    let install_complete: RwSignal<bool> = RwSignal::new(false);
    let campaigns: RwSignal<Vec<DiscoveredCampaign>> = RwSignal::new(Vec::new());
    let campaign_loading: RwSignal<bool> = RwSignal::new(true);

    // Seller profile state.
    let seller_profile: RwSignal<Option<UserProfile>> = RwSignal::new(None);
    let profile_loading: RwSignal<bool> = RwSignal::new(true);

    let auth = use_context::<AuthContext>().expect("AuthContext not provided");

    let on_buy = {
        let listing = listing.clone();

        Callback::new(move |()| {
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

            buy_loading.set(true);
            buy_error.set(None);
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
                        buy_loading.set(false);
                        on_invoice_created.run(());
                    }
                    Err(e) => {
                        buy_error.set(Some(e));
                        buy_loading.set(false);
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

    let confirm_with_preimage = {
        let listing = listing.clone();
        move |preimage: String| {
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

            buy_loading.set(true);
            buy_error.set(None);
            spawn_local(async move {
                let request = ConfirmPurchaseRequest {
                    publisher_npub,
                    listing_id,
                    server_url,
                    bolt11,
                    preimage,
                };
                match invoke_confirm_purchase(request).await {
                    Ok(_) => {
                        purchase_confirmed.set(true);
                        buy_loading.set(false);
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
                        buy_loading.set(false);
                    }
                }
            });
        }
    };

    let on_connect_nwc = Callback::new(move |()| {
        let connection_string = nwc_connection_input.get();
        if connection_string.trim().is_empty() {
            buy_error.set(Some(
                "Paste a Nostr Wallet Connect string first".to_string(),
            ));
            return;
        }
        buy_loading.set(true);
        buy_error.set(None);
        spawn_local(async move {
            match invoke_connect_nwc_wallet(ConnectNwcWalletRequest { connection_string }).await {
                Ok(_) => {
                    nwc_connected.set(true);
                    buy_loading.set(false);
                }
                Err(err) => {
                    buy_error.set(Some(err));
                    buy_loading.set(false);
                }
            }
        });
    });

    let on_pay_nwc = {
        let confirm_with_preimage = confirm_with_preimage.clone();
        Callback::new(move |()| {
            let Some(bolt11) = invoice.get() else {
                buy_error.set(Some("Request an invoice before paying".to_string()));
                return;
            };
            buy_loading.set(true);
            buy_error.set(None);
            let confirm_with_preimage = confirm_with_preimage.clone();
            spawn_local(async move {
                match invoke_pay_nwc_invoice(PayNwcInvoiceRequest { bolt11 }).await {
                    Ok(result) => {
                        manual_preimage.set(result.preimage.clone());
                        buy_loading.set(false);
                        confirm_with_preimage(result.preimage);
                    }
                    Err(err) => {
                        buy_error.set(Some(err));
                        buy_loading.set(false);
                    }
                }
            });
        })
    };

    let on_confirm_manual = {
        let confirm_with_preimage = confirm_with_preimage.clone();
        Callback::new(move |()| {
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
        Callback::new(move |()| {
            install_loading.set(true);
            buy_error.set(None);
            let listing = listing.clone();
            spawn_local(async move {
                match invoke_install_game(&listing).await {
                    Ok(()) => {
                        purchase_confirmed.set(true);
                        install_complete.set(true);
                        install_loading.set(false);
                    }
                    Err(err) => {
                        buy_error.set(Some(format!("Install failed: {err}")));
                        install_loading.set(false);
                    }
                }
            });
        })
    };

    let install_listing_id = listing.id.clone();
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

        let install_listing_id = install_listing_id.clone();
        let cleanup_handle = Rc::clone(&download_complete_cleanup_for_effect);
        let disposed = Rc::clone(&download_complete_disposed_for_effect);
        spawn_local(async move {
            if let Ok(listener) = listen_download_complete(move |payload| {
                if payload.listing_id == install_listing_id {
                    purchase_confirmed.set(true);
                    install_complete.set(true);
                    install_loading.set(false);
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
        let request = DiscoverCampaignsRequest {
            publisher_npub: campaign_publisher.clone(),
            listing_id: campaign_listing_id.clone(),
            pointers: campaign_pointers.clone(),
        };
        spawn_local(async move {
            campaign_loading.set(true);
            match invoke_discover_campaigns(request).await {
                Ok(discovered) => campaigns.set(discovered),
                Err(error) => buy_error.set(Some(format!("Campaign discovery failed: {error}"))),
            }
            campaign_loading.set(false);
        });
    });

    let claim_listing = listing.clone();
    let on_claim = Callback::new(move |campaign: DiscoveredCampaign| {
        if auth.npub.get().is_none() {
            buy_error.set(Some("Not authenticated".to_string()));
            return;
        }
        let Some(server_url) = adp_server_url(&claim_listing.specs, &claim_listing.download_url)
        else {
            buy_error.set(Some(
                "Listing does not include an ADP server URL".to_string(),
            ));
            return;
        };
        buy_loading.set(true);
        buy_error.set(None);
        let request = ClaimEntitlementRequest {
            publisher_npub: claim_listing.publisher_npub.clone(),
            listing_id: claim_listing.id.clone(),
            campaign_event_id: campaign.root_event_id,
            server_url,
        };
        spawn_local(async move {
            match invoke_claim_entitlement(request).await {
                Ok(_) => purchase_confirmed.set(true),
                Err(error) => buy_error.set(Some(error)),
            }
            buy_loading.set(false);
        });
    });

    let publisher_npub_for_fetch = publisher_npub.clone();
    let profile_store_for_fetch = try_use_profile_store();

    Effect::new(move |_| {
        let npub = publisher_npub_for_fetch.clone();
        let store = profile_store_for_fetch.clone();
        spawn_local(async move {
            profile_loading.set(true);

            let cached = store.as_ref().and_then(|s| s.get(&npub));
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
                Err(_) => seller_profile.set(None),
            }
            profile_loading.set(false);
        });
    });

    view! {
        <section class="v2-detail-wrap">
            <header class="v2-panel-glass v2-detail-hero" style=hero_style>
                <div>
                    <p class="v2-store-kicker">{kicker}</p>
                    <h1 class="v2-display v2-detail-title">{title.clone()}</h1>
                    <div class="v2-detail-rating-row">
                        <span>"star star star star star_half"</span>
                        <span>"4.8"</span>
                        <span>"|"</span>
                        <span>"bolt 12.4k Zaps"</span>
                    </div>
                    <p class="v2-hero-description">{description.clone()}</p>
                    <div class="v2-detail-tags">
                        {tags
                            .iter()
                            .take(4)
                            .map(|tag| view! { <span class="v2-chip">{tag.clone()}</span> })
                            .collect::<Vec<_>>()}
                    </div>
                </div>
                <aside class="v2-detail-buy-panel v2-panel">
                    <div class="v2-detail-price">{price_label}</div>
                    <button class="v2-btn-secondary" on:click=move |_| on_back.run(())>
                        "Back"
                    </button>

                    {move || {
                        if campaign_loading.get() {
                            view! { <p class="v2-social-meta">"Checking claim campaigns..."</p> }
                                .into_any()
                        } else {
                            let cards = campaigns
                                .get()
                                .into_iter()
                                .filter(|campaign| {
                                    matches!(
                                        campaign.classification.as_str(),
                                        "upcoming" | "active" | "ended" | "cancelled"
                                    )
                                })
                                .map(|campaign| {
                                    let claim_campaign = campaign.clone();
                                    let on_claim = on_claim.clone();
                                    let is_active = campaign.classification == "active";
                                    view! {
                                        <div class="v2-panel" style:padding="10px">
                                            <strong>{format!("Claim campaign: {}", campaign.campaign_id)}</strong>
                                            <p class="v2-social-meta">
                                                {format!(
                                                    "{} - {} | {}",
                                                    campaign.starts_at,
                                                    campaign.ends_at,
                                                    campaign.classification
                                                )}
                                            </p>
                                            <button
                                                class="v2-btn-primary"
                                                on:click=move |_| on_claim.run(claim_campaign.clone())
                                                disabled=move || !is_active || buy_loading.get()
                                            >
                                                {if is_active { "Claim" } else { "Unavailable" }}
                                            </button>
                                        </div>
                                    }
                                })
                                .collect::<Vec<_>>();
                            view! { <>{cards}</> }.into_any()
                        }
                    }}

                    {move || {
                        if purchase_confirmed.get() {
                            view! {
                                <>
                                    <button
                                        class="v2-btn-primary"
                                        on:click=move |_| on_download.run(())
                                        disabled=move || install_loading.get() || install_complete.get()
                                    >
                                        {move || {
                                            if install_complete.get() {
                                                "Installed"
                                            } else if install_loading.get() {
                                                "Installing..."
                                            } else {
                                                "Install"
                                            }
                                        }}
                                    </button>
                                    <p class="v2-social-meta">
                                        {move || {
                                            if install_complete.get() {
                                                "Installed. Runtime execution and extraction are not available in this gate."
                                            } else if install_loading.get() {
                                                "Installing game artifact..."
                                            } else {
                                                "Purchase confirmed. Download token stored."
                                            }
                                        }}
                                    </p>
                                </>
                            }.into_any()
                        } else if buy_loading.get() {
                            view! {
                                <button class="v2-btn-primary" disabled=true>
                                    "Processing purchase..."
                                </button>
                            }.into_any()
                        } else if show_invoice.get() {
                            view! {
                                <>
                                    <div class="v2-panel" style:padding="8px" style:font-size="0.8rem">
                                        <p style:word-break="break-all">
                                            {move || invoice.get().map(|bolt11| {
                                                if bolt11.len() > 40 {
                                                    format!("{}...", &bolt11[..40])
                                                } else {
                                                    bolt11
                                                }
                                            }).unwrap_or_default()}
                                        </p>
                                    </div>
                                    <button class="v2-btn-primary" on:click=move |_| on_copy_invoice.run(())>
                                        "Copy Invoice"
                                    </button>
                                    <button class="v2-btn-secondary" on:click=move |_| on_open_wallet.run(())>
                                        "Open in Wallet"
                                    </button>
                                    <input
                                        class="v2-input"
                                        type="password"
                                        placeholder="nostr+walletconnect://..."
                                        prop:value=move || nwc_connection_input.get()
                                        on:input:target=move |ev| nwc_connection_input.set(ev.target().value())
                                    />
                                    <button class="v2-btn-secondary" on:click=move |_| on_connect_nwc.run(())>
                                        {move || if nwc_connected.get() { "NWC Connected" } else { "Connect NWC Wallet" }}
                                    </button>
                                    <button class="v2-btn-primary" on:click=move |_| on_pay_nwc.run(()) disabled=move || !nwc_connected.get()>
                                        "Pay with Connected Wallet"
                                    </button>
                                    <input
                                        class="v2-input"
                                        type="text"
                                        placeholder="Paste payment preimage"
                                        prop:value=move || manual_preimage.get()
                                        on:input:target=move |ev| manual_preimage.set(ev.target().value())
                                    />
                                    <button class="v2-btn-secondary" on:click=move |_| on_confirm_manual.run(())>
                                        "Confirm Manual Payment"
                                    </button>
                                </>
                            }.into_any()
                        } else if direct_access {
                            let free_button_label = buy_button_label.clone();
                            view! {
                                <>
                                    <button
                                        class="v2-btn-primary"
                                        on:click=move |_| on_download.run(())
                                        disabled=move || install_loading.get() || install_complete.get()
                                    >
                                        {move || {
                                            if install_complete.get() {
                                                "Installed".to_string()
                                            } else if install_loading.get() {
                                                "Installing...".to_string()
                                            } else {
                                                free_button_label.clone()
                                            }
                                        }}
                                    </button>
                                    <button class="v2-btn-ghost">"Add to Library"</button>
                                </>
                            }.into_any()
                        } else if listing.price_sats == 0 {
                            view! {
                                <button class="v2-btn-primary" disabled=true>
                                    "Ownership required"
                                </button>
                            }.into_any()
                        } else if !has_lightning {
                            view! {
                                <>
                                    <button class="v2-btn-primary" disabled=true>
                                        "No Lightning address"
                                    </button>
                                    <button class="v2-btn-ghost">"Add to Library"</button>
                                </>
                            }.into_any()
                        } else {
                            view! {
                                <>
                                    <button
                                        class="v2-btn-primary"
                                        on:click=move |_| on_buy.run(())
                                    >
                                        {buy_button_label.clone()}
                                    </button>
                                    <button class="v2-btn-ghost">"Add to Library"</button>
                                </>
                            }.into_any()
                        }
                    }}

                    {move || {
                        buy_error.get().map(|err| {
                            view! {
                                <p class="v2-social-meta" style:color="var(--v2-danger)">{err}</p>
                            }
                        })
                    }}

                    {hero_metadata.iter().map(|item| {
                        view! { <p class="v2-social-meta">{item.clone()}</p> }
                    }).collect::<Vec<_>>()}

                    <section class="v2-detail-currently-playing">
                        <h4>"Currently Playing"</h4>
                        <div class="v2-playing-row">
                            <span>"SatoshiGamer"</span>
                            <span>"Streaming"</span>
                        </div>
                        <div class="v2-playing-row">
                            <span>"PlebsOnly"</span>
                            <span>"Level 12"</span>
                        </div>
                    </section>
                </aside>
            </header>

            <div class="v2-detail-grid">
                <section class="v2-panel-glass v2-detail-feed">
                    {if !gallery_images.is_empty() {
                        view! {
                            <div class="v2-detail-gallery-grid">
                                {gallery_images.iter().take(3).map(|url| {
                                    view! { <img src={url.clone()} alt="screenshot" /> }
                                }).collect::<Vec<_>>()}
                                {if gallery_images.len() > 3 {
                                    view! {
                                        <div style:position="relative">
                                            <img src={gallery_images.get(3).cloned().unwrap_or_default()} alt="more media" />
                                            <div style:position="absolute" style:inset="0" style:display="flex" style:align-items="center" style:justify-content="center" style:background="rgba(0,0,0,0.4)">
                                                <span style:color="white" style:font-weight="700">
                                                    {format!("+{} Media", gallery_images.len() - 3)}
                                                </span>
                                            </div>
                                        </div>
                                    }.into_any()
                                } else {
                                    view! { <></> }.into_any()
                                }}
                            </div>
                        }.into_any()
                    } else {
                        view! { <></> }.into_any()
                    }}

                    <div class="v2-section-header">
                        <h3>{title}</h3>
                    </div>
                    <p class="v2-hero-description">{description}</p>
                    <div class="v2-detail-tags">
                        {tags.iter().map(|tag| {
                            view! { <span class="v2-chip">{tag.clone()}</span> }
                        }).collect::<Vec<_>>()}
                    </div>

                    <div class="v2-section-header" style:margin-top="2rem">
                        <h3>"Nostr Feed"</h3>
                        <button class="v2-btn-ghost">"Write a Note"</button>
                    </div>
                    <div class="v2-live-note v2-detail-note-card">
                        <p class="v2-social-meta">"npub1...k9q2 - 2h ago"</p>
                        <p>"Nostr-powered community reviews will appear here once relay subscriptions are implemented."</p>
                        <div class="v2-social-actions">
                            <span>"bolt -"</span>
                            <span>"chat -"</span>
                            <span>"sync -"</span>
                        </div>
                    </div>
                </section>

                <div>
                    <section class="v2-panel v2-detail-specs" style:margin-bottom="1rem">
                        <h3>"Specs"</h3>
                        <div class="v2-spec-grid">
                            {if specs.is_empty() {
                                view! {
                                    <>
                                        <span>"OS"</span><span>"Cross-platform"</span>
                                        <span>"Type"</span><span>"Digital Download"</span>
                                    </>
                                }.into_any()
                            } else {
                                specs.iter().flat_map(|(key, value)| {
                                    vec![
                                        view! { <span>{key.clone()}</span> }.into_any(),
                                        view! { <span>{value.clone()}</span> }.into_any(),
                                    ]
                                }).collect::<Vec<_>>().into_any()
                            }}
                        </div>
                    </section>

                    <section class="v2-panel v2-detail-specs">
                        <h3>"Developer"</h3>
                        {move || {
                            if profile_loading.get() {
                                view! { <p class="v2-social-meta">"Loading seller info..."</p> }.into_any()
                            } else {
                                let npub = publisher_npub.clone();
                                let lud16_for_profile = seller_lud16.clone();
                                view! {
                                    <div>
                                        <ProfileRow
                                            npub={npub}
                                            avatar_size="48px"
                                            truncate_npub=20
                                        />
                                        {move || seller_profile.get().map(|p| {
                                            view! {
                                                <div style:margin-top="12px" style:padding-top="12px" style:border-top="1px solid var(--v2-outline-ghost)">
                                                    {p.about.clone().map(|about| {
                                                        if !about.is_empty() {
                                                            let truncated = truncate_chars(&about, 120);
                                                            view! { <p class="v2-social-meta">{truncated}</p> }.into_any()
                                                        } else {
                                                            view! { <></> }.into_any()
                                                        }
                                                    })}
                                                    {p.nip05.clone().map(|nip05| {
                                                        view! {
                                                            <p class="v2-social-meta">
                                                                {if p.nip05_verified { "verified " } else { "" }}{nip05}
                                                            </p>
                                                        }.into_any()
                                                    })}
                                                    {p.lud16.clone().or(Some(lud16_for_profile.clone())).map(|lud16| {
                                                        if !lud16.is_empty() {
                                                            view! {
                                                                <p class="v2-social-meta" style:color="var(--v2-primary)">
                                                                    {format!("Lightning: {lud16}")}
                                                                </p>
                                                            }.into_any()
                                                        } else {
                                                            view! { <></> }.into_any()
                                                        }
                                                    })}
                                                    {p.website.clone().map(|website| {
                                                        let website_href = website.clone();
                                                        view! {
                                                            <a href={website_href} target="_blank" rel="noopener" class="v2-social-meta" style:color="var(--v2-secondary)" style:text-decoration="none">
                                                                {website}
                                                            </a>
                                                        }.into_any()
                                                    })}
                                                </div>
                                            }
                                        })}
                                    </div>
                                }.into_any()
                            }
                        }}
                    </section>
                </div>
            </div>

            <BadgeEarnedModal badge=earned_badge_preview.into() on_close=close_badge_modal />
        </section>
    }
}

#[cfg(debug_assertions)]
fn debug_badge_preview() -> EarnedBadgeSummary {
    EarnedBadgeSummary {
        definition: BadgeDefinition {
            coordinate: "30009:debug:beta-tester".to_string(),
            issuer_pubkey: "debug_issuer_pubkey".to_string(),
            badge_id: "beta-tester".to_string(),
            name: Some("Beta Tester".to_string()),
            description: Some("Awarded for testing in debug mode.".to_string()),
            image_url: Some("https://example.com/badge-beta.png".to_string()),
            image_dimensions: None,
            thumb_url: Some("https://example.com/badge-beta-thumb.png".to_string()),
            thumb_dimensions: None,
            relay_url: None,
            event_id: "debug_definition_event".to_string(),
            created_at: 0,
        },
        award: BadgeAward {
            event_id: "debug_award_event".to_string(),
            issuer_pubkey: "debug_issuer_pubkey".to_string(),
            recipient_pubkey: "debug_recipient_pubkey".to_string(),
            badge_coordinate: "30009:debug:beta-tester".to_string(),
            relay_url: None,
            created_at: 0,
        },
        visible_on_profile: true,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
    fn hero_buy_panel_metadata_excludes_developer_profile() {
        let metadata =
            hero_buy_panel_metadata(Some("Arcade Vault"), "Release Date: 07/08/2026", "NIP-99");

        assert_eq!(
            metadata,
            vec![
                "Publisher: Arcade Vault".to_string(),
                "Release Date: 07/08/2026".to_string(),
                "Protocol: NIP-99".to_string(),
            ]
        );
        assert!(!metadata.iter().any(|item| item.starts_with("Developer:")));
    }
}
