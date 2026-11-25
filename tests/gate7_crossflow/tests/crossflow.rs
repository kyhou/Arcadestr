mod campaign_flow;
mod fixtures;
mod paid_flow;
mod publisher_restart;
use publisher_restart::publisher_interruption_recovery;
mod quote_response;
use arcadestr_core::{
    adp_client::{AdpClient, DownloadAuth},
    adp_publish::{build_adp_listing_event_builder, AdpListingInput},
    marketplace::AcquisitionPolicy,
};
use fixtures::*;
use nostr_sdk::{Event, Keys};
use sha2::{Digest, Sha256};
use std::sync::{Arc, Mutex};

fn listing(
    developer: &Keys,
    servers: &[String],
    bytes: &[u8],
    id: &str,
    policy: AcquisitionPolicy,
) -> Event {
    build_adp_listing_event_builder(&AdpListingInput {
        d_tag: id.into(),
        title: "Gate 7 local fixture".into(),
        description: "Synthetic acceptance fixture".into(),
        price_sats: 0,
        lud16: None,
        tags: vec!["game".into()],
        images: vec![],
        servers: servers.to_vec(),
        file_hash: Some(hex::encode(Sha256::digest(bytes))),
        version: Some("1".into()),
        fulfillment_authorizations: vec![],
        acquisition: policy,
        platforms: vec![],
        campaigns: vec![],
        nip94_event_id: None,
        preserved_tags: vec![],
    })
    .expect("actual listing builder")
    .sign_with_keys(developer)
    .expect("publisher signature")
}

