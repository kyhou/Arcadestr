-- Validated Store Page events and policy-versioned renderable content.
CREATE TABLE IF NOT EXISTS store_pages (
    store_page_coordinate TEXT PRIMARY KEY,
    event_id TEXT NOT NULL,
    publisher_pubkey TEXT NOT NULL,
    d_tag TEXT NOT NULL,
    created_at INTEGER NOT NULL,
    raw_event_json TEXT NOT NULL,
    schema_version INTEGER NOT NULL,
    sanitizer_policy_version INTEGER NOT NULL,
    sanitized_content_json TEXT NOT NULL,
    diagnostics_json TEXT NOT NULL,
    updated_at INTEGER NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_store_pages_publisher
    ON store_pages (publisher_pubkey);
