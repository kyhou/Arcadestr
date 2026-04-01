-- Initial schema for secure account storage
-- Created: 2026-03-31

-- Accounts table - stores user accounts with encrypted nsec
CREATE TABLE IF NOT EXISTS accounts (
    id TEXT PRIMARY KEY,
    pubkey TEXT UNIQUE NOT NULL,
    npub TEXT UNIQUE NOT NULL,
    signing_mode TEXT NOT NULL CHECK (signing_mode IN ('Local', 'Remote', 'ReadOnly')),
    encrypted_nsec BLOB,
    display_name TEXT,
    picture TEXT,
    created_at INTEGER NOT NULL,
    last_used INTEGER NOT NULL,
    is_active BOOLEAN NOT NULL DEFAULT 0
);

-- Indexes for fast lookups
CREATE INDEX IF NOT EXISTS idx_accounts_pubkey ON accounts(pubkey);
CREATE INDEX IF NOT EXISTS idx_accounts_npub ON accounts(npub);
CREATE INDEX IF NOT EXISTS idx_accounts_active ON accounts(is_active) WHERE is_active = 1;
CREATE INDEX IF NOT EXISTS idx_accounts_last_used ON accounts(last_used DESC);

-- Relay backups table - stores NIP-78 encrypted backup metadata
CREATE TABLE IF NOT EXISTS relay_backups (
    id TEXT PRIMARY KEY,
    account_id TEXT NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
    relay_url TEXT NOT NULL,
    encrypted_data BLOB NOT NULL,
    created_at INTEGER NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_backups_account ON relay_backups(account_id);

-- Remote signer URIs - for NIP-46 accounts
CREATE TABLE IF NOT EXISTS remote_uris (
    account_id TEXT PRIMARY KEY REFERENCES accounts(id) ON DELETE CASCADE,
    uri TEXT NOT NULL,
    client_key TEXT NOT NULL,
    updated_at INTEGER NOT NULL
);

-- Secure storage metadata - singleton table
CREATE TABLE IF NOT EXISTS secure_storage (
    id INTEGER PRIMARY KEY CHECK (id = 1),
    salt BLOB NOT NULL,
    created_at INTEGER NOT NULL
);

-- Migration tracking
CREATE TABLE IF NOT EXISTS _sqlx_migrations (
    version BIGINT PRIMARY KEY,
    description TEXT NOT NULL,
    installed_on TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP,
    success BOOLEAN NOT NULL,
    checksum BLOB NOT NULL,
    execution_time BIGINT NOT NULL
);
