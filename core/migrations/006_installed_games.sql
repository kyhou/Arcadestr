-- ADP installed game records. Archive extraction/launch are future work.
CREATE TABLE IF NOT EXISTS installed_games (
    game_coordinate TEXT PRIMARY KEY,
    file_path TEXT NOT NULL,
    file_hash TEXT NOT NULL,
    version TEXT,
    server_url TEXT NOT NULL,
    installed_at INTEGER NOT NULL
);
