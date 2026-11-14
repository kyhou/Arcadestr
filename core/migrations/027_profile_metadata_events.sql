-- Keep signed metadata independently of its refresh deadline.
CREATE TABLE IF NOT EXISTS profile_metadata_events (
    npub TEXT PRIMARY KEY NOT NULL,
    event_id TEXT NOT NULL,
    event_created_at INTEGER NOT NULL,
    raw_event_json TEXT NOT NULL
);
