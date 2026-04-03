//! Relay connection event types for real-time status reporting.
//!
//! These types communicate relay connection status from the backend
//! to the frontend via broadcast channels, replacing polling-based approaches.

use serde::{Deserialize, Serialize};

/// Events emitted when relay connections change state.
///
/// These events are broadcast via channels to notify subscribers of
/// connection state changes in real-time.
///
/// # Examples
/// ```
/// let event = RelayConnectionEvent::connected("wss://relay.damus.io");
/// ```
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum RelayConnectionEvent {
    /// Emitted when a relay successfully connects.
    Connected { url: String },
    /// Emitted when a relay disconnects, with optional reason.
    Disconnected { url: String, reason: Option<String> },
}

/// Current status of a relay connection.
///
/// Used to track the real-time state of a relay connection including
/// connectivity status and measured latency.
///
/// # Examples
/// ```
/// let status = RelayStatus {
///     url: "wss://relay.damus.io".to_string(),
///     connected: true,
///     latency_ms: Some(150),
/// };
/// ```
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
    ///
    /// # Examples
    /// ```
    /// let event = RelayConnectionEvent::connected("wss://relay.damus.io");
    /// ```
    pub fn connected(url: impl Into<String>) -> Self {
        Self::Connected { url: url.into() }
    }

    /// Create a Disconnected event with the given URL and optional reason.
    ///
    /// # Examples
    /// ```
    /// let event = RelayConnectionEvent::disconnected(
    ///     "wss://relay.damus.io",
    ///     Some("Connection reset".to_string()),
    /// );
    /// ```
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
        // Arrange
        let url = "wss://relay.damus.io";

        // Act
        let event = RelayConnectionEvent::connected(url);

        // Assert
        match event {
            RelayConnectionEvent::Connected { url } => {
                assert_eq!(url, "wss://relay.damus.io");
            }
            _ => panic!("Expected Connected event"),
        }
    }

    #[test]
    fn test_disconnected_event_with_reason() {
        // Arrange
        let url = "wss://relay.damus.io";
        let reason = Some("Connection reset".to_string());

        // Act
        let event = RelayConnectionEvent::disconnected(url, reason);

        // Assert
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
        // Arrange
        let url = "wss://relay.damus.io";

        // Act
        let event = RelayConnectionEvent::disconnected(url, None);

        // Assert
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
        // Arrange
        let url = "wss://relay.damus.io".to_string();
        let connected = true;
        let latency_ms = Some(150);

        // Act
        let status = RelayStatus {
            url,
            connected,
            latency_ms,
        };

        // Assert
        assert_eq!(status.url, "wss://relay.damus.io");
        assert!(status.connected);
        assert_eq!(status.latency_ms, Some(150));
    }

    #[test]
    fn test_serialization_roundtrip() {
        // Arrange
        let event = RelayConnectionEvent::connected("wss://relay.damus.io");

        // Act
        let json = serde_json::to_string(&event).expect("serialize");
        let deserialized: RelayConnectionEvent = serde_json::from_str(&json).expect("deserialize");

        // Assert
        match deserialized {
            RelayConnectionEvent::Connected { url } => {
                assert_eq!(url, "wss://relay.damus.io");
            }
            _ => panic!("Roundtrip failed"),
        }
    }
}

#[allow(dead_code)]
fn _assert_send_sync() {
    fn assert_send<T: Send>() {}
    fn assert_sync<T: Sync>() {}
    assert_send::<RelayConnectionEvent>();
    assert_send::<RelayStatus>();
    assert_sync::<RelayConnectionEvent>();
    assert_sync::<RelayStatus>();
}
