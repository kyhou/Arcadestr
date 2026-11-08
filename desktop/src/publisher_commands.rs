//! Persistence adapters around the existing publisher paths. Nothing resumes at startup.
use crate::AppState;
use arcadestr_core::{
    publisher_recovery::{
        repository::PublisherRepository, PublisherCheckpoint, PublisherDraft, PublisherOperation,
    },
    settlement_recovery::SessionFence,
};
use nostr::nips::nip19::FromBech32;
use nostr::{Event, UnsignedEvent};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::sync::Arc;
use tauri::State;

#[derive(Clone)]
pub(crate) struct PublisherJournal {
    pub repo: PublisherRepository,
    pub publisher: String,
    pub operation_id: String,
    pub allow_uncertain: bool,
}
tokio::task_local! { pub(crate) static JOURNAL: PublisherJournal; }
pub(crate) fn journal() -> Option<PublisherJournal> {
    JOURNAL.try_with(Clone::clone).ok()
}

impl PublisherJournal {
    pub async fn step(&self, id: &str) -> Result<Option<PublisherCheckpoint>, String> {
        Ok(self
            .repo
            .steps(&self.publisher, &self.operation_id)
            .await?
            .into_iter()
            .find(|s| s.step_id == id))
    }
    pub async fn prepare(
        &self,
        id: &str,
        kind: &str,
        target: &str,
        intent: Value,
    ) -> Result<(), String> {
        self.repo
            .checkpoint(
                &self.publisher,
                &self.operation_id,
                &PublisherCheckpoint {
                    step_id: id.into(),
                    kind: kind.into(),
                    target: target.into(),
                    intent,
                    state: "prepared".into(),
                    outcome: None,
                    last_error: None,
                },
            )
            .await
    }
    pub async fn ack(&self, id: &str, outcome: Value) -> Result<(), String> {
        self.repo
            .acknowledge(&self.publisher, &self.operation_id, id, &outcome)
            .await
    }
    pub async fn failed(&self, id: &str, safe_message: &str) -> Result<(), String> {
        self.repo
            .record_failure(&self.publisher, &self.operation_id, id, safe_message)
            .await
    }
    /// A write-ahead gate only. HTTP still runs exclusively through the existing clients.
    pub async fn external_checkpoint(
        &self,
        id: &str,
        kind: &str,
        target: &str,
        intent: Value,
    ) -> Result<Option<Value>, String> {
        let previous = self.step(id).await?;
        self.prepare(id, kind, target, intent).await?;
        if let Some(outcome) = previous.as_ref().and_then(|step| step.outcome.clone()) {
            return Ok(Some(outcome));
        }
        if previous.is_some() && !self.allow_uncertain {
            return Err("External outcome is uncertain; explicit same-scope, pinned-artifact retry authorization is required".into());
        }
        Ok(None)
    }
}

pub(crate) fn signing_key(event: &UnsignedEvent) -> Option<String> {
    let kind = event.kind.as_u16();
    let field = match kind {
        30402 | 30407 | 1031 | 1030 => "d",
        30406 => "a",
        _ => return None,
    };
    let identity = event.tags.iter().find_map(|tag| {
        let values = tag.as_slice();
        (values.first().map(String::as_str) == Some(field))
            .then(|| values.get(1).cloned())
            .flatten()
    })?;
    if kind == 30406 {
        let tag_value = |name: &str| {
            event.tags.iter().find_map(|tag| match tag.as_slice() {
                [key, value] if key == name => Some(value.clone()),
                _ => None,
            })
        };
        return Some(format!(
            "signed:{kind}:{identity}:{}:{}",
            tag_value("p")?,
            tag_value("status")?
        ));
    }
    Some(format!("signed:{kind}:{identity}"))
}

