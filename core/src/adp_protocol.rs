use nostr::EventId;
use thiserror::Error;

/// Implementation-local experimental kind. This is not an interoperable allocation.
pub const EXPERIMENTAL_ENTITLEMENT_GRANT_KIND: u16 = 1030;
/// Implementation-local experimental kind. This is not an interoperable allocation.
pub const EXPERIMENTAL_ADP_CAMPAIGN_KIND: u16 = 1031;
/// Implementation-local experimental kind. This is not an interoperable allocation.
pub const EXPERIMENTAL_STORE_PAGE_KIND: u16 = 30407;
pub const ENTITLEMENT_GRANT_KIND: u16 = EXPERIMENTAL_ENTITLEMENT_GRANT_KIND;
pub const ADP_CAMPAIGN_KIND: u16 = EXPERIMENTAL_ADP_CAMPAIGN_KIND;
pub const FULFILLMENT_AUTHORIZATION_KIND: u16 = 30406;
pub const NIP99_LISTING_KIND: u16 = 30402;

pub const TAG_IDENTIFIER: &str = "d";
pub const TAG_RECIPIENT: &str = "p";
pub const TAG_COORDINATE: &str = "a";
pub const TAG_PREDECESSOR: &str = "e";
pub const TAG_STATUS: &str = "status";
pub const TAG_MODE: &str = "mode";
pub const TAG_STARTS: &str = "starts";
pub const TAG_ENDS: &str = "ends";
pub const TAG_SOURCE_EVENT: &str = "source_event";
pub const TAG_REASON: &str = "reason";
pub const TAG_AUTHORIZATION_EVENT: &str = "authorization";

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum ProtocolEventError {
    #[error("invalid nostr event signature")]
    InvalidSignature,
    #[error("wrong event kind: expected {expected}, found {found}")]
    WrongKind { expected: u16, found: u16 },
    #[error("event is missing required tag: {0}")]
    MissingTag(&'static str),
    #[error("event contains duplicate tag: {0}")]
    DuplicateTag(String),
    #[error("event contains malformed tag: {0}")]
    MalformedTag(String),
    #[error("event contains forbidden tag: {0}")]
    ForbiddenTag(String),
    #[error("event signer does not match coordinate publisher")]
    WrongPublisher,
}

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum ChainError {
    #[error("chain is empty")]
    Empty,
    #[error("missing predecessor {0}")]
    MissingPredecessor(EventId),
    #[error("chain has multiple roots")]
    MultipleRoots,
    #[error("chain forks at {0}")]
    Fork(EventId),
    #[error("chain contains a cycle")]
    Cycle,
    #[error("chain contains disconnected events")]
    Disconnected,
    #[error("chain invariant changed: {0}")]
    InvariantMutation(&'static str),
    #[error("invalid chain transition: {0}")]
    InvalidTransition(String),
    #[error("successor timestamp does not increase")]
    TimestampRegression,
}

pub(crate) fn exact_tag(
    event: &nostr::Event,
    name: &'static str,
    required: bool,
) -> Result<Option<String>, ProtocolEventError> {
    let matches = event
        .tags
        .iter()
        .map(|tag| tag.clone().to_vec())
        .filter(|values| values.first().is_some_and(|value| value == name))
        .collect::<Vec<_>>();
    if matches.len() > 1 {
        return Err(ProtocolEventError::DuplicateTag(name.into()));
    }
    let Some(values) = matches.first() else {
        return if required {
            Err(ProtocolEventError::MissingTag(name))
        } else {
            Ok(None)
        };
    };
    if values.len() != 2 || values[1].is_empty() {
        return Err(ProtocolEventError::MalformedTag(name.into()));
    }
    Ok(Some(values[1].clone()))
}

pub(crate) fn coordinate_publisher(coordinate: &str) -> Option<nostr::PublicKey> {
    let mut parts = coordinate.splitn(3, ':');
    let kind = parts.next()?;
    let publisher = parts.next()?;
    let identifier = parts.next()?;
    if kind != "30402" || identifier.is_empty() {
        return None;
    }
    nostr::PublicKey::from_hex(publisher).ok()
}

#[cfg(test)]
mod kind_safety_tests {
    use super::*;

    // Assigned Nostr kinds and Arcadestr/ADP reservations documented in this repository.
    const RECORDED_EVENT_KINDS: &[u16] = &[
        0,
        1,
        3,
        7,
        8,
        1020,
        1021,
        EXPERIMENTAL_ENTITLEMENT_GRANT_KIND,
        EXPERIMENTAL_ADP_CAMPAIGN_KIND,
        1063,
        1985,
        9734,
        9735,
        10002,
        10008,
        23194,
        23195,
        24133,
        27235,
        30008,
        30009,
        30017,
        30018,
        30078,
        NIP99_LISTING_KIND,
        30403,
        30404,
        30405,
        FULFILLMENT_AUTHORIZATION_KIND,
    ];

    #[test]
    fn experimental_store_page_kind_does_not_collide_with_recorded_kinds() {
        assert!((30000..40000).contains(&EXPERIMENTAL_STORE_PAGE_KIND));
        assert!(!RECORDED_EVENT_KINDS.contains(&EXPERIMENTAL_STORE_PAGE_KIND));
    }
}
