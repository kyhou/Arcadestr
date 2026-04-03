// Relay connection event types for real-time status reporting
//
// These types are used to communicate relay connection status from the
// backend to the frontend via broadcast channels, replacing the previous
// polling-based approach.

use serde::{Deserialize, Serialize};

/// Events emitted when relay connections change state.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum RelayConnectionEvent {
    /// Emitted when a relay successfully connects.
    Connected { url: String },
    /// Emitted when a relay disconnects, with optional reason.
    Disconnected { url: String, reason: Option<String> },
}

/// Current status of a relay connection.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RelayStatus {
    /// The relay URL (e.g., "wss://relay.damus.io")
    pub url: String,
    /// Whether the relay is currently connected
    pub connected: bool,
    /// Connection latency in milliseconds, if known
    pub latency_ms: Option<u64>,
}

impl RelayConnectionEvent {
    /// Create a Connected event with the given URL.
    pub fn connected(url: impl Into<String>) -> Self {
        Self::Connected { url: url.into() }
    }

    /// Create a Disconnected event with the given URL and optional reason.
    pub fn disconnected(url: impl Into<String>, reason: Option<String>) -> Self {
        Self::Disconnected {
            url: url.into(),
            reason,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_connected_event() {
        let event = RelayConnectionEvent::connected("wss://relay.damus.io");
        match event {
            RelayConnectionEvent::Connected { url } => {
                assert_eq!(url, "wss://relay.damus.io");
            }
            _ => panic!("Expected Connected event"),
        }
    }

    #[test]
    fn test_disconnected_event_with_reason() {
        let event = RelayConnectionEvent::disconnected(
            "wss://relay.damus.io",
            Some("Connection reset".to_string()),
        );
        match event {
            RelayConnectionEvent::Disconnected { url, reason } => {
                assert_eq!(url, "wss://relay.damus.io");
                assert_eq!(reason, Some("Connection reset".to_string()));
            }
            _ => panic!("Expected Disconnected event"),
        }
    }

    #[test]
    fn test_disconnected_event_without_reason() {
        let event = RelayConnectionEvent::disconnected("wss://relay.damus.io", None);
        match event {
            RelayConnectionEvent::Disconnected { url, reason } => {
                assert_eq!(url, "wss://relay.damus.io");
                assert_eq!(reason, None);
            }
            _ => panic!("Expected Disconnected event"),
        }
    }

    #[test]
    fn test_relay_status() {
        let status = RelayStatus {
            url: "wss://relay.damus.io".to_string(),
            connected: true,
            latency_ms: Some(150),
        };
        assert_eq!(status.url, "wss://relay.damus.io");
        assert!(status.connected);
        assert_eq!(status.latency_ms, Some(150));
    }

    #[test]
    fn test_serialization_roundtrip() {
        let event = RelayConnectionEvent::connected("wss://relay.damus.io");
        let json = serde_json::to_string(&event).expect("serialize");
        let deserialized: RelayConnectionEvent = serde_json::from_str(&json).expect("deserialize");
        match deserialized {
            RelayConnectionEvent::Connected { url } => {
                assert_eq!(url, "wss://relay.damus.io");
            }
            _ => panic!("Roundtrip failed"),
        }
    }
}