pub(crate) async fn retained_signature(unsigned: &UnsignedEvent) -> Result<Option<Event>, String> {
    let (Some(journal), Some(key)) = (journal(), signing_key(unsigned)) else {
        return Ok(None);
    };
    // Phase 4 retained a single authority per coordinate. Keep that exact event
    // recoverable; its operator/status still must match the requested signature.
    let mut step = journal.step(&key).await?;
    if step.is_none() && unsigned.kind.as_u16() == 30406 {
        let old_key = key
            .rsplitn(3, ':')
            .last()
            .ok_or("Invalid authority checkpoint key")?;
        step = journal.step(old_key).await?;
    }
    let Some(step) = step else {
        return Ok(None);
    };
    let event: Event =
        serde_json::from_value(step.intent).map_err(|_| "Invalid retained publisher event")?;
    event
        .verify()
        .map_err(|_| "Retained publisher signature is invalid")?;
    let retained_unsigned: UnsignedEvent =
        serde_json::from_value(serde_json::to_value(&event).map_err(|_| "Invalid retained event")?)
            .map_err(|_| "Invalid retained event")?;
    if event.pubkey != unsigned.pubkey
        || event.kind != unsigned.kind
        || event.pubkey.to_hex() != journal.publisher
    {
        return Err("Retained publication identity does not match the active operation".into());
    }
    if signing_key(&retained_unsigned).as_deref() != Some(key.as_str()) {
        return Err("Retained signed event belongs to another publication coordinate".into());
    }
    if event.kind.as_u16() == 30402 {
        let hashes = |tags: &nostr::Tags| {
            tags.iter()
                .filter(|tag| tag.as_slice().first().map(String::as_str) == Some("file_hash"))
                .map(|tag| tag.clone().to_vec())
                .collect::<Vec<_>>()
        };
        if hashes(&event.tags) != hashes(&unsigned.tags) {
            return Err(
                "Retained signed listing pins another artifact; recovery cannot replace it".into(),
            );
        }
    }
    Ok(Some(event))
}

pub(crate) async fn retain_signature(event: &Event) -> Result<(), String> {
    let Some(journal) = journal() else {
        return Ok(());
    };
    event
        .verify()
        .map_err(|_| "Publisher signer returned an invalid signature")?;
    if event.pubkey.to_hex() != journal.publisher {
        return Err("Publisher signature belongs to another account".into());
    }
    let unsigned: UnsignedEvent =
        serde_json::from_value(serde_json::to_value(event).map_err(|_| "Invalid signed event")?)
            .map_err(|_| "Invalid signed event")?;
    if let Some(key) = signing_key(&unsigned) {
        journal
            .prepare(
                &key,
                "signed_event",
                &key,
                serde_json::to_value(event).map_err(|_| "Invalid signed event")?,
            )
            .await?;
        journal.ack(&key, json!({"event_id":event.id})).await?;
        let intent_key = format!("intent:{key}");
        if journal.step(&intent_key).await?.is_some() {
            journal
                .ack(&intent_key, json!({"event_id":event.id}))
                .await?;
        }
    }
    Ok(())
}

pub(crate) async fn before_publish(event: &Event) -> Result<Option<PublisherJournal>, String> {
    let Some(journal) = journal() else {
        return Ok(None);
    };
    retain_signature(event).await?;
    journal
        .prepare(
            &format!("relay:{}", event.id),
            "relay",
            &event.id.to_hex(),
            json!({"event":event}),
        )
        .await?;
    Ok(Some(journal))
}

pub(crate) async fn pointer_source(listing: &Event) -> Result<Option<Event>, String> {
    let Some(journal) = journal() else {
        return Ok(None);
    };
    let coordinate = arcadestr_core::store_page::listing_coordinate(listing)
        .map_err(|_| "Invalid listing source")?;
    let id = format!("source:{coordinate}");
    if let Some(previous) = journal.step(&id).await? {
        let previous: Event = serde_json::from_value(previous.intent)
            .map_err(|_| "Invalid retained listing source")?;
        let unsigned: UnsignedEvent =
            serde_json::from_value(serde_json::to_value(listing).map_err(|_| "Invalid source")?)
                .map_err(|_| "Invalid source")?;
        let retained = retained_signature(&unsigned).await?;
        if retained
            .as_ref()
            .is_some_and(|event| event.id == listing.id)
        {
            return Ok(retained);
        }
        if previous.id != listing.id {
            return Err(
                "Listing changed after the pointer operation; recovery will not overwrite it"
                    .into(),
            );
        }
    } else {
        journal
            .prepare(
                &id,
                "source_listing",
                &coordinate,
                serde_json::to_value(listing).map_err(|_| "Invalid listing source")?,
            )
            .await?;
    }
    Ok(None)
}

