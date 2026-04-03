//! Background NIP-05 validation worker
//! 
//! Validates NIP-05 identifiers asynchronously without blocking the UI.
//! Uses a queue-based system where profiles are queued for validation
//! and processed in the background.

use std::collections::VecDeque;
use std::sync::{Arc, Mutex};
use tokio::sync::mpsc;
use tracing::{debug, info, warn};

use crate::nostr::NostrClient;
use crate::user_cache::UserCache;

/// Command sent to the validator worker
#[derive(Debug, Clone)]
pub enum ValidationCommand {
    /// Queue a profile for NIP-05 validation
    Validate { npub: String, nip05: String },
    /// Shutdown the worker
    Shutdown,
}

/// Result of NIP-05 validation
#[derive(Debug, Clone)]
pub struct ValidationResult {
    pub npub: String,
    pub nip05: String,
    pub verified: bool,
}

/// Background NIP-05 validation worker
pub struct Nip05Validator {
    command_tx: mpsc::UnboundedSender<ValidationCommand>,
    result_rx: mpsc::UnboundedReceiver<ValidationResult>,
}

impl Nip05Validator {
    /// Spawn a new validator worker
    pub fn spawn(client: Arc<NostrClient>, user_cache: Arc<UserCache>) -> Self {
        let (command_tx, mut command_rx) = mpsc::unbounded_channel();
        let (result_tx, result_rx) = mpsc::unbounded_channel();

        tokio::spawn(async move {
            let mut queue: VecDeque<(String, String)> = VecDeque::new();
            let mut shutdown = false;

            loop {
                // Process commands or wait
                tokio::select! {
                    Some(cmd) = command_rx.recv() => {
                        match cmd {
                            ValidationCommand::Validate { npub, nip05 } => {
                                debug!("Queued NIP-05 validation for {}", npub);
                                queue.push_back((npub, nip05));
                            }
                            ValidationCommand::Shutdown => {
                                info!("NIP-05 validator shutting down");
                                shutdown = true;
                            }
                        }
                    }
                    _ = tokio::time::sleep(tokio::time::Duration::from_secs(1)), if !queue.is_empty() => {
                        // Process next item in queue
                        if let Some((npub, nip05)) = queue.pop_front() {
                            debug!("Validating NIP-05 for {}: {}", npub, nip05);
                            
                            // Perform validation
                            let verified = client.verify_nip05(&npub, &nip05).await;
                            
                            if verified {
                                info!("NIP-05 verified for {}: {}", npub, nip05);
                                
                                // Update cache with verified status
                                if let Some(mut profile) = user_cache.get(&npub).await {
                                    profile.nip05_verified = true;
                                    if let Err(e) = user_cache.put(&npub, &profile).await {
                                        warn!("Failed to update verified status in cache: {}", e);
                                    }
                                }
                            } else {
                                warn!("NIP-05 verification failed for {}: {}", npub, nip05);
                            }
                            
                            // Send result
                            let _ = result_tx.send(ValidationResult {
                                npub: npub.clone(),
                                nip05: nip05.clone(),
                                verified,
                            });
                        }
                    }
                    else => {
                        if shutdown && queue.is_empty() {
                            break;
                        }
                        tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;
                    }
                }
            }
        });

        Self {
            command_tx,
            result_rx,
        }
    }

    /// Queue a profile for NIP-05 validation
    pub fn queue_validation(&self, npub: String, nip05: String) {
        let _ = self.command_tx.send(ValidationCommand::Validate { npub, nip05 });
    }

    /// Try to receive a validation result (non-blocking)
    pub fn try_recv_result(&mut self) -> Option<ValidationResult> {
        self.result_rx.try_recv().ok()
    }

    /// Shutdown the validator
    pub fn shutdown(&self) {
        let _ = self.command_tx.send(ValidationCommand::Shutdown);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_validation_command_clone() {
        let cmd = ValidationCommand::Validate {
            npub: "test".to_string(),
            nip05: "test@example.com".to_string(),
        };
        let cloned = cmd.clone();
        
        match (cmd, cloned) {
            (ValidationCommand::Validate { npub: n1, nip05: nip1 }, 
             ValidationCommand::Validate { npub: n2, nip05: nip2 }) => {
                assert_eq!(n1, n2);
                assert_eq!(nip1, nip2);
            }
            _ => panic!("Clone mismatch"),
        }
    }
}
