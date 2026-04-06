//! Shared marketplace listing loader and presentation helpers for UI v2 views.

use leptos::prelude::*;
use wasm_bindgen_futures::spawn_local;

use crate::invoke_fetch_marketplace_stream;
use crate::models::{GameListing, ListingSource};
use crate::store::{try_use_marketplace_store, DEFAULT_LISTING_TTL_SECS};

#[derive(Clone, Copy)]
pub struct MarketplaceListingsState {
    pub listings: RwSignal<Vec<GameListing>>,
    pub loading: RwSignal<bool>,
    pub error: RwSignal<Option<String>>,
    pub received_count: RwSignal<usize>,
}

pub struct ListingPresentation {
    pub price_primary: String,
    pub price_hint: Option<String>,
    pub cta_label: &'static str,
    pub is_free: bool,
}

pub fn listing_presentation(listing: &GameListing) -> ListingPresentation {
    if listing.price_sats == 0 {
        ListingPresentation {
            price_primary: "FREE".to_string(),
            price_hint: None,
            cta_label: "Play Now",
            is_free: true,
        }
    } else {
        ListingPresentation {
            price_primary: format_sats(listing.price_sats),
            price_hint: Some(format_usd_hint(listing.price_sats)),
            cta_label: "Buy Now",
            is_free: false,
        }
    }
}

pub fn listing_publisher(listing: &GameListing) -> String {
    listing
        .stall_name
        .clone()
        .map(|name| format!("by {}", name))
        .unwrap_or_else(|| format!("by {}", short_npub(&listing.publisher_npub)))
}

pub fn use_marketplace_listings() -> MarketplaceListingsState {
    let marketplace_store = try_use_marketplace_store();
    let listings = RwSignal::new(Vec::<GameListing>::new());
    let loading = RwSignal::new(true);
    let error = RwSignal::new(None::<String>);
    let received_count = RwSignal::new(0);

    Effect::new(move |_| {
        let store = marketplace_store.clone();
        spawn_local(async move {
            loading.set(true);
            error.set(None);
            received_count.set(0);

            if crate::debug_storefront_bypass_enabled() {
                let mocked = debug_mock_listings();
                if let Some(s) = &store {
                    s.clear();
                    s.put_many(mocked.clone());
                }
                received_count.set(mocked.len());
                listings.set(mocked);
                loading.set(false);
                return;
            }

            let should_fetch = match &store {
                Some(s) => {
                    let cached = s.get_all();
                    let needs_refresh = s.needs_refresh(DEFAULT_LISTING_TTL_SECS);
                    if !cached.is_empty() && !needs_refresh {
                        listings.set(cached);
                        loading.set(false);
                        false
                    } else {
                        true
                    }
                }
                None => true,
            };

            if should_fetch {
                let store_for_listing = store.clone();
                let on_listing = move |listing: GameListing| {
                    received_count.update(|count| *count += 1);
                    if let Some(s) = &store_for_listing {
                        s.put_streaming(listing.clone());
                    }

                    loading.set(false);

                    listings.update(|items| {
                        if !items.iter().any(|existing| existing.id == listing.id) {
                            items.push(listing);
                        }
                    });
                };

                let on_complete = Some({
                    let store_for_complete = store.clone();
                    move || {
                        if let Some(s) = &store_for_complete {
                            s.mark_fresh();
                        }
                        loading.set(false);
                    }
                });

                match invoke_fetch_marketplace_stream(50, Some(30), on_listing, on_complete).await {
                    Ok((product_cleanup, completion_cleanup)) => {
                        loading.set(false);
                        product_cleanup();
                        completion_cleanup();
                    }
                    Err(e) => {
                        if let Some(s) = &store {
                            let cached = s.get_all();
                            if !cached.is_empty() {
                                listings.set(cached);
                                loading.set(false);
                            } else {
                                error.set(Some(e));
                                loading.set(false);
                            }
                        } else {
                            error.set(Some(e));
                            loading.set(false);
                        }
                    }
                }
            }
        });
    });

    MarketplaceListingsState {
        listings,
        loading,
        error,
        received_count,
    }
}

