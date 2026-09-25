use super::*;
use arcadestr_core::{
    adp_storage::{InstallationProfile, InstallationRecord},
    distribution::{
        PackageFormat, PlatformTarget, ProfileId, RelativePath, Sha256Digest, WorkingDir,
    },
    package_extraction::VerifiedPackage,
    storage::Database,
};
use nostr::{EventBuilder, Keys, Kind, Tag};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::{fs, sync::OnceLock, time::Instant};
use tempfile::TempDir;

fn fixture_binary() -> &'static [u8] {
    static BINARY: OnceLock<Vec<u8>> = OnceLock::new();
    BINARY.get_or_init(|| {
        let temp = TempDir::new().expect("compiler directory");
        let output = temp.path().join("fixture.exe");
        let source =
            Path::new(env!("CARGO_MANIFEST_DIR")).join("../tests/fixtures/launch/fixture.rs");
        let result = Command::new("rustc")
            .args(["--edition=2021", "--crate-name", "arcadestr_launch_fixture"])
            .arg(source)
            .arg("-o")
            .arg(&output)
            .output()
            .expect("rustc fixture");
        assert!(
            result.status.success(),
            "{}",
            String::from_utf8_lossy(&result.stderr)
        );
        fs::read(output).expect("native fixture")
    })
}

// Minimal stored ZIP writer for a real A4 extraction, without adding a test
// dependency or requiring a platform-specific archiver executable.
fn stored_zip(files: &[(&str, &[u8])]) -> Vec<u8> {
    fn u16le(out: &mut Vec<u8>, value: u16) {
        out.extend(value.to_le_bytes());
    }
    fn u32le(out: &mut Vec<u8>, value: u32) {
        out.extend(value.to_le_bytes());
    }
    fn crc32(bytes: &[u8]) -> u32 {
        let mut crc = !0_u32;
        for byte in bytes {
            crc ^= u32::from(*byte);
            for _ in 0..8 {
                crc = (crc >> 1) ^ (0xedb88320 & 0_u32.wrapping_sub(crc & 1));
            }
        }
        !crc
    }
    let mut out = Vec::new();
    let mut central = Vec::new();
    for (name, bytes) in files {
        let offset = out.len() as u32;
        let crc = crc32(bytes);
        u32le(&mut out, 0x04034b50);
        for value in [20, 0, 0, 0, 0] {
            u16le(&mut out, value);
        }
        for value in [crc, bytes.len() as u32, bytes.len() as u32] {
            u32le(&mut out, value);
        }
        u16le(&mut out, name.len() as u16);
        u16le(&mut out, 0);
        out.extend(name.as_bytes());
        out.extend(*bytes);
        u32le(&mut central, 0x02014b50);
        for value in [20, 20, 0, 0, 0, 0] {
            u16le(&mut central, value);
        }
        for value in [crc, bytes.len() as u32, bytes.len() as u32] {
            u32le(&mut central, value);
        }
        for value in [name.len() as u16, 0, 0, 0, 0] {
            u16le(&mut central, value);
        }
        u32le(&mut central, 0);
        u32le(&mut central, offset);
        central.extend(name.as_bytes());
    }
    let offset = out.len() as u32;
    let size = central.len() as u32;
    out.extend(central);
    u32le(&mut out, 0x06054b50);
    for value in [0, 0, files.len() as u16, files.len() as u16] {
        u16le(&mut out, value);
    }
    u32le(&mut out, size);
    u32le(&mut out, offset);
    u16le(&mut out, 0);
    out
}

// Fork inherits unrelated flock descriptors until exec. Keep these process
// fixtures isolated from one another so a parallel child cannot transiently
// retain another fixture's A4 operation lock during its install/launch handoff.
static PROCESS_FIXTURE: tokio::sync::Semaphore = tokio::sync::Semaphore::const_new(1);

struct Fixture {
    _permit: tokio::sync::SemaphorePermit<'static>,
    temp: TempDir,
    database: Database,
    repository: InstallationsRepository,
    id: InstallationId,
    root: PathBuf,
    profile: LaunchProfile,
}

