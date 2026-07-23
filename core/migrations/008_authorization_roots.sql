-- Preserve legacy provisioning rows for audit while preventing their listing-timestamp
-- metadata from authorizing new protocol activity.
ALTER TABLE adp_provisioning ADD COLUMN authorization_root_event_id TEXT;
ALTER TABLE adp_provisioning ADD COLUMN authorization_capabilities_json TEXT;
ALTER TABLE adp_provisioning ADD COLUMN authorization_profile_version INTEGER NOT NULL DEFAULT 1;
