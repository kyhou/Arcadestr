CREATE TABLE client_refund_attempts (
    account_pubkey TEXT NOT NULL,
    processor_url TEXT NOT NULL,
    quote_id TEXT NOT NULL,
    leg_id TEXT NOT NULL,
    attempt_id TEXT NOT NULL,
    payment_hash TEXT NOT NULL,
    amount_msat INTEGER NOT NULL CHECK(amount_msat>0),
    due_at INTEGER NOT NULL,
    encrypted_invoice BLOB NOT NULL,
    encrypted_proof BLOB,
    state TEXT NOT NULL,
    updated_at INTEGER NOT NULL,
    PRIMARY KEY(account_pubkey,processor_url,quote_id,leg_id),
    UNIQUE(account_pubkey,payment_hash)
);
CREATE TABLE client_settlement_progress (
    account_pubkey TEXT NOT NULL,
    processor_url TEXT NOT NULL,
    quote_id TEXT NOT NULL,
    attempt_id TEXT NOT NULL,
    state TEXT NOT NULL,
    message TEXT NOT NULL,
    updated_at INTEGER NOT NULL,
    PRIMARY KEY(account_pubkey,processor_url,quote_id,attempt_id)
);
-- NULL means a historical dispatch/lookup predates these audit counters.
-- New intents start at zero. Counts do not authorize any retry.
ALTER TABLE nwc_payment_journal ADD COLUMN dispatch_count INTEGER;
ALTER TABLE nwc_payment_journal ADD COLUMN lookup_count INTEGER;