impl Fixture {
    async fn install(working_dir: &str) -> Self {
        let permit = PROCESS_FIXTURE
            .acquire()
            .await
            .expect("process fixture permit");
        let temp = TempDir::new().expect("fixture directory");
        let database = Database::new(&temp.path().join("state.db"))
            .await
            .expect("database");
        let repository = InstallationsRepository::new(database.pool().clone());
        let profile = LaunchProfile {
            id: ProfileId::parse("game").expect("id"),
            display_name: "Native fixture".into(),
            executable: RelativePath::parse("bin/fixture.exe").expect("path"),
            working_dir: WorkingDir::parse(working_dir).expect("cwd"),
            arguments: vec![
                temp.path().join("report.json").display().to_string(),
                temp.path().join("release").display().to_string(),
                "7".into(),
                "".into(),
                "two words".into(),
                "\"quoted\"".into(),
                "$(touch must-not-exist); && & | > * $HOME `touch also-must-not-exist` %command%"
                    .into(),
                "trailing\\".into(),
                "héllo 🎮".into(),
                "line\nbreak".into(),
            ],
            launch_type: LaunchType::Game,
        };
        let zip = stored_zip(&[
            ("bin/fixture.exe", fixture_binary()),
            ("data/asset", b"asset"),
        ]);
        let digest = Sha256Digest::parse(&format!("{:x}", Sha256::digest(&zip))).expect("digest");
        let archive = temp.path().join("fixture.zip");
        fs::write(&archive, zip).expect("archive");
        let package = VerifiedPackage::verify(
            Arc::new(File::open(archive).expect("open package")),
            digest.clone(),
            PackageFormat::Zip,
        )
        .expect("verified package");
        let keys = Keys::generate();
        let platform = format!("{}-{}", std::env::consts::OS, std::env::consts::ARCH);
        let tags = vec![
            Tag::parse(["d", "native-fixture"]).expect("d"),
            Tag::parse(["file_hash", digest.as_str()]).expect("digest"),
            Tag::parse(["server", "http://localhost:9099"]).expect("server"),
            Tag::parse(["package", "zip"]).expect("package"),
            Tag::parse(["platform", platform.as_str()]).expect("platform"),
            Tag::parse(vec![
                "launch".to_string(),
                "game".into(),
                profile.executable.as_str().into(),
                working_dir.into(),
                serde_json::to_string(&profile.arguments).expect("arguments"),
                "game".into(),
                profile.display_name.clone(),
            ])
            .expect("launch"),
        ];
        let event = EventBuilder::new(Kind::Custom(30402), "fixture")
            .tags(tags)
            .sign_with_keys(&keys)
            .expect("signed event");
        let id = InstallationId::generate();
        let root = temp.path().join("installations").join("i-fixture");
        let record = InstallationRecord {
            id: id.clone(),
            game_coordinate: format!("30402:{}:native-fixture", keys.public_key().to_hex()),
            installation_root: root.clone(),
            package_digest: digest,
            package_format: PackageFormat::Zip,
            resolved_target: PlatformTarget {
                os: std::env::consts::OS.into(),
                arch: std::env::consts::ARCH.into(),
            },
            raw_event_json: serde_json::to_string(&event).expect("event JSON"),
            publisher_default_profile_id: None,
            state: InstallationState::Installing,
            recovery_discriminator: None,
            retained_package_coordinate: None,
            created_at: 1,
            updated_at: 1,
        };
        repository
            .begin(
                &record,
                &[InstallationProfile {
                    profile: profile.clone(),
                    availability: ProfileAvailability::Unavailable(
                        ProfileUnavailableReason::ExecutableMissing,
                    ),
                }],
            )
            .await
            .expect("begin install");
        let lock = OperationLock::acquire(&root).expect("install lock");
        let placed = crate::install::extract_and_place_verified_package(
            &package,
            &root,
            std::slice::from_ref(&profile),
            lock,
        )
        .expect("extract native package");
        crate::install::commit_placed_installation(&repository, &id, placed, 2)
            .await
            .expect("commit installation");
        Self {
            _permit: permit,
            temp,
            database,
            repository,
            id,
            root,
            profile,
        }
    }

    async fn launch(&self, launcher: Arc<Launcher>) -> Result<LaunchSession, LaunchError> {
        launch_installed(
            &self.repository,
            &self.temp.path().join("installations"),
            launcher,
            self.id.clone(),
            None,
        )
        .await
    }

    async fn report(&self) -> Value {
        let path = self.temp.path().join("report.json");
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            if let Ok(bytes) = fs::read(&path) {
                return serde_json::from_slice(&bytes).expect("fixture report");
            }
            assert!(Instant::now() < deadline, "child report timeout");
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    }

