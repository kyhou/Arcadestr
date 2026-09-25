//! Local, pinned-snapshot execution. No signer, ownership, relay or HTTP capability.
use std::{
    collections::HashMap,
    ffi::OsString,
    fs::File,
    io::Read,
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    sync::{Arc, Mutex},
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use arcadestr_core::{
    adp_storage::{
        InstallationId, InstallationSnapshot, InstallationState, InstallationsRepository,
        LaunchProfilePreference, LaunchProfilePreferencesRepository, ProfileAvailability,
        ProfileUnavailableReason,
    },
    distribution::{LaunchProfile, LaunchType, ProfileId},
    launch_contract::{
        LaunchConfiguration, LaunchError, LaunchFallbackReason, LaunchOption, LaunchOutcome,
        LaunchProfileSource, LaunchResolutionStatus, LaunchSession, ProfileReferenceStatus,
    },
};
use tauri::Manager;

use crate::install::OperationLock;

// Exact names only, no inherited-environment filtering and no prefix wildcards.
// In particular: no LD_*, DYLD_*, proxy/auth tokens, SSH/GPG agents, D-Bus/signer
// addresses, Nostr keys, NWC connection strings or Arcadestr settings. Display
// session locators, including XAUTHORITY's credential-file path, are required by
// games but do not expose the credential contents in the environment.
const CHILD_ENVIRONMENT: &[&str] = &[
    "PATH",
    "HOME",
    "USER",
    "LOGNAME",
    "LANG",
    "LANGUAGE",
    "LC_ALL",
    "LC_CTYPE",
    "LC_NUMERIC",
    "LC_TIME",
    "LC_COLLATE",
    "LC_MONETARY",
    "LC_MESSAGES",
    "TZ",
    "TMPDIR",
    "TMP",
    "TEMP",
    "DISPLAY",
    "XAUTHORITY",
    "WAYLAND_DISPLAY",
    "XDG_RUNTIME_DIR",
    "XDG_CONFIG_HOME",
    "XDG_DATA_HOME",
    "XDG_CACHE_HOME",
    "XDG_DATA_DIRS",
    "SystemRoot",
    "WINDIR",
    "USERPROFILE",
    "LOCALAPPDATA",
    "APPDATA",
    "PROGRAMDATA",
];

#[cfg(unix)]
const DEFAULT_UNIX_PATH: &str = "/usr/local/bin:/usr/bin:/bin";

fn build_launch_environment() -> Vec<(OsString, OsString)> {
    let mut environment = CHILD_ENVIRONMENT
        .iter()
        .filter_map(|name| std::env::var_os(name).map(|value| (OsString::from(name), value)))
        .collect::<Vec<_>>();
    #[cfg(unix)]
    if !environment.iter().any(|(name, _)| name == "PATH") {
        environment.push((OsString::from("PATH"), OsString::from(DEFAULT_UNIX_PATH)));
    }
    environment
}

fn timestamp() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| i64::try_from(duration.as_secs()).unwrap_or(i64::MAX))
        .unwrap_or(0)
}

struct SupervisedChild {
    child: Child,
    record_index: usize,
}

#[derive(Default)]
struct Sessions {
    running: HashMap<String, SupervisedChild>,
    records: Vec<LaunchSession>,
    shutting_down: bool,
}

/// Process handles and records live only for this client session. Nothing is put
/// in SQLite. In particular a saved PID would not identify a process after a crash.
pub(crate) struct Launcher {
    sessions: Arc<Mutex<Sessions>>,
}

impl Launcher {
    pub(crate) fn new() -> Result<Self, std::io::Error> {
        let sessions = Arc::new(Mutex::new(Sessions::default()));
        let weak = Arc::downgrade(&sessions);
        std::thread::Builder::new()
            .name("game-supervisor".into())
            .spawn(move || loop {
                let Some(sessions) = weak.upgrade() else {
                    break;
                };
                let Ok(mut guard) = sessions.lock() else {
                    break;
                };
                if guard.shutting_down {
                    break;
                }
                reap_exited(&mut guard);
                drop(guard);
                drop(sessions);
                std::thread::sleep(Duration::from_millis(100));
            })?;
        Ok(Self { sessions })
    }

    pub(crate) fn latest(&self, id: &str) -> Result<Option<LaunchSession>, LaunchError> {
        let mut sessions = self.sessions.lock().map_err(|_| supervisor_error())?;
        reap_exited(&mut sessions);
        Ok(sessions
            .records
            .iter()
            .rev()
            .find(|record| record.installation_id == id)
            .cloned())
    }

