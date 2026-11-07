-- Buyer-scoped discovery evidence. Raw signed content remains encrypted.
CREATE TABLE receipt_sync_events (
    buyer_pubkey TEXT NOT NULL,
    event_id TEXT NOT NULL,
    kind INTEGER NOT NULL,
    raw_event TEXT NOT NULL,
    PRIMARY KEY (buyer_pubkey, event_id)
);
CREATE TABLE receipt_chain_verifications (
    buyer_pubkey TEXT NOT NULL,
    listing_coordinate TEXT NOT NULL,
    order_id TEXT NOT NULL,
    verification TEXT NOT NULL CHECK(verification IN ('verified','indeterminate','invalid')),
    reason TEXT NOT NULL,
    tip_event_id TEXT,
    root_event_id TEXT,
    acquired_at INTEGER NOT NULL DEFAULT 0,
    lsp_pubkey TEXT,
    PRIMARY KEY (buyer_pubkey, listing_coordinate, order_id)
);
CREATE TABLE receipt_sync_cursors (
    buyer_pubkey TEXT NOT NULL,
    relay_url TEXT NOT NULL,
    newest_seen INTEGER NOT NULL DEFAULT 0,
    backfill_until INTEGER,
    backfill_done INTEGER NOT NULL DEFAULT 0,
    last_backfill INTEGER NOT NULL DEFAULT 0,
    PRIMARY KEY (buyer_pubkey, relay_url)
);
CREATE TABLE receipt_sync_leg_lsps (
    buyer_pubkey TEXT NOT NULL,
    receipt_root_id TEXT NOT NULL,
    leg_id TEXT NOT NULL,
    lsp_pubkey TEXT NOT NULL,
    PRIMARY KEY (buyer_pubkey, receipt_root_id, leg_id)
);
