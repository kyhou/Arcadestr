-- ADP provisioning relationships for operator-delegated fulfillment keys.
CREATE TABLE IF NOT EXISTS adp_provisioning (
    id TEXT PRIMARY KEY,
    developer_npub TEXT NOT NULL,
    server_url TEXT NOT NULL,
    operator_pubkey TEXT NOT NULL,
    scope TEXT,
    fulfillment_pubkey TEXT NOT NULL,
    attestation_event_id TEXT NOT NULL,
    acceptance_event_id TEXT NOT NULL,
    valid_from INTEGER NOT NULL,
    revoked_at INTEGER,
    created_at INTEGER NOT NULL
);

-- Use COALESCE so NULL scope participates in idempotency as the unscoped key.
CREATE UNIQUE INDEX IF NOT EXISTS idx_adp_provisioning_active
    ON adp_provisioning(developer_npub, server_url, COALESCE(scope, ''))
    WHERE revoked_at IS NULL;
