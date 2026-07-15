use std::collections::HashSet;
use std::sync::Arc;

use nostr::{Event, Filter, Kind};
use serde::{Deserialize, Serialize};
use tokio::sync::Mutex;

use crate::nostr::NostrError;
use crate::relay_manager::RelayManager;

/// Public ADP server announcement parsed from `kind:30403` events.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AdpServerAnnouncement {
    pub pubkey: String,
    pub url: String,
    pub name: Option<String>,
    pub supported_adp: Option<String>,
    pub contact: Option<String>,
}

/// Discovers ADP servers announced on currently connected relays.
///
/// This uses the existing best-effort relay fetch path and does not add a wider
/// discovery-relay or NIP-65 gossip layer. A narrow relay set can therefore
/// produce a narrow server list; every selected URL still requires live ADP
/// reachability validation before publish.
pub async fn discover_adp_servers(
    relay_manager: &Arc<Mutex<RelayManager>>,
) -> Result<Vec<AdpServerAnnouncement>, NostrError> {
    let filter = Filter::new().kind(Kind::Custom(30403));
    let events = relay_manager
        .lock()
        .await
        .fetch_events_best_effort(filter)
        .await
        .map_err(|err| NostrError::RelayError(format!("Failed to discover ADP servers: {err}")))?;

    Ok(parse_adp_server_announcements(events))
}

pub fn parse_adp_server_announcements(events: Vec<Event>) -> Vec<AdpServerAnnouncement> {
    let mut seen_urls = HashSet::new();
    let mut announcements = Vec::new();

    for event in events {
        if event.kind != Kind::Custom(30403) {
            continue;
        }

        let Some(url) = tag_value(&event, "url") else {
            continue;
        };
        if !is_http_url(&url) || !seen_urls.insert(url.clone()) {
            continue;
        }

        announcements.push(AdpServerAnnouncement {
            pubkey: event.pubkey.to_hex(),
            url,
            name: tag_value(&event, "name"),
            supported_adp: tag_value(&event, "supported_adp"),
            contact: tag_value(&event, "contact"),
        });
    }

    announcements
}

fn tag_value(event: &Event, name: &str) -> Option<String> {
    event.tags.iter().find_map(|tag| {
        let values = tag
            .clone()
            .to_vec()
            .iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>();
        (values.first().is_some_and(|kind| kind == name))
            .then(|| values.get(1).cloned())
            .flatten()
    })
}

fn is_http_url(value: &str) -> bool {
    value.starts_with("http://") || value.starts_with("https://")
}

#[cfg(test)]
mod tests {
    use super::*;
    use nostr::{EventBuilder, Keys, Kind, Tag, TagKind};

    fn announcement(keys: &Keys, d_tag: &str, url: Option<&str>) -> nostr::Event {
        let mut tags = vec![
            Tag::custom(TagKind::Custom("d".into()), [d_tag.to_string()]),
            Tag::custom(TagKind::Custom("name".into()), ["Arcade Host".to_string()]),
            Tag::custom(
                TagKind::Custom("supported_adp".into()),
                ["0.2.0".to_string()],
            ),
            Tag::custom(
                TagKind::Custom("contact".into()),
                ["ops@example.com".to_string()],
            ),
        ];
        if let Some(url) = url {
            tags.push(Tag::custom(
                TagKind::Custom("url".into()),
                [url.to_string()],
            ));
        }

        EventBuilder::new(Kind::Custom(30403), "")
            .tags(tags)
            .sign_with_keys(keys)
            .expect("announcement should sign")
    }

    #[test]
    fn parse_announcements_dedupes_by_url() {
        let keys = Keys::generate();
        let events = vec![
            announcement(&keys, "main", Some("https://dist.example.com")),
            announcement(&keys, "legacy", Some("https://dist.example.com")),
        ];

        let announcements = parse_adp_server_announcements(events);

        assert_eq!(announcements.len(), 1);
        assert_eq!(announcements[0].url, "https://dist.example.com");
        assert_eq!(announcements[0].name.as_deref(), Some("Arcade Host"));
        assert_eq!(announcements[0].supported_adp.as_deref(), Some("0.2.0"));
        assert_eq!(announcements[0].contact.as_deref(), Some("ops@example.com"));
    }

    #[test]
    fn parse_announcements_skips_missing_or_malformed_url() {
        let keys = Keys::generate();
        let events = vec![
            announcement(&keys, "missing", None),
            announcement(&keys, "malformed", Some("dist.example.com")),
            announcement(&keys, "ok", Some("https://dist.example.com")),
        ];

        let announcements = parse_adp_server_announcements(events);

        assert_eq!(announcements.len(), 1);
        assert_eq!(announcements[0].url, "https://dist.example.com");
    }
}
