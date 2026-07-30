CREATE TABLE IF NOT EXISTS blossom_server_settings (
    publisher_pubkey TEXT NOT NULL,
    origin TEXT NOT NULL,
    label TEXT,
    enabled INTEGER NOT NULL CHECK (enabled IN (0, 1)),
    sort_order INTEGER NOT NULL CHECK (sort_order >= 0),
    is_preferred INTEGER NOT NULL CHECK (is_preferred IN (0, 1)),
    created_at INTEGER NOT NULL CHECK (created_at >= 0),
    updated_at INTEGER NOT NULL CHECK (updated_at >= 0),
    PRIMARY KEY (publisher_pubkey, origin)
);

CREATE INDEX IF NOT EXISTS idx_blossom_server_settings_order
    ON blossom_server_settings (publisher_pubkey, sort_order, origin);

CREATE UNIQUE INDEX IF NOT EXISTS idx_blossom_server_settings_one_preferred
    ON blossom_server_settings (publisher_pubkey)
    WHERE is_preferred = 1;
