use leptos::prelude::*;

use crate::models::{AcquisitionPolicy, GameListing, StorePageCardPresentation};

const FALLBACK_COVER: &str = "data:image/svg+xml,%3Csvg xmlns='http://www.w3.org/2000/svg' viewBox='0 0 16 10'%3E%3Crect width='16' height='10' fill='%23191a22'/%3E%3Cpath d='M6 4h4v2H6z' fill='%23aaa4b5'/%3E%3C/svg%3E";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GameCardAccess {
    Paid,
    Owned,
    Public,
    TimedActive,
    TimedUpcoming,
    TimedExpired,
    Gated,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GameCardCampaign {
    Active,
    Upcoming,
    Ended,
    Claimed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlatformCompatibility {
    Compatible,
    Incompatible,
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GameCardAction {
    ViewDetails,
    Purchase,
    Claim,
    Install,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GameCardPresentation {
    pub access: GameCardAccess,
    pub campaign: Option<GameCardCampaign>,
    pub compatibility: PlatformCompatibility,
    pub installed: bool,
    pub action: Option<GameCardAction>,
}

impl GameCardPresentation {
    pub fn from_listing(
        listing: &GameListing,
        now: u64,
        compatibility: PlatformCompatibility,
        installed: bool,
    ) -> Self {
        let access = if listing.is_owned {
            GameCardAccess::Owned
        } else {
            match listing.acquisition {
                AcquisitionPolicy::Public => GameCardAccess::Public,
                AcquisitionPolicy::TimedAccess { starts_at, .. } if now < starts_at => {
                    GameCardAccess::TimedUpcoming
                }
                AcquisitionPolicy::TimedAccess { ends_at, .. } if now >= ends_at => {
                    GameCardAccess::TimedExpired
                }
                AcquisitionPolicy::TimedAccess { .. } => GameCardAccess::TimedActive,
                AcquisitionPolicy::Gated if listing.has_declared_price() => GameCardAccess::Paid,
                AcquisitionPolicy::Gated => GameCardAccess::Gated,
            }
        };

        let action = if compatibility == PlatformCompatibility::Incompatible {
            Some(GameCardAction::ViewDetails)
        } else if installed {
            Some(GameCardAction::ViewDetails)
        } else {
            match access {
                GameCardAccess::Paid => Some(GameCardAction::Purchase),
                GameCardAccess::Owned | GameCardAccess::Public | GameCardAccess::TimedActive => {
                    Some(GameCardAction::Install)
                }
                GameCardAccess::TimedUpcoming
                | GameCardAccess::TimedExpired
                | GameCardAccess::Gated => Some(GameCardAction::ViewDetails),
            }
        };

        Self {
            access,
            campaign: None,
            compatibility,
            installed,
            action,
        }
    }

    pub fn claim_and_keep(
        claimed: bool,
        compatibility: PlatformCompatibility,
        installed: bool,
    ) -> Self {
        let access = if claimed {
            GameCardAccess::Owned
        } else {
            GameCardAccess::Gated
        };
        let action = if compatibility == PlatformCompatibility::Incompatible || installed {
            Some(GameCardAction::ViewDetails)
        } else if claimed {
            Some(GameCardAction::Install)
        } else {
            Some(GameCardAction::Claim)
        };

        Self {
            access,
            campaign: Some(if claimed {
                GameCardCampaign::Claimed
            } else {
                GameCardCampaign::Active
            }),
            compatibility,
            installed,
            action,
        }
    }

    pub fn with_campaign(mut self, campaign: Option<GameCardCampaign>) -> Self {
        self.campaign = campaign;
        if campaign == Some(GameCardCampaign::Active)
            && self.access != GameCardAccess::Owned
            && self.compatibility != PlatformCompatibility::Incompatible
            && !self.installed
        {
            self.action = Some(GameCardAction::Claim);
        }
        self
    }
}

#[component]
pub fn GameCard(
    listing: GameListing,
    presentation: GameCardPresentation,
    on_open: Callback<GameListing>,
    #[prop(optional)] categories: Option<Vec<String>>,
    store_page: Option<StorePageCardPresentation>,
    #[prop(optional)] on_action: Option<Callback<GameCardAction>>,
) -> impl IntoView {
    let display = resolve_card_content(&listing, store_page.as_ref(), categories);
    let image_url = display.image_url;
    let title = display.title;
    let summary = display.summary;
    let title_for_open = listing.clone();
    let publisher = listing
        .stall_name
        .clone()
        .filter(|name| !name.trim().is_empty())
        .unwrap_or_else(|| crate::models::npub_fallback_label(&listing.publisher_npub));
    let tags = display.badges;
    let badge = access_badge(presentation.access);
    let price = price_label(&listing, presentation);
    let action = presentation.action;

    view! {
        <article class="group relative flex h-full flex-col overflow-hidden rounded-2xl bg-surface-container-high transition-colors hover:bg-surface-bright">
            <div class="relative aspect-[4/3] w-full overflow-hidden bg-surface-container-low">
                <img
                    src=image_url
                    alt={format!("{} cover art", title)}
                    loading="lazy"
                    class="h-full w-full object-cover transition-transform duration-500 group-hover:scale-105"
                    on:error=move |event| {
                        let image = event_target::<web_sys::HtmlElement>(&event);
                        let _ = image.set_attribute("src", FALLBACK_COVER);
                    }
                />
                <div class="absolute inset-x-0 bottom-0 h-24 bg-gradient-to-t from-black/80 to-transparent" aria-hidden="true"></div>
                <div class="absolute left-3 top-3 flex flex-wrap items-center gap-1.5">
                    <span class={access_badge_class(presentation.access)}>{badge}</span>
                    {campaign_badge(presentation.campaign)}
                    {compatibility_badge(presentation.compatibility)}
                </div>
                <Show when=move || presentation.installed>
                    <span class="absolute right-3 top-3 rounded-full bg-secondary/90 px-2.5 py-1 text-[10px] font-bold uppercase tracking-wider text-on-secondary">
                        "Installed"
                    </span>
                </Show>
            </div>
            <div class="flex flex-1 flex-col gap-3 p-4">
                <div class="flex items-start justify-between gap-3">
                    <div class="min-w-0">
                        <button
                            type="button"
                            class="block max-w-full truncate text-left font-display text-lg font-semibold text-on-surface outline-none ring-primary/60 hover:text-primary focus-visible:ring-2"
                            on:click=move |_| on_open.run(title_for_open.clone())
                        >
                            {title}
                        </button>
                        <p class="truncate text-xs text-on-surface-variant">
                            {if tags.is_empty() { "Game".to_string() } else { tags.join(" · ") }}
                        </p>
                    </div>
                    <p class="shrink-0 text-right text-sm font-semibold text-primary">{price}</p>
                </div>
                <p class="line-clamp-2 min-h-10 text-sm text-on-surface-variant">{summary}</p>
                <div class="mt-auto flex items-center justify-between gap-3">
                    <span class="min-w-0 truncate text-xs text-on-surface-variant">{format!("by {publisher}")}</span>
                    {move || match (action, on_action) {
                        (Some(action), Some(callback)) => view! {
                            <button
                                type="button"
                                class="shrink-0 rounded-full bg-primary px-3 py-1.5 text-xs font-semibold text-on-primary outline-none ring-primary/60 hover:brightness-110 focus-visible:ring-2"
                                on:click=move |_| callback.run(action)
                            >
                                {action_label(action, presentation)}
                            </button>
                        }.into_any(),
                        _ => view! { <></> }.into_any(),
                    }}
                </div>
            </div>
        </article>
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct CardDisplayContent {
    title: String,
    summary: String,
    image_url: String,
    badges: Vec<String>,
}

fn resolve_card_content(
    listing: &GameListing,
    store_page: Option<&StorePageCardPresentation>,
    categories: Option<Vec<String>>,
) -> CardDisplayContent {
    let title = store_page
        .and_then(|page| page.title.clone())
        .filter(|title| !title.trim().is_empty())
        .unwrap_or_else(|| listing.title.clone());
    let summary = store_page
        .and_then(|page| page.summary.clone())
        .filter(|summary| !summary.trim().is_empty())
        .unwrap_or_else(|| listing.description.chars().take(180).collect());
    let store_image = store_page
        .and_then(|page| page.capsule_url.as_ref().or(page.hero_url.as_ref()))
        .cloned();
    let image_url = store_image
        .and_then(|url| valid_cover_url(&[url]))
        .or_else(|| valid_cover_url(&listing.images))
        .unwrap_or_else(|| FALLBACK_COVER.to_string());
    let badges = store_page
        .map(|page| {
            page.genres
                .iter()
                .chain(&page.features)
                .take(2)
                .cloned()
                .collect::<Vec<_>>()
        })
        .filter(|badges| !badges.is_empty())
        .unwrap_or_else(|| match categories {
            Some(categories) => categories.into_iter().take(2).collect(),
            None => listing.tags.iter().take(2).cloned().collect(),
        });
    CardDisplayContent {
        title,
        summary,
        image_url,
        badges,
    }
}

fn valid_cover_url(images: &[String]) -> Option<String> {
    images.iter().find_map(|candidate| {
        let parsed = url::Url::parse(candidate.trim()).ok()?;
        if !matches!(parsed.scheme(), "http" | "https") {
            return None;
        }
        let host = parsed.host_str()?;
        if host == "example"
            || host.ends_with(".example")
            || host == "example.com"
            || host.ends_with(".example.com")
        {
            return None;
        }
        Some(candidate.trim().to_string())
    })
}

fn access_badge(access: GameCardAccess) -> &'static str {
    match access {
        GameCardAccess::Paid => "Paid",
        GameCardAccess::Owned => "Owned",
        GameCardAccess::Public => "Public access",
        GameCardAccess::TimedActive => "Timed access",
        GameCardAccess::TimedUpcoming => "Access upcoming",
        GameCardAccess::TimedExpired => "Access ended",
        GameCardAccess::Gated => "Unavailable",
    }
}

fn access_badge_class(access: GameCardAccess) -> &'static str {
    match access {
        GameCardAccess::Public | GameCardAccess::TimedActive => {
            "rounded-full bg-black/70 px-2.5 py-1 text-[10px] font-semibold uppercase tracking-wider text-secondary backdrop-blur"
        }
        _ => "rounded-full bg-black/70 px-2.5 py-1 text-[10px] font-semibold uppercase tracking-wider text-on-surface backdrop-blur",
    }
}

fn campaign_badge(campaign: Option<GameCardCampaign>) -> AnyView {
    let Some(campaign) = campaign else {
        return view! { <></> }.into_any();
    };
    let label = match campaign {
        GameCardCampaign::Active => "Claim and keep",
        GameCardCampaign::Upcoming => "Claim upcoming",
        GameCardCampaign::Ended => "Claim ended",
        GameCardCampaign::Claimed => "Entitlement claimed",
    };
    view! {
        <span class="rounded-full bg-black/70 px-2.5 py-1 text-[10px] font-semibold uppercase tracking-wider text-tertiary backdrop-blur">
            {label}
        </span>
    }
    .into_any()
}

fn compatibility_badge(compatibility: PlatformCompatibility) -> AnyView {
    match compatibility {
        PlatformCompatibility::Incompatible => view! {
            <span class="rounded-full bg-error/90 px-2.5 py-1 text-[10px] font-semibold uppercase tracking-wider text-on-surface">
                "Incompatible"
            </span>
        }
        .into_any(),
        PlatformCompatibility::Compatible => view! {
            <span class="rounded-full bg-black/60 px-2.5 py-1 text-[10px] font-semibold uppercase tracking-wider text-secondary">
                "Compatible"
            </span>
        }
        .into_any(),
        PlatformCompatibility::Unknown => view! { <></> }.into_any(),
    }
}

fn price_label(listing: &GameListing, presentation: GameCardPresentation) -> String {
    if presentation.access == GameCardAccess::Owned {
        return "Owned".to_string();
    }
    if presentation.campaign == Some(GameCardCampaign::Active) {
        return "Free claim".to_string();
    }
    if presentation.campaign == Some(GameCardCampaign::Claimed) {
        return "Claimed".to_string();
    }
    match presentation.access {
        GameCardAccess::Public => "Public".to_string(),
        GameCardAccess::TimedActive => "Available now".to_string(),
        GameCardAccess::TimedUpcoming => "Upcoming".to_string(),
        GameCardAccess::TimedExpired => "Ended".to_string(),
        GameCardAccess::Gated => "Unavailable".to_string(),
        GameCardAccess::Paid if listing.price_sats > 0 => {
            format!("{} sats", listing.price_sats)
        }
        GameCardAccess::Paid if !listing.currency.is_empty() => {
            format!("{} {}", listing.price, listing.currency)
        }
        GameCardAccess::Paid => "Paid".to_string(),
        GameCardAccess::Owned => "Owned".to_string(),
    }
}

fn action_label(action: GameCardAction, presentation: GameCardPresentation) -> &'static str {
    match action {
        GameCardAction::ViewDetails if presentation.installed => "Installed",
        GameCardAction::ViewDetails => "Details",
        GameCardAction::Purchase => "Buy",
        GameCardAction::Claim => "Claim",
        GameCardAction::Install => "Install",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn listing() -> GameListing {
        serde_json::from_value(serde_json::json!({
            "id": "game-id",
            "title": "Game",
            "description": "Description",
            "publisher_npub": "npub1publisher",
            "created_at": 1
        }))
        .expect("minimal listing should deserialize")
    }

    #[test]
    fn listing_policy_maps_to_truthful_card_states() {
        let mut value = listing();
        value.price_sats = 2_100;
        let paid = GameCardPresentation::from_listing(
            &value,
            100,
            PlatformCompatibility::Compatible,
            false,
        );
        assert_eq!(paid.access, GameCardAccess::Paid);
        assert_eq!(paid.action, Some(GameCardAction::Purchase));

        value.is_owned = true;
        let owned = GameCardPresentation::from_listing(
            &value,
            100,
            PlatformCompatibility::Compatible,
            false,
        );
        assert_eq!(owned.access, GameCardAccess::Owned);
        assert_eq!(owned.action, Some(GameCardAction::Install));

        value.is_owned = false;
        value.acquisition = AcquisitionPolicy::Public;
        assert_eq!(
            GameCardPresentation::from_listing(
                &value,
                100,
                PlatformCompatibility::Compatible,
                false,
            )
            .access,
            GameCardAccess::Public
        );

        value.acquisition = AcquisitionPolicy::TimedAccess {
            starts_at: 100,
            ends_at: 200,
        };
        assert_eq!(
            GameCardPresentation::from_listing(
                &value,
                99,
                PlatformCompatibility::Compatible,
                false,
            )
            .access,
            GameCardAccess::TimedUpcoming
        );
        assert_eq!(
            GameCardPresentation::from_listing(
                &value,
                150,
                PlatformCompatibility::Compatible,
                false,
            )
            .access,
            GameCardAccess::TimedActive
        );
        assert_eq!(
            GameCardPresentation::from_listing(
                &value,
                200,
                PlatformCompatibility::Compatible,
                false,
            )
            .access,
            GameCardAccess::TimedExpired
        );
    }

    #[test]
    fn claim_install_and_compatibility_never_invent_play_actions() {
        let available =
            GameCardPresentation::claim_and_keep(false, PlatformCompatibility::Compatible, false);
        assert_eq!(available.access, GameCardAccess::Gated);
        assert_eq!(available.campaign, Some(GameCardCampaign::Active));
        assert_eq!(available.action, Some(GameCardAction::Claim));

        let claimed =
            GameCardPresentation::claim_and_keep(true, PlatformCompatibility::Compatible, false);
        assert_eq!(claimed.access, GameCardAccess::Owned);
        assert_eq!(claimed.campaign, Some(GameCardCampaign::Claimed));
        assert_eq!(claimed.action, Some(GameCardAction::Install));

        let incompatible =
            GameCardPresentation::claim_and_keep(true, PlatformCompatibility::Incompatible, false);
        assert_eq!(incompatible.action, Some(GameCardAction::ViewDetails));

        let installed =
            GameCardPresentation::claim_and_keep(true, PlatformCompatibility::Compatible, true);
        assert_eq!(installed.action, Some(GameCardAction::ViewDetails));
    }

    #[test]
    fn non_sats_price_stays_paid_while_zero_price_stays_gated() {
        let mut value = listing();
        value.price = 9.99;
        value.currency = "USD".to_string();
        let paid = GameCardPresentation::from_listing(
            &value,
            100,
            PlatformCompatibility::Compatible,
            false,
        );
        assert_eq!(paid.access, GameCardAccess::Paid);
        assert_eq!(price_label(&value, paid), "9.99 USD");

        value.price = 0.0;
        let gated = GameCardPresentation::from_listing(
            &value,
            100,
            PlatformCompatibility::Compatible,
            false,
        );
        assert_eq!(gated.access, GameCardAccess::Gated);
    }

    #[test]
    fn store_page_content_enriches_only_card_display_with_listing_fallbacks() {
        let mut listing = listing();
        listing.images = vec!["https://cdn.example.org/listing.webp".into()];
        listing.tags = vec!["listing-genre".into()];
        let page = StorePageCardPresentation {
            listing_coordinate: "30402:publisher:game-id".into(),
            store_page_coordinate: "30407:publisher:page".into(),
            event_id: "event".into(),
            title: Some("Store title".into()),
            summary: Some("Store summary".into()),
            capsule_url: Some("https://cdn.example.org/capsule.webp".into()),
            hero_url: Some("https://cdn.example.org/hero.webp".into()),
            genres: vec!["action".into(), "adventure".into()],
            features: vec!["windows-x86_64".into()],
            release_date: Some("2026-10-12".into()),
        };
        let display = resolve_card_content(&listing, Some(&page), None);
        assert_eq!(display.title, "Store title");
        assert_eq!(display.summary, "Store summary");
        assert_eq!(display.image_url, "https://cdn.example.org/capsule.webp");
        assert_eq!(display.badges, vec!["action", "adventure"]);

        let fallback = resolve_card_content(&listing, None, None);
        assert_eq!(fallback.title, listing.title);
        assert_eq!(fallback.image_url, listing.images[0]);
        assert_eq!(fallback.badges, listing.tags);

        let authoritative = GameCardPresentation::from_listing(
            &listing,
            1,
            PlatformCompatibility::Incompatible,
            false,
        );
        assert_eq!(
            authoritative.compatibility,
            PlatformCompatibility::Incompatible
        );
        assert_eq!(authoritative.action, Some(GameCardAction::ViewDetails));
    }
}
