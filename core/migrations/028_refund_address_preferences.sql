-- Device-local, account-scoped refund Lightning address chosen in Settings.
-- It is never published; it only overrides the wallet/profile address that
-- the buyer would otherwise commit as a split purchase's refund destination.
CREATE TABLE refund_address_preferences (
    account_pubkey TEXT PRIMARY KEY NOT NULL CHECK (length(account_pubkey) = 64),
    lud16 TEXT NOT NULL CHECK (length(lud16) BETWEEN 3 AND 320),
    updated_at INTEGER NOT NULL CHECK (updated_at >= 0 AND updated_at < 9223372036854775807)
);
