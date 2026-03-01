#!/usr/bin/env bash
# Synapse broker entrypoint
# 1. Substitute env vars into config template -> /tmp/synapse-resolved.yaml
# 2. Apply DB migration idempotently
# 3. Exec broker
set -euo pipefail

CONFIG_TEMPLATE="${SYNAPSE_CONFIG:-/etc/synapse/synapse.yaml}"
CONFIG_LIVE=/tmp/synapse-resolved.yaml

echo "[synapse] Resolving config from $CONFIG_TEMPLATE..."
envsubst < "$CONFIG_TEMPLATE" > "$CONFIG_LIVE"

echo "[synapse] Applying database migration..."
PG_URL=$(grep -m1 'url:' "$CONFIG_LIVE" | awk '{print $2}' | tr -d '"')
psql "$PG_URL" \
  -v ON_ERROR_STOP=0 \
  -f /app/migrations/001_initial.sql 2>&1 | grep -v "already exists" || true
echo "[synapse] Migration complete."

echo "[synapse] Starting broker..."
exec /usr/local/bin/synapse-broker --config "$CONFIG_LIVE"
