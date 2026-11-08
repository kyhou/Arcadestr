-- Advisory discovery projection only. Never authorization or entitlement evidence.
ALTER TABLE marketplace_listings ADD COLUMN campaigns_json TEXT NOT NULL DEFAULT '[]';
