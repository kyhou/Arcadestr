-- ADP download token cache. Losing rows only forces receipt/NIP-98 fallback.
CREATE TABLE IF NOT EXISTS download_tokens (
    buyer_pubkey TEXT NOT NULL,
    game_coordinate TEXT NOT NULL,
    server_url TEXT NOT NULL,
    token TEXT NOT NULL,
    expires_at INTEGER NOT NULL,
    PRIMARY KEY (buyer_pubkey, game_coordinate, server_url)
);
