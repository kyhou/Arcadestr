use leptos::prelude::*;
use wasm_bindgen_futures::spawn_local;

use crate::components::{BadgeEarnedModal, ProfileRow};
use crate::models::{
    BadgeAward, BadgeDefinition, EarnedBadgeSummary, GameListing, ListingSource, UserProfile,
    ZapInvoice, ZapRequest,
};
use crate::store::try_use_profile_store;
use crate::{invoke_fetch_profile, invoke_request_invoice, AuthContext};

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
    ]
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
    let price_label = if listing.price_sats > 0 {
        format!("{} Sats", listing.price_sats)
    } else {
        "Free".to_string()
    };
    let buy_button_label = if listing.price_sats > 0 {
        format!("Buy with Lightning - {} sats", listing.price_sats)
    } else {
        "Free - Download".to_string()
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
    let download_url = listing.download_url.clone();
    let has_download_url = !download_url.trim().is_empty();
    let title = listing.title.clone();
    let description = listing.description.clone();
    let hero_metadata = hero_buy_panel_metadata(
        listing.stall_name.as_deref(),
        &release_label,
        protocol_label,
    );

    // Buy flow state.
    let invoice: RwSignal<Option<ZapInvoice>> = RwSignal::new(None);
    let buy_loading: RwSignal<bool> = RwSignal::new(false);
    let buy_error: RwSignal<Option<String>> = RwSignal::new(None);
    let show_invoice: RwSignal<bool> = RwSignal::new(false);

    // Seller profile state.
    let seller_profile: RwSignal<Option<UserProfile>> = RwSignal::new(None);
    let profile_loading: RwSignal<bool> = RwSignal::new(true);

    let auth = use_context::<AuthContext>().expect("AuthContext not provided");

    let on_buy = {
        let listing = listing.clone();
        let on_invoice_created = on_invoice_created.clone();

        Callback::new(move |()| {
            let buyer_npub = match auth.npub.get() {
                Some(npub) => npub,
                None => {
                    buy_error.set(Some("Not authenticated".to_string()));
                    return;
                }
            };

            let event_id = listing
                .event_id
                .clone()
                .unwrap_or_else(|| listing.id.clone());

            let zap_req = ZapRequest {
                seller_npub: listing.publisher_npub.clone(),
                seller_lud16: listing.lud16.clone(),
                listing_event_id: event_id,
                amount_sats: listing.price_sats,
                buyer_npub,
                relays: vec![
                    "wss://relay.damus.io".to_string(),
                    "wss://relay.nostr.band".to_string(),
                ],
            };

            buy_loading.set(true);
            buy_error.set(None);
            show_invoice.set(false);

            spawn_local(async move {
                match invoke_request_invoice(zap_req).await {
                    Ok(zap_invoice) => {
                        invoice.set(Some(zap_invoice));
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
            if let Some(inv) = invoice.get() {
                if let Some(window) = leptos::web_sys::window() {
                    let _ = window.navigator().clipboard().write_text(&inv.bolt11);
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
            if let Some(inv) = invoice.get() {
                let lightning_uri = format!("lightning:{}", inv.bolt11);
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

    let on_download = {
        let download_url = download_url.clone();
        Callback::new(move |()| {
            if download_url.trim().is_empty() {
                buy_error.set(Some("No download URL available".to_string()));
                return;
            }

            #[cfg(target_arch = "wasm32")]
            {
                if let Some(win) = leptos::web_sys::window() {
                    let _ = win.location().set_href(&download_url);
                }
            }

            #[cfg(not(target_arch = "wasm32"))]
            {
                let _ = &download_url;
            }
        })
    };

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
                        if buy_loading.get() {
                            view! {
                                <button class="v2-btn-primary" disabled=true>
                                    "Requesting invoice..."
                                </button>
                            }.into_any()
                        } else if show_invoice.get() {
                            view! {
                                <>
                                    <div class="v2-panel" style:padding="8px" style:font-size="0.8rem">
                                        <p style:word-break="break-all">
                                            {move || invoice.get().map(|inv| {
                                                if inv.bolt11.len() > 40 {
                                                    format!("{}...", &inv.bolt11[..40])
                                                } else {
                                                    inv.bolt11.clone()
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
                                </>
                            }.into_any()
                        } else if listing.price_sats == 0 {
                            view! {
                                <>
                                    <button
                                        class="v2-btn-primary"
                                        on:click=move |_| on_download.run(())
                                        disabled=move || !has_download_url
                                    >
                                        {buy_button_label.clone()}
                                    </button>
                                    <button class="v2-btn-ghost">"Add to Library"</button>
                                </>
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
