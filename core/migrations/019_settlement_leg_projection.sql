-- Repair only derived leg progress. Never create payment intents, receipts,
-- ownership, or promote quotes from unverified/pending state.
UPDATE settlement_legs SET state='settled'
WHERE EXISTS (
    SELECT 1 FROM settlement_invoice_attempts a
    WHERE a.account_pubkey=settlement_legs.account_pubkey
      AND a.quote_id=settlement_legs.quote_id
      AND a.leg_id=settlement_legs.leg_id AND a.state='settled'
) OR EXISTS (
    SELECT 1 FROM settlement_quotes q
    WHERE q.account_pubkey=settlement_legs.account_pubkey
      AND q.quote_id=settlement_legs.quote_id
      AND q.state='settled' AND q.receipt_event_json IS NOT NULL
);