    fn start(
        &self,
        id: &str,
        profile: &LaunchProfile,
        mut command: Command,
    ) -> Result<LaunchSession, LaunchError> {
        let mut sessions = self.sessions.lock().map_err(|_| supervisor_error())?;
        if sessions.shutting_down {
            return Err(LaunchError::ShuttingDown);
        }
        reap_exited(&mut sessions);
        if sessions.running.contains_key(id) {
            return Err(LaunchError::AlreadyRunning);
        }
        // This synchronous spawn is called on the blocking pool. The lock closes
        // the double-click race; it is never held across an await.
        let child = command.spawn().map_err(spawn_error)?;
        let record = LaunchSession {
            session_id: uuid::Uuid::new_v4().to_string(),
            installation_id: id.to_owned(),
            profile_id: profile.id.as_str().to_owned(),
            pid: child.id(),
            started_at: timestamp(),
            ended_at: None,
            outcome: None,
        };
        tracing::info!(installation_id = id, profile_id = %record.profile_id, pid = record.pid, "Game started");
        let record_index = sessions.records.len();
        sessions.records.push(record.clone());
        sessions.running.insert(
            id.to_owned(),
            SupervisedChild {
                child,
                record_index,
            },
        );
        Ok(record)
    }

    pub(crate) fn detach_all(&self) {
        // std::process::Child never kills on drop. No join/wait on shutdown, no
        // pipes back to a disappearing client, and no kill-on-close job object.
        if let Ok(mut sessions) = self.sessions.lock() {
            reap_exited(&mut sessions);
            sessions.shutting_down = true;
            let running = std::mem::take(&mut sessions.running);
            for (id, child) in running {
                let record = &mut sessions.records[child.record_index];
                record.outcome = Some(LaunchOutcome::Detached);
                // Detach is not an observed exit: ended_at stays None.
                tracing::info!(installation_id = %id, pid = record.pid, "Game detached on client shutdown");
                drop(child);
            }
        }
    }
}

fn spawn_error(error: std::io::Error) -> LaunchError {
    let errno = error.raw_os_error();
    tracing::error!(?errno, %error, "Could not spawn launch target");
    #[cfg(unix)]
    if errno == Some(libc::ENOENT) {
        return LaunchError::Spawn(format!(
            "The launch target exists, but a required interpreter or loader could not be found ({error})"
        ));
    }
    LaunchError::Spawn(error.to_string())
}

impl Drop for Launcher {
    fn drop(&mut self) {
        self.detach_all();
    }
}

fn supervisor_error() -> LaunchError {
    LaunchError::Operation("process supervisor is unavailable".into())
}

fn reap_exited(sessions: &mut Sessions) {
    sessions.running.retain(|id, child| {
        match child.child.try_wait() {
            Ok(Some(status)) => {
                #[cfg(unix)]
                let signal = {
                    use std::os::unix::process::ExitStatusExt;
                    status.signal()
                };
                #[cfg(not(unix))]
                let signal = None;
                let record = &mut sessions.records[child.record_index];
                record.ended_at = Some(timestamp());
                record.outcome = Some(LaunchOutcome::Exited { code: status.code(), signal });
                tracing::info!(installation_id = %id, pid = record.pid, exit_code = ?status.code(), signal, "Game exited");
                false
            }
            Ok(None) => true,
            Err(error) => {
                // Do not declare a still-possibly-running game stopped, or allow
                // another copy, merely because observation failed.
                tracing::warn!(installation_id = %id, %error, "Could not observe game exit");
                true
            }
        }
    });
}

fn ready_on_host(snapshot: &InstallationSnapshot) -> Result<(), LaunchError> {
    if snapshot.installation.state != InstallationState::Ready {
        return Err(LaunchError::NotReady);
    }
    let target = &snapshot.installation.resolved_target;
    if target.os != std::env::consts::OS || target.arch != std::env::consts::ARCH {
        return Err(LaunchError::UnsupportedPlatform);
    }
    Ok(())
}

