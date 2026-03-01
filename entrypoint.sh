#!/usr/bin/env bash
# Synapse broker entrypoint
# 1. Substitute env vars into config template -> /tmp/synapse-resolved.yaml
# 2. Apply DB migration idempotently
# 3. Exec broker
set -euo pipefail

CONFIG_TEMPLATE="${SYNAPSE_CONFIG_TEMPLATE:-/etc/synapse/synapse.yaml}"
CONFIG_LIVE=/tmp/synapse-resolved.yaml

echo "[synapse] Resolving config from $CONFIG_TEMPLATE..."
envsubst < "$CONFIG_TEMPLATE" > "$CONFIG_LIVE"

echo "[synapse] Starting broker (sqlx applies migrations on startup)..."
export SYNAPSE_CONFIG="$CONFIG_LIVE"
exec /usr/local/bin/synapse-broker
