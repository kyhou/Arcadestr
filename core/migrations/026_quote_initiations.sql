-- Local idempotency identity must survive a lost quote response or restart.
-- Store public references and a digest only, never private refund destinations.
CREATE TABLE IF NOT EXISTS quote_initiations (
    account_pubkey TEXT NOT NULL,
    processor_url TEXT NOT NULL,
    game_coordinate TEXT NOT NULL,
    purchase_id TEXT NOT NULL,
    listing_event_id TEXT NOT NULL,
    request_commitment TEXT NOT NULL,
    created_at INTEGER NOT NULL DEFAULT (unixepoch()),
    PRIMARY KEY (account_pubkey, processor_url, game_coordinate),
    UNIQUE (account_pubkey, processor_url, purchase_id)
);
