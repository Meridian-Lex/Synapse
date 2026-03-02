#!/usr/bin/env bash
# Idempotent fleet bootstrap: create fleet, human operator agent, and default channel.
# Usage: FLEET_SECRET=<secret> ./scripts/bootstrap-fleet.sh <fleet-name> <agent-name> [default-channel]
# Example: FLEET_SECRET="$(cat /run/secrets/my-fleet-secret)" ./scripts/bootstrap-fleet.sh lex commander '#general'
#
# The secret is read from the FLEET_SECRET environment variable rather than a positional
# argument to prevent it from appearing in process listings (ps aux / /proc/<pid>/cmdline).
set -euo pipefail

FLEET_NAME="${1:?Usage: FLEET_SECRET=<secret> $0 <fleet-name> <agent-name> [default-channel]}"
AGENT_NAME="${2:?}"
AGENT_SECRET="${FLEET_SECRET:?FLEET_SECRET environment variable must be set}"
DEFAULT_CHANNEL="${3:-#general}"

# psql -v sets named variables; ::'text' applies proper quoting in the SQL,
# preventing injection regardless of what the shell variables contain.
PSQL="docker exec -i stratavore-postgres psql -U postgres -d synapse -v ON_ERROR_STOP=1"

echo "[bootstrap-fleet] Fleet='${FLEET_NAME}' Agent='${AGENT_NAME}' Channel='${DEFAULT_CHANNEL}'"

$PSQL \
  -v "fleet_name=${FLEET_NAME}" \
  -v "agent_name=${AGENT_NAME}" \
  -v "agent_secret=${AGENT_SECRET}" \
  -v "channel_name=${DEFAULT_CHANNEL}" \
  <<'SQL'
DO $$
DECLARE
  v_agent_id   BIGINT;
  v_fleet_id   BIGINT;
  v_channel_id BIGINT;
BEGIN
  -- Upsert human agent (:'var' uses psql quoting — safe against injection)
  INSERT INTO agents (name, secret_hash, is_human)
  VALUES (:'agent_name', :'agent_secret', true)
  ON CONFLICT (name)
  DO UPDATE SET secret_hash = EXCLUDED.secret_hash, is_human = true
  RETURNING id INTO v_agent_id;

  -- Idempotent fleet: insert or find existing (do not overwrite owner on conflict).
  INSERT INTO fleets (name, owner_id)
  VALUES (:'fleet_name', v_agent_id)
  ON CONFLICT (name) DO NOTHING
  RETURNING id INTO v_fleet_id;
  IF v_fleet_id IS NULL THEN
    SELECT id INTO v_fleet_id FROM fleets WHERE name = :'fleet_name';
  END IF;

  -- Assign agent to fleet
  UPDATE agents SET fleet_id = v_fleet_id WHERE id = v_agent_id;

  -- Idempotent channel: select existing fleet channel or insert new one.
  -- Avoids ON CONFLICT (name) which could steal a channel from another fleet.
  SELECT id INTO v_channel_id FROM channels
    WHERE name = :'channel_name' AND fleet_id = v_fleet_id;
  IF v_channel_id IS NULL THEN
    INSERT INTO channels (name, fleet_id, created_by)
    VALUES (:'channel_name', v_fleet_id, v_agent_id)
    RETURNING id INTO v_channel_id;
  END IF;

  -- Set default channel on agent
  UPDATE agents SET default_channel_id = v_channel_id WHERE id = v_agent_id;

  RAISE NOTICE 'Done: fleet_id=% agent_id=% channel_id=%',
    v_fleet_id, v_agent_id, v_channel_id;
END;
$$;
SQL

echo "[bootstrap-fleet] Complete."
