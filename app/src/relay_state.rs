// Relay state helpers for merging snapshot data and incremental relay events.

/// Merge a snapshot of connected relays into current UI state.
///
/// Preserves existing entries and appends only missing relays.
pub fn merge_relay_snapshot(current: &mut Vec<String>, snapshot: Vec<String>) {
    for relay in snapshot {
        if !current.contains(&relay) {
            current.push(relay);
        }
    }
}

/// Apply one incremental relay connection event to UI relay state.
///
/// Returns `true` if the state changed, `false` otherwise.
pub fn apply_relay_event(current: &mut Vec<String>, event_type: &str, url: &str) -> bool {
    match event_type {
        "connected" => {
            if current.iter().any(|r| r == url) {
                false
            } else {
                current.push(url.to_string());
                true
            }
        }
        "disconnected" => {
            let before_len = current.len();
            current.retain(|r| r != url);
            current.len() != before_len
        }
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::{apply_relay_event, merge_relay_snapshot};

    #[test]
    fn merge_relay_snapshot_adds_missing_relays_without_duplicates() {
        let mut current = vec!["wss://relay.damus.io".to_string()];

        merge_relay_snapshot(
            &mut current,
            vec![
                "wss://relay.damus.io".to_string(),
                "wss://relay.primal.net".to_string(),
            ],
        );

        assert_eq!(current.len(), 2);
        assert!(current.contains(&"wss://relay.damus.io".to_string()));
        assert!(current.contains(&"wss://relay.primal.net".to_string()));
    }

    #[test]
    fn apply_relay_event_connected_and_disconnected_updates_state() {
        let mut current = Vec::<String>::new();

        apply_relay_event(&mut current, "connected", "wss://relay.damus.io");
        assert_eq!(current, vec!["wss://relay.damus.io".to_string()]);

        apply_relay_event(&mut current, "disconnected", "wss://relay.damus.io");
        assert!(current.is_empty());
    }

    #[test]
    fn snapshot_recovers_state_when_initial_events_are_missed() {
        let mut current = Vec::<String>::new();

        // Simulate listener subscribing late and missing early connected events.
        merge_relay_snapshot(
            &mut current,
            vec![
                "wss://relay.damus.io".to_string(),
                "wss://relay.primal.net".to_string(),
                "wss://nos.lol".to_string(),
            ],
        );

        assert_eq!(current.len(), 3);
    }
}
