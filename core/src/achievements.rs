//! NIP-58 badge parsing, validation, relay fetching, and cache coordination.

use serde::{Deserialize, Serialize};
use thiserror::Error;

#[cfg(feature = "native")]
use nostr::{Event, PublicKey};
#[cfg(feature = "native")]
use nostr_sdk::{Filter, Kind};
#[cfg(feature = "native")]
use std::sync::Arc;
#[cfg(feature = "native")]
use std::time::Duration;
#[cfg(feature = "native")]
use tracing::{info, warn};

pub const KIND_BADGE_AWARD: u16 = 8;
pub const KIND_PROFILE_BADGES_CURRENT: u16 = 10008;
pub const KIND_PROFILE_BADGES_DEPRECATED: u16 = 30008;
pub const KIND_BADGE_DEFINITION: u16 = 30009;
pub const PROFILE_BADGES_DEPRECATED_D: &str = "profile_badges";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct BadgeDefinition {
    pub coordinate: String,
    pub issuer_pubkey: String,
    pub badge_id: String,
    pub name: Option<String>,
    pub description: Option<String>,
    pub image_url: Option<String>,
    pub image_dimensions: Option<String>,
    pub thumb_url: Option<String>,
    pub thumb_dimensions: Option<String>,
    pub relay_url: Option<String>,
    pub event_id: String,
    pub created_at: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct BadgeAward {
    pub event_id: String,
    pub issuer_pubkey: String,
    pub recipient_pubkey: String,
    pub badge_coordinate: String,
    pub relay_url: Option<String>,
    pub created_at: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ProfileBadgeSelection {
    pub badge_coordinate: String,
    pub award_event_id: String,
    pub relay_url: Option<String>,
    pub display_order: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ProfileBadgeList {
    pub profile_pubkey: String,
    pub event_id: String,
    pub kind: u16,
    pub created_at: u64,
    pub entries: Vec<ProfileBadgeSelection>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ProfileBadgeEntry {
    pub definition: BadgeDefinition,
    pub award: BadgeAward,
    pub display_order: usize,
    pub visible: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct EarnedBadgeSummary {
    pub definition: BadgeDefinition,
    pub award: BadgeAward,
    pub visible_on_profile: bool,
}

#[derive(Debug, Error)]
pub enum AchievementError {
    #[error("badge definition must be kind 30009")]
    InvalidDefinitionKind,
    #[error("badge definition missing non-empty d tag")]
    MissingDefinitionDTag,
    #[error("badge award must be kind 8")]
    InvalidAwardKind,
    #[error("badge award missing a tag")]
    MissingAwardCoordinate,
    #[error("badge award missing p tag for recipient")]
    MissingAwardRecipient,
    #[error("profile badge event pubkey does not match profile owner")]
    ProfileOwnerMismatch,
    #[error("profile badge event must be kind 10008 or deprecated kind 30008")]
    InvalidProfileBadgeKind,
    #[error("award issuer does not match definition issuer")]
    IssuerMismatch,
    #[error("relay error: {0}")]
    Relay(String),
    #[error("storage error: {0}")]
    Storage(String),
}

/// Parse a NIP-58 kind-30009 badge definition.
///
/// # Errors
/// Returns `AchievementError` when the event has the wrong kind or lacks a non-empty `d` tag.
pub fn parse_badge_definition(
    event: &nostr::Event,
    relay_url: Option<String>,
) -> Result<BadgeDefinition, AchievementError> {
    if event.kind.as_u16() != KIND_BADGE_DEFINITION {
        return Err(AchievementError::InvalidDefinitionKind);
    }

    let badge_id = first_tag_value(event, "d").ok_or(AchievementError::MissingDefinitionDTag)?;
    let issuer_pubkey = event.pubkey.to_hex();

    Ok(BadgeDefinition {
        coordinate: format!("{KIND_BADGE_DEFINITION}:{issuer_pubkey}:{badge_id}"),
        issuer_pubkey,
        badge_id,
        name: first_tag_value(event, "name"),
        description: first_tag_value(event, "description"),
        image_url: first_tag_value(event, "image"),
        image_dimensions: nth_tag_value(event, "image", 2),
        thumb_url: first_tag_value(event, "thumb"),
        thumb_dimensions: nth_tag_value(event, "thumb", 2),
        relay_url,
        event_id: event.id.to_hex(),
        created_at: event.created_at.as_secs(),
    })
}

/// Parse ordered profile badge selections from a NIP-58 profile badges event.
///
/// # Errors
/// Returns `AchievementError` when the profile owner or profile badge kind is invalid.
pub fn parse_profile_badge_list(
    event: &nostr::Event,
    profile_pubkey: &str,
) -> Result<ProfileBadgeList, AchievementError> {
    if event.pubkey.to_hex() != profile_pubkey {
        return Err(AchievementError::ProfileOwnerMismatch);
    }

    let kind = event.kind.as_u16();
    if kind == KIND_PROFILE_BADGES_DEPRECATED {
        let d_tag = first_tag_value(event, "d").ok_or(AchievementError::MissingDefinitionDTag)?;
        if d_tag != PROFILE_BADGES_DEPRECATED_D {
            return Err(AchievementError::MissingDefinitionDTag);
        }
    } else if kind != KIND_PROFILE_BADGES_CURRENT {
        return Err(AchievementError::InvalidProfileBadgeKind);
    }

    Ok(ProfileBadgeList {
        profile_pubkey: profile_pubkey.to_string(),
        event_id: event.id.to_hex(),
        kind,
        created_at: event.created_at.as_secs(),
        entries: parse_profile_badge_entries(event),
    })
}

/// Validate that an award was issued by the badge definition owner.
///
/// # Errors
/// Returns `AchievementError::IssuerMismatch` when the issuer pubkeys differ.
pub fn validate_award_issuer(
    award: &BadgeAward,
    definition: &BadgeDefinition,
) -> Result<(), AchievementError> {
    if award.issuer_pubkey == definition.issuer_pubkey {
        Ok(())
    } else {
        Err(AchievementError::IssuerMismatch)
    }
}

/// Fetch earned badge awards and lazily cache referenced definitions.
///
/// # Errors
/// Returns `AchievementError` when relay or storage access fails.
#[cfg(feature = "native")]
pub async fn fetch_user_badges(
    client: Arc<nostr_sdk::Client>,
    database: &crate::storage::Database,
    profile_pubkey: &str,
) -> Result<Vec<EarnedBadgeSummary>, AchievementError> {
    let profile_public_key = PublicKey::from_hex(profile_pubkey)
        .map_err(|error| AchievementError::Relay(error.to_string()))?;

    cache_awards_for_profile(&client, database, profile_pubkey, profile_public_key).await?;

    database
        .earned_badges_for_profile(profile_pubkey)
        .await
        .map_err(|error| AchievementError::Storage(error.to_string()))
}

/// Fetch profile-selected badges, preferring kind 10008 over deprecated 30008.
///
/// # Errors
/// Returns `AchievementError` when relay or storage access fails.
#[cfg(feature = "native")]
pub async fn fetch_profile_badges(
    client: Arc<nostr_sdk::Client>,
    database: &crate::storage::Database,
    profile_pubkey: &str,
) -> Result<Vec<ProfileBadgeEntry>, AchievementError> {
    let profile_public_key = PublicKey::from_hex(profile_pubkey)
        .map_err(|error| AchievementError::Relay(error.to_string()))?;

    let current_filter = Filter::new()
        .kind(Kind::Custom(KIND_PROFILE_BADGES_CURRENT))
        .author(profile_public_key)
        .limit(1);
    let current_events = client
        .fetch_events(current_filter, Duration::from_secs(10))
        .await
        .map_err(|error| AchievementError::Relay(error.to_string()))?
        .into_iter()
        .collect::<Vec<_>>();

    let profile_event = if let Some(event) = latest_event(current_events) {
        Some(event)
    } else {
        let deprecated_filter = Filter::new()
            .kind(Kind::ProfileBadges)
            .author(profile_public_key)
            .identifier(PROFILE_BADGES_DEPRECATED_D)
            .limit(1);
        let deprecated_events = client
            .fetch_events(deprecated_filter, Duration::from_secs(10))
            .await
            .map_err(|error| AchievementError::Relay(error.to_string()))?
            .into_iter()
            .collect::<Vec<_>>();

        let fallback = latest_event(deprecated_events);
        if fallback.is_some() {
            info!("Using deprecated NIP-58 kind-30008 profile_badges fallback");
        }
        fallback
    };

    if let Some(event) = profile_event {
        let list = parse_profile_badge_list(&event, profile_pubkey)?;
        let raw_event_json = serde_json::to_string(&event)
            .map_err(|error| AchievementError::Storage(error.to_string()))?;
        database
            .cache_profile_badge_list(&list, &raw_event_json)
            .await
            .map_err(|error| AchievementError::Storage(error.to_string()))?;

        let coordinates = list
            .entries
            .iter()
            .map(|entry| entry.badge_coordinate.clone())
            .collect::<Vec<_>>();
        cache_missing_definitions(&client, database, &coordinates).await?;
    }

    cache_awards_for_profile(&client, database, profile_pubkey, profile_public_key).await?;

    database
        .profile_badges_for_profile(profile_pubkey)
        .await
        .map_err(|error| AchievementError::Storage(error.to_string()))
}

#[cfg(feature = "native")]
async fn cache_awards_for_profile(
    client: &Arc<nostr_sdk::Client>,
    database: &crate::storage::Database,
    profile_pubkey: &str,
    profile_public_key: PublicKey,
) -> Result<(), AchievementError> {
    let awards_filter = Filter::new()
        .kind(Kind::BadgeAward)
        .pubkey(profile_public_key);
    let award_events = client
        .fetch_events(awards_filter, Duration::from_secs(10))
        .await
        .map_err(|error| AchievementError::Relay(error.to_string()))?
        .into_iter()
        .collect::<Vec<_>>();

    let mut awards = Vec::new();
    let mut coordinates = Vec::new();
    for event in award_events {
        match parse_badge_award_for_recipient(&event, profile_pubkey) {
            Ok(award) => {
                coordinates.push(award.badge_coordinate.clone());
                awards.push((award, event));
            }
            Err(error) => warn!("Skipping malformed badge award: {error}"),
        }
    }

    cache_missing_definitions(client, database, &coordinates).await?;

    for (award, event) in awards {
        let Some((_, definition_issuer, _)) = split_badge_coordinate(&award.badge_coordinate)
        else {
            warn!("Skipping badge award with invalid coordinate");
            continue;
        };
        if award.issuer_pubkey != definition_issuer {
            warn!(
                award_event_id = %award.event_id,
                award_issuer = %award.issuer_pubkey,
                definition_issuer = %definition_issuer,
                "Skipping badge award with issuer mismatch"
            );
            continue;
        }

        let raw_event_json = serde_json::to_string(&event)
            .map_err(|error| AchievementError::Storage(error.to_string()))?;
        database
            .cache_badge_award(&award, &raw_event_json)
            .await
            .map_err(|error| AchievementError::Storage(error.to_string()))?;
    }

    Ok(())
}

#[cfg(feature = "native")]
async fn cache_missing_definitions(
    client: &Arc<nostr_sdk::Client>,
    database: &crate::storage::Database,
    coordinates: &[String],
) -> Result<(), AchievementError> {
    for coordinate in coordinates {
        if badge_definition_is_cached(database, coordinate).await? {
            continue;
        }

        let Some((kind, issuer_pubkey, badge_id)) = split_badge_coordinate(coordinate) else {
            warn!("Skipping invalid badge definition coordinate '{coordinate}'");
            continue;
        };
        if kind != KIND_BADGE_DEFINITION {
            warn!("Skipping unsupported badge definition coordinate kind {kind}");
            continue;
        }

        let issuer_public_key = PublicKey::from_hex(&issuer_pubkey)
            .map_err(|error| AchievementError::Relay(error.to_string()))?;
        let definition_filter = Filter::new()
            .kind(Kind::Custom(KIND_BADGE_DEFINITION))
            .author(issuer_public_key)
            .identifier(&badge_id)
            .limit(1);
        let definition_events = client
            .fetch_events(definition_filter, Duration::from_secs(10))
            .await
            .map_err(|error| AchievementError::Relay(error.to_string()))?
            .into_iter()
            .collect::<Vec<_>>();

        for event in definition_events {
            let definition = match parse_badge_definition(&event, None) {
                Ok(definition) => definition,
                Err(error) => {
                    warn!(
                        coordinate = %coordinate,
                        event_id = %event.id.to_hex(),
                        "Skipping malformed badge definition event: {error}"
                    );
                    continue;
                }
            };
            let raw_event_json = serde_json::to_string(&event)
                .map_err(|error| AchievementError::Storage(error.to_string()))?;
            database
                .cache_badge_definition(&definition, &raw_event_json)
                .await
                .map_err(|error| AchievementError::Storage(error.to_string()))?;
        }
    }

    Ok(())
}

#[cfg(feature = "native")]
async fn badge_definition_is_cached(
    database: &crate::storage::Database,
    coordinate: &str,
) -> Result<bool, AchievementError> {
    let cached: Option<i64> =
        sqlx::query_scalar("SELECT 1 FROM badge_definitions WHERE coordinate = ? LIMIT 1")
            .bind(coordinate)
            .fetch_optional(database.pool())
            .await
            .map_err(|error| AchievementError::Storage(error.to_string()))?;

    Ok(cached.is_some())
}

#[cfg(feature = "native")]
fn parse_badge_award_for_recipient(
    event: &Event,
    profile_pubkey: &str,
) -> Result<BadgeAward, AchievementError> {
    if event.kind.as_u16() != KIND_BADGE_AWARD {
        return Err(AchievementError::InvalidAwardKind);
    }

    let badge_coordinate = first_tag_value(event, "a")
        .filter(|coordinate| is_valid_badge_coordinate(coordinate))
        .ok_or(AchievementError::MissingAwardCoordinate)?;
    let recipient_matches = event.tags.iter().any(|tag| {
        let parts = tag.as_slice();
        tag_name(parts) == Some("p") && tag_content(parts) == Some(profile_pubkey)
    });
    if !recipient_matches {
        return Err(AchievementError::MissingAwardRecipient);
    }

    Ok(BadgeAward {
        event_id: event.id.to_hex(),
        issuer_pubkey: event.pubkey.to_hex(),
        recipient_pubkey: profile_pubkey.to_string(),
        badge_coordinate,
        relay_url: None,
        created_at: event.created_at.as_secs(),
    })
}

#[cfg(feature = "native")]
fn latest_event(events: Vec<Event>) -> Option<Event> {
    events.into_iter().max_by(|left, right| {
        left.created_at
            .cmp(&right.created_at)
            .then_with(|| right.id.to_hex().cmp(&left.id.to_hex()))
    })
}

#[cfg(feature = "native")]
fn split_badge_coordinate(coordinate: &str) -> Option<(u16, String, String)> {
    let mut parts = coordinate.splitn(3, ':');
    let kind = parts.next()?.parse::<u16>().ok()?;
    let issuer = parts.next()?.to_string();
    let identifier = parts.next()?.to_string();

    if issuer.is_empty() || identifier.is_empty() {
        return None;
    }

    Some((kind, issuer, identifier))
}

fn parse_profile_badge_entries(event: &nostr::Event) -> Vec<ProfileBadgeSelection> {
    let mut entries = Vec::new();
    let tags: Vec<Vec<String>> = event
        .tags
        .iter()
        .map(|tag| tag.as_slice().to_vec())
        .collect();
    let mut index = 0;

    while index + 1 < tags.len() {
        let current = &tags[index];
        let next = &tags[index + 1];

        if tag_name(current) == Some("a") && tag_name(next) == Some("e") {
            if let (Some(badge_coordinate), Some(award_event_id)) =
                (tag_content(current), tag_content(next))
            {
                if is_valid_badge_coordinate(badge_coordinate) && is_valid_event_id(award_event_id)
                {
                    entries.push(ProfileBadgeSelection {
                        badge_coordinate: badge_coordinate.to_string(),
                        award_event_id: award_event_id.to_string(),
                        relay_url: next.get(2).filter(|value| !value.is_empty()).cloned(),
                        display_order: entries.len(),
                    });
                }
            }
            index += 2;
        } else {
            index += 1;
        }
    }

    entries
}

fn is_valid_badge_coordinate(value: &str) -> bool {
    let mut parts = value.splitn(3, ':');

    matches!(parts.next(), Some("30009"))
        && parts
            .next()
            .is_some_and(|pubkey| nostr::PublicKey::from_hex(pubkey).is_ok())
        && parts.next().is_some_and(|badge_id| !badge_id.is_empty())
}

fn is_valid_event_id(value: &str) -> bool {
    nostr::EventId::from_hex(value).is_ok()
}

fn first_tag_value(event: &nostr::Event, name: &str) -> Option<String> {
    nth_tag_value(event, name, 1)
}

fn nth_tag_value(event: &nostr::Event, name: &str, index: usize) -> Option<String> {
    event.tags.iter().find_map(|tag| {
        let parts = tag.as_slice();
        if tag_name(parts) == Some(name) {
            parts.get(index).filter(|value| !value.is_empty()).cloned()
        } else {
            None
        }
    })
}

fn tag_name(parts: &[String]) -> Option<&str> {
    parts.first().map(String::as_str)
}

fn tag_content(parts: &[String]) -> Option<&str> {
    parts
        .get(1)
        .filter(|value| !value.is_empty())
        .map(String::as_str)
}
