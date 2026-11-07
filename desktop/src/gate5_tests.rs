// Isolated signed listings, private SQLite fixtures and mock HTTP only.
#[tokio::test]
async fn gate5_committed_restart_and_atomic_rejection() {
    let buyer = Keys::generate();
    let merchant = Keys::generate();
    let id = "gate5-committed-restart";
    let coordinate = coordinate(&merchant, id);
    let url = format!(
        "https://one.example/game/{}",
        urlencoding::encode(&coordinate)
    );
    let http = MockHttpClient::new().with_download_response(&url, []);
    let state = app_state_with_http(
        &buyer,
        test_db("gate5-committed-restart").await,
        Arc::new(http),
    )
    .await;
    grant_ownership(&state, &buyer, &merchant, id).await;
    let root = unique_test_path("gate5-committed-restart-data", "dir");
    gate5_install(
        &state,
        &merchant,
        id,
        gate5_listing(&merchant, id, &["https://one.example"], "1"),
        &root,
    )
    .await
    .expect("initial install");
    let repo = CoreInstallationsRepository::new(state.database.pool().clone());
    let old = repo
        .get_by_coordinate(&coordinate)
        .await
        .expect("snapshot")
        .expect("old");
    let old_package = repo
        .get_retained_package(&coordinate)
        .await
        .expect("retained")
        .expect("old package");
    let mut candidate = old.installation.clone();
    candidate.state = InstallationState::Installing;
    candidate.installation_root = root
        .join("installations")
        .join(InstallationId::generate().as_str());
    candidate.retained_package_coordinate = None;
    candidate.raw_event_json =
        serde_json::to_string(&gate5_listing(&merchant, id, &["https://one.example"], "2"))
            .expect("signed candidate");
    let mut package = old_package.clone();
    package.package_id = RetainedPackageId::generate();
    package.package_path = root
        .join("retained-packages")
        .join(format!("{}.package", package.package_id.as_str()));
    package.reinstall.raw_event_json = candidate.raw_event_json.clone();
    let mut stale_candidate = candidate.clone();
    stale_candidate.raw_event_json = old.installation.raw_event_json.clone();
    assert!(
        repo.begin_update(&old, &stale_candidate, &old.profiles, &package.package_path)
            .await
            .is_err(),
        "stale signed listing cannot roll back an installed revision"
    );
    let journal = repo
        .begin_update(&old, &candidate, &old.profiles, &package.package_path)
        .await
        .expect("intent");
    let lock = install::OperationLock::acquire(&candidate.installation_root).expect("root lock");
    std::fs::copy(&old_package.package_path, &package.package_path)
        .expect("verified candidate bytes");
    let verified = arcadestr_core::package_extraction::VerifiedPackage::verify(
        Arc::new(std::fs::File::open(&package.package_path).expect("candidate file")),
        package.package_digest.clone(),
        package.package_format.clone(),
    )
    .expect("verified");
    let placed = install::extract_and_place_verified_package(
        &verified,
        &candidate.installation_root,
        &package.reinstall.launch_profiles,
        lock,
    )
    .expect("placed candidate");
    let mut mismatched = package.clone();
    let mut unavailable = old.profiles.clone();
    for profile in &mut unavailable {
        profile.availability =
            ProfileAvailability::Unavailable(ProfileUnavailableReason::ExecutableMissing);
    }
    assert!(
        repo.commit_update(&journal, &package, &unavailable, &old.manifest, 2)
            .await
            .is_err(),
        "do not replace a working build with a known unlaunchable replacement"
    );
    mismatched.package_path = old_package.package_path.clone();
    assert!(repo
        .commit_update(&journal, &mismatched, &old.profiles, &old.manifest, 2)
        .await
        .is_err());
    assert!(
        repo.commit_update(&journal, &package, &old.profiles, &[], 2)
            .await
            .is_err(),
        "incomplete manifest rolls back entire transaction"
    );
    assert_eq!(
        repo.get_by_coordinate(&coordinate).await.expect("snapshot"),
        Some(old.clone())
    );
    assert_eq!(
        repo.get_retained_package(&coordinate)
            .await
            .expect("package"),
        Some(old_package.clone())
    );
    sqlx::query("UPDATE installations SET updated_at=updated_at+1 WHERE id=?")
        .bind(old.installation.id.as_str())
        .execute(state.database.pool())
        .await
        .expect("simulate stale snapshot");
    assert!(
        repo.commit_update(&journal, &package, &old.profiles, &old.manifest, 3)
            .await
            .is_err(),
        "stale update must not overwrite a concurrent snapshot"
    );
    sqlx::query("UPDATE installations SET updated_at=? WHERE id=?")
        .bind(old.installation.updated_at)
        .bind(old.installation.id.as_str())
        .execute(state.database.pool())
        .await
        .expect("restore fixture snapshot");
    install::commit_placed_update(&repo, &journal, &package, placed, 3)
        .await
        .expect("atomic commit");
    // Simulate exit after DB commit, before old-file cleanup. Both versions
    // still exist, but only the verified replacement is visible to launch.
    assert!(old.installation.installation_root.exists());
    assert!(old_package.package_path.exists());
    assert!(repo.pending_updates().await.expect("intent")[0].committed);
    install::recover_updates(&repo, &root, 4, None)
        .await
        .expect("post-commit restart");
    let current = repo
        .get_by_coordinate(&coordinate)
        .await
        .expect("snapshot")
        .expect("active");
    assert_eq!(
        current.installation.installation_root,
        candidate.installation_root
    );
    assert!(current
        .installation
        .installation_root
        .join("bin/Game.exe")
        .is_file());
    assert!(package.package_path.is_file());
    assert!(!old.installation.installation_root.exists());
    assert!(!old_package.package_path.exists());
    install::recover_updates(&repo, &root, 5, None)
        .await
        .expect("idempotent restart");
    assert!(repo.pending_updates().await.expect("intent").is_empty());
}

