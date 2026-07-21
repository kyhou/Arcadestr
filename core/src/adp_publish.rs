//! Pure ADP publish event builders.

use std::collections::HashSet;

use nostr::{EventBuilder, Kind, Tag, TagKind};
use thiserror::Error;

use crate::authorization::FULFILLMENT_AUTHORIZATION_KIND;
use crate::marketplace::AcquisitionPolicy;

/// Input required to construct an ADP NIP-99 listing event.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AdpListingInput {
    pub d_tag: String,
    pub title: String,
    pub description: String,
    pub price_sats: u64,
    pub lud16: Option<String>,
    pub tags: Vec<String>,
    pub images: Vec<String>,
    pub servers: Vec<String>,
    pub file_hash: Option<String>,
    pub version: Option<String>,
    pub fulfillment_pubkey: Option<String>,
    pub fulfillment_valid_from: Option<u64>,
    pub fulfillment_revoked_at: Option<u64>,
    pub acquisition: AcquisitionPolicy,
    pub platforms: Vec<String>,
    pub campaigns: Vec<(String, Option<String>)>,
    pub nip94_event_id: Option<String>,
    pub preserved_tags: Vec<Vec<String>>,
}

/// Errors returned while building ADP publish events.
#[derive(Debug, Error, PartialEq, Eq)]
pub enum AdpPublishError {
    /// Priced ADP listings require a listing-level LUD-16 address.
    #[error("priced ADP listing requires lud16 tag")]
    MissingLud16,
    /// Fulfillment fields must be provided together.
    #[error("fulfillment tier is incomplete; missing: {missing}")]
    IncompleteFulfillmentTier { missing: String },
    /// Server tags must be absolute HTTP(S) URLs.
    #[error("malformed ADP server URL: {url}")]
    MalformedServerUrl { url: String },
    /// Timed access requires a non-empty interval.
    #[error("timed access end must be after its start")]
    InvalidTimedAccess,
    /// A tag preserved from the current listing could not be reconstructed.
    #[error("malformed preserved listing tag")]
    MalformedPreservedTag,
    /// Hosted image fields must be absolute HTTP(S) URLs.
    #[error("malformed image URL: {url}")]
    MalformedImageUrl { url: String },
    /// Platform values must be unique `<os>-<arch>` tags.
    #[error("malformed or duplicate platform tag: {platform}")]
    InvalidPlatform { platform: String },
    /// Fulfillment hashes must be SHA-256 hex.
    #[error("malformed SHA-256 file hash")]
    InvalidFileHash,
    /// Campaign pointers must contain a valid event ID and optional relay URL.
    #[error("malformed campaign pointer")]
    InvalidCampaignPointer,
    /// NIP-94 links must contain a valid event ID.
    #[error("malformed NIP-94 event ID")]
    InvalidNip94EventId,
}

fn is_http_url(value: &str) -> bool {
    url::Url::parse(value).is_ok_and(|parsed| {
        matches!(parsed.scheme(), "http" | "https")
            && parsed.host_str().is_some()
            && !value
                .strip_prefix("https://")
                .or_else(|| value.strip_prefix("http://"))
                .is_some_and(|remainder| remainder.starts_with('/'))
    })
}

fn is_platform_tag(value: &str) -> bool {
    let Some((os, arch)) = value.split_once('-') else {
        return false;
    };
    !os.is_empty()
        && !arch.is_empty()
        && !arch.contains('-')
        && os
            .chars()
            .chain(arch.chars())
            .all(|character| character.is_ascii_alphanumeric() || character == '_')
}

/// Builds a developer-signed fulfillment authorization root.
pub fn build_fulfillment_authorization_event_builder(
    authorization_id: &str,
    listing_coordinate: &str,
    fulfillment_pubkey: &str,
    valid_from: u64,
) -> EventBuilder {
    EventBuilder::new(Kind::Custom(FULFILLMENT_AUTHORIZATION_KIND), "").tags([
        Tag::custom(TagKind::Custom("d".into()), [authorization_id.to_string()]),
        Tag::custom(
            TagKind::Custom("authorization_id".into()),
            [authorization_id.to_string()],
        ),
        Tag::custom(
            TagKind::Custom("a".into()),
            [listing_coordinate.to_string()],
        ),
        Tag::custom(TagKind::p(), [fulfillment_pubkey.to_string()]),
        Tag::custom(
            TagKind::Custom("valid_from".into()),
            [valid_from.to_string()],
        ),
        Tag::custom(TagKind::Custom("status".into()), ["active"]),
    ])
}

