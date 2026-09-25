-- Account-scoped games explicitly saved to a user's library.
CREATE TABLE IF NOT EXISTS library_games (
    buyer_pubkey TEXT NOT NULL,
    game_coordinate TEXT NOT NULL,
    added_at INTEGER NOT NULL,
    PRIMARY KEY (buyer_pubkey, game_coordinate)
);

CREATE INDEX IF NOT EXISTS idx_library_games_buyer_added
    ON library_games (buyer_pubkey, added_at DESC);
