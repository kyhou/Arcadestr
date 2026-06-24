-- NIP-102 kind:1020 receipts. One row per receipt event in the order chain.
-- The chain for a given order_id must be traversed in created_at order to
-- determine authoritative status (latest wins).
CREATE TABLE IF NOT EXISTS purchases (
    event_id         TEXT PRIMARY KEY,
    order_id         TEXT NOT NULL,
    listing_coordinate TEXT NOT NULL,     -- "30402:<merchant_pubkey>:<d-tag>"
    buyer_pubkey     TEXT NOT NULL,
    merchant_pubkey  TEXT NOT NULL,
    payment_hash     TEXT,                -- hex, from bolt11 parsing or zap receipt
    status           TEXT NOT NULL DEFAULT 'paid',  -- 'paid' | 'refunded' | 'disputed' | 'fulfilled'
    created_at       INTEGER NOT NULL,
    raw_event        TEXT NOT NULL        -- full JSON for re-verification
);

CREATE INDEX IF NOT EXISTS idx_purchases_buyer       ON purchases(buyer_pubkey);
CREATE INDEX IF NOT EXISTS idx_purchases_order       ON purchases(order_id);
CREATE INDEX IF NOT EXISTS idx_purchases_coordinate  ON purchases(listing_coordinate);