/// Builds the developer-signed `kind:30402` listing event.
///
/// # Errors
/// Returns [`AdpPublishError::MissingLud16`] when `price_sats > 0` and no LUD-16 is provided.
/// Returns [`AdpPublishError::IncompleteFulfillmentTier`] when only part of the fulfillment tier is present.
/// Returns [`AdpPublishError::MalformedServerUrl`] when a declared server is not an HTTP(S) URL.
/// Returns [`AdpPublishError::InvalidTimedAccess`] when a timed policy has an empty interval.
pub fn build_adp_listing_event_builder(
    input: &AdpListingInput,
) -> Result<EventBuilder, AdpPublishError> {
    if input.price_sats > 0 && input.lud16.as_deref().unwrap_or_default().is_empty() {
        return Err(AdpPublishError::MissingLud16);
    }

    let fulfillment_fields = [
        ("server", !input.servers.is_empty()),
        (
            "file_hash",
            input
                .file_hash
                .as_deref()
                .is_some_and(|value| !value.is_empty()),
        ),
        (
            "version",
            input
                .version
                .as_deref()
                .is_some_and(|value| !value.is_empty()),
        ),
        (
            "fulfillment_pubkey",
            input
                .fulfillment_pubkey
                .as_deref()
                .is_some_and(|value| !value.is_empty()),
        ),
    ];
    let has_any_fulfillment = fulfillment_fields.iter().any(|(_, present)| *present)
        || input.fulfillment_valid_from.is_some()
        || input.fulfillment_revoked_at.is_some();
    let missing_fulfillment = fulfillment_fields
        .iter()
        .filter_map(|(name, present)| (!present).then_some(*name))
        .collect::<Vec<_>>();
    if has_any_fulfillment && !missing_fulfillment.is_empty() {
        return Err(AdpPublishError::IncompleteFulfillmentTier {
            missing: missing_fulfillment.join(", "),
        });
    }

    for server in &input.servers {
        if !is_http_url(server) {
            return Err(AdpPublishError::MalformedServerUrl {
                url: server.clone(),
            });
        }
    }

    if matches!(
        input.acquisition,
        AcquisitionPolicy::TimedAccess { starts_at, ends_at } if starts_at >= ends_at
    ) {
        return Err(AdpPublishError::InvalidTimedAccess);
    }

    for image in &input.images {
        if !image.is_empty() && !is_http_url(image) {
            return Err(AdpPublishError::MalformedImageUrl { url: image.clone() });
        }
    }

    let mut platforms = HashSet::new();
    for platform in &input.platforms {
        if !is_platform_tag(platform) || !platforms.insert(platform) {
            return Err(AdpPublishError::InvalidPlatform {
                platform: platform.clone(),
            });
        }
    }

    if input
        .file_hash
        .as_deref()
        .is_some_and(|hash| !crate::is_sha256_hex(hash))
    {
        return Err(AdpPublishError::InvalidFileHash);
    }

    for (root_event_id, relay_hint) in &input.campaigns {
        if nostr::EventId::from_hex(root_event_id).is_err()
            || relay_hint
                .as_deref()
                .is_some_and(|relay| nostr::RelayUrl::parse(relay).is_err())
        {
            return Err(AdpPublishError::InvalidCampaignPointer);
        }
    }

    if input
        .nip94_event_id
        .as_deref()
        .is_some_and(|event_id| nostr::EventId::from_hex(event_id).is_err())
    {
        return Err(AdpPublishError::InvalidNip94EventId);
    }

    let mut tags = vec![
        Tag::custom(TagKind::Custom("d".into()), [input.d_tag.clone()]),
        Tag::custom(TagKind::Custom("title".into()), [input.title.clone()]),
        Tag::custom(
            TagKind::Custom("price".into()),
            [input.price_sats.to_string(), "sat".to_string()],
        ),
        Tag::custom(TagKind::Custom("t".into()), ["game".to_string()]),
    ];

    for values in &input.preserved_tags {
        tags.push(Tag::parse(values.clone()).map_err(|_| AdpPublishError::MalformedPreservedTag)?);
    }

    tags.push(match &input.acquisition {
        AcquisitionPolicy::Gated => Tag::custom(TagKind::Custom("acquisition".into()), ["gated"]),
        AcquisitionPolicy::Public => Tag::custom(TagKind::Custom("acquisition".into()), ["public"]),
        AcquisitionPolicy::TimedAccess { starts_at, ends_at } => Tag::custom(
            TagKind::Custom("acquisition".into()),
            [
                "timed-access".to_string(),
                starts_at.to_string(),
                ends_at.to_string(),
            ],
        ),
    });

    for image in &input.images {
        if !image.is_empty() {
            tags.push(Tag::custom(
                TagKind::Custom("image".into()),
                [image.clone()],
            ));
        }
    }

    for tag in &input.tags {
        if !tag.is_empty() {
            tags.push(Tag::custom(TagKind::Custom("t".into()), [tag.clone()]));
        }
    }

    if has_any_fulfillment {
        for server in &input.servers {
            tags.push(Tag::custom(
                TagKind::Custom("server".into()),
                [server.clone()],
            ));
        }
        tags.push(Tag::custom(
            TagKind::Custom("file_hash".into()),
            [input.file_hash.clone().expect("file_hash checked above")],
        ));
        tags.push(Tag::custom(
            TagKind::Custom("version".into()),
            [input.version.clone().expect("version checked above")],
        ));
        tags.push(Tag::custom(
            TagKind::Custom("fulfillment_pubkey".into()),
            [
                input
                    .fulfillment_pubkey
                    .clone()
                    .expect("fulfillment_pubkey checked above"),
                input
                    .fulfillment_valid_from
                    .map(|value| value.to_string())
                    .unwrap_or_default(),
                input
                    .fulfillment_revoked_at
                    .map(|value| value.to_string())
                    .unwrap_or_default(),
            ],
        ));
    }

    if let Some(lud16) = &input.lud16 {
        if !lud16.is_empty() {
            tags.push(Tag::custom(
                TagKind::Custom("lud16".into()),
                [lud16.clone()],
            ));
        }
    }

    for platform in &input.platforms {
        tags.push(Tag::custom(
            TagKind::Custom("platform".into()),
            [platform.clone()],
        ));
    }

    for (root_event_id, relay_hint) in &input.campaigns {
        if root_event_id.is_empty() {
            continue;
        }
        let mut values = vec![root_event_id.clone()];
        if let Some(relay_hint) = relay_hint.as_ref().filter(|value| !value.is_empty()) {
            values.push(relay_hint.clone());
        }
        tags.push(Tag::custom(TagKind::Custom("campaign".into()), values));
    }

    if let Some(nip94_event_id) = input
        .nip94_event_id
        .as_ref()
        .filter(|value| !value.is_empty())
    {
        tags.push(Tag::custom(
            TagKind::Custom("nip94".into()),
            [nip94_event_id.clone()],
        ));
    }

    Ok(EventBuilder::new(Kind::Custom(30402), input.description.clone()).tags(tags))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::authorization::{parse_authorization_event, AuthorizationTransition};
    use nostr::{Keys, Kind};

    const VALID_SHA256: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";

    fn tag_values(event: &nostr::Event, name: &str) -> Vec<Vec<String>> {
        event
            .tags
            .iter()
            .map(|tag| {
                tag.clone()
                    .to_vec()
                    .iter()
                    .map(ToString::to_string)
                    .collect::<Vec<_>>()
            })
            .filter(|values| values.first().is_some_and(|kind| kind == name))
            .collect()
    }

    fn minimal_listing_input() -> AdpListingInput {
        AdpListingInput {
            d_tag: "game".into(),
            title: "Game".into(),
            description: "Description".into(),
            price_sats: 0,
            lud16: None,
            tags: Vec::new(),
            images: Vec::new(),
            servers: Vec::new(),
            file_hash: None,
            version: None,
            fulfillment_pubkey: None,
            fulfillment_valid_from: None,
            fulfillment_revoked_at: None,
            acquisition: AcquisitionPolicy::Gated,
            platforms: Vec::new(),
            campaigns: Vec::new(),
            nip94_event_id: None,
            preserved_tags: Vec::new(),
        }
    }

    #[test]
    fn rejects_malformed_phase7_metadata_before_signing() {
        let mut input = minimal_listing_input();
        input.images = vec!["file:///tmp/cover.png".into()];
        assert!(matches!(
            build_adp_listing_event_builder(&input),
            Err(AdpPublishError::MalformedImageUrl { .. })
        ));

        input.images.clear();
        input.platforms = vec!["linux-x86_64".into(), "linux-x86_64".into()];
        assert!(matches!(
            build_adp_listing_event_builder(&input),
            Err(AdpPublishError::InvalidPlatform { .. })
        ));

        input.platforms.clear();
        input.campaigns = vec![("not-an-event-id".into(), None)];
        assert_eq!(
            build_adp_listing_event_builder(&input),
            Err(AdpPublishError::InvalidCampaignPointer)
        );

        input.campaigns.clear();
        input.nip94_event_id = Some("not-an-event-id".into());
        assert_eq!(
            build_adp_listing_event_builder(&input),
            Err(AdpPublishError::InvalidNip94EventId)
        );

        input.nip94_event_id = None;
        input.servers = vec!["https://dist.example.com".into()];
        input.file_hash = Some("not-a-sha256".into());
        input.version = Some("1.0.0".into());
        input.fulfillment_pubkey = Some("fulfillment-key".into());
        assert_eq!(
            build_adp_listing_event_builder(&input),
            Err(AdpPublishError::InvalidFileHash)
        );
    }

    #[tokio::test]
    async fn builds_parseable_fulfillment_authorization_root() {
        let developer = Keys::generate();
        let fulfillment = Keys::generate();
        let authorization_id = "authorization-1";
        let coordinate = format!("30402:{}:game", developer.public_key().to_hex());
        let valid_from = 1_700_000_000;

        let event = build_fulfillment_authorization_event_builder(
            authorization_id,
            &coordinate,
            &fulfillment.public_key().to_hex(),
            valid_from,
        )
        .sign_with_keys(&developer)
        .expect("event should sign");

        assert_eq!(event.kind, Kind::Custom(FULFILLMENT_AUTHORIZATION_KIND));
        assert_eq!(event.tags.len(), 6);

        let parsed = parse_authorization_event(&event).expect("authorization should parse");
        let AuthorizationTransition::ActiveRoot(terms) = parsed.transition else {
            panic!("authorization should be an active root");
        };
        assert_eq!(terms.authorization_id, authorization_id);
        assert_eq!(terms.coordinate, coordinate);
        assert_eq!(terms.fulfillment_pubkey, fulfillment.public_key());
        assert_eq!(terms.valid_from, valid_from);
        assert_eq!(terms.valid_until, None);
    }

    #[tokio::test]
    async fn builds_kind_30402_adp_listing_with_required_tags() {
        let keys = Keys::generate();
        let input = AdpListingInput {
            d_tag: "my-game".to_string(),
            title: "My Game".to_string(),
            description: "Fun game".to_string(),
            price_sats: 2100,
            lud16: Some("studio@example.com".to_string()),
            tags: vec!["arcade".to_string(), "nostr".to_string()],
            images: vec!["https://cdn.example.com/cover.png".to_string()],
            servers: vec![
                "https://dist.example.com".to_string(),
                "https://mirror.example.com".to_string(),
            ],
            file_hash: Some(VALID_SHA256.to_string()),
            version: Some("1.0.0".to_string()),
            fulfillment_pubkey: Some("fulfillment-pubkey".to_string()),
            fulfillment_valid_from: Some(1_725_000_000),
            fulfillment_revoked_at: Some(1_725_000_999),
            acquisition: AcquisitionPolicy::Gated,
            platforms: vec!["linux-x86_64".to_string(), "windows-x86_64".to_string()],
            campaigns: vec![(
                "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa".to_string(),
                Some("wss://relay.example.com".to_string()),
            )],
            nip94_event_id: Some(
                "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb".to_string(),
            ),
            preserved_tags: vec![vec!["summary".into(), "Short description".into()]],
        };

        let event = build_adp_listing_event_builder(&input)
            .expect("listing builder should be created")
            .sign_with_keys(&keys)
            .expect("event should sign");

        assert_eq!(event.kind, Kind::Custom(30402));
        assert_eq!(
            tag_values(&event, "acquisition")[0],
            vec!["acquisition", "gated"]
        );
        assert_eq!(tag_values(&event, "d")[0], vec!["d", "my-game"]);
        assert_eq!(tag_values(&event, "title")[0], vec!["title", "My Game"]);
        assert_eq!(
            tag_values(&event, "summary")[0],
            vec!["summary", "Short description"]
        );
        assert_eq!(
            tag_values(&event, "campaign")[0],
            vec![
                "campaign",
                "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
                "wss://relay.example.com"
            ]
        );
        assert_eq!(
            tag_values(&event, "nip94")[0],
            vec![
                "nip94",
                "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"
            ]
        );
        assert_eq!(tag_values(&event, "price")[0], vec!["price", "2100", "sat"]);
        assert_eq!(
            tag_values(&event, "image"),
            vec![vec![
                "image".to_string(),
                "https://cdn.example.com/cover.png".to_string()
            ]]
        );
        assert_eq!(
            tag_values(&event, "t"),
            vec![
                vec!["t".to_string(), "game".to_string()],
                vec!["t".to_string(), "arcade".to_string()],
                vec!["t".to_string(), "nostr".to_string()],
            ]
        );
        assert_eq!(
            tag_values(&event, "server"),
            vec![
                vec!["server".to_string(), "https://dist.example.com".to_string()],
                vec![
                    "server".to_string(),
                    "https://mirror.example.com".to_string()
                ],
            ]
        );
        assert_eq!(
            tag_values(&event, "file_hash")[0],
            vec!["file_hash", VALID_SHA256]
        );
        assert_eq!(tag_values(&event, "version")[0], vec!["version", "1.0.0"]);
        assert_eq!(
            tag_values(&event, "fulfillment_pubkey")[0],
            vec![
                "fulfillment_pubkey",
                "fulfillment-pubkey",
                "1725000000",
                "1725000999"
            ]
        );
        assert_eq!(
            tag_values(&event, "lud16")[0],
            vec!["lud16", "studio@example.com"]
        );
        assert_eq!(tag_values(&event, "platform").len(), 2);
    }

    #[tokio::test]
    async fn emits_public_and_timed_acquisition_tags() {
        let keys = Keys::generate();
        let mut input = AdpListingInput {
            d_tag: "public-game".to_string(),
            title: "Public Game".to_string(),
            description: "Publicly accessible".to_string(),
            price_sats: 0,
            lud16: None,
            tags: vec![],
            images: vec![],
            servers: vec![],
            file_hash: None,
            version: None,
            fulfillment_pubkey: None,
            fulfillment_valid_from: None,
            fulfillment_revoked_at: None,
            acquisition: AcquisitionPolicy::Public,
            platforms: vec![],
            campaigns: Vec::new(),
            nip94_event_id: None,
            preserved_tags: Vec::new(),
        };

        let public_event = build_adp_listing_event_builder(&input)
            .expect("public listing should build")
            .sign_with_keys(&keys)
            .expect("public listing should sign");
        assert_eq!(
            tag_values(&public_event, "acquisition")[0],
            vec!["acquisition", "public"]
        );

        input.acquisition = AcquisitionPolicy::TimedAccess {
            starts_at: 100,
            ends_at: 200,
        };
        let timed_event = build_adp_listing_event_builder(&input)
            .expect("timed listing should build")
            .sign_with_keys(&keys)
            .expect("timed listing should sign");
        assert_eq!(
            tag_values(&timed_event, "acquisition")[0],
            vec!["acquisition", "timed-access", "100", "200"]
        );
    }

    #[test]
    fn rejects_invalid_timed_acquisition_window() {
        let input = AdpListingInput {
            d_tag: "invalid-timed-game".to_string(),
            title: "Invalid Timed Game".to_string(),
            description: "Invalid timed access".to_string(),
            price_sats: 0,
            lud16: None,
            tags: vec![],
            images: vec![],
            servers: vec![],
            file_hash: None,
            version: None,
            fulfillment_pubkey: None,
            fulfillment_valid_from: None,
            fulfillment_revoked_at: None,
            acquisition: AcquisitionPolicy::TimedAccess {
                starts_at: 200,
                ends_at: 100,
            },
            platforms: vec![],
            campaigns: Vec::new(),
            nip94_event_id: None,
            preserved_tags: Vec::new(),
        };

        assert_eq!(
            build_adp_listing_event_builder(&input).expect_err("invalid window should fail"),
            AdpPublishError::InvalidTimedAccess
        );
    }

    #[test]
    fn priced_listing_requires_lud16() {
        let input = AdpListingInput {
            d_tag: "my-game".to_string(),
            title: "My Game".to_string(),
            description: "Fun game".to_string(),
            price_sats: 1,
            lud16: None,
            tags: vec![],
            images: vec![],
            servers: vec![],
            file_hash: None,
            version: None,
            fulfillment_pubkey: None,
            fulfillment_valid_from: None,
            fulfillment_revoked_at: None,
            acquisition: AcquisitionPolicy::Gated,
            platforms: vec![],
            campaigns: Vec::new(),
            nip94_event_id: None,
            preserved_tags: Vec::new(),
        };

        let err = build_adp_listing_event_builder(&input)
            .expect_err("priced listing without lud16 should fail");

        assert!(matches!(err, AdpPublishError::MissingLud16));
    }

    #[tokio::test]
    async fn buy_only_paid_listing_with_lud16_emits_no_fulfillment_tags() {
        let keys = Keys::generate();
        let input = AdpListingInput {
            d_tag: "buy-only".to_string(),
            title: "Buy Only".to_string(),
            description: "Purchasable without automated install".to_string(),
            price_sats: 2100,
            lud16: Some("studio@example.com".to_string()),
            tags: vec!["puzzle".to_string()],
            images: vec![],
            servers: vec![],
            file_hash: None,
            version: None,
            fulfillment_pubkey: None,
            fulfillment_valid_from: None,
            fulfillment_revoked_at: None,
            acquisition: AcquisitionPolicy::Gated,
            platforms: vec!["linux-x86_64".to_string()],
            campaigns: Vec::new(),
            nip94_event_id: None,
            preserved_tags: Vec::new(),
        };

        let event = build_adp_listing_event_builder(&input)
            .expect("buy-only listing should be valid")
            .sign_with_keys(&keys)
            .expect("event should sign");

        assert_eq!(event.kind, Kind::Custom(30402));
        assert_eq!(tag_values(&event, "server"), Vec::<Vec<String>>::new());
        assert_eq!(tag_values(&event, "file_hash"), Vec::<Vec<String>>::new());
        assert_eq!(tag_values(&event, "version"), Vec::<Vec<String>>::new());
        assert_eq!(
            tag_values(&event, "fulfillment_pubkey"),
            Vec::<Vec<String>>::new()
        );
        assert_eq!(
            tag_values(&event, "lud16")[0],
            vec!["lud16", "studio@example.com"]
        );
    }

    #[test]
    fn partial_fulfillment_tier_reports_missing_fields() {
        let input = AdpListingInput {
            d_tag: "partial".to_string(),
            title: "Partial".to_string(),
            description: "Missing fulfillment fields".to_string(),
            price_sats: 0,
            lud16: None,
            tags: vec![],
            images: vec![],
            servers: vec!["https://dist.example.com".to_string()],
            file_hash: Some(VALID_SHA256.to_string()),
            version: None,
            fulfillment_pubkey: None,
            fulfillment_valid_from: None,
            fulfillment_revoked_at: None,
            acquisition: AcquisitionPolicy::Gated,
            platforms: vec![],
            campaigns: Vec::new(),
            nip94_event_id: None,
            preserved_tags: Vec::new(),
        };

        let err = build_adp_listing_event_builder(&input)
            .expect_err("partial fulfillment tier should fail");

        assert!(matches!(
            err,
            AdpPublishError::IncompleteFulfillmentTier { .. }
        ));
        assert!(err.to_string().contains("version"));
        assert!(err.to_string().contains("fulfillment_pubkey"));
        assert!(!err.to_string().contains("fulfillment_valid_from"));
    }

    #[test]
    fn malformed_server_url_is_rejected() {
        let input = AdpListingInput {
            d_tag: "bad-server".to_string(),
            title: "Bad Server".to_string(),
            description: "Invalid URL".to_string(),
            price_sats: 0,
            lud16: None,
            tags: vec![],
            images: vec![],
            servers: vec!["dist.example.com".to_string()],
            file_hash: Some(VALID_SHA256.to_string()),
            version: Some("1.0.0".to_string()),
            fulfillment_pubkey: Some("fulfillment-pubkey".to_string()),
            fulfillment_valid_from: Some(1_725_000_000),
            fulfillment_revoked_at: None,
            acquisition: AcquisitionPolicy::Gated,
            platforms: vec![],
            campaigns: Vec::new(),
            nip94_event_id: None,
            preserved_tags: Vec::new(),
        };

        let err =
            build_adp_listing_event_builder(&input).expect_err("malformed server URL should fail");

        assert!(matches!(err, AdpPublishError::MalformedServerUrl { .. }));
        assert!(err.to_string().contains("dist.example.com"));
    }

    #[tokio::test]
    async fn direct_signing_fulfillment_emits_developer_pubkey() {
        let keys = Keys::generate();
        let developer_pubkey = keys.public_key().to_hex();
        let input = AdpListingInput {
            d_tag: "direct".to_string(),
            title: "Direct".to_string(),
            description: "Direct fulfillment".to_string(),
            price_sats: 0,
            lud16: None,
            tags: vec![],
            images: vec![],
            servers: vec!["https://dist.example.com".to_string()],
            file_hash: Some(VALID_SHA256.to_string()),
            version: Some("1.0.0".to_string()),
            fulfillment_pubkey: Some(developer_pubkey.clone()),
            fulfillment_valid_from: Some(1_725_000_000),
            fulfillment_revoked_at: None,
            acquisition: AcquisitionPolicy::Gated,
            platforms: vec![],
            campaigns: Vec::new(),
            nip94_event_id: None,
            preserved_tags: Vec::new(),
        };

        let event = build_adp_listing_event_builder(&input)
            .expect("direct signing listing should be valid")
            .sign_with_keys(&keys)
            .expect("event should sign");

        assert_eq!(
            tag_values(&event, "fulfillment_pubkey"),
            vec![vec![
                "fulfillment_pubkey".to_string(),
                developer_pubkey,
                "1725000000".to_string(),
                String::new(),
            ]]
        );
    }

    #[tokio::test]
    async fn fulfillment_tag_preserves_empty_validity_and_revocation_positions() {
        let keys = Keys::generate();
        let input = AdpListingInput {
            d_tag: "legacy-delegated".to_string(),
            title: "Legacy Delegated".to_string(),
            description: "Existing fulfillment metadata without timestamps".to_string(),
            price_sats: 0,
            lud16: None,
            tags: vec![],
            images: vec![],
            servers: vec!["https://dist.example.com".to_string()],
            file_hash: Some(VALID_SHA256.to_string()),
            version: Some("1.0.0".to_string()),
            fulfillment_pubkey: Some("fulfillment-pubkey".to_string()),
            fulfillment_valid_from: None,
            fulfillment_revoked_at: None,
            acquisition: AcquisitionPolicy::Gated,
            platforms: vec![],
            campaigns: Vec::new(),
            nip94_event_id: None,
            preserved_tags: Vec::new(),
        };

        let event = build_adp_listing_event_builder(&input)
            .expect("legacy fulfillment metadata should remain publishable")
            .sign_with_keys(&keys)
            .expect("event should sign");

        assert_eq!(
            tag_values(&event, "fulfillment_pubkey"),
            vec![vec![
                "fulfillment_pubkey".to_string(),
                "fulfillment-pubkey".to_string(),
                String::new(),
                String::new(),
            ]]
        );
    }
}
