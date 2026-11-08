//! Management coordinates the same signer, chain resolvers and publisher journal.
use crate::{
    adp_commands::{resolve_active_signer, PublicationSigner},
    publisher_commands::{account, PublisherJournal, JOURNAL},
    AppState,
};
use arcadestr_core::{
    nip46::AppSignerState, publisher_management::*,
    publisher_recovery::repository::PublisherRepository, signers::NostrSigner,
};
use nostr::{Event, EventId, Filter, PublicKey};
use std::sync::Arc;
use tauri::State;

pub use arcadestr_core::publisher_management::RevokePublisherCredentialRequest;

async fn query(state: &State<'_, AppState>, filter: Filter) -> Result<Vec<Event>, String> {
    let manager = { state.nostr.lock().await.get_relay_manager().clone() };
    let manager = { manager.lock().await.clone() };
    let sink = Arc::new(std::sync::Mutex::new(Vec::new()));
    let output = sink.clone();
    manager
        .fetch_events_streaming(filter, 10, 10, move |_, events| {
            if let Ok(mut output) = output.lock() {
                output.extend(events);
            }
        })
        .await
        .map_err(|_| "Current credential evidence is unavailable; no transition was published")?;
    let result = sink
        .lock()
        .map_err(|_| "Credential evidence unavailable")?
        .clone();
    Ok(result)
}

async fn root_and_events(
    state: &State<'_, AppState>,
    id: EventId,
    authority: bool,
) -> Result<(Event, Vec<Event>), String> {
    let root = query(state, Filter::new().id(id))
        .await?
        .into_iter()
        .find(|event| event.id == id)
        .ok_or("Credential root unavailable")?;
    let d = if authority {
        arcadestr_core::authorization::parse_authorization_event(&root)
            .map_err(|e| e.to_string())?
            .terms
            .authorization_id
    } else {
        arcadestr_core::entitlements::parse_entitlement_event(&root)
            .map_err(|e| e.to_string())?
            .grant_id
    };
    let mut filter = Filter::new().kind(root.kind).identifier(d);
    if authority {
        filter = filter.author(root.pubkey);
    }
    let mut events = query(state, filter).await?;
    events.push(root.clone());
    Ok((root, events))
}

