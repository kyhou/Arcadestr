-- Arcadestr-owned approvals and irreversible dispatch admissions. No wallet schema.
CREATE TABLE purchase_execution_approvals (
    account_pubkey TEXT NOT NULL,
    quote_id TEXT NOT NULL,
    snapshot TEXT NOT NULL,
    halted INTEGER NOT NULL DEFAULT 0 CHECK(halted IN (0,1)),
    PRIMARY KEY(account_pubkey,quote_id),
    FOREIGN KEY(account_pubkey,quote_id) REFERENCES settlement_quotes(account_pubkey,quote_id)
);
CREATE TABLE purchase_execution_admissions (
    account_pubkey TEXT NOT NULL,
    quote_id TEXT NOT NULL,
    leg_id TEXT NOT NULL,
    attempt_id TEXT NOT NULL,
    payment_hash TEXT NOT NULL,
    invoice_commitment TEXT NOT NULL,
    amount_msat INTEGER NOT NULL CHECK(amount_msat>0),
    fee_bound_msat INTEGER NOT NULL CHECK(fee_bound_msat>=0),
    state TEXT NOT NULL CHECK(state IN ('uncertain','verified')),
    fees_paid_msat INTEGER CHECK(fees_paid_msat>=0),
    PRIMARY KEY(account_pubkey,quote_id,leg_id),
    UNIQUE(account_pubkey,payment_hash),
    UNIQUE(account_pubkey,attempt_id),
    FOREIGN KEY(account_pubkey,quote_id) REFERENCES purchase_execution_approvals(account_pubkey,quote_id)
);
