#![cfg(feature = "native")]
//! Isolated SQLite/signed fixtures; no automatic recovery executor or live service.
use arcadestr_core::{
    publisher_recovery::{repository::PublisherRepository, *},
    storage::Database,
};
use serde_json::json;

fn operation(id: &str) -> PublisherOperation {
    PublisherOperation {
        operation_id: id.into(),
        kind: "listing".into(),
        coordinate: "30402:publisher:game".into(),
        intent: json!({"title":"Game","terms":["exact-root"],"authority":["historical-root"]}),
        artifact_hash: Some("ab".repeat(32)),
        state: "incomplete".into(),
        result: None,
    }
}
fn checkpoint(id: &str, kind: &str) -> PublisherCheckpoint {
    PublisherCheckpoint {
        step_id: id.into(),
        kind: kind.into(),
        target: "https://operator.test".into(),
        intent: json!({"coordinate":"30402:publisher:game","file_hash":"ab".repeat(32)}),
        state: "prepared".into(),
        outcome: None,
        last_error: None,
    }
}

#[tokio::test]
async fn drafts_survive_restart_are_account_scoped_and_reject_stale_writers() {
    let temp = tempfile::tempdir().expect("temporary directory");
    let path = temp.path().join("publisher.db");
    let db = Database::new(&path).await.expect("database");
    let repo = PublisherRepository::new(db.pool().clone());
    let mut draft = PublisherDraft {
        draft_id: "listing:new".into(),
        kind: "listing".into(),
        payload: json!({"title":"Draft","store_page":{"markdown":"# Exact content"},"operators":["one","two"],"terms":["unchanged-root"],"hash":"ab".repeat(32)}),
        revision: 0,
    };
    assert_eq!(
        repo.save_draft("alice", &draft, None).await.expect("save"),
        1
    );
    assert!(repo.save_draft("alice", &draft, None).await.is_err());
    draft.payload["title"] = json!("Updated draft");
    assert_eq!(
        repo.save_draft("alice", &draft, Some(1))
            .await
            .expect("update"),
        2
    );
    assert!(repo.save_draft("alice", &draft, Some(1)).await.is_err());
    assert!(repo.drafts("bob").await.expect("bob").is_empty());
    db.pool().close().await;
    let db = Database::new(&path).await.expect("restart");
    let repo = PublisherRepository::new(db.pool().clone());
    let restored = repo.drafts("alice").await.expect("restore");
    assert_eq!(restored[0].payload, draft.payload);
    assert_eq!(restored[0].revision, 2);
    assert!(repo.drafts("bob").await.expect("isolation").is_empty());
}

#[tokio::test]
async fn interruption_before_and_after_each_side_effect_retains_uncertainty_without_execution() {
    let temp = tempfile::tempdir().expect("temporary directory");
    for (index, kind) in [
        "signing",
        "signed_event",
        "provision",
        "relay",
        "upload",
        "pointer_update",
    ]
    .into_iter()
    .enumerate()
    {
        let path = temp.path().join(format!("checkpoint-{index}.db"));
        let db = Database::new(&path).await.expect("database");
        let repo = PublisherRepository::new(db.pool().clone());
        repo.begin("alice", &operation("original"))
            .await
            .expect("begin");
        let step = checkpoint("original-step", kind);
        repo.checkpoint("alice", "original", &step)
            .await
            .expect("before external action");
        // Simulate both a crash before dispatch and a crash after dispatch but
        // before acknowledgement. The same prepared state must remain unknown.
        db.pool().close().await;
        let db = Database::new(&path)
            .await
            .expect("restart before acknowledgement");
        let repo = PublisherRepository::new(db.pool().clone());
        let restored = repo.steps("alice", "original").await.expect("restore");
        assert_eq!(restored.len(), 1);
        assert_eq!(restored[0].state, "prepared");
        assert!(restored[0].outcome.is_none());
        assert_eq!(
            repo.operation("alice", "original")
                .await
                .expect("operation")
                .state,
            "incomplete"
        );
        repo.acknowledge(
            "alice",
            "original",
            "original-step",
            &json!({"acknowledged":true}),
        )
        .await
        .expect("after response");
        db.pool().close().await;
        let db = Database::new(&path).await.expect("restart after response");
        let repo = PublisherRepository::new(db.pool().clone());
        let restored = repo
            .steps("alice", "original")
            .await
            .expect("restore acknowledgement");
        assert_eq!(restored[0].state, "acknowledged");
        assert_eq!(restored[0].outcome, Some(json!({"acknowledged":true})));
    }
}

