//! Pure ADP publish event builders.

use nostr::{EventBuilder, Kind, Tag, TagKind};
use thiserror::Error;

/// Input required to construct an ADP NIP-99 listing event.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AdpListingInput {
    pub d_tag: String,
    pub title: String,
    pub description: String,
    pub price_sats: u64,
    pub lud16: Option<String>,
    pub server_url: String,
    pub file_hash: String,
    pub version: String,
    pub fulfillment_pubkey: String,
    pub fulfillment_valid_from: u64,
    pub platforms: Vec<String>,
}

/// Errors returned while building ADP publish events.
#[derive(Debug, Error, PartialEq, Eq)]
pub enum AdpPublishError {
    /// Priced ADP listings require a listing-level LUD-16 address.
    #[error("priced ADP listing requires lud16 tag")]
    MissingLud16,
}

/// Builds the developer-signed `kind:30406` provisioning acceptance event.
pub fn build_provisioning_acceptance_event_builder(
    operator_pubkey: &str,
    fulfillment_pubkey: &str,
) -> EventBuilder {
    EventBuilder::new(Kind::Custom(30406), "").tags([
        Tag::custom(
            TagKind::Custom("d".into()),
            [format!("{operator_pubkey}:{fulfillment_pubkey}")],
        ),
        Tag::custom(TagKind::p(), [operator_pubkey.to_string()]),
        Tag::custom(
            TagKind::Custom("fulfillment_pubkey".into()),
            [fulfillment_pubkey.to_string()],
        ),
    ])
}

/// Builds the developer-signed `kind:30402` ADP listing event.
///
/// # Errors
/// Returns [`AdpPublishError::MissingLud16`] when `price_sats > 0` and no LUD-16 is provided.
pub fn build_adp_listing_event_builder(
    input: &AdpListingInput,
) -> Result<EventBuilder, AdpPublishError> {
    if input.price_sats > 0 && input.lud16.as_deref().unwrap_or_default().is_empty() {
        return Err(AdpPublishError::MissingLud16);
    }

    let mut tags = vec![
        Tag::custom(TagKind::Custom("d".into()), [input.d_tag.clone()]),
        Tag::custom(TagKind::Custom("title".into()), [input.title.clone()]),
        Tag::custom(
            TagKind::Custom("price".into()),
            [input.price_sats.to_string(), "sat".to_string()],
        ),
        Tag::custom(TagKind::Custom("server".into()), [input.server_url.clone()]),
        Tag::custom(
            TagKind::Custom("file_hash".into()),
            [input.file_hash.clone()],
        ),
        Tag::custom(TagKind::Custom("version".into()), [input.version.clone()]),
        Tag::custom(
            TagKind::Custom("fulfillment_pubkey".into()),
            [
                input.fulfillment_pubkey.clone(),
                input.fulfillment_valid_from.to_string(),
                String::new(),
            ],
        ),
        Tag::custom(TagKind::Custom("t".into()), ["game".to_string()]),
    ];

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

    Ok(EventBuilder::new(Kind::Custom(30402), input.description.clone()).tags(tags))
}

#[cfg(test)]
mod tests {
    use super::*;
    use nostr::{Keys, Kind};

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

    #[tokio::test]
    async fn builds_kind_30406_acceptance_with_expected_tags() {
        let keys = Keys::generate();
        let operator_pubkey = "operator-pubkey";
        let fulfillment_pubkey = "fulfillment-pubkey";

        let event =
            build_provisioning_acceptance_event_builder(operator_pubkey, fulfillment_pubkey)
                .sign_with_keys(&keys)
                .expect("event should sign");

        assert_eq!(event.kind, Kind::Custom(30406));
        assert_eq!(
            tag_values(&event, "d"),
            vec![vec![
                "d".to_string(),
                "operator-pubkey:fulfillment-pubkey".to_string()
            ]]
        );
        assert_eq!(
            tag_values(&event, "p"),
            vec![vec!["p".to_string(), "operator-pubkey".to_string()]]
        );
        assert_eq!(
            tag_values(&event, "fulfillment_pubkey"),
            vec![vec![
                "fulfillment_pubkey".to_string(),
                "fulfillment-pubkey".to_string()
            ]]
        );
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
            server_url: "https://dist.example.com".to_string(),
            file_hash: "abc123".to_string(),
            version: "1.0.0".to_string(),
            fulfillment_pubkey: "fulfillment-pubkey".to_string(),
            fulfillment_valid_from: 1_725_000_000,
            platforms: vec!["linux-x86_64".to_string(), "windows-x86_64".to_string()],
        };

        let event = build_adp_listing_event_builder(&input)
            .expect("listing builder should be created")
            .sign_with_keys(&keys)
            .expect("event should sign");

        assert_eq!(event.kind, Kind::Custom(30402));
        assert_eq!(tag_values(&event, "d")[0], vec!["d", "my-game"]);
        assert_eq!(tag_values(&event, "title")[0], vec!["title", "My Game"]);
        assert_eq!(tag_values(&event, "price")[0], vec!["price", "2100", "sat"]);
        assert_eq!(
            tag_values(&event, "server")[0],
            vec!["server", "https://dist.example.com"]
        );
        assert_eq!(
            tag_values(&event, "file_hash")[0],
            vec!["file_hash", "abc123"]
        );
        assert_eq!(tag_values(&event, "version")[0], vec!["version", "1.0.0"]);
        assert_eq!(
            tag_values(&event, "fulfillment_pubkey")[0],
            vec!["fulfillment_pubkey", "fulfillment-pubkey", "1725000000", ""]
        );
        assert_eq!(
            tag_values(&event, "lud16")[0],
            vec!["lud16", "studio@example.com"]
        );
        assert_eq!(tag_values(&event, "platform").len(), 2);
    }

    #[test]
    fn priced_listing_requires_lud16() {
        let input = AdpListingInput {
            d_tag: "my-game".to_string(),
            title: "My Game".to_string(),
            description: "Fun game".to_string(),
            price_sats: 1,
            lud16: None,
            server_url: "https://dist.example.com".to_string(),
            file_hash: "abc123".to_string(),
            version: "1.0.0".to_string(),
            fulfillment_pubkey: "fulfillment-pubkey".to_string(),
            fulfillment_valid_from: 1_725_000_000,
            platforms: vec![],
        };

        let err = build_adp_listing_event_builder(&input)
            .expect_err("priced listing without lud16 should fail");

        assert!(matches!(err, AdpPublishError::MissingLud16));
    }
}
