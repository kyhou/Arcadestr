-- Account-scoped durable client state for experimental split settlement v1.
CREATE TABLE settlement_quotes (
    account_pubkey TEXT NOT NULL,
    quote_id TEXT NOT NULL,
    processor_url TEXT NOT NULL,
    quote_event_json TEXT NOT NULL,
    listing_event_id TEXT NOT NULL,
    state TEXT NOT NULL,
    failure_at INTEGER,
    receipt_event_json TEXT,
    updated_at INTEGER NOT NULL,
    PRIMARY KEY (account_pubkey, quote_id)
);

CREATE TABLE settlement_legs (
    account_pubkey TEXT NOT NULL,
    quote_id TEXT NOT NULL,
    leg_id TEXT NOT NULL,
    leg_order INTEGER NOT NULL,
    recipient_pubkey TEXT NOT NULL,
    amount_msat INTEGER NOT NULL CHECK (amount_msat > 0),
    state TEXT NOT NULL,
    PRIMARY KEY (account_pubkey, quote_id, leg_id),
    UNIQUE (account_pubkey, quote_id, leg_order),
    FOREIGN KEY (account_pubkey, quote_id) REFERENCES settlement_quotes(account_pubkey, quote_id) ON DELETE CASCADE
);

CREATE TABLE settlement_invoice_attempts (
    account_pubkey TEXT NOT NULL,
    quote_id TEXT NOT NULL,
    leg_id TEXT NOT NULL,
    attempt_id TEXT NOT NULL,
    encrypted_attempt BLOB NOT NULL,
    payment_hash TEXT NOT NULL,
    state TEXT NOT NULL,
    updated_at INTEGER NOT NULL,
    PRIMARY KEY (account_pubkey, attempt_id),
    UNIQUE (account_pubkey, payment_hash),
    FOREIGN KEY (account_pubkey, quote_id, leg_id) REFERENCES settlement_legs(account_pubkey, quote_id, leg_id) ON DELETE CASCADE
);

CREATE TABLE settlement_refund_destinations (
    account_pubkey TEXT NOT NULL,
    quote_id TEXT NOT NULL,
    revision INTEGER NOT NULL,
    commitment TEXT NOT NULL,
    encrypted_destination BLOB NOT NULL,
    authorized_at INTEGER NOT NULL,
    PRIMARY KEY (account_pubkey, quote_id, revision),
    FOREIGN KEY (account_pubkey, quote_id) REFERENCES settlement_quotes(account_pubkey, quote_id) ON DELETE CASCADE
);

CREATE TABLE settlement_recovery (
    account_pubkey TEXT NOT NULL,
    quote_id TEXT NOT NULL,
    leg_id TEXT NOT NULL,
    state TEXT NOT NULL CHECK (state IN ('refund_owed','initiated','verified','overdue','disputed')),
    principal_msat INTEGER NOT NULL,
    due_at INTEGER NOT NULL,
    evidence_event_json TEXT,
    encrypted_proof BLOB,
    PRIMARY KEY (account_pubkey, quote_id, leg_id),
    FOREIGN KEY (account_pubkey, quote_id, leg_id) REFERENCES settlement_legs(account_pubkey, quote_id, leg_id) ON DELETE CASCADE
);
