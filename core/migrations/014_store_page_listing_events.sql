-- Signed listing events retained only for offline Store Page association validation.
CREATE TABLE IF NOT EXISTS store_page_listing_events (
    listing_coordinate TEXT PRIMARY KEY,
    event_id TEXT NOT NULL,
    publisher_pubkey TEXT NOT NULL,
    d_tag TEXT NOT NULL,
    created_at INTEGER NOT NULL,
    raw_event_json TEXT NOT NULL,
    updated_at INTEGER NOT NULL
);