fn unavailable_reason(reason: ProfileUnavailableReason) -> &'static str {
    match reason {
        ProfileUnavailableReason::ExecutableMissing => "Executable is missing",
        ProfileUnavailableReason::ExecutableNotFile => "Executable is not a regular file",
        ProfileUnavailableReason::WorkingDirectoryMissing => "Working directory is missing",
        ProfileUnavailableReason::WorkingDirectoryNotDirectory => {
            "Working directory is not a directory"
        }
        ProfileUnavailableReason::OutsideInstallationRoot => {
            "Launch path resolves outside the installation root"
        }
    }
}

fn options(snapshot: &InstallationSnapshot) -> Vec<LaunchOption> {
    snapshot
        .profiles
        .iter()
        .map(|installed| LaunchOption {
            profile_id: installed.profile.id.as_str().to_owned(),
            display_name: installed.profile.display_name.clone(),
            launch_type: installed.profile.launch_type.clone(),
            unavailable_reason: match installed.availability {
                ProfileAvailability::Available => None,
                ProfileAvailability::Unavailable(reason) => Some(format!(
                    "{} (executable: {}; working directory: {})",
                    unavailable_reason(reason),
                    installed.profile.executable.as_str(),
                    installed.profile.working_dir.as_str()
                )),
            },
        })
        .collect()
}

fn reference_status(
    snapshot: &InstallationSnapshot,
    profile_id: Option<&str>,
) -> ProfileReferenceStatus {
    let Some(profile_id) = profile_id else {
        return ProfileReferenceStatus::NotConfigured;
    };
    let Ok(profile_id) = ProfileId::parse(profile_id) else {
        return ProfileReferenceStatus::Invalid;
    };
    let Some(profile) = snapshot
        .profiles
        .iter()
        .find(|profile| profile.profile.id == profile_id)
    else {
        return ProfileReferenceStatus::Missing;
    };
    if profile.profile.launch_type != LaunchType::Game {
        ProfileReferenceStatus::NonGame
    } else if profile.availability != ProfileAvailability::Available {
        ProfileReferenceStatus::Unavailable
    } else {
        ProfileReferenceStatus::Available
    }
}

fn fallback_reason(status: ProfileReferenceStatus, saved: bool) -> Option<LaunchFallbackReason> {
    match (saved, status) {
        (_, ProfileReferenceStatus::NotConfigured | ProfileReferenceStatus::Available) => None,
        (true, ProfileReferenceStatus::Invalid) => {
            Some(LaunchFallbackReason::SavedPreferenceInvalid)
        }
        (true, ProfileReferenceStatus::Missing) => {
            Some(LaunchFallbackReason::SavedPreferenceMissing)
        }
        (true, ProfileReferenceStatus::NonGame) => {
            Some(LaunchFallbackReason::SavedPreferenceNonGame)
        }
        (true, ProfileReferenceStatus::Unavailable) => {
            Some(LaunchFallbackReason::SavedPreferenceUnavailable)
        }
        (false, ProfileReferenceStatus::Missing) => {
            Some(LaunchFallbackReason::PublisherDefaultMissing)
        }
        (false, ProfileReferenceStatus::Invalid) => {
            Some(LaunchFallbackReason::PublisherDefaultInvalid)
        }
        (false, ProfileReferenceStatus::NonGame) => {
            Some(LaunchFallbackReason::PublisherDefaultNonGame)
        }
        (false, ProfileReferenceStatus::Unavailable) => {
            Some(LaunchFallbackReason::PublisherDefaultUnavailable)
        }
    }
}

fn configuration(
    snapshot: &InstallationSnapshot,
    saved_preference: Option<LaunchProfilePreference>,
) -> LaunchConfiguration {
    let publisher_default = snapshot.installation.publisher_default_profile_id.as_ref();
    let saved_profile_id = saved_preference
        .as_ref()
        .and_then(|preference| preference.profile_id.as_ref());
    let saved_status = reference_status(snapshot, saved_profile_id.map(String::as_str));
    let publisher_status = reference_status(snapshot, publisher_default.map(ProfileId::as_str));
    let mut fallback_reasons = Vec::new();
    if let Some(reason) = fallback_reason(saved_status, true) {
        fallback_reasons.push(reason);
    }

    let selected = if saved_status == ProfileReferenceStatus::Available {
        saved_profile_id.map(|id| (id.clone(), LaunchProfileSource::SavedPreference))
    } else {
        if let Some(reason) = fallback_reason(publisher_status, false) {
            fallback_reasons.push(reason);
        }
        if publisher_status == ProfileReferenceStatus::Available {
            publisher_default.map(|id| {
                (
                    id.as_str().to_owned(),
                    LaunchProfileSource::PublisherDefault,
                )
            })
        } else {
            snapshot
                .profiles
                .iter()
                .find(|entry| {
                    entry.profile.launch_type == LaunchType::Game
                        && entry.availability == ProfileAvailability::Available
                })
                .map(|entry| {
                    (
                        entry.profile.id.as_str().to_owned(),
                        LaunchProfileSource::PublisherOrder,
                    )
                })
        }
    };
    let (effective_profile_id, resolution) = selected.map_or(
        (None, LaunchResolutionStatus::NoGameProfile),
        |(profile_id, source)| (Some(profile_id), LaunchResolutionStatus::Selected(source)),
    );

    LaunchConfiguration {
        options: options(snapshot),
        publisher_default_profile_id: publisher_default.map(|id| id.as_str().to_owned()),
        saved_preference_profile_id: saved_profile_id.cloned(),
        preference_updated_at: saved_preference.map(|preference| preference.updated_at),
        publisher_default_status: publisher_status,
        saved_preference_status: saved_status,
        effective_profile_id,
        resolution,
        fallback_reasons,
    }
}