    fn release(&self) {
        fs::write(self.temp.path().join("release"), "exit").expect("release game");
    }
}

#[cfg(unix)]
fn replace_with_script(fixture: &Fixture, shebang: &str) -> PathBuf {
    use std::os::unix::fs::PermissionsExt;

    let executable = fixture.root.join("bin/fixture.exe");
    let helper = fixture.root.join("bin/fixture-native");
    fs::write(&helper, fixture_binary()).expect("native script helper");
    fs::set_permissions(&helper, fs::Permissions::from_mode(0o700)).expect("helper permissions");
    let script = format!(
        "{shebang}\nprintf '%s' \"$0\" > \"$1.script-argv0\"\nHERE=\"$(cd \"$(dirname \"$0\")\" && pwd)\"\nexec \"$HERE/fixture-native\" \"$@\"\n"
    );
    fs::write(&executable, script).expect("launcher script");
    fs::set_permissions(&executable, fs::Permissions::from_mode(0o700))
        .expect("script permissions");
    executable
}

fn expected_environment() -> serde_json::Map<String, Value> {
    build_launch_environment()
        .into_iter()
        .map(|(name, value)| {
            (
                name.to_string_lossy().into_owned(),
                Value::String(value.to_string_lossy().into_owned()),
            )
        })
        .collect()
}

async fn wait_exit(launcher: &Launcher, id: &InstallationId) -> LaunchSession {
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        let record = launcher
            .latest(id.as_str())
            .expect("status")
            .expect("record");
        if !record.is_running() {
            return record;
        }
        assert!(Instant::now() < deadline, "exit timeout");
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}

#[tokio::test]
async fn installed_native_child_reports_exact_argv_cwd_environment_parent_and_exit() {
    for cwd in [".", "data"] {
        let fixture = Fixture::install(cwd).await;
        let launcher = Arc::new(Launcher::new().expect("supervisor"));
        let start = fixture.launch(launcher.clone()).await.expect("launch");
        let report = fixture.report().await;
        assert_eq!(report["pid"], start.pid);
        #[cfg(target_os = "linux")]
        {
            assert_eq!(
                report["parent_pid"],
                std::process::id(),
                "no interpreter or shell parent"
            );
            assert_eq!(report["no_new_privs"], 1);
            assert_eq!(
                report["stdio"],
                serde_json::json!(["/dev/null", "/dev/null", "/dev/null"])
            );
        }
        assert_eq!(
            report["argv"][0],
            fixture
                .root
                .join("bin/fixture.exe")
                .canonicalize()
                .expect("canonical executable")
                .display()
                .to_string()
        );
        assert_eq!(
            report["argv"].as_array().expect("argv")[1..],
            serde_json::to_value(&fixture.profile.arguments)
                .expect("argv JSON")
                .as_array()
                .expect("argv")[..]
        );
        assert_eq!(
            report["cwd"],
            fixture
                .root
                .join(cwd)
                .canonicalize()
                .expect("cwd")
                .display()
                .to_string()
        );
        let environment = report["environment"].as_object().expect("environment");
        assert_eq!(environment, &expected_environment());
        assert!(environment
            .keys()
            .all(
                |name| CHILD_ENVIRONMENT.iter().any(|allowed| if cfg!(windows) {
                    name.eq_ignore_ascii_case(allowed)
                } else {
                    name == allowed
                })
            ));
        for forbidden in [
            "NSEC",
            "ARCADESTR_NSEC",
            "NWC_URI",
            "ADP_TOKEN",
            "SSH_AUTH_SOCK",
            "GPG_AGENT_INFO",
            "LD_PRELOAD",
            "DYLD_INSERT_LIBRARIES",
            "HTTP_PROXY",
            "DBUS_SESSION_BUS_ADDRESS",
        ] {
            assert!(!environment.contains_key(forbidden), "leaked {forbidden}");
        }
        assert_eq!(
            fixture.launch(launcher.clone()).await,
            Err(LaunchError::AlreadyRunning)
        );
        fixture.release();
        let exit = wait_exit(&launcher, &fixture.id).await;
        assert_eq!(
            exit.outcome,
            Some(LaunchOutcome::Exited {
                code: Some(7),
                signal: None
            })
        );
        assert!(exit.ended_at.is_some());
        assert_eq!(exit.session_id, start.session_id);
        assert_eq!(
            fixture
                .repository
                .get_by_id(&fixture.id)
                .await
                .expect("snapshot")
                .expect("installation")
                .installation
                .state,
            InstallationState::Ready
        );
    }
}