fn debug_mock_listings() -> Vec<GameListing> {
    vec![
        GameListing {
            id: "debug-neon-velocity".to_string(),
            source: ListingSource::Nip15Product,
            title: "Neon Velocity".to_string(),
            description: "Drift through a neon skyline in a high-speed action experience built for multiplayer tournaments.".to_string(),
            images: vec!["https://lh3.googleusercontent.com/aida-public/AB6AXuAkSqV1ZOY7qDKBQQ-nU-WKmOwR16envOE_TMPHQep0afObsmDW51MoGnuCDehLWvRSiX2M-G1ipCeBVnLuSnk_GtSaKNiiKAL3NGBqfTVvkZErj92gogHgjo8Dm9s9qZAoKzMpmCEwTLAaasaklmpG0EvebxYhk_pgx9zFciCa6eEvQAequV2_VwSfxkp8qFHQKZDgSfcZX7ItUvMkkVo9gJMVU1kvmoEqtnEdxEgw_XCFEvda_kG_L7oqZh2ranJukSzpwKvU8ow".to_string()],
            download_url: "https://example.com/neon-velocity".to_string(),
            price: 21000.0,
            currency: "SATS".to_string(),
            price_sats: 21000,
            quantity: None,
            tags: vec!["action".to_string(), "multiplayer".to_string()],
            specs: vec![("genre".to_string(), "Action".to_string())],
            publisher_npub: "npub1debugstorefront000000000000000000000000000000000000000000".to_string(),
            stall_id: "debug-stall-1".to_string(),
            stall_name: Some("Neon Arcade".to_string()),
            lud16: "debug@arcadestr.dev".to_string(),
            event_id: None,
            created_at: 1_710_000_001,
        },
        GameListing {
            id: "debug-bit-runners".to_string(),
            source: ListingSource::Nip15Product,
            title: "Bit-Runners".to_string(),
            description: "Cyberpunk platformer with chain-linked item economy and community-run speedrun ladders.".to_string(),
            images: vec!["https://lh3.googleusercontent.com/aida-public/AB6AXuBppEh6duJunDcRrAlDyCHwjcLgKSLNrLn7urlFTA1JDEkbmtnYBzd_8RTWxEH0yhfLUX6wQa3QLRpQt89K69EpDGFa4DG6BcbpzyvRD9MKUR4kFURF1OHnGUsMf8pBgOnoVi2rpRC8MhhdLRTwAZGOCXgv4HUTOLToqmpkDBz1btGwBcD05i3nH5GAd2JOlqCOUwMiPrEuVPBCjSKOLd6HZ8owiUNaSNfVauMEYH3RM5Gx5tWR72rlRSNaHzmv2votTLxYPeMXM5k".to_string()],
            download_url: "https://example.com/bit-runners".to_string(),
            price: 0.0,
            currency: "SATS".to_string(),
            price_sats: 0,
            quantity: None,
            tags: vec!["platformer".to_string(), "cyberpunk".to_string()],
            specs: vec![("genre".to_string(), "Platformer".to_string())],
            publisher_npub: "npub1debugstorefront000000000000000000000000000000000000000000".to_string(),
            stall_id: "debug-stall-1".to_string(),
            stall_name: Some("Neon Arcade".to_string()),
            lud16: "debug@arcadestr.dev".to_string(),
            event_id: None,
            created_at: 1_710_000_002,
        },
        GameListing {
            id: "debug-dune-settlers".to_string(),
            source: ListingSource::Nip15Product,
            title: "Dune Settlers".to_string(),
            description: "Build a resilient off-world economy in a tactical strategy game tuned for async league play.".to_string(),
            images: vec!["https://lh3.googleusercontent.com/aida-public/AB6AXuBalkh2NCA6UZ04qa-pFXIL4N2iVby1eMnZRzDd9a2oAa9WYnFWl8OIykQNH3c4AcYN_aUwFcdGEXpllBbQf7Hz_j2HDQGKQaQXRZAmXB0nrdVNADrOeO4o5chwWjZYJKlC9Zp48Rwgt9m66yqG-k_rZ-Aot35r46iWmWCWdpye8690JqLNYoO0KmKmmTtAS2g8EsoY7eG58kSRXTaRsTqPVGPu7q43eYjpHizKEqucFvwzRT8C14m3Gji3_-ym2ZZqXrJI8pdohOA".to_string()],
            download_url: "https://example.com/dune-settlers".to_string(),
            price: 39000.0,
            currency: "SATS".to_string(),
            price_sats: 39000,
            quantity: None,
            tags: vec!["strategy".to_string(), "scifi".to_string()],
            specs: vec![("genre".to_string(), "Strategy".to_string())],
            publisher_npub: "npub1debugstorefront000000000000000000000000000000000000000000".to_string(),
            stall_id: "debug-stall-1".to_string(),
            stall_name: Some("Neon Arcade".to_string()),
            lud16: "debug@arcadestr.dev".to_string(),
            event_id: None,
            created_at: 1_710_000_003,
        },
    ]
}

fn format_sats(value: u64) -> String {
    let mut chars: Vec<char> = value.to_string().chars().collect();
    let mut i = chars.len() as isize - 3;
    while i > 0 {
        chars.insert(i as usize, ',');
        i -= 3;
    }
    format!("{} SATS", chars.into_iter().collect::<String>())
}

fn format_usd_hint(price_sats: u64) -> String {
    let usd = (price_sats as f64) / 2000.0;
    format!("~${:.2} USD", usd)
}

fn short_npub(npub: &str) -> String {
    if npub.len() <= 16 {
        return npub.to_string();
    }
    format!("{}...{}", &npub[..8], &npub[npub.len() - 4..])
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::ListingSource;

    fn listing_with_sats(price_sats: u64) -> GameListing {
        GameListing {
            id: "test".into(),
            source: ListingSource::Nip15Product,
            title: "Test".into(),
            description: "Desc".into(),
            images: vec![],
            download_url: "https://example.com".into(),
            price: price_sats as f64,
            currency: "SATS".into(),
            price_sats,
            quantity: None,
            tags: vec![],
            specs: vec![],
            publisher_npub: "npub1test0000".into(),
            stall_id: "stall".into(),
            stall_name: Some("Test Publisher".into()),
            lud16: "test@example.com".into(),
            event_id: None,
            created_at: 0,
        }
    }

    #[test]
    fn free_listing_uses_play_now() {
        let listing = listing_with_sats(0);
        let presentation = listing_presentation(&listing);
        assert_eq!(presentation.price_primary, "FREE");
        assert_eq!(presentation.cta_label, "Play Now");
        assert!(presentation.is_free);
        assert!(presentation.price_hint.is_none());
    }

    #[test]
    fn paid_listing_uses_buy_now() {
        let listing = listing_with_sats(8500);
        let presentation = listing_presentation(&listing);
        assert_eq!(presentation.price_primary, "8,500 SATS");
        assert_eq!(presentation.cta_label, "Buy Now");
        assert!(!presentation.is_free);
        assert_eq!(presentation.price_hint.as_deref(), Some("~$4.25 USD"));
    }
}