fn select_profile<'a>(
    snapshot: &'a InstallationSnapshot,
    selected: Option<&str>,
) -> Result<&'a LaunchProfile, LaunchError> {
    if let Some(id) = selected {
        let entry = snapshot
            .profiles
            .iter()
            .find(|entry| entry.profile.id.as_str() == id)
            .ok_or_else(|| LaunchError::ProfileNotFound(id.to_owned()))?;
        if let ProfileAvailability::Unavailable(reason) = entry.availability {
            return Err(LaunchError::ProfileUnavailable(format!(
                "{}: {}",
                entry.profile.display_name,
                unavailable_reason(reason)
            )));
        }
        return Ok(&entry.profile);
    }
    Err(LaunchError::NoGameProfile)
}

fn resolve_root(root: &Path) -> Result<PathBuf, LaunchError> {
    let resolved = root.canonicalize().map_err(|error| {
        if error.kind() == std::io::ErrorKind::NotFound {
            LaunchError::RootMissing(root.display().to_string())
        } else {
            LaunchError::Operation(format!("{}: {error}", root.display()))
        }
    })?;
    if !resolved.is_dir() {
        return Err(LaunchError::RootNotDirectory(root.display().to_string()));
    }
    Ok(resolved)
}

fn resolved_launch_path(
    root: &Path,
    relative: &str,
    executable: bool,
) -> Result<PathBuf, LaunchError> {
    let path = root.join(relative);
    let text = path.display().to_string();
    let resolved = path.canonicalize().map_err(|error| {
        if error.kind() == std::io::ErrorKind::NotFound {
            if executable {
                LaunchError::ExecutableMissing(text.clone())
            } else {
                LaunchError::WorkingDirectoryMissing(text.clone())
            }
        } else {
            LaunchError::Operation(format!("{text}: {error}"))
        }
    })?;
    if !resolved.starts_with(root) {
        return Err(LaunchError::OutsideRoot(text));
    }
    let metadata = resolved
        .metadata()
        .map_err(|error| LaunchError::Operation(format!("{text}: {error}")))?;
    if executable && !metadata.is_file() {
        return Err(LaunchError::ExecutableNotFile(text));
    }
    if !executable && !metadata.is_dir() {
        return Err(LaunchError::WorkingDirectoryNotDirectory(text));
    }
    #[cfg(unix)]
    if executable {
        use std::os::unix::fs::PermissionsExt;
        let mode = metadata.permissions().mode();
        if mode & 0o111 == 0 || mode & 0o6000 != 0 {
            return Err(LaunchError::ExecutablePermissions(text));
        }
    }
    Ok(resolved)
}

