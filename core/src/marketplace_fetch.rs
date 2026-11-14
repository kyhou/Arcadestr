//! Typed outcome of a marketplace relay fetch, shared by the desktop command
//! and the frontend so callers never infer the outcome from error text.
//!
//! On Nostr anyone can run a relay, so some relay is always down, slow or
//! refusing: partial coverage is the normal result of a refresh, not an error.
//! A refresh succeeds when any relay answered (EOSE) or any listing arrived.
//! It fails only when nothing answered at all, or the fetch could not run.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, thiserror::Error)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum MarketplaceFetchError {
    /// Every queried relay failed or timed out without delivering anything.
    #[error("No relay responded: {failed} failed, {timed_out} timed out")]
    Unreachable { failed: usize, timed_out: usize },
    /// The fetch could not run (no connected relays, internal error).
    #[error("{message}")]
    Failed { message: String },
}

impl MarketplaceFetchError {
    pub fn failed(message: impl Into<String>) -> Self {
        Self::Failed {
            message: message.into(),
        }
    }

    /// Decodes an error that crossed the command bridge. Structured payloads
    /// keep their variant; anything else is a plain failure message.
    pub fn from_bridge(raw: &str) -> Self {
        serde_json::from_str(raw).unwrap_or_else(|_| Self::failed(raw))
    }

    pub fn is_unreachable(&self) -> bool {
        matches!(self, Self::Unreachable { .. })
    }
}

/// Applies the marketplace policy to a raw relay-manager outcome. The relay
/// manager reports every non-answering relay as `FetchIncomplete`; that report
/// stays accurate for callers that need full coverage, but for discovery any
/// answer is a successful refresh.
#[cfg(feature = "native")]
pub fn classify_relay_fetch(
    result: Result<usize, crate::relay_manager::RelayManagerError>,
) -> Result<(), MarketplaceFetchError> {
    use crate::relay_manager::RelayManagerError;
    match result {
        Ok(_) => Ok(()),
        Err(RelayManagerError::FetchIncomplete {
            succeeded, events, ..
        }) if succeeded > 0 || events > 0 => Ok(()),
        Err(RelayManagerError::FetchIncomplete {
            failed, timed_out, ..
        }) => Err(MarketplaceFetchError::Unreachable { failed, timed_out }),
        Err(other) => Err(MarketplaceFetchError::failed(other.to_string())),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unreachable_round_trips_across_the_bridge() {
        let error = MarketplaceFetchError::Unreachable {
            failed: 6,
            timed_out: 3,
        };
        let wire = serde_json::to_string(&error).expect("serializable");
        assert!(wire.contains("\"kind\":\"unreachable\""));
        assert_eq!(MarketplaceFetchError::from_bridge(&wire), error);
        assert!(error.is_unreachable());
        assert_eq!(
            error.to_string(),
            "No relay responded: 6 failed, 3 timed out"
        );
    }

    #[test]
    fn unstructured_or_malformed_bridge_errors_are_plain_failures() {
        for raw in [
            "connection refused",
            "{\"kind\":\"unknown\"}",
            "{not json",
            "",
        ] {
            let error = MarketplaceFetchError::from_bridge(raw);
            assert_eq!(error, MarketplaceFetchError::failed(raw));
            assert!(!error.is_unreachable());
        }
    }

    #[test]
    fn a_failed_message_cannot_be_forged_into_unreachable() {
        let forged = "No relay responded: 1 failed, 0 timed out";
        assert!(!MarketplaceFetchError::from_bridge(forged).is_unreachable());
    }

    #[cfg(feature = "native")]
    mod classification {
        use super::super::*;
        use crate::relay_manager::RelayManagerError;

        fn incomplete(
            succeeded: usize,
            failed: usize,
            timed_out: usize,
            events: usize,
        ) -> Result<usize, RelayManagerError> {
            Err(RelayManagerError::FetchIncomplete {
                succeeded,
                failed,
                timed_out,
                events,
            })
        }

        #[test]
        fn any_answer_is_a_successful_refresh() {
            assert_eq!(classify_relay_fetch(Ok(0)), Ok(()));
            // 43 answered, 9 did not: the normal state of the network.
            assert_eq!(classify_relay_fetch(incomplete(43, 6, 3, 5)), Ok(()));
            // Answered with nothing: a real empty result.
            assert_eq!(classify_relay_fetch(incomplete(1, 4, 0, 0)), Ok(()));
            // No EOSE, but listings arrived before the timeout.
            assert_eq!(classify_relay_fetch(incomplete(0, 0, 2, 7)), Ok(()));
        }

        #[test]
        fn nothing_answering_is_unreachable() {
            assert_eq!(
                classify_relay_fetch(incomplete(0, 6, 3, 0)),
                Err(MarketplaceFetchError::Unreachable {
                    failed: 6,
                    timed_out: 3
                })
            );
        }

        #[test]
        fn other_relay_errors_are_failures() {
            assert_eq!(
                classify_relay_fetch(Err(RelayManagerError::QueryTimeout)),
                Err(MarketplaceFetchError::failed("Query timeout"))
            );
        }
    }
}