pub(crate) async fn begin_simple_operation(
    state: &State<'_, AppState>,
    npub: &str,
    kind: &str,
    coordinate: String,
    intent: Value,
    resume_id: Option<String>,
) -> Result<PublisherJournal, String> {
    let (publisher, session) = account(state, npub).await?;
    let repo = PublisherRepository::new(state.database.pool().clone());
    let resume = resume_id.is_some();
    let id = resume_id.unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
    if !resume {
        repo.begin(
            &publisher,
            &PublisherOperation {
                operation_id: id.clone(),
                kind: kind.into(),
                coordinate,
                intent,
                artifact_hash: None,
                state: "incomplete".into(),
                result: None,
            },
        )
        .await?;
    }
    session.ensure_current()?;
    Ok(PublisherJournal {
        repo,
        publisher,
        operation_id: id,
        allow_uncertain: false,
    })
}

pub use arcadestr_core::publisher_recovery::PublisherRecoveryState;
#[derive(Deserialize)]
pub struct SavePublisherDraftRequest {
    pub expected_publisher_npub: String,
    pub draft: PublisherDraft,
    pub expected_revision: Option<u64>,
}

pub(crate) async fn account(
    state: &State<'_, AppState>,
    npub: &str,
) -> Result<(String, SessionFence), String> {
    let session = state.settlement_recovery.session_fence()?;
    let expected = nostr::PublicKey::from_bech32(npub).map_err(|_| "Invalid publisher identity")?;
    if state.auth.lock().await.public_key() != Some(expected) {
        return Err("Switch to the draft publisher account".into());
    }
    session.ensure_current()?;
    Ok((expected.to_hex(), session))
}

#[tauri::command]
pub async fn save_publisher_draft(
    request: SavePublisherDraftRequest,
    state: State<'_, AppState>,
) -> Result<u64, String> {
    let (publisher, session) = account(&state, &request.expected_publisher_npub).await?;
    let repo = PublisherRepository::new(state.database.pool().clone());
    session.ensure_current()?;
    let revision = repo
        .save_draft(&publisher, &request.draft, request.expected_revision)
        .await?;
    session.ensure_current()?;
    Ok(revision)
}

#[tauri::command]
pub async fn publisher_recovery_state(
    expected_publisher_npub: String,
    state: State<'_, AppState>,
) -> Result<PublisherRecoveryState, String> {
    let (publisher, session) = account(&state, &expected_publisher_npub).await?;
    let repo = PublisherRepository::new(state.database.pool().clone());
    let drafts = repo.drafts(&publisher).await?;
    let operations = repo.operations(&publisher).await?;
    let mut checkpoints = Vec::new();
    for operation in &operations {
        checkpoints.push((
            operation.operation_id.clone(),
            repo.steps(&publisher, &operation.operation_id).await?,
        ));
    }
    session.ensure_current()?;
    Ok(PublisherRecoveryState {
        drafts,
        operations,
        checkpoints,
    })
}

/// Native archive capabilities are never serialized into recoverable intent.
pub(crate) fn durable_request<T: Serialize>(request: &T) -> Result<Value, String> {
    let mut value = serde_json::to_value(request).map_err(|_| "Invalid publisher intent")?;
    if let Some(fields) = value.as_object_mut() {
        fields.remove("selected_build");
        fields.remove("file_path");
    }
    Ok(value)
}

#[derive(Deserialize)]
pub struct ResumePublisherOperationRequest {
    pub expected_publisher_npub: String,
    pub operation_id: String,
    pub confirm_resume: bool,
    pub confirm_uncertain_retry: bool,
    pub selected_build: Option<arcadestr_core::archive_inspection_contract::BuildSelectionRef>,
}