#[tokio::test]
async fn missing_root_executable_cwd_and_non_native_files_have_specific_errors() {
    let fixture = Fixture::install("data").await;
    let launcher = Arc::new(Launcher::new().expect("supervisor"));
    let executable = fixture.root.join("bin/fixture.exe");
    fs::rename(&executable, fixture.root.join("saved.exe")).expect("move executable");
    assert!(matches!(
        fixture.launch(launcher.clone()).await,
        Err(LaunchError::ExecutableMissing(_))
    ));
    fs::create_dir(&executable).expect("directory in executable slot");
    assert!(matches!(
        fixture.launch(launcher.clone()).await,
        Err(LaunchError::ExecutableNotFile(_))
    ));
    fs::remove_dir(&executable).expect("remove directory");
    fs::rename(fixture.root.join("saved.exe"), &executable).expect("restore executable");
    fs::remove_dir_all(fixture.root.join("data")).expect("remove cwd");
    assert!(matches!(
        fixture.launch(launcher.clone()).await,
        Err(LaunchError::WorkingDirectoryMissing(_))
    ));
    fs::write(fixture.root.join("data"), b"file").expect("file as cwd");
    assert!(matches!(
        fixture.launch(launcher.clone()).await,
        Err(LaunchError::WorkingDirectoryNotDirectory(_))
    ));
    fs::remove_file(fixture.root.join("data")).expect("remove file");
    fs::create_dir(fixture.root.join("data")).expect("restore cwd");
    fs::write(&executable, b"executable plain text\n").expect("plain text");
    assert!(matches!(
        fixture.launch(launcher.clone()).await,
        Err(LaunchError::UnsupportedExecutable(_))
    ));
    fs::remove_dir_all(&fixture.root).expect("remove installation");
    assert!(matches!(
        fixture.launch(launcher.clone()).await,
        Err(LaunchError::RootMissing(_))
    ));
    assert!(launcher
        .latest(fixture.id.as_str())
        .expect("status")
        .is_none());
    assert_eq!(
        fixture
            .repository
            .get_by_id(&fixture.id)
            .await
            .expect("snapshot")
            .expect("row")
            .installation
            .state,
        InstallationState::Ready
    );
}

#[cfg(unix)]
#[tokio::test]
async fn executable_shebang_scripts_preserve_launch_contract() {
    for shebang in ["#!/bin/sh", "#!/usr/bin/env sh"] {
        let fixture = Fixture::install("data").await;
        let executable = replace_with_script(&fixture, shebang);
        let launcher = Arc::new(Launcher::new().expect("supervisor"));
        let start = fixture
            .launch(launcher.clone())
            .await
            .expect("script launch");
        let report = fixture.report().await;

        assert_eq!(report["pid"], start.pid);
        assert_eq!(
            report["argv"].as_array().expect("argv")[1..],
            serde_json::to_value(&fixture.profile.arguments)
                .expect("argv JSON")
                .as_array()
                .expect("argv")[..]
        );
        assert_eq!(
            report["cwd"],
            fixture
                .root
                .join("data")
                .canonicalize()
                .expect("cwd")
                .display()
                .to_string()
        );
        let environment = report["environment"].as_object().expect("environment");
        for (name, value) in expected_environment() {
            assert_eq!(environment.get(&name), Some(&value), "environment {name}");
        }
        for forbidden in ["NSEC", "NWC_URI", "ADP_TOKEN", "LD_PRELOAD"] {
            assert!(!environment.contains_key(forbidden), "leaked {forbidden}");
        }
        assert!(report["environment"]["PATH"]
            .as_str()
            .is_some_and(|path| !path.is_empty()));
        assert_eq!(
            fs::read_to_string(format!("{}.script-argv0", fixture.profile.arguments[0]))
                .expect("script argv0"),
            executable
                .canonicalize()
                .expect("script path")
                .display()
                .to_string()
        );
        assert!(!fixture.root.join("data/must-not-exist").exists());
        assert!(!fixture.root.join("data/also-must-not-exist").exists());
        #[cfg(target_os = "linux")]
        {
            assert_eq!(report["no_new_privs"], 1);
            assert_eq!(
                report["stdio"],
                serde_json::json!(["/dev/null", "/dev/null", "/dev/null"])
            );
        }
        assert_eq!(
            fixture.launch(launcher.clone()).await,
            Err(LaunchError::AlreadyRunning)
        );
        fixture.release();
        assert_eq!(
            wait_exit(&launcher, &fixture.id).await.outcome,
            Some(LaunchOutcome::Exited {
                code: Some(7),
                signal: None,
            })
        );
    }
}

