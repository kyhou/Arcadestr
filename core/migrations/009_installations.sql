-- Validated installation snapshots. This replaces the pre-release installed_games cache;
-- migration 9 was never shipped, so deliberately no legacy data migration exists.
-- Side-by-side update intent; ready snapshots remain active until atomic commit.
CREATE TABLE installation_updates (
    id TEXT PRIMARY KEY,
    game_coordinate TEXT NOT NULL UNIQUE,
    old_snapshot_json TEXT NOT NULL,
    old_retained_json TEXT,
    candidate_record_json TEXT NOT NULL,
    candidate_package_path TEXT NOT NULL,
    state TEXT NOT NULL CHECK(state IN ('preparing','committed'))
);

CREATE TABLE retained_packages (
    game_coordinate TEXT PRIMARY KEY,
    package_id TEXT NOT NULL UNIQUE,
    package_path TEXT NOT NULL COLLATE NOCASE UNIQUE,
    package_digest TEXT NOT NULL,
    package_format TEXT NOT NULL CHECK (package_format IN ('zip', 'tar_zstd', 'tar_gz')),
    raw_event_json TEXT NOT NULL,
    resolved_os TEXT NOT NULL,
    resolved_arch TEXT NOT NULL,
    profiles_json TEXT NOT NULL,
    size_bytes INTEGER NOT NULL CHECK (size_bytes >= 0),
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL
);

CREATE TABLE installations (
    id TEXT PRIMARY KEY,
    game_coordinate TEXT NOT NULL UNIQUE,
    installation_root TEXT NOT NULL COLLATE NOCASE UNIQUE,
    package_digest TEXT NOT NULL,
    package_format TEXT NOT NULL CHECK (package_format IN ('zip', 'tar_zstd', 'tar_gz')),
    resolved_os TEXT NOT NULL,
    resolved_arch TEXT NOT NULL,
    raw_event_json TEXT NOT NULL,
    state TEXT NOT NULL CHECK (state IN ('installing', 'ready', 'failed', 'uninstalling')),
    recovery_discriminator TEXT CHECK (recovery_discriminator IN (
        'final_root_absent',
        'final_root_present'
    )),
    retained_package_coordinate TEXT REFERENCES retained_packages(game_coordinate) ON DELETE SET NULL,
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL,
    CHECK (retained_package_coordinate IS NULL OR retained_package_coordinate = game_coordinate),
    CHECK (recovery_discriminator IS NULL OR state = 'failed')
);

CREATE INDEX idx_installations_coordinate ON installations(game_coordinate);

CREATE TABLE installation_profiles (
    installation_id TEXT NOT NULL REFERENCES installations(id) ON DELETE CASCADE,
    profile_order INTEGER NOT NULL CHECK (profile_order >= 0),
    profile_json TEXT NOT NULL,
    availability TEXT NOT NULL CHECK (availability IN ('available', 'unavailable')),
    unavailable_reason TEXT CHECK (unavailable_reason IN (
        'executable_missing',
        'executable_not_file',
        'working_directory_missing',
        'working_directory_not_directory',
        'outside_installation_root'
    )),
    CHECK (
        (availability = 'available' AND unavailable_reason IS NULL) OR
        (availability = 'unavailable' AND unavailable_reason IS NOT NULL)
    ),
    PRIMARY KEY (installation_id, profile_order)
);

CREATE TABLE installation_manifest_entries (
    installation_id TEXT NOT NULL REFERENCES installations(id) ON DELETE CASCADE,
    path TEXT NOT NULL,
    path_key TEXT NOT NULL,
    entry_type TEXT NOT NULL CHECK (entry_type IN ('file', 'directory')),
    file_digest TEXT,
    CHECK (
        (entry_type = 'file' AND file_digest IS NOT NULL) OR
        (entry_type = 'directory' AND file_digest IS NULL)
    ),
    PRIMARY KEY (installation_id, path_key)
);

-- The primary key is the A7 exact-path index and also supports an installation-local
-- ordered path walk because path uses NOCASE collation.
CREATE INDEX idx_installation_manifest_path_walk
    ON installation_manifest_entries(installation_id, path_key);