#[tokio::test]
async fn partial_multi_server_results_and_exact_signed_events_are_immutable() {
    use nostr::{EventBuilder, Keys, Kind, Tag};
    let temp = tempfile::tempdir().expect("temporary directory");
    let db = Database::new(&temp.path().join("publisher.db"))
        .await
        .expect("database");
    let repo = PublisherRepository::new(db.pool().clone());
    repo.begin("alice", &operation("original"))
        .await
        .expect("begin");
    let event = EventBuilder::new(Kind::Custom(30402), "Pinned content")
        .tag(Tag::parse(["d", "game"]).expect("tag"))
        .sign_with_keys(&Keys::generate())
        .expect("signed");
    let mut signed = checkpoint("signed-listing", "signed_event");
    signed.intent = serde_json::to_value(&event).expect("event JSON");
    repo.checkpoint("alice", "original", &signed)
        .await
        .expect("retain exact event");
    repo.checkpoint("alice", "original", &signed)
        .await
        .expect("duplicate is identical");
    let mut mutated = signed.clone();
    mutated.intent["created_at"] = json!(0);
    assert!(repo
        .checkpoint("alice", "original", &mutated)
        .await
        .is_err());
    repo.checkpoint("alice", "original", &checkpoint("upload:one", "upload"))
        .await
        .expect("one");
    repo.acknowledge(
        "alice",
        "original",
        "upload:one",
        &json!({"file_hash":"ab".repeat(32)}),
    )
    .await
    .expect("one acknowledged");
    repo.checkpoint("alice", "original", &checkpoint("upload:two", "upload"))
        .await
        .expect("two uncertain");
    repo.record_failure(
        "alice",
        "original",
        "upload:two",
        "Unacknowledged upload response",
    )
    .await
    .expect("retain safe diagnostic");
    assert!(repo
        .record_failure("bob", "original", "upload:two", "foreign overwrite")
        .await
        .is_err());
    let steps = repo.steps("alice", "original").await.expect("read");
    assert_eq!(
        steps
            .iter()
            .find(|s| s.step_id == "upload:one")
            .expect("one")
            .state,
        "acknowledged"
    );
    assert_eq!(
        steps
            .iter()
            .find(|s| s.step_id == "upload:two")
            .expect("two")
            .state,
        "prepared"
    );
    assert_eq!(
        steps
            .iter()
            .find(|step| step.step_id == "upload:two")
            .expect("two")
            .last_error
            .as_deref(),
        Some("Unacknowledged upload response")
    );
    assert_eq!(
        steps
            .iter()
            .find(|s| s.step_id == "signed-listing")
            .expect("event")
            .intent,
        signed.intent
    );
    assert!(repo.operation("bob", "original").await.is_err());
    assert!(repo
        .steps("bob", "original")
        .await
        .expect("isolation")
        .is_empty());
    assert!(repo
        .acknowledge("bob", "original", "upload:two", &json!({}))
        .await
        .is_err());
}

#[tokio::test]
async fn checkpoints_and_results_are_atomic_conflicting_retries_cannot_replace_evidence() {
    let temp = tempfile::tempdir().expect("temporary directory");
    let db = Database::new(&temp.path().join("publisher.db"))
        .await
        .expect("database");
    let repo = PublisherRepository::new(db.pool().clone());
    repo.begin("alice", &operation("original"))
        .await
        .expect("begin");
    assert!(repo.begin("alice", &operation("original")).await.is_err());
    let step = checkpoint("upload", "upload");
    repo.checkpoint("alice", "original", &step)
        .await
        .expect("prepared");
    repo.acknowledge("alice", "original", "upload", &json!({"hash":"original"}))
        .await
        .expect("ack");
    repo.acknowledge("alice", "original", "upload", &json!({"hash":"original"}))
        .await
        .expect("idempotent acknowledgement");
    assert!(repo
        .acknowledge("alice", "original", "upload", &json!({"hash":"different"}))
        .await
        .is_err());
    let mut different = step;
    different.target = "https://substitution.test".into();
    assert!(repo
        .checkpoint("alice", "original", &different)
        .await
        .is_err());
    assert_eq!(
        repo.steps("alice", "original").await.expect("unchanged")[0].target,
        "https://operator.test"
    );
    repo.complete("alice", "original", &json!({"event":"original"}))
        .await
        .expect("complete");
    assert!(repo
        .complete("alice", "original", &json!({"event":"substitute"}))
        .await
        .is_err());
    assert!(repo
        .checkpoint("alice", "original", &checkpoint("later", "relay"))
        .await
        .is_err());
    assert!(repo
        .checkpoint("bob", "original", &checkpoint("later", "relay"))
        .await
        .is_err());
    assert_eq!(
        repo.operation("alice", "original")
            .await
            .expect("original result")
            .result,
        Some(json!({"event":"original"}))
    );
}

#[test]
fn durable_payload_never_restores_credentials_or_native_capabilities() {
    for payload in [
        json!({"selected_build":null}),
        json!({"nested":[{"selection_id":"expired"}]}),
        json!({"private_key":"never"}),
        json!({"file_path":"/private/game.zip"}),
        json!({"server":"https://publisher:secret@operator.test"}),
        json!({"markdown":"x".repeat(2 * 1024 * 1024)}),
    ] {
        assert!(validate_durable_payload(&payload).is_err());
    }
    assert!(validate_durable_payload(&json!({"artifact_hash":"ab".repeat(32),"profiles":[{"executable":"bin/game","arguments":["--safe"]}],"accepted_terms":["immutable"]})).is_ok());
}

#[test]
fn reselected_archive_must_match_exact_canonical_pinned_digest() {
    let hash = "ab".repeat(32);
    assert!(verify_reselected_artifact(&hash, &hash).is_ok());
    assert!(verify_reselected_artifact(&hash, &"cd".repeat(32)).is_err());
    assert!(verify_reselected_artifact(&hash.to_uppercase(), &hash.to_uppercase()).is_err());
    assert!(verify_reselected_artifact("expired-handle", "expired-handle").is_err());
}

#[tokio::test]
async fn concurrent_draft_saves_admit_only_one_revision() {
    let temp = tempfile::tempdir().expect("temporary directory");
    let db = Database::new(&temp.path().join("publisher.db"))
        .await
        .expect("database");
    let repo = PublisherRepository::new(db.pool().clone());
    let draft = PublisherDraft {
        draft_id: "store_page:game".into(),
        kind: "store_page".into(),
        payload: json!({"markdown":"original"}),
        revision: 0,
    };
    repo.save_draft("alice", &draft, None).await.expect("first");
    let (first, second) = tokio::join!(
        repo.save_draft("alice", &draft, Some(1)),
        repo.save_draft("alice", &draft, Some(1))
    );
    assert_ne!(first.is_ok(), second.is_ok());
    assert_eq!(repo.drafts("alice").await.expect("read")[0].revision, 2);
}
