use leptos::prelude::*;

use crate::models::{AcquisitionPolicy, GameListing, StorePageCardPresentation};
use crate::ui_v2::components::{
    artwork_state_from_url, GameArtwork, GameCardSkeleton, StatusChip, StatusChipSize,
    StatusChipVariant,
};

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

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum GameCardDensity {
    #[default]
    Home,
    Browse,
}

impl GameCardDensity {
    const fn class(self) -> &'static str {
        match self {
            Self::Home => "arc-game-card-home",
            Self::Browse => "arc-game-card-browse",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GameCardPresentation {
    pub access: GameCardAccess,
    pub owned: bool,
    pub campaign: Option<GameCardCampaign>,
    pub compatibility: PlatformCompatibility,
    pub installed: bool,
    pub action: Option<GameCardAction>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GameCardStatus {
    pub label: String,
    pub variant: StatusChipVariant,
    pub icon: Option<&'static str>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GameCardActionPresentation {
    pub label: String,
    pub disabled: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GameCardVisualContent {
    pub title: String,
    pub publisher: String,
    pub artwork_url: Option<String>,
    pub summary: Option<String>,
    pub secondary_metadata: Vec<String>,
    pub price_or_acquisition: Option<String>,
    pub statuses: Vec<GameCardStatus>,
}

impl GameCardPresentation {
    pub fn from_listing(
        listing: &GameListing,
        now: u64,
        compatibility: PlatformCompatibility,
        installed: bool,
    ) -> Self {
        let access = match listing.acquisition {
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
        };

        let action = if compatibility == PlatformCompatibility::Incompatible {
            Some(GameCardAction::ViewDetails)
        } else if installed {
            Some(GameCardAction::ViewDetails)
        } else if listing.is_owned {
            Some(GameCardAction::Install)
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
            owned: listing.is_owned,
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
        let access = GameCardAccess::Gated;
        let action = if compatibility == PlatformCompatibility::Incompatible || installed {
            Some(GameCardAction::ViewDetails)
        } else if claimed {
            Some(GameCardAction::Install)
        } else {
            Some(GameCardAction::Claim)
        };

        Self {
            access,
            owned: claimed,
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
            && !self.owned
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
    #[prop(optional)] density: GameCardDensity,
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
    let price = price_label(&listing, presentation);
    let action = presentation.action;
    let statuses = card_statuses(presentation);
    let action_presentation = action.map(|action| GameCardActionPresentation {
        label: action_label(action, presentation).to_string(),
        disabled: false,
    });
    let action_callback = on_action.map(|callback| {
        Callback::new(move |_| {
            if let Some(action) = action {
                callback.run(action);
            }
        })
    });
    let content = GameCardVisualContent {
        title,
        publisher,
        artwork_url: image_url,
        summary: (!summary.trim().is_empty()).then_some(summary),
        secondary_metadata: tags,
        price_or_acquisition: Some(price),
        statuses,
    };

    view! {
        <GameCardVisual
            content=content
            on_open=Callback::new(move |_| on_open.run(title_for_open.clone()))
            action=action_presentation
            on_action=action_callback
            favorite=None
            on_favorite=None
            density=density
        />
    }
}

#[component]
pub fn GameCardVisual(
    content: GameCardVisualContent,
    on_open: Callback<()>,
    action: Option<GameCardActionPresentation>,
    on_action: Option<Callback<()>>,
    favorite: Option<bool>,
    on_favorite: Option<Callback<bool>>,
    #[prop(optional)] density: GameCardDensity,
    #[prop(optional)] disabled: bool,
    #[prop(optional)] loading: bool,
) -> impl IntoView {
    if loading {
        return view! {
            <GameCardSkeleton announce=true browse=density == GameCardDensity::Browse />
        }
        .into_any();
    }

    let title = content.title.clone();
    let title_for_art = title.clone();
    let metadata = content.secondary_metadata.join(" · ");
    let has_metadata = !metadata.is_empty();
    let artwork = artwork_state_from_url(content.artwork_url.clone());

    view! {
        <article class=format!("arc-game-card {}", density.class()) class:arc-game-card-disabled=disabled>
            <button
                type="button"
                class="arc-game-card-main"
                disabled=disabled
                aria-label={format!("Open details for {title}")}
                on:click=move |_| {
                    if !disabled {
                        on_open.run(());
                    }
                }
            >
                <div class="arc-game-card-art">
                    <GameArtwork title=title_for_art state=artwork />
                    <span class="arc-game-card-art-shade" aria-hidden="true"></span>
                    <strong class="arc-game-card-title">{content.title}</strong>
                </div>
                <div class="arc-game-card-body">
                    <div class="arc-game-card-meta-row">
                        <span class="arc-game-card-publisher">{content.publisher}</span>
                        {content.price_or_acquisition.map(|label| view! {
                            <span class="arc-game-card-price">{label}</span>
                        })}
                    </div>
                    {has_metadata.then(|| view! { <span class="arc-game-card-metadata">{metadata}</span> })}
                    {content.summary.map(|summary| view! { <span class="arc-game-card-summary">{summary}</span> })}
                    <div class="arc-game-card-statuses">
                        {content.statuses
                            .into_iter()
                            .map(|status| view! {
                                <StatusChip
                                    label=status.label
                                    variant=status.variant
                                    icon=status.icon
                                    size=StatusChipSize::Compact
                                />
                            })
                            .collect_view()}
                    </div>
                </div>
            </button>

            {favorite.zip(on_favorite).map(|(selected, callback)| view! {
                <button
                    type="button"
                    class="arc-game-card-favorite"
                    class:arc-game-card-favorite-active=selected
                    aria-label=if selected { "Remove from favorites" } else { "Add to favorites" }
                    aria-pressed=selected
                    disabled=disabled
                    on:click=move |_| callback.run(!selected)
                >
                    <span class="material-symbols-outlined" aria-hidden="true">"favorite"</span>
                </button>
            })}

            {action.zip(on_action).map(|(action, callback)| view! {
                <button
                    type="button"
                    class="arc-game-card-action"
                    disabled=disabled || action.disabled
                    on:click=move |_| callback.run(())
                >
                    {action.label}
                </button>
            })}
        </article>
    }
    .into_any()
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct CardDisplayContent {
    title: String,
    summary: String,
    image_url: Option<String>,
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
        .or_else(|| valid_cover_url(&listing.images));
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

fn card_statuses(presentation: GameCardPresentation) -> Vec<GameCardStatus> {
    let mut statuses = vec![GameCardStatus {
        label: access_badge(presentation.access).to_string(),
        variant: match presentation.access {
            GameCardAccess::Paid => StatusChipVariant::Active,
            GameCardAccess::Owned => StatusChipVariant::Owned,
            GameCardAccess::Public => StatusChipVariant::Public,
            GameCardAccess::TimedActive
            | GameCardAccess::TimedUpcoming
            | GameCardAccess::TimedExpired => StatusChipVariant::TimedAccess,
            GameCardAccess::Gated => StatusChipVariant::Gated,
        },
        icon: None,
    }];

    if let Some(campaign) = presentation.campaign {
        statuses.push(GameCardStatus {
            label: match campaign {
                GameCardCampaign::Active => "Claim and keep",
                GameCardCampaign::Upcoming => "Claim upcoming",
                GameCardCampaign::Ended => "Claim ended",
                GameCardCampaign::Claimed => "Entitlement claimed",
            }
            .to_string(),
            variant: match campaign {
                GameCardCampaign::Active | GameCardCampaign::Claimed => StatusChipVariant::Success,
                GameCardCampaign::Upcoming => StatusChipVariant::Pending,
                GameCardCampaign::Ended => StatusChipVariant::Expired,
            },
            icon: None,
        });
    }

    if presentation.compatibility == PlatformCompatibility::Incompatible {
        statuses.push(GameCardStatus {
            label: "Incompatible".to_string(),
            variant: StatusChipVariant::Error,
            icon: None,
        });
    }
    if presentation.installed {
        statuses.push(GameCardStatus {
            label: "Installed".to_string(),
            variant: StatusChipVariant::Installed,
            icon: None,
        });
    }
    if presentation.owned && presentation.access != GameCardAccess::Owned {
        statuses.push(GameCardStatus {
            label: "Owned".to_string(),
            variant: StatusChipVariant::Owned,
            icon: None,
        });
    }
    statuses
}

fn price_label(listing: &GameListing, presentation: GameCardPresentation) -> String {
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
        assert_eq!(owned.access, GameCardAccess::Paid);
        assert!(owned.owned);
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
        assert_eq!(claimed.access, GameCardAccess::Gated);
        assert!(claimed.owned);
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
        assert_eq!(
            display.image_url.as_deref(),
            Some("https://cdn.example.org/capsule.webp")
        );
        assert_eq!(display.badges, vec!["action", "adventure"]);

        let fallback = resolve_card_content(&listing, None, None);
        assert_eq!(fallback.title, listing.title);
        assert_eq!(
            fallback.image_url.as_deref(),
            Some(listing.images[0].as_str())
        );
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

    #[test]
    fn visual_statuses_keep_access_install_and_compatibility_separate() {
        let presentation = GameCardPresentation {
            access: GameCardAccess::Public,
            owned: false,
            campaign: None,
            compatibility: PlatformCompatibility::Incompatible,
            installed: true,
            action: Some(GameCardAction::ViewDetails),
        };
        let statuses = card_statuses(presentation);

        assert!(statuses
            .iter()
            .any(|status| status.variant == StatusChipVariant::Public));
        assert!(statuses
            .iter()
            .any(|status| status.variant == StatusChipVariant::Installed));
        assert!(statuses
            .iter()
            .any(|status| status.variant == StatusChipVariant::Error));
    }

    #[test]
    fn card_primary_favorite_and_action_controls_remain_separate() {
        let source = include_str!("game_card.rs");
        assert!(source.contains("class=\"arc-game-card-main\""));
        assert!(source.contains("class=\"arc-game-card-favorite\""));
        assert!(source.contains("class=\"arc-game-card-action\""));
        assert!(source.contains("aria-label={format!(\"Open details for {title}\")}"));
    }

    #[test]
    fn home_and_browse_cards_have_distinct_artwork_density() {
        assert_eq!(GameCardDensity::Home.class(), "arc-game-card-home");
        assert_eq!(GameCardDensity::Browse.class(), "arc-game-card-browse");
    }
}
