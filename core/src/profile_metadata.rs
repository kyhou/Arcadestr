//! Validation and replaceable-event ordering shared by native and browser profiles.

use nostr::{Event, Kind, PublicKey};
use serde::Deserialize;

#[derive(Debug, Default, Clone, Deserialize)]
pub struct UserProfileContent {
    pub name: Option<String>,
    pub display_name: Option<String>,
    pub about: Option<String>,
    pub picture: Option<String>,
    pub website: Option<String>,
    pub nip05: Option<String>,
    pub lud16: Option<String>,
}

pub fn validate_metadata_event(
    event: &Event,
    author: PublicKey,
) -> Result<UserProfileContent, String> {
    if event.kind != Kind::Metadata || event.pubkey != author {
        return Err("Metadata event kind or author does not match".into());
    }
    event.verify().map_err(|error| error.to_string())?;
    let value: serde_json::Value =
        serde_json::from_str(&event.content).map_err(|error| error.to_string())?;
    if !value.is_object() {
        return Err("Metadata content must be an object".into());
    }
    serde_json::from_value(value).map_err(|error| error.to_string())
}

pub fn metadata_event_is_newer(candidate: &Event, current: &Event) -> bool {
    crate::is_replaceable_event_newer(
        candidate.created_at.as_secs(),
        Some(&candidate.id.to_hex()),
        current.created_at.as_secs(),
        Some(&current.id.to_hex()),
    )
}

pub fn newest_metadata_event(events: &[Event], author: PublicKey) -> Option<&Event> {
    events
        .iter()
        .filter(|event| validate_metadata_event(event, author).is_ok())
        .reduce(|current, candidate| {
            if metadata_event_is_newer(candidate, current) {
                candidate
            } else {
                current
            }
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use nostr::{EventBuilder, Keys, Timestamp};

    fn event(keys: &Keys, content: &str, timestamp: u64) -> Event {
        EventBuilder::new(Kind::Metadata, content)
            .custom_created_at(Timestamp::from(timestamp))
            .sign_with_keys(keys)
            .expect("signed metadata")
    }

    #[test]
    fn latest_valid_metadata_wins_independent_of_arrival_order() {
        let keys = Keys::generate();
        let old = event(&keys, r#"{"name":"old"}"#, 1);
        let new = event(&keys, r#"{"name":"new"}"#, 2);
        for events in [vec![old.clone(), new.clone()], vec![new.clone(), old]] {
            assert_eq!(
                newest_metadata_event(&events, keys.public_key()),
                Some(&new)
            );
        }
        let tie = event(&keys, r#"{"name":"tie"}"#, 2);
        let expected = if new.id < tie.id { &new } else { &tie };
        assert_eq!(
            newest_metadata_event(&[new.clone(), tie.clone()], keys.public_key()),
            Some(expected)
        );
    }

    #[test]
    fn rejects_malformed_mismatched_and_forged_metadata() {
        let keys = Keys::generate();
        for content in ["not json", "null", "[]", r#"{"name":12}"#] {
            assert!(validate_metadata_event(&event(&keys, content, 1), keys.public_key()).is_err());
        }
        let mut forged = event(&keys, "{}", 1);
        assert!(validate_metadata_event(&forged, Keys::generate().public_key()).is_err());
        forged.content = r#"{"name":"forged"}"#.into();
        assert!(validate_metadata_event(&forged, keys.public_key()).is_err());
        assert!(validate_metadata_event(&event(&keys, "{}", 1), keys.public_key()).is_ok());
    }

    #[test]
    fn browser_login_fixtures_are_valid_signed_metadata() {
        let fixture: serde_json::Value =
            serde_json::from_str(include_str!("../../tests/fixtures/login_metadata.json"))
                .expect("fixture JSON");
        let author = PublicKey::parse(fixture["npub"].as_str().expect("npub")).expect("public key");
        for key in ["event", "updated"] {
            let event: Event =
                serde_json::from_value(fixture[key].clone()).expect("metadata event");
            assert!(validate_metadata_event(&event, author).is_ok());
        }
    }
}
