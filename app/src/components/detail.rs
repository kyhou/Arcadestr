// Detail view component

use std::sync::{Arc, Mutex};

use leptos::prelude::*;
use wasm_bindgen_futures::spawn_local;

use crate::models::{GameListing, ZapInvoice, ZapRequest};
use crate::{invoke_request_invoice, AuthContext};

/// Detail view component - displays full listing information with Buy flow.
#[component]
pub fn DetailView(
    listing: GameListing,
    on_back: Callback<()>,
    #[prop(default = String::new())]
    listing_event_id: String,
) -> impl IntoView {
    let auth = use_context::<AuthContext>().expect("AuthContext not provided");
    
    // Format price display
    let price_sats = listing.price_sats;
    let price_display = move || -> String {
        if price_sats == 0 {
            "Free".to_string()
        } else {
            format!("⚡ {} sats", price_sats)
        }
    };

    // Download button handler - used in the view! macro below
    let _download_url = listing.download_url.clone();

    // Buy flow state
    let invoice: RwSignal<Option<ZapInvoice>> = RwSignal::new(None);
    let buy_loading: RwSignal<bool> = RwSignal::new(false);
    let buy_error: RwSignal<Option<String>> = RwSignal::new(None);
    let show_invoice: RwSignal<bool> = RwSignal::new(false);

    // Buy button handler - wrapped in Arc<Mutex> for thread-safe cloning
    let on_buy = {
        let listing = listing.clone();
        let listing_event_id = listing_event_id.clone();
        
        Arc::new(Mutex::new(move || {
            // Get buyer npub
            let buyer_npub = match auth.npub.get() {
                Some(n) => n,
                None => {
                    buy_error.set(Some("Not authenticated".to_string()));
                    return;
                }
            };
            
            // Use listing.id as fallback if listing_event_id is empty
            let event_id = if listing_event_id.is_empty() {
                listing.id.clone()
            } else {
                listing_event_id.clone()
            };
            
            // Build zap request using listing.lud16
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
                    }
                    Err(e) => {
                        buy_error.set(Some(e));
                        buy_loading.set(false);
                    }
                }
            });
        }))
    };

    // Copy invoice to clipboard - wrapped in Arc<Mutex>
    let on_copy_invoice: Arc<Mutex<dyn Fn() + Send>> = {
        Arc::new(Mutex::new(move || {
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
        }))
    };

    // Open invoice in wallet - wrapped in Arc<Mutex>
    let on_open_wallet: Arc<Mutex<dyn Fn() + Send>> = {
        Arc::new(Mutex::new(move || {
            #[cfg(target_arch = "wasm32")]
            {
                if let Some(inv) = invoice.get() {
                    let lightning_uri = format!("lightning:{}", inv.bolt11);
                    use leptos::web_sys::window;
                    if let Some(win) = window() {
                        let _ = win.location().set_href(&lightning_uri);
                    }
                }
            }
            #[cfg(not(target_arch = "wasm32"))]
            {
                let _ = invoice;
            }
        }))
    };

    let on_back_click = move |_| {
        on_back.run(());
    };

    // Truncate bolt11 for display
    let invoice_display = move || {
        invoice.get().map(|inv| {
            if inv.bolt11.len() > 40 {
                format!("{}...", &inv.bolt11[..40])
            } else {
                inv.bolt11.clone()
            }
        }).unwrap_or_default()
    };

    // Clone listing for closures
    let listing_for_lud16 = listing.clone();

    view! {
        <div class="detail-container">
            <button
                class="back-button"
                on:click={on_back_click}
            >
                "← Back"
            </button>

            <div class="detail-content">
                <h2 class="detail-title">{listing.title.clone()}</h2>

                <div class="detail-meta">
                    <p class="detail-publisher">
                        <span class="meta-label">"Publisher: "</span>
                        {listing.publisher_npub.clone()}
                    </p>
                    <p class="detail-price">
                        <span class="meta-label">"Price: "</span>
                        <span class={if price_sats == 0 { "price-free" } else { "price-paid" }}>
                            {price_display}
                        </span>
                    </p>
                    // Lightning address hint under price
                    {let listing = listing_for_lud16.clone(); move || {
                        if !listing.lud16.is_empty() {
                            view! {
                                <p class="lud16-hint">
                                    <span class="lud16-label">"⚡ "</span>
                                    {listing.lud16.clone()}
                                </p>
                            }.into_any()
                        } else if price_sats > 0 {
                            view! {
                                <p class="no-lud16-hint">
                                    "No Lightning address — purchases unavailable"
                                </p>
                            }.into_any()
                        } else {
                            let _: () = view! { <></> };
                            ().into_any()
                        }
                    }}
                </div>

                <div class="detail-description">
                    <h3>"Description"</h3>
                    <p>{listing.description.clone()}</p>
                </div>

                <div class="detail-tags">
                    <h3>"Tags"</h3>
                    <div class="tags-container">
                        {if listing.tags.is_empty() {
                            view! { <span class="no-tags">"No tags"</span> }.into_any()
                        } else {
                            listing.tags.iter().map(|tag| {
                                view! { <span class="tag-badge">{tag.clone()}</span> }
                            }).collect::<Vec<_>>().into_any()
                        }}
                    </div>
                </div>

                <div class="detail-actions">
                    {{
                        #[cfg(target_arch = "wasm32")]
                        {
                            let download_url = listing.download_url.clone();
                            view! {
                                <button
                                    class="download-button"
                                    on:click={move |_| {
                                        use leptos::web_sys::window;
                                        if let Some(win) = window() {
                                            let _ = win.open_with_url_and_target(&download_url, "_blank");
                                        }
                                    }}
                                >
                                    "Download"
                                </button>
                            }
                        }
                        #[cfg(not(target_arch = "wasm32"))]
                        {
                            view! {
                                <button
                                    class="download-button disabled"
                                    disabled=true
                                    title="Open in desktop browser — coming soon"
                                >
                                    "Download"
                                </button>
                            }
                        }
                    }}
                </div>

                // Buy section (only for paid listings with lud16)
                {let on_buy = on_buy.clone(); let on_copy_invoice = on_copy_invoice.clone(); let on_open_wallet = on_open_wallet.clone(); let listing = listing.clone(); move || {
                    if price_sats > 0 {
                        if listing.lud16.is_empty() {
                            // No Lightning address - show disabled notice
                            view! {
                                <div class="buy-section">
                                    <h3>"Purchase"</h3>
                                    <p class="no-lud16-notice">
                                        "No Lightning address — purchases unavailable"
                                    </p>
                                </div>
                            }.into_any()
                        } else {
                            // Has Lightning address - show full buy section
                            view! {
                                <div class="buy-section">
                                    <h3>"Purchase"</h3>
                                    
                                    {let on_buy = on_buy.clone(); let on_copy_invoice = on_copy_invoice.clone(); let on_open_wallet = on_open_wallet.clone(); move || {
                                        if buy_loading.get() {
                                            view! {
                                                <button class="buy-button" disabled=true>
                                                    "Requesting invoice..."
                                                </button>
                                            }.into_any()
                                        } else if show_invoice.get() {
                                            view! {
                                                <div class="invoice-card">
                                                    <h4>"⚡ Lightning Invoice"</h4>
                                                    <p class="invoice-text">{invoice_display}</p>
                                                    <div class="invoice-actions">
                                                        <button
                                                            class="copy-button"
                                                            on:click={let on_copy_invoice = on_copy_invoice.clone(); move |_| { on_copy_invoice.lock().unwrap()(); }}
                                                        >
                                                            "📋 Copy Invoice"
                                                        </button>
                                                        <button
                                                            class="wallet-button"
                                                            on:click={let on_open_wallet = on_open_wallet.clone(); move |_| { on_open_wallet.lock().unwrap()(); }}
                                                        >
                                                            "Open in Wallet"
                                                        </button>
                                                    </div>
                                                    <p class="invoice-hint">
                                                        "Pay with any Lightning wallet. Amount: "
                                                        {format!("{} sats", price_sats)}
                                                    </p>
                                                </div>
                                            }.into_any()
                                        } else {
                                            view! {
                                                <button
                                                    class="buy-button"
                                                    on:click={let on_buy = on_buy.clone(); move |_| { on_buy.lock().unwrap()(); }}
                                                >
                                                    {format!("⚡ Buy for {} sats", price_sats)}
                                                </button>
                                            }.into_any()
                                        }
                                    }}
                                    
                                    {move || {
                                        buy_error.get().map(|err| {
                                            view! {
                                                <div class="error-message">{err}</div>
                                            }
                                        })
                                    }}
                                </div>
                            }.into_any()
                        }
                    } else {
                        view! {
                            <div class="free-note">
                                <p>"🎁 Free — just download!"</p>
                            </div>
                        }.into_any()
                    }
                }}
            </div>
        </div>
    }
}