fn require_supported_executable(executable: &Path) -> Result<(), LaunchError> {
    let mut file =
        File::open(executable).map_err(|error| LaunchError::Operation(error.to_string()))?;
    let mut magic = [0; 4];
    if file.read_exact(&mut magic).is_err() {
        return Err(LaunchError::UnsupportedExecutable(
            executable.display().to_string(),
        ));
    }
    #[cfg(unix)]
    let supported = magic == *b"\x7fELF" || magic[..2] == *b"#!";
    #[cfg(target_os = "windows")]
    let supported = {
        use std::io::{Seek, SeekFrom};
        // Never let Rust's batch-file handling introduce cmd.exe, even if a
        // publisher puts image-looking bytes into a .bat or .cmd file.
        let extension = executable.extension().and_then(|value| value.to_str());
        let mut offset = [0; 4];
        let mut signature = [0; 4];
        magic[..2] == *b"MZ"
            && !extension.is_some_and(|value| {
                value.eq_ignore_ascii_case("bat") || value.eq_ignore_ascii_case("cmd")
            })
            && file.seek(SeekFrom::Start(0x3c)).is_ok()
            && file.read_exact(&mut offset).is_ok()
            && file
                .seek(SeekFrom::Start(u64::from(u32::from_le_bytes(offset))))
                .is_ok()
            && file.read_exact(&mut signature).is_ok()
            && signature == *b"PE\0\0"
    };
    #[cfg(not(any(unix, target_os = "windows")))]
    let supported = false;
    if !supported {
        return Err(LaunchError::UnsupportedExecutable(
            executable.display().to_string(),
        ));
    }
    Ok(())
}

/// Immutable storage for a direct execve call. Pointer arrays are built only
/// after all C-string storage is final and remain valid when this owner moves.
#[cfg(unix)]
struct PreparedExecve {
    target: std::ffi::CString,
    _argv_storage: Vec<std::ffi::CString>,
    argv_ptrs: Vec<*const libc::c_char>,
    _env_storage: Vec<std::ffi::CString>,
    envp_ptrs: Vec<*const libc::c_char>,
}

// SAFETY: every pointer refers to an immutable allocation owned by the same
// object. Construction finalizes all storage before deriving the pointers, and
// PreparedExecve exposes no mutators.
#[cfg(unix)]
unsafe impl Send for PreparedExecve {}
#[cfg(unix)]
unsafe impl Sync for PreparedExecve {}

#[cfg(unix)]
impl PreparedExecve {
    fn new(
        target: &Path,
        arguments: &[String],
        environment: Vec<(OsString, OsString)>,
    ) -> Result<Self, LaunchError> {
        use std::os::unix::ffi::OsStrExt;

        let target = std::ffi::CString::new(target.as_os_str().as_bytes())
            .map_err(|_| LaunchError::Operation("NUL in executable path".into()))?;
        let argv_storage = std::iter::once(target.as_bytes().to_vec())
            .chain(arguments.iter().map(|value| value.as_bytes().to_vec()))
            .map(|value| {
                std::ffi::CString::new(value)
                    .map_err(|_| LaunchError::Operation("NUL in process arguments".into()))
            })
            .collect::<Result<Vec<_>, _>>()?;
        let env_storage = environment
            .into_iter()
            .map(|(key, value)| {
                let mut entry = key;
                entry.push("=");
                entry.push(value);
                std::ffi::CString::new(entry.as_bytes())
                    .map_err(|_| LaunchError::Operation("NUL in process environment".into()))
            })
            .collect::<Result<Vec<_>, _>>()?;
        let mut argv_ptrs = argv_storage
            .iter()
            .map(|value| value.as_ptr())
            .collect::<Vec<_>>();
        argv_ptrs.push(std::ptr::null());
        let mut envp_ptrs = env_storage
            .iter()
            .map(|value| value.as_ptr())
            .collect::<Vec<_>>();
        envp_ptrs.push(std::ptr::null());
        Ok(Self {
            target,
            _argv_storage: argv_storage,
            argv_ptrs,
            _env_storage: env_storage,
            envp_ptrs,
        })
    }

    unsafe fn execute(&self) -> libc::c_int {
        // SAFETY: construction guarantees null-terminated pointer arrays whose
        // entries remain owned by self for the duration of this call.
        unsafe {
            libc::execve(
                self.target.as_ptr(),
                self.argv_ptrs.as_ptr(),
                self.envp_ptrs.as_ptr(),
            )
        }
    }
}

/// Bypass execvp's ENOEXEC shell fallback with one pre_exec callback whose
/// successful path never returns. All callback inputs were allocated before fork.
#[cfg(unix)]
fn direct_unix_exec(command: &mut Command, prepared: PreparedExecve) {
    use std::os::unix::process::CommandExt;

    // SAFETY: all allocations and pointer arrays are prepared before fork. The
    // callback performs only prctl/execve and errno capture. Returning an error
    // prevents Command from ever reaching execvp, even for an invalid image.
    unsafe {
        command.pre_exec(move || {
            #[cfg(target_os = "linux")]
            {
                if libc::prctl(libc::PR_SET_NO_NEW_PRIVS, 1, 0, 0, 0) != 0 {
                    return Err(std::io::Error::last_os_error());
                }
            }
            prepared.execute();
            Err(std::io::Error::last_os_error())
        });
    }
}

