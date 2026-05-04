//! NIP-58 badge parsing, validation, relay fetching, and cache coordination.

use serde::{Deserialize, Serialize};
use thiserror::Error;

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
        return Err(AchievementError::Storage(
            "profile badge event must be kind 10008 or deprecated kind 30008".to_string(),
        ));
    }

    Ok(ProfileBadgeList {
        profile_pubkey: profile_pubkey.to_string(),
        event_id: event.id.to_hex(),
        kind,
        created_at: event.created_at.as_secs(),
        entries: parse_profile_badge_entries(event),
    })
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
                entries.push(ProfileBadgeSelection {
                    badge_coordinate: badge_coordinate.to_string(),
                    award_event_id: award_event_id.to_string(),
                    relay_url: next.get(2).filter(|value| !value.is_empty()).cloned(),
                    display_order: entries.len(),
                });
            }
            index += 2;
        } else {
            index += 1;
        }
    }

    entries
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