#[cfg(unix)]
#[tokio::test]
async fn non_executable_shebang_script_is_rejected() {
    use std::os::unix::fs::PermissionsExt;

    let fixture = Fixture::install(".").await;
    let executable = fixture.root.join("bin/fixture.exe");
    fs::write(&executable, b"#!/bin/sh\nexit 0\n").expect("script");
    fs::set_permissions(&executable, fs::Permissions::from_mode(0o600))
        .expect("remove execute permission");
    let result = fixture
        .launch(Arc::new(Launcher::new().expect("supervisor")))
        .await;
    assert!(matches!(result, Err(LaunchError::ExecutablePermissions(_))));
}

#[cfg(unix)]
#[tokio::test]
async fn escaped_symlink_and_privileged_executable_are_refused_without_mutating_tree() {
    use std::os::unix::fs::{symlink, PermissionsExt};
    let fixture = Fixture::install(".").await;
    let launcher = Arc::new(Launcher::new().expect("supervisor"));
    let executable = fixture.root.join("bin/fixture.exe");
    fs::set_permissions(&executable, fs::Permissions::from_mode(0o4700)).expect("setuid fixture");
    assert!(matches!(
        fixture.launch(launcher.clone()).await,
        Err(LaunchError::ExecutablePermissions(_))
    ));
    fs::set_permissions(&executable, fs::Permissions::from_mode(0o700)).expect("permissions");
    let outside = fixture.temp.path().join("outside.exe");
    fs::rename(&executable, &outside).expect("move outside");
    symlink(&outside, &executable).expect("escaped symlink");
    assert!(matches!(
        fixture.launch(launcher).await,
        Err(LaunchError::OutsideRoot(_))
    ));
    assert!(outside.is_file());
}

