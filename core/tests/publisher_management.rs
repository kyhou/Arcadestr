#![cfg(feature = "native")]
use arcadestr_core::{
    authorization::*, entitlements::*, publisher_management::*,
    publisher_recovery::PublisherCheckpoint,
};
use nostr::{Event, EventBuilder, Keys, Kind, Tag, Timestamp};
use serde_json::json;
use std::collections::BTreeSet;

fn tag(values: &[&str]) -> Tag {
    Tag::parse(values.iter().copied()).expect("tag")
}

#[test]
fn uncertain_revocation_cannot_be_bypassed_with_another_operation_or_changed_intent() {
    use arcadestr_core::publisher_recovery::PublisherOperation;
    let mut request = RevokePublisherCredentialRequest {
        expected_publisher_npub: "alice".into(),
        coordinate: "game".into(),
        root_event_id: "original-root".into(),
        expected_tip_event_id: "original-root".into(),
        action: RevocationAction::CancelAuthority,
        operation_id: "original-operation".into(),
        confirm: true,
    };
    let operation = PublisherOperation {
        operation_id: request.operation_id.clone(),
        kind: "credential_revocation".into(),
        coordinate: request.coordinate.clone(),
        intent: serde_json::to_value(&request).expect("intent"),
        artifact_hash: None,
        state: "incomplete".into(),
        result: None,
    };
    assert!(!validate_revocation_submission(&request, &[]).expect("new explicit action"));
    assert!(
        validate_revocation_submission(&request, &[operation.clone()])
            .expect("resume exact original")
    );
    request.operation_id = "replacement-operation".into();
    assert!(validate_revocation_submission(&request, &[operation.clone()]).is_err());
    request.operation_id = operation.operation_id.clone();
    request.expected_tip_event_id = "changed-tip".into();
    assert!(validate_revocation_submission(&request, &[operation.clone()]).is_err());
    request.expected_tip_event_id = "original-root".into();
    request.action = RevocationAction::RevokeGrant;
    assert!(validate_revocation_submission(&request, &[operation.clone()]).is_err());
    request.action = RevocationAction::CancelAuthority;
    request.confirm = false;
    assert!(validate_revocation_submission(&request, &[operation]).is_err());
}
fn authority(publisher: &Keys, operator: &Keys, key: &Keys) -> Event {
    arcadestr_core::adp_publish::build_fulfillment_authorization_event_builder(
        &AuthorizationTerms {
            authorization_id: "authority".into(),
            coordinate: format!("30402:{}:game", publisher.public_key()),
            operator_pubkey: operator.public_key(),
            fulfillment_pubkey: key.public_key(),
            capabilities: BTreeSet::from([
                CAPABILITY_ISSUE_GRANT.into(),
                CAPABILITY_ISSUE_RECEIPT.into(),
                CAPABILITY_UPLOAD_BUILD.into(),
            ]),
            valid_from: 100,
        },
    )
    .expect("builder")
    .sign_with_keys(publisher)
    .expect("signature")
}
fn grant(
    publisher: &Keys,
    issuer: &Keys,
    buyer: &Keys,
    source: nostr::EventId,
    auth: Option<nostr::EventId>,
    id: &str,
) -> Event {
    let mut tags = vec![
        tag(&["d", id]),
        tag(&["a", &format!("30402:{}:game", publisher.public_key())]),
        tag(&["p", &buyer.public_key().to_hex()]),
        tag(&["source_event", &source.to_hex()]),
        tag(&["status", "granted"]),
    ];
    if let Some(id) = auth {
        tags.push(tag(&["authorization", &id.to_hex()]));
    }
    EventBuilder::new(Kind::Custom(1030), "")
        .tags(tags)
        .custom_created_at(Timestamp::from_secs(150))
        .sign_with_keys(issuer)
        .expect("grant")
}

#[test]
fn multiple_operators_and_duplicate_urls_have_independent_minimal_roles() {
    let first = Keys::generate().public_key().to_hex();
    let second = Keys::generate().public_key().to_hex();
    assert!(
        operator_plans(
            &[
                ("https://drift.test".into(), first.clone()),
                ("https://drift.test".into(), second.clone())
            ],
            &[],
            None
        )
        .is_err(),
        "duplicate endpoints cannot substitute their operator identity"
    );
    let servers = vec![
        ("https://one.test".into(), first.clone()),
        ("https://alias.test".into(), first.clone()),
        ("https://two.test".into(), second.clone()),
        ("https://two.test".into(), second),
    ];
    let roles = vec![
        ServerRoleIntent {
            server_url: "https://one.test".into(),
            issue_receipt: true,
            issue_grant: false,
        },
        ServerRoleIntent {
            server_url: "https://alias.test".into(),
            issue_receipt: true,
            issue_grant: false,
        },
    ];
    let plans = operator_plans(&servers, &roles, None).expect("plans");
    assert_eq!(plans.len(), 2);
    let issuer = plans
        .iter()
        .find(|plan| plan.operator.to_hex() == first)
        .expect("issuer");
    assert_eq!(issuer.urls.len(), 2);
    assert!(issuer.capabilities.contains(CAPABILITY_ISSUE_RECEIPT));
    assert!(!issuer.capabilities.contains(CAPABILITY_ISSUE_GRANT));
    assert!(plans
        .iter()
        .any(|plan| plan.capabilities == BTreeSet::from([CAPABILITY_UPLOAD_BUILD.into()])));
    assert!(
        operator_plans(&servers, &roles[..1], None).is_err(),
        "conflicting alias roles do not silently expand authority"
    );
    assert!(operator_plans(&servers, &[roles[0].clone(), roles[0].clone()], None).is_err());
    assert!(operator_plans(&[("https://user:secret@one.test".into(), first)], &[], None).is_err());
}