#[tauri::command]
pub async fn resume_publisher_operation<R: tauri::Runtime>(
    request: ResumePublisherOperationRequest,
    app: tauri::AppHandle<R>,
    state: State<'_, AppState>,
    signer_state: State<'_, Arc<tokio::sync::Mutex<arcadestr_core::nip46::AppSignerState>>>,
    archive_service: State<'_, crate::archive_selection::ArchiveSelectionService>,
) -> Result<Value, String> {
    if !request.confirm_resume {
        return Err("Explicit authorization is required to resume publisher operations".into());
    }
    let (publisher, session) = account(&state, &request.expected_publisher_npub).await?;
    let _lease = state.settlement_recovery.claim(
        &publisher,
        &format!("publisher-resume:{}", request.operation_id),
    )?;
    let repo = PublisherRepository::new(state.database.pool().clone());
    let operation = repo.operation(&publisher, &request.operation_id).await?;
    if operation.state == "complete" {
        return operation
            .result
            .ok_or("Completed operation result is unavailable".into());
    }
    let journal = PublisherJournal {
        repo: repo.clone(),
        publisher: publisher.clone(),
        operation_id: request.operation_id.clone(),
        allow_uncertain: request.confirm_uncertain_retry,
    };
    if operation.kind == "listing" {
        if operation.artifact_hash.is_none() && request.selected_build.is_some() {
            return Err("This operation did not select a new archive. Resume without an archive; use a new publication to change artifacts".into());
        }
        let mut intent: crate::adp_commands::PublishAdpListingRequest =
            serde_json::from_value(operation.intent["request"].clone())
                .map_err(|_| "Invalid retained publication intent")?;
        let mut extension: arcadestr_core::adp_publish_contract::PublishAdpListingRequestExtension =
            serde_json::from_value(operation.intent["extension"].clone())
                .map_err(|_| "Invalid retained publication extension")?;
        if intent.expected_publisher_npub != request.expected_publisher_npub {
            return Err("Retained publication belongs to another account".into());
        }
        intent.selected_build = request.selected_build;
        extension.operation_id = Some(request.operation_id.clone());
        let steps = repo.steps(&publisher, &request.operation_id).await?;
        let signed_listing = steps
            .iter()
            .find(|s| s.kind == "signed_event" && s.step_id.starts_with("signed:30402:"))
            .map(|s| serde_json::from_value::<Event>(s.intent.clone()))
            .transpose()
            .map_err(|_| "Invalid retained listing")?;
        let current =
            crate::adp_commands::recovery_current_listing(&state, &operation.coordinate).await?;
        if let Some(current) = current {
            if signed_listing
                .as_ref()
                .is_some_and(|saved| saved.id == current.id)
            {
                intent.existing_event_id = Some(current.id.to_hex());
                intent.existing_d_tag = operation
                    .coordinate
                    .splitn(3, ':')
                    .nth(2)
                    .map(str::to_owned);
                if let Some(hash) = &operation.artifact_hash {
                    intent.existing_file_hash = Some(hash.clone());
                }
            } else if intent.existing_event_id.as_deref() != Some(current.id.to_hex().as_str()) {
                return Err("Listing changed after the retained operation; review the new revision instead of replaying old metadata".into());
            }
        } else if intent.existing_event_id.is_some() {
            return Err(
                "Original listing revision is unavailable; recovery remains pending".into(),
            );
        }
        session.ensure_current()?;
        return crate::adp_commands::publish_adp_listing_pipeline(
            intent,
            extension,
            app,
            state,
            signer_state,
            archive_service,
            true,
            request.confirm_uncertain_retry,
        )
        .await
        .map_err(|error| error.to_string())
        .and_then(|result| {
            serde_json::to_value(result).map_err(|_| "Invalid publication result".into())
        });
    }
    if operation.kind == "store_page" {
        let mut intent: crate::store_page_commands::PublishStorePageRequest =
            serde_json::from_value(operation.intent)
                .map_err(|_| "Invalid retained Store Page intent")?;
        intent.operation_id = Some(request.operation_id);
        return crate::store_page_commands::publish_store_page_recoverable(
            state,
            signer_state,
            intent,
            true,
        )
        .await
        .and_then(|result| {
            serde_json::to_value(result).map_err(|_| "Invalid Store Page result".into())
        });
    }
    if operation.kind == "store_page_pointers" {
        let intent: crate::store_page_commands::RetryStorePagePointersRequest =
            serde_json::from_value(operation.intent)
                .map_err(|_| "Invalid retained pointer intent")?;
        return crate::store_page_commands::retry_store_page_pointer_sync_recoverable(
            state,
            signer_state,
            intent,
            Some(request.operation_id),
        )
        .await
        .and_then(|result| {
            serde_json::to_value(result).map_err(|_| "Invalid pointer result".into())
        });
    }
    if operation.kind == "campaign" {
        let intent = serde_json::from_value(operation.intent)
            .map_err(|_| "Invalid retained campaign intent")?;
        return crate::adp_commands::publish_campaign_recoverable(
            intent,
            state,
            signer_state,
            Some(request.operation_id),
        )
        .await
        .and_then(|result| {
            serde_json::to_value(result).map_err(|_| "Invalid campaign result".into())
        });
    }
    if operation.kind == "campaign_pointer" {
        let intent = serde_json::from_value(operation.intent)
            .map_err(|_| "Invalid retained campaign pointer")?;
        return crate::adp_commands::update_campaign_pointer_recoverable(
            intent,
            state,
            signer_state,
            Some(request.operation_id),
        )
        .await
        .map(Value::String);
    }
    if operation.kind == "credential_revocation" {
        let intent = serde_json::from_value(operation.intent)
            .map_err(|_| "Invalid retained revocation intent")?;
        return crate::publisher_management::revoke_recoverable(intent, state, signer_state)
            .await
            .and_then(|event| {
                serde_json::to_value(event).map_err(|_| "Invalid revocation result".into())
            });
    }
    Err("This publisher operation requires review in its original editor".into())
}

