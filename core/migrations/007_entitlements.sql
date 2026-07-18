-- NIP-103/ADP Entitlement Grant history. One row per immutable grant event.
CREATE TABLE IF NOT EXISTS entitlement_events (
    event_id             TEXT PRIMARY KEY,
    grant_id             TEXT NOT NULL,
    buyer_pubkey         TEXT NOT NULL,
    game_coordinate      TEXT NOT NULL,
    campaign_root_id     TEXT NOT NULL,
    issuer_pubkey        TEXT NOT NULL,
    status               TEXT NOT NULL,
    predecessor_event_id TEXT,
    created_at           INTEGER NOT NULL,
    raw_event_json       TEXT NOT NULL,
    validated            INTEGER NOT NULL DEFAULT 0
);

CREATE INDEX IF NOT EXISTS idx_entitlements_buyer_game
    ON entitlement_events(buyer_pubkey, game_coordinate);
CREATE INDEX IF NOT EXISTS idx_entitlements_grant
    ON entitlement_events(grant_id);
CREATE INDEX IF NOT EXISTS idx_entitlements_predecessor
    ON entitlement_events(predecessor_event_id);
