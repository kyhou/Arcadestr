-- NIP-58 achievement badge cache

CREATE TABLE IF NOT EXISTS badge_definitions (
    coordinate TEXT PRIMARY KEY,
    issuer_pubkey TEXT NOT NULL,
    badge_id TEXT NOT NULL,
    event_id TEXT NOT NULL,
    name TEXT,
    description TEXT,
    image_url TEXT,
    image_dimensions TEXT,
    thumb_url TEXT,
    thumb_dimensions TEXT,
    relay_url TEXT,
    created_at INTEGER NOT NULL,
    raw_event_json TEXT NOT NULL,
    UNIQUE(issuer_pubkey, badge_id)
);

CREATE INDEX IF NOT EXISTS idx_badge_definitions_issuer_badge
    ON badge_definitions(issuer_pubkey, badge_id);

CREATE TABLE IF NOT EXISTS badge_awards (
    event_id TEXT PRIMARY KEY,
    issuer_pubkey TEXT NOT NULL,
    recipient_pubkey TEXT NOT NULL,
    badge_coordinate TEXT NOT NULL REFERENCES badge_definitions(coordinate),
    relay_url TEXT,
    created_at INTEGER NOT NULL,
    raw_event_json TEXT NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_badge_awards_recipient_created
    ON badge_awards(recipient_pubkey, created_at DESC);

CREATE INDEX IF NOT EXISTS idx_badge_awards_coordinate
    ON badge_awards(badge_coordinate);

CREATE TABLE IF NOT EXISTS profile_badge_lists (
    profile_pubkey TEXT PRIMARY KEY,
    event_id TEXT NOT NULL,
    kind INTEGER NOT NULL,
    created_at INTEGER NOT NULL,
    raw_event_json TEXT NOT NULL,
    updated_at INTEGER NOT NULL
);

CREATE TABLE IF NOT EXISTS profile_badge_entries (
    profile_pubkey TEXT NOT NULL REFERENCES profile_badge_lists(profile_pubkey) ON DELETE CASCADE,
    badge_coordinate TEXT NOT NULL,
    award_event_id TEXT NOT NULL,
    relay_url TEXT,
    display_order INTEGER NOT NULL,
    PRIMARY KEY(profile_pubkey, display_order)
);

CREATE INDEX IF NOT EXISTS idx_profile_badge_entries_coordinate
    ON profile_badge_entries(profile_pubkey, badge_coordinate);

CREATE INDEX IF NOT EXISTS idx_profile_badge_entries_award
    ON profile_badge_entries(award_event_id);