#[test]
fn authority_cancellation_is_prospective_and_preserves_exact_terms() {
    let publisher = Keys::generate();
    let key = Keys::generate();
    let root = authority(&publisher, &Keys::generate(), &key);
    let initial = resolve_authorization(root.id, &[root.clone()]).expect("root");
    let cancel = cancel_authority_builder(&initial, publisher.public_key(), root.id, 200)
        .expect("cancel")
        .sign_with_keys(&publisher)
        .expect("signed");
    validate_revocation(&root, &cancel, true).expect("valid transition");
    let result = resolve_authorization(root.id, &[root.clone(), cancel.clone()]).expect("resolved");
    assert_eq!(result.terms, initial.terms);
    assert!(result.authorizes(
        &key.public_key(),
        &initial.terms.coordinate,
        CAPABILITY_ISSUE_RECEIPT,
        199
    ));
    assert!(!result.authorizes(
        &key.public_key(),
        &initial.terms.coordinate,
        CAPABILITY_ISSUE_RECEIPT,
        200
    ));
    assert!(cancel_authority_builder(&result, publisher.public_key(), root.id, 201).is_err());
    assert!(
        cancel_authority_builder(&initial, Keys::generate().public_key(), root.id, 200).is_err()
    );
    assert!(cancel_authority_builder(&initial, publisher.public_key(), cancel.id, 200).is_err());
    assert!(cancel_authority_builder(&initial, publisher.public_key(), root.id, 100).is_err());
    let competing = cancel_authority_builder(&initial, publisher.public_key(), root.id, 201)
        .expect("builder")
        .sign_with_keys(&publisher)
        .expect("signature");
    assert!(
        resolve_authorization(root.id, &[root, cancel, competing]).is_err(),
        "fork blocks recovery"
    );
}

#[test]
fn campaign_cancellation_and_authority_cancellation_do_not_revoke_old_grants() {
    use arcadestr_core::campaign::*;
    let publisher = Keys::generate();
    let operator = Keys::generate();
    let key = Keys::generate();
    let buyer = Keys::generate();
    let coordinate = format!("30402:{}:game", publisher.public_key());
    let auth = authority(&publisher, &operator, &key);
    let initial = resolve_authorization(auth.id, &[auth.clone()]).expect("auth");
    let cancel = cancel_authority_builder(&initial, publisher.public_key(), auth.id, 200)
        .expect("builder")
        .sign_with_keys(&publisher)
        .expect("signature");
    let resolved_auth = resolve_authorization(auth.id, &[auth, cancel]).expect("cancelled");
    let campaign = EventBuilder::new(Kind::Custom(1031), "")
        .tags([
            tag(&["d", "campaign"]),
            tag(&["a", &coordinate]),
            tag(&["status", "active"]),
            tag(&["mode", "claim"]),
            tag(&["starts", "120"]),
            tag(&["ends", "300"]),
        ])
        .custom_created_at(Timestamp::from_secs(100))
        .sign_with_keys(&publisher)
        .expect("campaign");
    let campaign_cancel = build_campaign_event_builder(&CampaignBuildParams::cancel(
        "campaign".into(),
        coordinate.clone(),
        campaign.id,
    ))
    .expect("cancel builder")
    .custom_created_at(Timestamp::from_secs(200))
    .sign_with_keys(&publisher)
    .expect("campaign cancel");
    let resolved_campaign = resolve_campaign(
        &[
            parse_campaign_event(&campaign).expect("campaign"),
            parse_campaign_event(&campaign_cancel).expect("cancel"),
        ],
        publisher.public_key(),
        &coordinate,
    )
    .expect("campaign chain");
    let root = grant(
        &publisher,
        &key,
        &buyer,
        campaign.id,
        Some(resolved_auth.root_event_id),
        "one",
    );
    let other = grant(
        &publisher,
        &key,
        &buyer,
        campaign.id,
        Some(resolved_auth.root_event_id),
        "two",
    );
    let initial_grant =
        resolve_entitlement_grant(&[parse_entitlement_event(&root).expect("grant")]).expect("root");
    validate_adp_entitlement(&initial_grant, &resolved_campaign, Some(&resolved_auth))
        .expect("historical grant remains valid");
    let revoked = revoke_grant_builder(&initial_grant, publisher.public_key(), root.id, 210)
        .expect("builder")
        .sign_with_keys(&publisher)
        .expect("revocation");
    validate_revocation(&root, &revoked, false).expect("NIP-103 transition");
    let chain = resolve_entitlement_grant(&[
        parse_entitlement_event(&root).expect("root"),
        parse_entitlement_event(&revoked).expect("revocation"),
    ])
    .expect("chain");
    validate_adp_entitlement(&chain, &resolved_campaign, Some(&resolved_auth))
        .expect("valid historical chain");
    assert_eq!(chain.status(), Some(GrantStatus::Revoked));
    let other = resolve_entitlement_grant(&[parse_entitlement_event(&other).expect("other")])
        .expect("independent grant");
    assert_eq!(other.status(), Some(GrantStatus::Granted));
    assert!(revoke_grant_builder(&chain, publisher.public_key(), root.id, 220).is_err());
    assert!(revoke_grant_builder(&initial_grant, operator.public_key(), root.id, 210).is_err());
    assert!(revoke_grant_builder(&initial_grant, publisher.public_key(), revoked.id, 210).is_err());
}

