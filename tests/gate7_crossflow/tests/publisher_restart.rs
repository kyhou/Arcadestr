use arcadestr_core::{
    adp_client::AdpClient,
    publisher_recovery::{
        repository::PublisherRepository, verify_reselected_artifact, PublisherCheckpoint,
        PublisherOperation,
    },
    signers::NostrSigner,
    storage::Database,
};
use nostr_sdk::{Event, Keys};
use sha2::{Digest, Sha256};
use std::path::Path;

pub async fn publisher_interruption_recovery(
    client: &AdpClient,
    signer: &dyn NostrSigner,
    developer: &Keys,
    sale: &Event,
    archive: &Path,
    db_path: &Path,
) {
    let publisher = developer.public_key().to_hex();
    let hash = hex::encode(Sha256::digest(
        std::fs::read(archive).expect("fixture bytes"),
    ));
    let db = Database::new(db_path).await.expect("publisher database");
    let repo = PublisherRepository::new(db.pool().clone());
    let operation = PublisherOperation {
        operation_id: "interrupted-publication".into(),
        kind: "listing".into(),
        coordinate: format!("30402:{}:public", developer.public_key()),
        intent: serde_json::json!({"signed_event":sale}),
        artifact_hash: Some(hash.clone()),
        state: "incomplete".into(),
        result: None,
    };
    repo.begin(&publisher, &operation)
        .await
        .expect("immutable publisher intent");
    let checkpoint = PublisherCheckpoint {
        step_id: "upload-a".into(),
        kind: "upload".into(),
        target: "original-authorized-server".into(),
        intent: serde_json::json!({"signed_event":sale,"artifact_hash":hash}),
        state: "prepared".into(),
        outcome: None,
        last_error: None,
    };
    repo.checkpoint(&publisher, &operation.operation_id, &checkpoint)
        .await
        .expect("checkpoint before side effect");
    let first = client
        .upload(signer, sale, archive)
        .await
        .expect("actual committed upload before simulated lost acknowledgement");
    repo.record_failure(
        &publisher,
        &operation.operation_id,
        &checkpoint.step_id,
        "fixture: response acknowledgement lost",
    )
    .await
    .expect("retain uncertain intent");
    db.close().await;
    let reopened = Database::new(db_path).await.expect("publisher restart");
    let repo = PublisherRepository::new(reopened.pool().clone());
    let stored = repo
        .operation(&publisher, &operation.operation_id)
        .await
        .expect("original operation");
    assert_eq!(stored.intent, operation.intent);
    assert!(
        repo.operation(
            &Keys::generate().public_key().to_hex(),
            &operation.operation_id
        )
        .await
        .is_err(),
        "other account cannot recover publication"
    );
    verify_reselected_artifact(stored.artifact_hash.as_deref().expect("pinned hash"), &hash)
        .expect("explicit matching reselect");
    assert!(
        verify_reselected_artifact(&hash, &"ff".repeat(32)).is_err(),
        "no artifact replacement during recovery"
    );
    let mut changed = checkpoint.clone();
    changed.intent = serde_json::json!({"signed_event":"different"});
    assert!(
        repo.checkpoint(&publisher, &operation.operation_id, &changed)
            .await
            .is_err(),
        "publisher commitment remains immutable"
    );
    let replay = client
        .upload(signer, sale, archive)
        .await
        .expect("explicit actual-server idempotent recovery");
    assert_eq!(first.file_hash, replay.file_hash);
    assert_eq!(first.game_coordinate, replay.game_coordinate);
    let outcome = serde_json::to_value(&replay).expect("public acknowledgement");
    repo.acknowledge(
        &publisher,
        &operation.operation_id,
        &checkpoint.step_id,
        &outcome,
    )
    .await
    .expect("persist acknowledgement");
    repo.complete(&publisher, &operation.operation_id, &outcome)
        .await
        .expect("completion");
    assert_eq!(
        repo.operation(&publisher, &operation.operation_id)
            .await
            .expect("completed recovery")
            .state,
        "complete"
    );
}