fn gate5_listing(merchant: &Keys, id: &str, servers: &[&str], version: &str) -> Event {
    let event = listing_event(merchant, id, servers[0], "ignored", version);
    let mut tags = event.tags.iter().cloned().collect::<Vec<_>>();
    for server in &servers[1..] {
        tags.push(Tag::parse(["server", server]).expect("server tag"));
    }
    EventBuilder::new(event.kind, event.content)
        .tags(tags)
        .custom_created_at(Timestamp::from(
            1_700_000_200 + version.parse::<u64>().unwrap_or(0),
        ))
        .sign_with_keys(merchant)
        .expect("signed fixture")
}

async fn gate5_install(
    state: &AppState,
    merchant: &Keys,
    id: &str,
    event: Event,
    app_data: &Path,
) -> Result<(), String> {
    install_game_with_fetcher(
        app_listing(merchant, id),
        state,
        None::<&tauri::AppHandle<tauri::test::MockRuntime>>,
        app_data.to_owned(),
        &StaticFreshListingFetcher {
            event,
            calls: Arc::new(AtomicUsize::new(0)),
        },
    )
    .await
}

#[tokio::test]
async fn gate5_fallback_transport_corruption_and_status_policy() {
    for failure in [
        HttpClientError::Request("offline secret".into()),
        HttpClientError::Status(503),
        HttpClientError::Status(500),
        HttpClientError::Status(401),
        HttpClientError::Status(403),
        HttpClientError::Status(451),
        HttpClientError::Status(413),
        HttpClientError::InsufficientStorage("insufficient_storage fixture".into()),
        HttpClientError::RedirectBlocked("blocked".into()),
        HttpClientError::DownloadIo("permission denied".into()),
    ] {
        let buyer = Keys::generate();
        let merchant = Keys::generate();
        let id = "gate5-failover";
        let coordinate = coordinate(&merchant, id);
        let encoded = urlencoding::encode(&coordinate);
        let first = format!("https://one.example/game/{encoded}");
        let second = format!("https://two.example/game/{encoded}");
        let should_retry = matches!(
            failure,
            HttpClientError::Request(_) | HttpClientError::Status(503 | 500)
        );
        let http = MockHttpClient::new()
            .with_error(&first, failure)
            .with_download_response(&second, []);
        let state = app_state_with_http(
            &buyer,
            test_db("gate5-fallback").await,
            Arc::new(http.clone()),
        )
        .await;
        grant_ownership(&state, &buyer, &merchant, id).await;
        let root = unique_test_path("gate5-failover-data", "dir");
        let result = gate5_install(
            &state,
            &merchant,
            id,
            gate5_listing(
                &merchant,
                id,
                &["https://one.example", "https://two.example"],
                "1",
            ),
            &root,
        )
        .await;
        assert_eq!(result.is_ok(), should_retry, "{result:?}");
        assert_eq!(http.call_count(&first), 1);
        assert_eq!(http.call_count(&second), usize::from(should_retry));
        if let Err(error) = result {
            assert!(!error.to_string().contains("secret"));
        }
    }
    // Truncate the private candidate before retry: a corrupt, longer source
    // must not leave a suffix on the successful second source.
    let buyer = Keys::generate();
    let merchant = Keys::generate();
    let id = "gate5-corrupt";
    let coordinate = coordinate(&merchant, id);
    let encoded = urlencoding::encode(&coordinate);
    let first = format!("https://one.example/game/{encoded}");
    let second = format!("https://two.example/game/{encoded}");
    let http = MockHttpClient::new()
        .with_exact_bytes(&first, vec![0; install_fixture_bytes().len() + 1000])
        .with_download_response(&second, []);
    let state = app_state_with_http(
        &buyer,
        test_db("gate5-corrupt").await,
        Arc::new(http.clone()),
    )
    .await;
    grant_ownership(&state, &buyer, &merchant, id).await;
    gate5_install(
        &state,
        &merchant,
        id,
        gate5_listing(
            &merchant,
            id,
            &["https://one.example", "https://two.example"],
            "1",
        ),
        &unique_test_path("gate5-corrupt-data", "dir"),
    )
    .await
    .expect("fallback verifies exact artifact");
    assert_eq!(http.call_count(&second), 1);
}