#[tokio::test]
async fn publisher_registration_public_and_timed_access_use_actual_servers_and_nostr() {
    let events: Events = Arc::new(Mutex::new(vec![]));
    let (one, _one) = relay(events.clone()).await;
    let (two, _two) = relay(events.clone()).await;
    let relays = vec![one, two];
    let a = Server::start(&relays).await;
    let b = Server::start(&relays).await;
    let developer = Keys::generate();
    let buyer = Keys::generate();
    let signer = signer(&developer);
    let buyer_signer = fixtures::signer(&buyer);
    let client_a = AdpClient::new(&a.url, http());
    let client_b = AdpClient::new(&b.url, http());
    let provision_a = client_a
        .provision(&signer, None)
        .await
        .expect("actual provision A");
    let provision_b = client_b
        .provision(&signer, None)
        .await
        .expect("actual provision B");
    assert_ne!(
        provision_a.fulfillment_pubkey,
        provision_b.fulfillment_pubkey
    );
    let info = client_a.well_known().await.expect("operator metadata");
    assert_eq!(info.pubkey, a.operator.public_key().to_hex());
    // Actual server that no listing names in its `server` tags.
    let unlisted = Server::start(&relays).await;
    let dir = tempfile::tempdir().expect("buyer files");
    let bytes = b"synthetic game build";
    let path = dir.path().join("build.zip");
    tokio::fs::write(&path, bytes).await.expect("artifact");
    let file_hash = hex::encode(Sha256::digest(bytes));
    for (id, policy, allowed) in [
        ("public", AcquisitionPolicy::Public, true),
        ("gated", AcquisitionPolicy::Gated, false),
        (
            "timed",
            AcquisitionPolicy::TimedAccess {
                starts_at: 1,
                ends_at: nostr_sdk::Timestamp::now().as_secs() + 3600,
            },
            true,
        ),
        (
            "expired",
            AcquisitionPolicy::TimedAccess {
                starts_at: 1,
                ends_at: nostr_sdk::Timestamp::now().as_secs() - 1,
            },
            false,
        ),
        (
            "upcoming",
            AcquisitionPolicy::TimedAccess {
                starts_at: nostr_sdk::Timestamp::now().as_secs() + 3600,
                ends_at: nostr_sdk::Timestamp::now().as_secs() + 7200,
            },
            false,
        ),
    ] {
        let sale = listing(
            &developer,
            &[a.url.clone(), b.url.clone()],
            bytes,
            id,
            policy,
        );
        // Fixture relay publication uses real Nostr protocol, not external uploads.
        let publisher = nostr_sdk::Client::new(developer.clone());
        for url in &relays {
            publisher.add_relay(url).await.expect("relay");
        }
        publisher.connect().await;
        publisher
            .send_event(&sale)
            .await
            .expect("local listing publication");
        if id == "public" {
            publisher_interruption_recovery(
                &client_a,
                &signer,
                &developer,
                &sale,
                &path,
                &dir.path().join("publisher.db"),
            )
            .await;
        } else {
            client_a
                .upload(&signer, &sale, &path)
                .await
                .expect("actual upload A");
        }
        client_b
            .upload(&signer, &sale, &path)
            .await
            .expect("actual upload B");
        let coordinate = format!("30402:{}:{id}", developer.public_key());
        let destination = dir.path().join(format!("{id}.zip"));
        let download = client_a
            .download(
                &coordinate,
                DownloadAuth::Nip98 {
                    signer: &buyer_signer,
                },
                &destination,
                |_, _| {},
            )
            .await;
        assert_eq!(download.is_ok(), allowed, "{id}");
        if !allowed {
            assert!(
                matches!(
                    download,
                    Err(arcadestr_core::adp_client::AdpClientError::DownloadOwnership(_))
                ),
                "{id} must fail on access policy, not an unrelated transport failure"
            );
        }
        if allowed {
            assert_eq!(
                tokio::fs::read(&destination).await.expect("download"),
                bytes
            );
        }
        // ADP-01 §Access evaluation: public and active timed access serve
        // anonymous requests; gated, expired and upcoming policies grant no
        // anonymous access and answer 401.
        let game_path = format!("/game/{}", coordinate.replace(':', "%3A"));
        let anonymous = reqwest::get(format!("{}{game_path}", a.url))
            .await
            .expect("anonymous local request");
        if allowed {
            assert_eq!(
                anonymous.status(),
                reqwest::StatusCode::OK,
                "{id}: explicit policy serves anonymous downloads"
            );
            let body = anonymous.bytes().await.expect("anonymous archive");
            assert_eq!(&body[..], bytes, "{id}: anonymous archive bytes");
            assert_eq!(
                hex::encode(Sha256::digest(&body)),
                file_hash,
                "{id}: anonymous archive matches the signed file hash"
            );
        } else {
            assert_eq!(
                anonymous.status(),
                reqwest::StatusCode::UNAUTHORIZED,
                "{id}: policy grants no anonymous access"
            );
        }
        // An invalid NIP-98 token is rejected, never downgraded to anonymous.
        let forged = reqwest::Client::new()
            .get(format!("{}{game_path}", a.url))
            .header("Authorization", "Nostr bm90LWEtbmlwOTgtZXZlbnQ=")
            .send()
            .await
            .expect("forged-auth local request");
        assert_eq!(
            forged.status(),
            reqwest::StatusCode::UNAUTHORIZED,
            "{id}: invalid NIP-98 is not treated as anonymous"
        );
        // Distribution authorization applies to every access basis: a server
        // absent from the listing's `server` tags never serves it (451).
        let foreign = reqwest::get(format!("{}{game_path}", unlisted.url))
            .await
            .expect("unlisted-server request");
        assert_eq!(
            foreign.status(),
            reqwest::StatusCode::UNAVAILABLE_FOR_LEGAL_REASONS,
            "{id}: unlisted server must not serve"
        );
        assert!(b
            .storage
            .stored_files()
            .await
            .expect("server B artifact query")
            .iter()
            .any(|f| f.file_hash == file_hash));
    }

    // Hosted-file integrity applies to anonymous policy access too: once
    // server B's stored bytes no longer match the signed hash, it must not
    // serve them.
    let stored = b
        .storage
        .file_path_for_hash(&file_hash)
        .await
        .expect("server B file lookup")
        .expect("server B hosts the artifact");
    tokio::fs::write(&stored, b"tampered game build")
        .await
        .expect("corrupt server B copy");
    let coordinate = format!("30402:{}:public", developer.public_key());
    let tampered = reqwest::get(format!("{}/game/{}", b.url, coordinate.replace(':', "%3A")))
        .await
        .expect("anonymous request to corrupted server");
    assert!(
        !tampered.status().is_success(),
        "corrupted hosted file must not be served anonymously"
    );
    let body = tampered.bytes().await.expect("error body");
    assert_ne!(&body[..], b"tampered game build");
}