#[tauri::command]
pub async fn review_publisher_credential(
    expected_publisher_npub: String,
    coordinate: String,
    root_event_id: String,
    action: RevocationAction,
    state: State<'_, AppState>,
) -> Result<CredentialReview, String> {
    let (publisher, session) = account(&state, &expected_publisher_npub).await?;
    let public = PublicKey::from_hex(&publisher).map_err(|_| "Invalid publisher")?;
    if arcadestr_core::adp_protocol::coordinate_publisher(&coordinate) != Some(public) {
        return Err("Wrong credential publisher".into());
    }
    let id = EventId::from_hex(&root_event_id).map_err(|_| "Invalid credential root")?;
    let authority = matches!(action, RevocationAction::CancelAuthority);
    let (_, events) = root_and_events(&state, id, authority).await?;
    let result = if authority {
        let resolved = arcadestr_core::authorization::resolve_authorization(id, &events)
            .map_err(|e| e.to_string())?;
        if resolved.terms.coordinate != coordinate {
            return Err("Authority coordinate mismatch".into());
        }
        CredentialReview {
            root_event_id,
            tip_event_id: resolved
                .events
                .last()
                .ok_or("Missing authority tip")?
                .event
                .id
                .to_hex(),
            coordinate,
            subject: format!(
                "Operator {} · fulfillment key {} · capabilities {}",
                resolved.terms.operator_pubkey,
                resolved.terms.fulfillment_pubkey,
                resolved
                    .terms
                    .capabilities
                    .into_iter()
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
            terminal: resolved.cancelled_at.is_some(),
        }
    } else {
        let nodes = events
            .iter()
            .filter_map(|event| arcadestr_core::entitlements::parse_entitlement_event(event).ok())
            .filter(|node| node.coordinate == coordinate)
            .collect::<Vec<_>>();
        let resolved = arcadestr_core::entitlements::resolve_entitlement_grant(&nodes)
            .map_err(|e| e.to_string())?;
        if resolved.root_event_id != id {
            return Err("Grant root mismatch".into());
        }
        let tip = resolved.events.last().ok_or("Missing grant tip")?;
        CredentialReview {
            root_event_id,
            tip_event_id: tip.event.id.to_hex(),
            coordinate,
            subject: format!(
                "Grant {} · recipient {} · source {}",
                tip.grant_id, tip.recipient, tip.source_event
            ),
            terminal: tip.status == arcadestr_core::entitlements::GrantStatus::Revoked,
        }
    };
    session.ensure_current()?;
    Ok(result)
}

#[tauri::command]
pub async fn revoke_publisher_credential(
    request: RevokePublisherCredentialRequest,
    state: State<'_, AppState>,
    signer_state: State<'_, Arc<tokio::sync::Mutex<AppSignerState>>>,
) -> Result<Event, String> {
    revoke_recoverable(request, state, signer_state).await
}

pub(crate) async fn revoke_recoverable(
    request: RevokePublisherCredentialRequest,
    state: State<'_, AppState>,
    signer_state: State<'_, Arc<tokio::sync::Mutex<AppSignerState>>>,
) -> Result<Event, String> {
    if !request.confirm {
        return Err(
            "Explicit developer confirmation is required for credential cancellation/revocation"
                .into(),
        );
    }
    uuid::Uuid::parse_str(&request.operation_id).map_err(|_| "Invalid revocation operation ID")?;
    EventId::from_hex(&request.root_event_id).map_err(|_| "Invalid credential root ID")?;
    EventId::from_hex(&request.expected_tip_event_id)
        .map_err(|_| "Invalid expected credential tip")?;
    let (publisher, session) = account(&state, &request.expected_publisher_npub).await?;
    let public = PublicKey::from_hex(&publisher).map_err(|_| "Invalid publisher")?;
    if arcadestr_core::adp_protocol::coordinate_publisher(&request.coordinate) != Some(public) {
        return Err("Credential coordinate belongs to another publisher".into());
    }
    let _lease = state
        .settlement_recovery
        .claim(&publisher, &format!("publish:{}", request.coordinate))?;
    let repo = PublisherRepository::new(state.database.pool().clone());
    let operations = repo.operations(&publisher).await?;
    let exists = validate_revocation_submission(&request, &operations)?;
    if exists {
        let previous = operations
            .iter()
            .find(|op| op.operation_id == request.operation_id)
            .ok_or("Missing revocation operation")?;
        if previous.state == "complete" {
            session.ensure_current()?;
            return serde_json::from_value(
                previous.result.clone().ok_or("Missing revocation result")?,
            )
            .map_err(|_| "Invalid revocation result".into());
        }
    } else {
        repo.begin(
            &publisher,
            &arcadestr_core::publisher_recovery::PublisherOperation {
                operation_id: request.operation_id.clone(),
                kind: "credential_revocation".into(),
                coordinate: request.coordinate.clone(),
                intent: serde_json::to_value(&request).map_err(|_| "Invalid revocation intent")?,
                artifact_hash: None,
                state: "incomplete".into(),
                result: None,
            },
        )
        .await?;
    }
    let journal = PublisherJournal {
        repo,
        publisher,
        operation_id: request.operation_id.clone(),
        allow_uncertain: false,
    };
    JOURNAL
        .scope(journal.clone(), async {
            let auth = { state.auth.lock().await.clone() };
            let signer = PublicationSigner {
                signer: resolve_active_signer(signer_state.inner(), &auth).await?,
                session: session.clone(),
            };
            if signer
                .get_public_key()
                .await
                .map_err(|_| "Publisher signer unavailable")?
                != public
            {
                return Err("Active signer is not the credential publisher".into());
            }
            let root_id =
                EventId::from_hex(&request.root_event_id).map_err(|_| "Invalid root ID")?;
            let expected_tip = EventId::from_hex(&request.expected_tip_event_id)
                .map_err(|_| "Invalid expected tip")?;
            let authority = matches!(request.action, RevocationAction::CancelAuthority);
            let (root, events) = root_and_events(&state, root_id, authority).await?;
            let key = if authority {
                "cancel_authority"
            } else {
                "revoke_grant"
            };
            let saved = journal
                .step("revocation-event")
                .await?
                .map(|step| serde_json::from_value::<Event>(step.intent))
                .transpose()
                .map_err(|_| "Invalid retained revocation")?;
            let now = nostr::Timestamp::now().as_secs();
            if let Some(saved) = &saved {
                validate_revocation(&root, saved, authority)?;
            }
            let mut grant_evidence = None;
            let builder = if authority {
                let resolved =
                    arcadestr_core::authorization::resolve_authorization(root_id, &events)
                        .map_err(|e| e.to_string())?;
                if resolved.terms.coordinate != request.coordinate {
                    return Err("Authority coordinate mismatch".into());
                }
                if let Some(saved) = &saved {
                    if resolved.events.last().map(|tip| tip.event.id) == Some(saved.id) {
                        session.ensure_current()?;
                        return finish(&journal, saved).await;
                    }
                }
                cancel_authority_builder(&resolved, public, expected_tip, now)?
            } else {
                let nodes = events
                    .iter()
                    .filter_map(|event| {
                        arcadestr_core::entitlements::parse_entitlement_event(event).ok()
                    })
                    .filter(|node| node.coordinate == request.coordinate)
                    .collect::<Vec<_>>();
                let resolved = arcadestr_core::entitlements::resolve_entitlement_grant(&nodes)
                    .map_err(|e| e.to_string())?;
                if resolved.root_event_id != root_id {
                    return Err("Grant root mismatch".into());
                }
                // Reuse historical campaign/authorization verification; neither a
                // cancelled campaign nor prospective authority cancellation revokes old grants.
                let source = resolved.events.first().ok_or("Missing grant root")?;
                let manager = { state.nostr.lock().await.get_relay_manager().clone() };
                let manager = { manager.lock().await.clone() };
                let report =
                    arcadestr_core::campaign_discovery::CampaignDiscoveryService::new(&manager)
                        .discover(
                            &[arcadestr_core::marketplace::CampaignPointer {
                                root_event_id: source.source_event,
                                relay_hint: None,
                            }],
                            public,
                            &request.coordinate,
                            now,
                        )
                        .await
                        .map_err(|e| e.to_string())?;
                let campaign = report
                    .campaigns
                    .into_iter()
                    .find(|item| item.campaign.root_event_id == source.source_event)
                    .ok_or("Grant campaign evidence unavailable")?
                    .campaign;
                let authorization = if let Some(id) = source.authorization {
                    Some(
                        arcadestr_core::authorization::discover_authorization(&manager, id, public)
                            .await
                            .map_err(|e| e.to_string())?,
                    )
                } else {
                    None
                };
                arcadestr_core::entitlements::validate_adp_entitlement(
                    &resolved,
                    &campaign,
                    authorization.as_ref(),
                )
                .map_err(|e| e.to_string())?;
                if let Some(saved) = &saved {
                    if resolved.events.last().map(|tip| tip.event.id) == Some(saved.id) {
                        session.ensure_current()?;
                        persist_grant(&state, &root, saved, &campaign, authorization.as_ref())
                            .await?;
                        return finish(&journal, saved).await;
                    }
                }
                grant_evidence = Some((campaign, authorization));
                revoke_grant_builder(&resolved, public, expected_tip, now)?
            };
            session.ensure_current()?;
            journal
                .prepare(
                    "revocation-source",
                    key,
                    &request.root_event_id,
                    serde_json::to_value(&root).map_err(|_| "Invalid revocation source")?,
                )
                .await?;
            let event = if let Some(saved) = saved {
                validate_revocation(&root, &saved, authority)?;
                saved
            } else {
                let event = signer
                    .sign_event(builder.build(public))
                    .await
                    .map_err(|e| e.to_string())?;
                validate_revocation(&root, &event, authority)?;
                journal
                    .prepare(
                        "revocation-event",
                        key,
                        &request.root_event_id,
                        serde_json::to_value(&event).map_err(|_| "Invalid revocation event")?,
                    )
                    .await?;
                event
            };
            // A remote signer can take arbitrarily long: reconcile again before publication.
            let (_, latest) = root_and_events(&state, root_id, authority).await?;
            let mut with_transition = latest;
            with_transition.push(event.clone());
            if authority {
                let chain =
                    arcadestr_core::authorization::resolve_authorization(root_id, &with_transition)
                        .map_err(|e| e.to_string())?;
                if chain.events.last().map(|tip| tip.event.id) != Some(event.id) {
                    return Err("Authority advanced while signing".into());
                }
            } else {
                let nodes = with_transition
                    .iter()
                    .filter_map(|e| arcadestr_core::entitlements::parse_entitlement_event(e).ok())
                    .filter(|e| e.coordinate == request.coordinate)
                    .collect::<Vec<_>>();
                let chain = arcadestr_core::entitlements::resolve_entitlement_grant(&nodes)
                    .map_err(|e| e.to_string())?;
                if chain.events.last().map(|tip| tip.event.id) != Some(event.id) {
                    return Err("Grant advanced while signing".into());
                }
            }
            crate::adp_commands::ensure_publish_account_current(
                &state,
                signer_state.inner(),
                public,
                &session,
            )
            .await?;
            crate::adp_commands::publish_event(&state, &event, &session).await?;
            if let Some((campaign, authorization)) = grant_evidence {
                session.ensure_current()?;
                persist_grant(&state, &root, &event, &campaign, authorization.as_ref()).await?;
            }
            finish(&journal, &event).await
        })
        .await
}

async fn persist_grant(
    state: &State<'_, AppState>,
    root: &Event,
    event: &Event,
    campaign: &arcadestr_core::campaign::ResolvedCampaign,
    authorization: Option<&arcadestr_core::authorization::ResolvedAuthorization>,
) -> Result<(), String> {
    let repo = arcadestr_core::entitlements_repository::EntitlementsRepository::new(
        state.database.pool().clone(),
    );
    repo.ingest_event(root, campaign, authorization)
        .await
        .map_err(|e| e.to_string())?;
    repo.ingest_event(event, campaign, authorization)
        .await
        .map_err(|e| e.to_string())
}

async fn finish(journal: &PublisherJournal, event: &Event) -> Result<Event, String> {
    // Both paths into finish have selected-relay evidence: a publication ACK
    // or observation of this exact retained transition during reconciliation.
    crate::publisher_commands::before_publish(event).await?;
    journal
        .ack(
            &format!("relay:{}", event.id),
            serde_json::json!({"accepted":true}),
        )
        .await?;
    journal
        .repo
        .complete(
            &journal.publisher,
            &journal.operation_id,
            &serde_json::to_value(event).map_err(|_| "Invalid revocation result")?,
        )
        .await?;
    Ok(event.clone())
}

#[tauri::command]
pub async fn publisher_distribution_state(
    expected_publisher_npub: String,
    coordinate: String,
    state: State<'_, AppState>,
) -> Result<Vec<DistributionEvidence>, String> {
    let (publisher, session) = account(&state, &expected_publisher_npub).await?;
    if arcadestr_core::adp_protocol::coordinate_publisher(&coordinate).map(|key| key.to_hex())
        != Some(publisher.clone())
    {
        return Err("Distribution belongs to another account".into());
    }
    let repo = PublisherRepository::new(state.database.pool().clone());
    let mut evidence = Vec::new();
    for operation in repo
        .operations(&publisher)
        .await?
        .into_iter()
        .filter(|op| op.coordinate == coordinate && op.kind == "listing")
    {
        let steps = repo.steps(&publisher, &operation.operation_id).await?;
        if let Some(servers) = operation.intent["request"]["servers"].as_array() {
            for server in servers.iter().filter_map(|server| server.as_str()) {
                evidence.push(distribution_evidence(
                    server,
                    &operation.operation_id,
                    &steps,
                ));
            }
        }
    }
    session.ensure_current()?;
    Ok(evidence)
}
