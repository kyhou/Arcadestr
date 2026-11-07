-- App-owned review evidence; never a wallet/server schema or permission change.
CREATE TABLE purchase_quote_verification (
    account_pubkey TEXT NOT NULL,
    quote_id TEXT NOT NULL,
    listing_event_json TEXT NOT NULL,
    terms_events_json TEXT NOT NULL,
    PRIMARY KEY(account_pubkey,quote_id),
    FOREIGN KEY(account_pubkey,quote_id) REFERENCES settlement_quotes(account_pubkey,quote_id)
);