#[tokio::test]
async fn pinned_profiles_selection_and_specific_unavailability_do_not_use_current_listing() {
    fn preference(profile_id: &str) -> LaunchProfilePreference {
        LaunchProfilePreference {
            profile_id: Some(profile_id.into()),
            updated_at: 42,
        }
    }

    let fixture = Fixture::install(".").await;
    let mut snapshot = fixture
        .repository
        .get_by_id(&fixture.id)
        .await
        .expect("snapshot")
        .expect("row");
    let mut tool = snapshot.profiles[0].clone();
    tool.profile.id = ProfileId::parse("config").expect("id");
    tool.profile.launch_type = LaunchType::Config;
    snapshot.profiles.push(tool);
    let initial = configuration(&snapshot, None);
    assert_eq!(initial.effective_profile_id.as_deref(), Some("game"));
    assert_eq!(
        initial.resolution,
        LaunchResolutionStatus::Selected(LaunchProfileSource::PublisherOrder)
    );
    assert_eq!(
        select_profile(&snapshot, Some("config"))
            .expect("explicit tool")
            .launch_type,
        LaunchType::Config
    );
    let mut second = snapshot.profiles[0].clone();
    second.profile.id = ProfileId::parse("vulkan").expect("id");
    snapshot.profiles.push(second);
    assert_eq!(
        configuration(&snapshot, None)
            .effective_profile_id
            .as_deref(),
        Some("game")
    );
    snapshot.installation.publisher_default_profile_id =
        Some(ProfileId::parse("vulkan").expect("publisher default"));
    let publisher_default = configuration(&snapshot, None);
    assert_eq!(
        publisher_default.effective_profile_id.as_deref(),
        Some("vulkan")
    );
    assert_eq!(
        publisher_default.resolution,
        LaunchResolutionStatus::Selected(LaunchProfileSource::PublisherDefault)
    );
    let saved = configuration(&snapshot, Some(preference("game")));
    assert_eq!(saved.effective_profile_id.as_deref(), Some("game"));
    assert_eq!(
        saved.resolution,
        LaunchResolutionStatus::Selected(LaunchProfileSource::SavedPreference)
    );
    let saved_tool = configuration(&snapshot, Some(preference("config")));
    assert_eq!(
        saved_tool.saved_preference_status,
        ProfileReferenceStatus::NonGame
    );
    assert_eq!(saved_tool.effective_profile_id.as_deref(), Some("vulkan"));
    assert_eq!(
        saved_tool.fallback_reasons,
        vec![LaunchFallbackReason::SavedPreferenceNonGame]
    );
    let saved_missing = configuration(&snapshot, Some(preference("missing")));
    assert_eq!(
        saved_missing.saved_preference_profile_id.as_deref(),
        Some("missing")
    );
    assert_eq!(
        saved_missing.saved_preference_status,
        ProfileReferenceStatus::Missing
    );
    assert_eq!(
        saved_missing.effective_profile_id.as_deref(),
        Some("vulkan")
    );
    let saved_invalid = configuration(&snapshot, Some(preference("Bad")));
    assert_eq!(
        saved_invalid.saved_preference_status,
        ProfileReferenceStatus::Invalid
    );
    assert_eq!(
        saved_invalid.effective_profile_id.as_deref(),
        Some("vulkan")
    );
    assert_eq!(
        saved_invalid.fallback_reasons,
        vec![LaunchFallbackReason::SavedPreferenceInvalid]
    );
    snapshot.installation.publisher_default_profile_id =
        Some(ProfileId::parse("config").expect("non-game default"));
    let non_game_default = configuration(&snapshot, None);
    assert_eq!(
        non_game_default.publisher_default_status,
        ProfileReferenceStatus::NonGame
    );
    assert_eq!(
        non_game_default.effective_profile_id.as_deref(),
        Some("game")
    );
    assert_eq!(
        non_game_default.fallback_reasons,
        vec![LaunchFallbackReason::PublisherDefaultNonGame]
    );
    snapshot.installation.publisher_default_profile_id =
        Some(ProfileId::parse("missing").expect("missing default"));
    let missing_default = configuration(&snapshot, None);
    assert_eq!(
        missing_default.publisher_default_status,
        ProfileReferenceStatus::Missing
    );
    assert_eq!(
        missing_default.effective_profile_id.as_deref(),
        Some("game")
    );
    snapshot.installation.publisher_default_profile_id =
        Some(ProfileId::parse("vulkan").expect("publisher default"));
    snapshot.profiles[0].availability =
        ProfileAvailability::Unavailable(ProfileUnavailableReason::ExecutableMissing);
    assert!(
        matches!(select_profile(&snapshot, Some("game")), Err(LaunchError::ProfileUnavailable(reason)) if reason.contains("Executable is missing"))
    );
    assert!(options(&snapshot)[0]
        .unavailable_reason
        .as_ref()
        .expect("reason")
        .contains("Executable is missing"));
    snapshot.profiles[2].availability =
        ProfileAvailability::Unavailable(ProfileUnavailableReason::WorkingDirectoryMissing);
    let unavailable_saved = configuration(&snapshot, Some(preference("game")));
    assert_eq!(
        unavailable_saved.saved_preference_status,
        ProfileReferenceStatus::Unavailable
    );
    assert!(unavailable_saved
        .fallback_reasons
        .contains(&LaunchFallbackReason::SavedPreferenceUnavailable));
    assert_eq!(
        configuration(&snapshot, None).resolution,
        LaunchResolutionStatus::NoGameProfile
    );
    assert!(options(&snapshot)
        .iter()
        .all(|option| !option.is_play_candidate()));

    // A pinned profile may differ from its raw historical event; A4's read
    // contract explicitly permits that. Prove launch consumes the stored row.
    let mut pinned = fixture.profile.clone();
    pinned.arguments[3] = "pinned, not re-derived".into();
    sqlx::query("UPDATE installation_profiles SET profile_json = ? WHERE installation_id = ?")
        .bind(serde_json::to_string(&pinned).expect("profile"))
        .bind(fixture.id.as_str())
        .execute(fixture.database.pool())
        .await
        .expect("pin profile fixture");
    let launcher = Arc::new(Launcher::new().expect("supervisor"));
    fixture
        .launch(launcher.clone())
        .await
        .expect("launch pinned");
    assert_eq!(fixture.report().await["argv"][4], "pinned, not re-derived");
    fixture.release();
    wait_exit(&launcher, &fixture.id).await;
}

