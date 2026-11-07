-- Additive: retain all existing accounts, purchases and invoice attempts.
CREATE TABLE nwc_payment_journal (
    account_pubkey TEXT NOT NULL,
    payment_hash TEXT NOT NULL,
    wallet_identity TEXT NOT NULL,
    invoice_commitment TEXT NOT NULL,
    amount_msat INTEGER NOT NULL CHECK (amount_msat > 0),
    encrypted_invoice BLOB NOT NULL,
    encrypted_payment BLOB,
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL,
    PRIMARY KEY (account_pubkey, payment_hash)
);
ALTER TABLE settlement_invoice_attempts ADD COLUMN encrypted_proof BLOB;