#[tokio::test]
async fn gate5_token_preference_scope_expiry_and_no_eligibility_downgrade() {
    for (owned, expires_at, success) in [
        (true, 4_000_000_000, true),
        (false, 4_000_000_000, false),
        (true, 1, true),
        (false, 1, false),
    ] {
        let buyer = Keys::generate();
        let merchant = Keys::generate();
        let id = "gate5-token";
        let coordinate = coordinate(&merchant, id);
        let encoded = urlencoding::encode(&coordinate);
        let token_url = format!("https://two.example/game/{encoded}?token=host-two-secret");
        let first = format!("https://one.example/game/{encoded}");
        let http = MockHttpClient::new()
            .with_error(&token_url, HttpClientError::Status(503))
            .with_download_response(&first, []);
        let state =
            app_state_with_http(&buyer, test_db("gate5-token").await, Arc::new(http.clone())).await;
        if owned {
            grant_ownership(&state, &buyer, &merchant, id).await;
        }
        DownloadTokensRepository::new(state.database.pool().clone())
            .upsert(&DownloadToken {
                buyer_pubkey: buyer.public_key().to_hex(),
                game_coordinate: coordinate.clone(),
                server_url: "https://two.example".into(),
                token: "host-two-secret".into(),
                expires_at,
            })
            .await
            .expect("token");
        let result = gate5_install(
            &state,
            &merchant,
            id,
            gate5_listing(
                &merchant,
                id,
                &["https://one.example", "https://two.example"],
                "1",
            ),
            &unique_test_path("gate5-token-data", "dir"),
        )
        .await;
        assert_eq!(result.is_ok(), success, "{result:?}");
        assert_eq!(http.call_count(&token_url), usize::from(expires_at > 1));
        assert_eq!(http.call_count(&first), usize::from(success));
        assert_eq!(
            http.call_count(&format!(
                "https://one.example/game/{encoded}?token=host-two-secret"
            )),
            0
        );
        if success {
            assert!(http
                .last_download_headers(&first)
                .expect("NIP98 headers")
                .iter()
                .any(|(name, _)| name == "Authorization"));
        }
    }
}

#[tokio::test]
async fn gate5_update_success_failure_and_rollback() {
    for mode in 0..6 {
        let fail = mode != 0;
        let buyer = Keys::generate();
        let merchant = Keys::generate();
        let id = "gate5-update";
        let coordinate = coordinate(&merchant, id);
        let url = format!(
            "https://one.example/game/{}",
            urlencoding::encode(&coordinate)
        );
        let http = MockHttpClient::new().with_download_response(&url, []);
        let state = app_state_with_http(
            &buyer,
            test_db("gate5-update").await,
            Arc::new(http.clone()),
        )
        .await;
        grant_ownership(&state, &buyer, &merchant, id).await;
        let root = unique_test_path("gate5-update-data", "dir");
        gate5_install(
            &state,
            &merchant,
            id,
            gate5_listing(&merchant, id, &["https://one.example"], "1"),
            &root,
        )
        .await
        .expect("initial install");
        let repo = CoreInstallationsRepository::new(state.database.pool().clone());
        let old = repo
            .get_by_coordinate(&coordinate)
            .await
            .expect("snapshot")
            .expect("old");
        let old_package = repo
            .get_retained_package(&coordinate)
            .await
            .expect("retained")
            .expect("old package");
        if mode == 1 {
            http.clone()
                .with_exact_bytes(&url, b"corrupt replacement".to_vec());
        } else if mode >= 2 {
            http.clone().with_error(
                &url,
                match mode {
                    2 => {
                        HttpClientError::InsufficientStorage("insufficient_storage fixture".into())
                    }
                    3 => HttpClientError::Status(503),
                    4 => HttpClientError::Status(403),
                    _ => HttpClientError::Status(451),
                },
            );
        } else {
            http.clone().with_download_response(&url, []);
        }
        let result = gate5_install(
            &state,
            &merchant,
            id,
            gate5_listing(&merchant, id, &["https://one.example"], "2"),
            &root,
        )
        .await;
        assert_eq!(result.is_ok(), !fail, "{result:?}");
        let current = repo
            .get_by_coordinate(&coordinate)
            .await
            .expect("snapshot")
            .expect("active");
        let package = repo
            .get_retained_package(&coordinate)
            .await
            .expect("retained")
            .expect("active package");
        assert_eq!(current.installation.state, InstallationState::Ready);
        assert!(current
            .installation
            .installation_root
            .join("bin/Game.exe")
            .is_file());
        if fail {
            assert_eq!(current, old);
            assert_eq!(package, old_package);
        } else {
            assert_ne!(
                current.installation.installation_root,
                old.installation.installation_root
            );
            assert!(!old.installation.installation_root.exists());
            assert!(!old_package.package_path.exists());
            assert_ne!(package.package_path, old_package.package_path);
        }
        assert!(repo.pending_updates().await.expect("journal").is_empty());
    }
}