/// Called by the original pipeline only after validating any reselected archive.
pub(crate) async fn reconcile_authorizations(
    state: &State<'_, AppState>,
    journal: &PublisherJournal,
    session: &SessionFence,
) -> Result<(), String> {
    let operation = journal
        .repo
        .operation(&journal.publisher, &journal.operation_id)
        .await?;
    for step in journal
        .repo
        .steps(&journal.publisher, &journal.operation_id)
        .await?
        .into_iter()
        .filter(|step| step.kind == "signed_event" && step.step_id.starts_with("signed:30406:"))
    {
        let event: Event =
            serde_json::from_value(step.intent).map_err(|_| "Invalid retained authorization")?;
        event
            .verify()
            .map_err(|_| "Invalid retained authorization signature")?;
        let manager = { state.nostr.lock().await.get_relay_manager().clone() };
        let manager = { manager.lock().await.clone() };
        let collected = Arc::new(std::sync::Mutex::new(Vec::new()));
        let sink = collected.clone();
        manager
            .fetch_events_streaming(
                nostr::Filter::new()
                    .kind(nostr::Kind::Custom(30406))
                    .author(event.pubkey),
                10,
                10,
                move |_, events| {
                    if let Ok(mut sink) = sink.lock() {
                        sink.extend(events);
                    }
                },
            )
            .await
            .map_err(|_| "Authorization recovery evidence is incomplete")?;
        let mut events = collected
            .lock()
            .map_err(|_| "Authorization evidence unavailable")?
            .clone();
        let already_seen = events.iter().any(|candidate| candidate.id == event.id);
        events.push(event.clone());
        let authority = arcadestr_core::authorization::resolve_authorization(event.id, &events)
            .map_err(|_| "Retained authority is invalid or ambiguous")?;
        if authority.terms.coordinate != operation.coordinate
            || authority.developer_pubkey.to_hex() != journal.publisher
        {
            return Err(
                "Retained authority belongs to another operation coordinate or publisher".into(),
            );
        }
        if authority.cancelled_at.is_some() {
            return Err("Retained authority was cancelled; recovery never renews it".into());
        }
        if !already_seen {
            session.ensure_current()?;
            crate::adp_commands::publish_event(state, &event, session).await?;
        } else if let Some(journal) = before_publish(&event).await? {
            journal
                .ack(&format!("relay:{}", event.id), json!({"accepted":true}))
                .await?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use arcadestr_core::{
        settlement_recovery::RecoveryCoordinator,
        signers::{NostrSigner, SignerError},
        storage::Database,
    };
    use std::sync::atomic::{AtomicUsize, Ordering};
    struct CountingSigner {
        keys: nostr::Keys,
        calls: AtomicUsize,
    }
    #[async_trait::async_trait]
    impl NostrSigner for CountingSigner {
        async fn get_public_key(&self) -> Result<nostr::PublicKey, SignerError> {
            Ok(self.keys.public_key())
        }
        async fn sign_event(&self, unsigned: UnsignedEvent) -> Result<Event, SignerError> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            unsigned
                .sign(&self.keys)
                .await
                .map_err(|e| SignerError::SigningFailed(e.to_string()))
        }
    }
    fn unsigned(keys: &nostr::Keys, time: u64) -> UnsignedEvent {
        nostr::EventBuilder::new(nostr::Kind::Custom(30402), "unchanged content")
            .tag(nostr::Tag::parse(["d", "game"]).expect("tag"))
            .custom_created_at(nostr::Timestamp::from(time))
            .build(keys.public_key())
    }
    async fn context(db: &Database, publisher: &str) -> PublisherJournal {
        let repo = PublisherRepository::new(db.pool().clone());
        repo.begin(
            publisher,
            &PublisherOperation {
                operation_id: "operation".into(),
                kind: "listing".into(),
                coordinate: format!("30402:{publisher}:game"),
                intent: json!({"original":"immutable"}),
                artifact_hash: None,
                state: "incomplete".into(),
                result: None,
            },
        )
        .await
        .expect("operation");
        PublisherJournal {
            repo,
            publisher: publisher.into(),
            operation_id: "operation".into(),
            allow_uncertain: false,
        }
    }
    #[tokio::test]
    async fn independent_operator_authorities_and_revocations_replay_exact_bytes_after_restart() {
        use arcadestr_core::authorization::*;
        let temp = tempfile::tempdir().expect("temp");
        let path = temp.path().join("multi.db");
        let db = Database::new(&path).await.expect("db");
        let signer = Arc::new(CountingSigner {
            keys: nostr::Keys::generate(),
            calls: AtomicUsize::new(0),
        });
        let publisher = signer.keys.public_key();
        let ctx = context(&db, &publisher.to_hex()).await;
        let coordinator = RecoveryCoordinator::default();
        coordinator.activate(Some(publisher.to_hex()));
        let fenced = crate::adp_commands::PublicationSigner {
            signer: signer.clone(),
            session: coordinator.session_fence().expect("session"),
        };
        let mut originals = Vec::new();
        for index in 0..2 {
            let terms = AuthorizationTerms {
                authorization_id: format!("operator-{index}"),
                coordinate: format!("30402:{publisher}:game"),
                operator_pubkey: nostr::Keys::generate().public_key(),
                fulfillment_pubkey: nostr::Keys::generate().public_key(),
                capabilities: std::collections::BTreeSet::from([CAPABILITY_UPLOAD_BUILD.into()]),
                valid_from: 100,
            };
            let unsigned =
                arcadestr_core::adp_publish::build_fulfillment_authorization_event_builder(&terms)
                    .expect("builder")
                    .build(publisher);
            let event = JOURNAL
                .scope(ctx.clone(), fenced.sign_event(unsigned))
                .await
                .expect("independent root");
            originals.push(event);
        }
        assert_ne!(originals[0].id, originals[1].id);
        let resolved =
            resolve_authorization(originals[0].id, &[originals[0].clone()]).expect("authority");
        let cancelled = JOURNAL
            .scope(
                ctx.clone(),
                fenced.sign_event(
                    arcadestr_core::publisher_management::cancel_authority_builder(
                        &resolved,
                        publisher,
                        originals[0].id,
                        200,
                    )
                    .expect("cancel")
                    .build(publisher),
                ),
            )
            .await
            .expect("cancellation distinct from active root");
        originals.push(cancelled);
        let grant = nostr::EventBuilder::new(nostr::Kind::Custom(1030), "")
            .tags([
                nostr::Tag::parse(["d", "grant"]).expect("tag"),
                nostr::Tag::parse(["status", "revoked"]).expect("tag"),
            ])
            .build(publisher);
        originals.push(
            JOURNAL
                .scope(ctx.clone(), fenced.sign_event(grant))
                .await
                .expect("grant signature"),
        );
        assert_eq!(signer.calls.load(Ordering::SeqCst), 4);
        db.pool().close().await;
        let db = Database::new(&path).await.expect("restart");
        let resumed = PublisherJournal {
            repo: PublisherRepository::new(db.pool().clone()),
            ..ctx
        };
        for original in &originals {
            let mut unsigned: UnsignedEvent =
                serde_json::from_value(json!(original)).expect("unsigned");
            unsigned.created_at = nostr::Timestamp::from_secs(500);
            let replay = JOURNAL
                .scope(resumed.clone(), fenced.sign_event(unsigned))
                .await
                .expect("exact replay");
            assert_eq!(json!(original), json!(replay));
        }
        assert_eq!(
            signer.calls.load(Ordering::SeqCst),
            4,
            "no additional signer call during explicit recovery"
        );
    }

    #[tokio::test]
    async fn restart_reuses_exact_signed_event_without_signer_and_fences_reconnected_sessions() {
        let temp = tempfile::tempdir().expect("temporary directory");
        let path = temp.path().join("publisher.db");
        let db = Database::new(&path).await.expect("database");
        let signer = Arc::new(CountingSigner {
            keys: nostr::Keys::generate(),
            calls: AtomicUsize::new(0),
        });
        let publisher = signer.keys.public_key().to_hex();
        let coordinator = RecoveryCoordinator::default();
        coordinator.activate(Some(publisher.clone()));
        let context = context(&db, &publisher).await;
        let fenced = crate::adp_commands::PublicationSigner {
            signer: signer.clone(),
            session: coordinator.session_fence().expect("session"),
        };
        let first = JOURNAL
            .scope(
                context.clone(),
                fenced.sign_event(unsigned(&signer.keys, 10)),
            )
            .await
            .expect("first signature");
        JOURNAL
            .scope(context.clone(), before_publish(&first))
            .await
            .expect("checkpoint before relay side effect");
        assert_eq!(signer.calls.load(Ordering::SeqCst), 1);
        db.pool().close().await;
        let db = Database::new(&path).await.expect("restart");
        coordinator.activate(Some(publisher.clone()));
        let resumed = PublisherJournal {
            repo: PublisherRepository::new(db.pool().clone()),
            ..context
        };
        let fresh = crate::adp_commands::PublicationSigner {
            signer: signer.clone(),
            session: coordinator.session_fence().expect("new session"),
        };
        let replay = JOURNAL
            .scope(
                resumed.clone(),
                fresh.sign_event(unsigned(&signer.keys, 20)),
            )
            .await
            .expect("retained signature");
        assert_eq!(
            serde_json::to_value(first).expect("event"),
            serde_json::to_value(replay).expect("event")
        );
        assert_eq!(signer.calls.load(Ordering::SeqCst), 1);
        assert!(JOURNAL
            .scope(resumed, fenced.sign_event(unsigned(&signer.keys, 30)))
            .await
            .is_err());
    }
    #[tokio::test]
    async fn interruption_before_signature_preserves_original_unsigned_revision() {
        let temp = tempfile::tempdir().expect("temporary directory");
        let db = Database::new(&temp.path().join("publisher.db"))
            .await
            .expect("database");
        let signer = Arc::new(CountingSigner {
            keys: nostr::Keys::generate(),
            calls: AtomicUsize::new(0),
        });
        let publisher = signer.keys.public_key().to_hex();
        let context = context(&db, &publisher).await;
        let original = unsigned(&signer.keys, 10);
        let key = format!("intent:{}", signing_key(&original).expect("key"));
        context
            .prepare(&key, "signing", &key, json!(original))
            .await
            .expect("intent before signer await");
        let coordinator = RecoveryCoordinator::default();
        coordinator.activate(Some(publisher));
        let fenced = crate::adp_commands::PublicationSigner {
            signer: signer.clone(),
            session: coordinator.session_fence().expect("session"),
        };
        let signed = JOURNAL
            .scope(context, fenced.sign_event(unsigned(&signer.keys, 20)))
            .await
            .expect("resume signing");
        assert_eq!(signed.created_at.as_secs(), 10);
        assert_eq!(signer.calls.load(Ordering::SeqCst), 1);
    }
    #[tokio::test]
    async fn retained_signed_event_cannot_be_substituted_from_another_coordinate() {
        let temp = tempfile::tempdir().expect("temporary directory");
        let db = Database::new(&temp.path().join("publisher.db"))
            .await
            .expect("database");
        let keys = nostr::Keys::generate();
        let context = context(&db, &keys.public_key().to_hex()).await;
        let foreign = nostr::EventBuilder::new(nostr::Kind::Custom(30402), "another game")
            .tag(nostr::Tag::parse(["d", "another-game"]).expect("tag"))
            .sign_with_keys(&keys)
            .expect("signed fixture");
        context
            .prepare(
                "signed:30402:game",
                "signed_event",
                "signed:30402:game",
                json!(foreign),
            )
            .await
            .expect("simulate corrupted checkpoint association");
        assert!(JOURNAL
            .scope(context, retained_signature(&unsigned(&keys, 10)))
            .await
            .is_err());
        let pin_db = Database::new(&temp.path().join("pin.db"))
            .await
            .expect("pin database");
        let pin_context = self::context(&pin_db, &keys.public_key().to_hex()).await;
        let original = nostr::EventBuilder::new(nostr::Kind::Custom(30402), "same game")
            .tags([
                nostr::Tag::parse(["d", "game"]).expect("d"),
                nostr::Tag::parse(["file_hash", &"ab".repeat(32)]).expect("hash"),
            ])
            .sign_with_keys(&keys)
            .expect("signed artifact pin");
        pin_context
            .prepare(
                "signed:30402:game",
                "signed_event",
                "signed:30402:game",
                json!(original),
            )
            .await
            .expect("retained pin");
        let changed = nostr::EventBuilder::new(nostr::Kind::Custom(30402), "same game")
            .tags([
                nostr::Tag::parse(["d", "game"]).expect("d"),
                nostr::Tag::parse(["file_hash", &"cd".repeat(32)]).expect("hash"),
            ])
            .build(keys.public_key());
        assert!(
            JOURNAL
                .scope(pin_context, retained_signature(&changed))
                .await
                .is_err(),
            "a different artifact cannot be introduced through signature replay"
        );
    }

    #[tokio::test]
    async fn uncertain_external_steps_require_explicit_retry_and_acknowledged_results_are_reused() {
        let temp = tempfile::tempdir().expect("temporary directory");
        let db = Database::new(&temp.path().join("publisher.db"))
            .await
            .expect("database");
        let mut context = context(&db, &nostr::Keys::generate().public_key().to_hex()).await;
        for kind in ["provision", "upload"] {
            let target = "https://operator.test";
            let intent = json!({"scope":"game","hash":"ab".repeat(32)});
            assert!(context
                .external_checkpoint(kind, kind, target, intent.clone())
                .await
                .expect("first user-authorized action")
                .is_none());
            assert!(
                context
                    .external_checkpoint(kind, kind, target, intent.clone())
                    .await
                    .is_err(),
                "no automatic repeat after an unknown outcome"
            );
            context.allow_uncertain = true;
            assert!(context
                .external_checkpoint(kind, kind, target, intent.clone())
                .await
                .expect("explicit retry")
                .is_none());
            context
                .ack(kind, json!({"response":"original"}))
                .await
                .expect("checkpoint response");
            context.allow_uncertain = false;
            assert_eq!(
                context
                    .external_checkpoint(kind, kind, target, intent)
                    .await
                    .expect("reuse"),
                Some(json!({"response":"original"}))
            );
            assert!(context
                .external_checkpoint(
                    kind,
                    kind,
                    "https://substitution.test",
                    json!({"changed":true})
                )
                .await
                .is_err());
        }
    }

    #[tokio::test]
    async fn checkpoint_failure_stops_before_external_publication() {
        let temp = tempfile::tempdir().expect("temporary directory");
        let db = Database::new(&temp.path().join("publisher.db"))
            .await
            .expect("database");
        let keys = nostr::Keys::generate();
        let context = context(&db, &keys.public_key().to_hex()).await;
        sqlx::query("CREATE TRIGGER reject_publisher_checkpoint BEFORE INSERT ON publisher_checkpoints BEGIN SELECT RAISE(ABORT,'fixture'); END;").execute(db.pool()).await.expect("fault injection");
        let event = unsigned(&keys, 10)
            .sign(&keys)
            .await
            .expect("signed fixture");
        let sends = AtomicUsize::new(0);
        let result = JOURNAL
            .scope(context, async {
                before_publish(&event).await?;
                sends.fetch_add(1, Ordering::SeqCst);
                Ok::<_, String>(())
            })
            .await;
        assert!(result.is_err());
        assert_eq!(sends.load(Ordering::SeqCst), 0);
    }
}