#[tokio::test]
async fn detaching_releases_supervision_and_a_new_client_can_launch_a_second_copy() {
    let fixture = Fixture::install(".").await;
    let launcher = Arc::new(Launcher::new().expect("supervisor"));
    let first = fixture
        .launch(launcher.clone())
        .await
        .expect("first launch");
    fixture.report().await;
    launcher.detach_all();
    let detached = launcher
        .latest(fixture.id.as_str())
        .expect("status")
        .expect("record");
    assert_eq!(detached.outcome, Some(LaunchOutcome::Detached));
    assert_eq!(detached.ended_at, None);
    let restarted = Arc::new(Launcher::new().expect("new client supervisor"));
    assert!(restarted
        .latest(fixture.id.as_str())
        .expect("new state")
        .is_none());
    let second = fixture
        .launch(restarted.clone())
        .await
        .expect("session-only duplicate prevention");
    assert_ne!(first.pid, second.pid);
    #[cfg(target_os = "linux")]
    assert!(
        Path::new(&format!("/proc/{}", first.pid)).exists(),
        "detached child survived"
    );
    fixture.release();
    wait_exit(&restarted, &fixture.id).await;
}

#[cfg(unix)]
#[tokio::test]
async fn malformed_native_image_is_not_retried_through_a_shell() {
    let fixture = Fixture::install(".").await;
    fs::write(
        fixture.root.join("bin/fixture.exe"),
        b"\x7fELF\nprintf shell > must-not-exist\n",
    )
    .expect("malformed ELF fixture");
    let launcher = Arc::new(Launcher::new().expect("supervisor"));
    let result = fixture.launch(launcher).await;
    assert!(
        matches!(result, Err(LaunchError::Spawn(_))),
        "invalid image must fail spawn: {result:?}"
    );
    assert!(!fixture.root.join("must-not-exist").exists());
}

#[cfg(unix)]
#[tokio::test]
async fn malformed_shebang_is_not_retried_through_a_shell() {
    let fixture = Fixture::install(".").await;
    let marker = fixture.root.join("malformed-shebang-marker");
    fs::write(
        fixture.root.join("bin/fixture.exe"),
        format!("#!\ntouch {}\n", marker.display()),
    )
    .expect("malformed shebang fixture");
    let result = fixture
        .launch(Arc::new(Launcher::new().expect("supervisor")))
        .await;
    assert!(matches!(result, Err(LaunchError::Spawn(_))));
    assert!(!marker.exists());
}

#[cfg(unix)]
#[tokio::test]
async fn missing_interpreter_has_specific_error_without_shell_fallback() {
    let fixture = Fixture::install(".").await;
    let marker = fixture.root.join("missing-interpreter-marker");
    fs::write(
        fixture.root.join("bin/fixture.exe"),
        format!(
            "#!/arcadestr-test-interpreter-that-does-not-exist\ntouch {}\n",
            marker.display()
        ),
    )
    .expect("missing interpreter fixture");
    let result = fixture
        .launch(Arc::new(Launcher::new().expect("supervisor")))
        .await;
    match result {
        Err(LaunchError::Spawn(detail)) => {
            assert!(detail.contains(
                "The launch target exists, but a required interpreter or loader could not be found"
            ));
            assert!(detail.contains("os error 2"), "raw errno missing: {detail}");
        }
        other => panic!("missing interpreter returned {other:?}"),
    }
    assert!(!marker.exists());
}

#[cfg(target_os = "linux")]
#[tokio::test]
async fn truncated_interpreter_path_is_not_retried_through_a_shell() {
    let fixture = Fixture::install(".").await;
    let marker = fixture.root.join("truncated-shebang-marker");
    let interpreter = format!("/{}", "arcadestr-missing-interpreter".repeat(20));
    fs::write(
        fixture.root.join("bin/fixture.exe"),
        format!("#!{interpreter}\ntouch {}\n", marker.display()),
    )
    .expect("overlong interpreter fixture");
    let result = fixture
        .launch(Arc::new(Launcher::new().expect("supervisor")))
        .await;
    assert!(matches!(result, Err(LaunchError::Spawn(_))));
    assert!(!marker.exists());
}