fn command_for(root: &Path, profile: &LaunchProfile) -> Result<Command, LaunchError> {
    let root = resolve_root(root)?;
    let executable = resolved_launch_path(&root, profile.executable.as_str(), true)?;
    let working_dir = resolved_launch_path(&root, profile.working_dir.as_str(), false)?;
    require_supported_executable(&executable)?;
    // Validation and execution remain path-based. The same-UID trust model
    // accepts replacement of the target between this check and execve.
    let environment = build_launch_environment();
    #[cfg(unix)]
    let prepared = PreparedExecve::new(&executable, &profile.arguments, environment)?;
    let mut command = Command::new(&executable);
    // Windows transport uses Rust's standard encoding, never raw_arg or client
    // quoting. The target runtime must use the corresponding standard decoding.
    #[cfg(not(unix))]
    command
        .args(&profile.arguments)
        .env_clear()
        .envs(environment);
    command
        .current_dir(working_dir)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        // Survive the client's terminal process-group signals as well as normal
        // GUI shutdown. This is still a direct child, not a double-fork launcher.
        command.process_group(0);
    }
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const DETACHED_PROCESS: u32 = 0x00000008;
        const CREATE_NEW_PROCESS_GROUP: u32 = 0x00000200;
        command.creation_flags(DETACHED_PROCESS | CREATE_NEW_PROCESS_GROUP);
    }
    #[cfg(unix)]
    direct_unix_exec(&mut command, prepared);
    Ok(command)
}

async fn snapshot(
    repository: &InstallationsRepository,
    id: &InstallationId,
) -> Result<InstallationSnapshot, LaunchError> {
    let snapshot = repository
        .get_by_id(id)
        .await
        .map_err(|error| LaunchError::Operation(error.to_string()))?
        .ok_or(LaunchError::NotFound)?;
    ready_on_host(&snapshot)?;
    Ok(snapshot)
}

pub(crate) async fn launch_installed(
    repository: &InstallationsRepository,
    managed_installations: &Path,
    launcher: Arc<Launcher>,
    id: InstallationId,
    profile_id: Option<String>,
) -> Result<LaunchSession, LaunchError> {
    let initial = snapshot(repository, &id).await?;
    crate::install::require_managed_direct_child(
        managed_installations,
        &initial.installation.installation_root,
    )
    .map_err(LaunchError::Operation)?;
    let root = initial.installation.installation_root.clone();
    let operation_lock = tokio::task::spawn_blocking(move || {
        resolve_root(&root)?; // Missing root gets its specific diagnostic, not a lock error.
        OperationLock::acquire_existing_for_recovery(&root)
            .map_err(|error| LaunchError::Operation(error.to_string()))
    })
    .await
    .map_err(|error| LaunchError::Operation(error.to_string()))??;
    // Serialize against A4 until spawn completes; do not change uninstall's
    // lifetime behavior or hold its lock while the game runs.
    let current = snapshot(repository, &id).await?;
    if current.installation.installation_root != initial.installation.installation_root {
        return Err(LaunchError::Operation(
            "installation changed before launch".into(),
        ));
    }
    let selected_profile_id = match profile_id {
        Some(profile_id) => profile_id,
        None => {
            let preferences = LaunchProfilePreferencesRepository::new(repository.pool().clone());
            let saved = preferences
                .get(&current.installation.game_coordinate)
                .await
                .map_err(|error| LaunchError::Operation(error.to_string()))?;
            configuration(&current, saved)
                .effective_profile_id
                .ok_or(LaunchError::NoGameProfile)?
        }
    };
    let profile = select_profile(&current, Some(&selected_profile_id))?.clone();
    tokio::task::spawn_blocking(move || {
        let _operation_lock = operation_lock;
        let command = command_for(&current.installation.installation_root, &profile)?;
        launcher.start(id.as_str(), &profile, command)
    })
    .await
    .map_err(|error| LaunchError::Operation(error.to_string()))?
}

