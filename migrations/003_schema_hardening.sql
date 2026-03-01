-- migrations/003_schema_hardening.sql
-- Add NOT NULL to channels.name, self-share guard on fleet_shares, FK indexes.

-- channels.name should always be non-null
ALTER TABLE channels ALTER COLUMN name SET NOT NULL;

-- Prevent a fleet from sharing with itself
ALTER TABLE fleet_shares
    ADD CONSTRAINT fleet_shares_no_self_share
    CHECK (fleet_id <> shared_with_fleet_id);

-- Indexes on FK columns for JOIN and lookup performance
CREATE INDEX IF NOT EXISTS idx_agents_fleet_id     ON agents(fleet_id)     WHERE fleet_id IS NOT NULL;
CREATE INDEX IF NOT EXISTS idx_channels_fleet_id   ON channels(fleet_id)   WHERE fleet_id IS NOT NULL;
CREATE INDEX IF NOT EXISTS idx_fleet_shares_shared ON fleet_shares(shared_with_fleet_id);
CREATE INDEX IF NOT EXISTS idx_sessions_agent_id   ON sessions(agent_id);