#[cfg(all(target_os = "linux", target_env = "gnu"))]
#[test]
fn malformed_elf_fixture_detects_glibc_execvp_shell_fallback() {
    use std::{
        ffi::CString,
        os::unix::{ffi::OsStrExt, fs::PermissionsExt},
    };

    let temp = TempDir::new().expect("control fixture directory");
    let executable = temp.path().join("malformed-elf");
    let marker = temp.path().join("execvp-fallback-marker");
    fs::write(
        &executable,
        format!("\x7fELF\ntouch {}\n", marker.display()),
    )
    .expect("malformed ELF control fixture");
    fs::set_permissions(&executable, fs::Permissions::from_mode(0o700))
        .expect("control fixture permissions");
    let target = CString::new(executable.as_os_str().as_bytes()).expect("target CString");
    let argv = [target.as_ptr(), std::ptr::null()];

    // SAFETY: all child-side storage is prepared before fork, and the child
    // performs only execvp or _exit. This is a glibc control, not launch code.
    let pid = unsafe { libc::fork() };
    assert!(pid >= 0, "fork failed: {}", std::io::Error::last_os_error());
    if pid == 0 {
        unsafe {
            libc::execvp(target.as_ptr(), argv.as_ptr());
            libc::_exit(127);
        }
    }
    let mut status = 0;
    assert_eq!(unsafe { libc::waitpid(pid, &mut status, 0) }, pid);
    assert!(libc::WIFEXITED(status));
    if !marker.exists() {
        assert_ne!(
            libc::WEXITSTATUS(status),
            127,
            "execvp returned instead of invoking /bin/sh"
        );
        eprintln!(
            "skipping execvp marker assertion: this host's /bin/sh rejected the binary-looking fixture"
        );
    }
}

// Real client-process exit, rather than only dropping a Child in a live test.
#[tokio::test]
#[ignore = "subprocess helper invoked by client_exit_detaches_real_child"]
async fn detaching_client_helper() {
    let directory = PathBuf::from(std::env::var("A5_HELPER_DIRECTORY").expect("helper directory"));
    let id = InstallationId::parse(&std::env::var("A5_HELPER_ID").expect("helper id")).expect("id");
    let database = Database::new(&directory.join("state.db"))
        .await
        .expect("database");
    let repository = InstallationsRepository::new(database.pool().clone());
    let launcher = Arc::new(Launcher::new().expect("supervisor"));
    let record = launch_installed(
        &repository,
        &directory.join("installations"),
        launcher.clone(),
        id,
        None,
    )
    .await
    .expect("launch");
    fs::write(
        directory.join("start.json"),
        serde_json::to_vec(&record).expect("record JSON"),
    )
    .expect("start record");
    let deadline = Instant::now() + Duration::from_secs(10);
    while !directory.join("report.json").exists() {
        assert!(Instant::now() < deadline);
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    launcher.detach_all();
    std::process::exit(0);
}

#[tokio::test]
async fn client_exit_detaches_child_and_preserves_display_authority_without_secrets() {
    let fixture = Fixture::install(".").await;
    let mut client = Command::new(std::env::current_exe().expect("test executable"));
    client
        .args([
            "--exact",
            "launcher::tests::detaching_client_helper",
            "--ignored",
            "--nocapture",
        ])
        .env("A5_HELPER_DIRECTORY", fixture.temp.path())
        .env("A5_HELPER_ID", fixture.id.as_str())
        .env("XAUTHORITY", "/run/user/1000/arcadestr-test-xauthority");
    for name in [
        "NSEC",
        "ARCADESTR_NSEC",
        "NWC_URI",
        "ADP_TOKEN",
        "SSH_AUTH_SOCK",
        "GPG_AGENT_INFO",
        "HTTP_PROXY",
        "DBUS_SESSION_BUS_ADDRESS",
        "NEW_UNRECOGNIZED_SECRET",
    ] {
        client.env(name, "a5-synthetic-secret");
    }
    let mut child = client.spawn().expect("client process");
    let client_pid = child.id();
    assert!(child.wait().expect("client exit").success());
    let report = fixture.report().await;
    #[cfg(target_os = "linux")]
    {
        assert_eq!(report["parent_pid"], client_pid);
        let game_pid = report["pid"].as_u64().expect("pid");
        let status = fs::read_to_string(format!("/proc/{game_pid}/status"))
            .expect("game survives actual client exit");
        assert!(!status
            .lines()
            .any(|line| line.starts_with("State:") && line.contains("Z (zombie)")));
    }
    assert!(report["environment"]
        .as_object()
        .expect("environment")
        .values()
        .all(|value| value != "a5-synthetic-secret"));
    assert_eq!(
        report["environment"]["XAUTHORITY"],
        "/run/user/1000/arcadestr-test-xauthority"
    );
    fixture.release();
}
