ALTER TABLE installations ADD COLUMN publisher_default_profile_id TEXT;

ALTER TABLE retained_packages ADD COLUMN publisher_default_profile_id TEXT;

-- Device-local user intent is keyed by the stable game coordinate rather than
-- an installation identity so uninstall/reinstall does not erase the choice.
CREATE TABLE launch_profile_preferences (
    game_coordinate TEXT PRIMARY KEY,
    profile_id TEXT,
    updated_at INTEGER NOT NULL CHECK (updated_at >= 0 AND updated_at < 9223372036854775807)
);