#[tokio::test]
async fn gate5_restart_rolls_back_candidate_and_serializes_recovery() {
    for phase in 0..3 {
        let buyer = Keys::generate();
        let merchant = Keys::generate();
        let id = "gate5-restart";
        let coordinate = coordinate(&merchant, id);
        let url = format!(
            "https://one.example/game/{}",
            urlencoding::encode(&coordinate)
        );
        let http = MockHttpClient::new().with_download_response(&url, []);
        let state =
            app_state_with_http(&buyer, test_db("gate5-restart").await, Arc::new(http)).await;
        grant_ownership(&state, &buyer, &merchant, id).await;
        let root = unique_test_path("gate5-restart-data", "dir");
        gate5_install(
            &state,
            &merchant,
            id,
            gate5_listing(&merchant, id, &["https://one.example"], "1"),
            &root,
        )
        .await
        .expect("initial install");
        let repo = CoreInstallationsRepository::new(state.database.pool().clone());
        let old = repo
            .get_by_coordinate(&coordinate)
            .await
            .expect("snapshot")
            .expect("old");
        let mut candidate = old.installation.clone();
        candidate.state = InstallationState::Installing;
        candidate.installation_root = root
            .join("installations")
            .join(InstallationId::generate().as_str());
        candidate.retained_package_coordinate = None;
        candidate.raw_event_json =
            serde_json::to_string(&gate5_listing(&merchant, id, &["https://one.example"], "2"))
                .expect("new signed revision");
        let package_path = root.join("retained-packages").join(format!(
            "{}.package",
            RetainedPackageId::generate().as_str()
        ));
        let journal = repo
            .begin_update(&old, &candidate, &old.profiles, &package_path)
            .await
            .expect("intent");
        assert!(
            repo.begin_update(&old, &candidate, &old.profiles, &package_path)
                .await
                .is_err(),
            "one intent per game"
        );
        let candidate_lock =
            install::OperationLock::acquire(&candidate.installation_root).expect("candidate lock");
        if phase == 2 {
            std::fs::create_dir(&candidate.installation_root).expect("partly placed candidate");
        }
        let staging = root.join("installations").join(format!(
            ".{}.{}.tmp",
            candidate
                .installation_root
                .file_name()
                .expect("root name")
                .to_string_lossy(),
            uuid::Uuid::new_v4()
        ));
        if phase == 1 {
            std::fs::create_dir(&staging).expect("interrupted extraction");
            std::fs::write(staging.join("partial"), b"partial extraction")
                .expect("staging content");
        }
        std::fs::write(&package_path, b"partial download").expect("candidate archive");
        let lifecycle = acquire_coordinate_lifecycle_lock(&root, &coordinate)
            .await
            .expect("active operation");
        install::recover_updates(&repo, &root, 1, None)
            .await
            .expect("busy recovery skips");
        assert_eq!(repo.pending_updates().await.expect("journal").len(), 1);
        drop(lifecycle);
        drop(candidate_lock);
        install::recover_updates(&repo, &root, 2, None)
            .await
            .expect("restart rollback");
        assert_eq!(
            repo.get_by_coordinate(&coordinate).await.expect("snapshot"),
            Some(old)
        );
        assert!(!candidate.installation_root.exists());
        assert!(!package_path.exists());
        assert!(!staging.exists());
        assert!(repo.pending_updates().await.expect("journal").is_empty());
        repo.finish_update(&journal)
            .await
            .expect("idempotent cleanup");
    }
}