fn step(kind: &str, target: &str, outcome: Option<serde_json::Value>) -> PublisherCheckpoint {
    PublisherCheckpoint {
        step_id: kind.into(),
        kind: kind.into(),
        target: target.into(),
        intent: json!({}),
        state: if outcome.is_some() {
            "acknowledged"
        } else {
            "prepared"
        }
        .into(),
        outcome,
        last_error: None,
    }
}
#[test]
fn partial_distribution_evidence_never_claims_availability_or_merges_operations() {
    let first = "https://one.test";
    let second = "https://two.test";
    let mut steps = vec![
        step("provision", first, Some(json!({"key":"public"}))),
        step("upload", second, None),
    ];
    steps[1].last_error = Some("transport interrupted".into());
    assert_eq!(
        distribution_evidence(first, "original", &steps).state,
        DistributionState::Provisioned
    );
    assert_eq!(
        distribution_evidence(second, "original", &steps).state,
        DistributionState::Unknown
    );
    steps.push(step(
        "upload",
        first,
        Some(json!({"file_hash":"ab".repeat(32)})),
    ));
    assert_eq!(
        distribution_evidence(first, "original", &steps).state,
        DistributionState::RegistrationAcknowledged
    );
    assert_eq!(
        distribution_evidence(first, "new-revision", &[]).state,
        DistributionState::Declared
    );
    let mut pending = step("distribution", first, None);
    pending.intent = json!({"upload_expected":true});
    assert_eq!(
        distribution_evidence(first, "new-revision", &[pending.clone()]).state,
        DistributionState::UploadPending
    );
    pending.last_error = Some("authority unavailable; upload not started".into());
    assert_eq!(
        distribution_evidence(first, "new-revision", &[pending]).state,
        DistributionState::Failed
    );
}

#[tokio::test]
async fn partial_operator_state_and_revocation_intent_survive_restart_and_account_isolation() {
    use arcadestr_core::{
        publisher_recovery::{repository::PublisherRepository, PublisherOperation},
        storage::Database,
    };
    let temp = tempfile::tempdir().expect("temp");
    let path = temp.path().join("management.db");
    let db = Database::new(&path).await.expect("db");
    let repo = PublisherRepository::new(db.pool().clone());
    repo.begin(
        "alice",
        &PublisherOperation {
            operation_id: "partial".into(),
            kind: "listing".into(),
            coordinate: "game".into(),
            intent: json!({"roles":"original"}),
            artifact_hash: None,
            state: "incomplete".into(),
            result: None,
        },
    )
    .await
    .expect("begin");
    for (url, outcome) in [
        (
            "https://one.test",
            Some(json!({"fulfillment_pubkey":"public"})),
        ),
        ("https://two.test", None),
    ] {
        let mut checkpoint = step("provision", url, None);
        checkpoint.step_id = url.into();
        repo.checkpoint("alice", "partial", &checkpoint)
            .await
            .expect("prepare");
        if let Some(outcome) = outcome {
            repo.acknowledge("alice", "partial", url, &outcome)
                .await
                .expect("ack");
        }
    }
    drop(repo);
    db.pool().close().await;
    let db = Database::new(&path).await.expect("reopen");
    let repo = PublisherRepository::new(db.pool().clone());
    let steps = repo.steps("alice", "partial").await.expect("steps");
    assert_eq!(
        distribution_evidence("https://one.test", "partial", &steps).state,
        DistributionState::Provisioned
    );
    assert_eq!(
        distribution_evidence("https://two.test", "partial", &steps).state,
        DistributionState::Unknown
    );
    assert!(repo
        .operations("bob")
        .await
        .expect("other account")
        .is_empty());
    assert!(repo
        .checkpoint("bob", "partial", &step("upload", "https://two.test", None))
        .await
        .is_err());
}