#[tauri::command]
pub(crate) async fn get_launch_options(
    installation_id: String,
    state: tauri::State<'_, crate::AppState>,
) -> Result<LaunchConfiguration, LaunchError> {
    let id = InstallationId::parse(&installation_id)
        .map_err(|error| LaunchError::Operation(error.to_string()))?;
    let repository = InstallationsRepository::new(state.database.pool().clone());
    let snapshot = snapshot(&repository, &id).await?;
    let preferences = LaunchProfilePreferencesRepository::new(state.database.pool().clone());
    let saved = preferences
        .get(&snapshot.installation.game_coordinate)
        .await
        .map_err(|error| LaunchError::Operation(error.to_string()))?;
    Ok(configuration(&snapshot, saved))
}

#[tauri::command]
pub(crate) async fn set_launch_profile_preference(
    installation_id: String,
    profile_id: Option<String>,
    preference_updated_at: i64,
    app: tauri::AppHandle,
    state: tauri::State<'_, crate::AppState>,
) -> Result<LaunchConfiguration, LaunchError> {
    let id = InstallationId::parse(&installation_id)
        .map_err(|error| LaunchError::Operation(error.to_string()))?;
    let repository = InstallationsRepository::new(state.database.pool().clone());
    let initial = snapshot(&repository, &id).await?;
    let managed = app
        .path()
        .app_local_data_dir()
        .map_err(|error| LaunchError::Operation(error.to_string()))?
        .join("installations");
    crate::install::require_managed_direct_child(&managed, &initial.installation.installation_root)
        .map_err(LaunchError::Operation)?;
    let root = initial.installation.installation_root.clone();
    let _operation_lock = tokio::task::spawn_blocking(move || {
        OperationLock::acquire_existing_for_recovery(&root)
            .map_err(|error| LaunchError::Operation(error.to_string()))
    })
    .await
    .map_err(|error| LaunchError::Operation(error.to_string()))??;
    let current = snapshot(&repository, &id).await?;
    if current.installation.installation_root != initial.installation.installation_root {
        return Err(LaunchError::Operation(
            "installation changed while setting launch preference".into(),
        ));
    }

    let preferences = LaunchProfilePreferencesRepository::new(state.database.pool().clone());
    match profile_id {
        Some(value) => {
            let profile_id = ProfileId::parse(&value)
                .map_err(|error| LaunchError::Operation(error.to_string()))?;
            let profile = current
                .profiles
                .iter()
                .find(|entry| entry.profile.id == profile_id)
                .ok_or_else(|| LaunchError::ProfileNotFound(value.clone()))?;
            if profile.profile.launch_type != LaunchType::Game {
                return Err(LaunchError::ProfileNotGame(value));
            }
            if let ProfileAvailability::Unavailable(reason) = profile.availability {
                return Err(LaunchError::ProfileUnavailable(format!(
                    "{}: {}",
                    profile.profile.display_name,
                    unavailable_reason(reason)
                )));
            }
            preferences
                .set(
                    &current.installation.game_coordinate,
                    &profile_id,
                    preference_updated_at,
                )
                .await
                .map_err(|error| LaunchError::Operation(error.to_string()))?;
        }
        None => {
            preferences
                .clear(&current.installation.game_coordinate, preference_updated_at)
                .await
                .map_err(|error| LaunchError::Operation(error.to_string()))?;
        }
    }
    let saved = preferences
        .get(&current.installation.game_coordinate)
        .await
        .map_err(|error| LaunchError::Operation(error.to_string()))?;
    Ok(configuration(&current, saved))
}

#[tauri::command]
pub(crate) async fn launch_game(
    installation_id: String,
    profile_id: Option<String>,
    app: tauri::AppHandle,
    state: tauri::State<'_, crate::AppState>,
    launcher: tauri::State<'_, Arc<Launcher>>,
) -> Result<LaunchSession, LaunchError> {
    let id = InstallationId::parse(&installation_id)
        .map_err(|error| LaunchError::Operation(error.to_string()))?;
    let repository = InstallationsRepository::new(state.database.pool().clone());
    let managed = app
        .path()
        .app_local_data_dir()
        .map_err(|error| LaunchError::Operation(error.to_string()))?
        .join("installations");
    launch_installed(
        &repository,
        &managed,
        launcher.inner().clone(),
        id,
        profile_id,
    )
    .await
}

#[tauri::command]
pub(crate) fn get_launch_status(
    installation_id: String,
    launcher: tauri::State<'_, Arc<Launcher>>,
) -> Result<Option<LaunchSession>, LaunchError> {
    launcher.latest(&installation_id)
}

#[cfg(test)]
mod tests;
