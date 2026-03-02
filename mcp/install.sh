#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
# IDENTITY-EXCEPTION: functional internal reference — not for public exposure
PLUGIN_DIR="${HOME}/.claude/plugins/synapse"

echo "Building synapse-mcp..."
cd "$SCRIPT_DIR"
npm ci
npm run build

echo "Installing plugin to ${PLUGIN_DIR}..."
mkdir -p "$(dirname "$PLUGIN_DIR")"

if [ -L "$PLUGIN_DIR" ]; then
  echo "Removing existing symlink at ${PLUGIN_DIR}"
  rm "$PLUGIN_DIR"
elif [ -d "$PLUGIN_DIR" ]; then
  echo "ERROR: ${PLUGIN_DIR} exists and is not a symlink. Remove it manually before installing."
  exit 1
fi

ln -s "$SCRIPT_DIR" "$PLUGIN_DIR"
echo "Done. Plugin installed at ${PLUGIN_DIR} -> ${SCRIPT_DIR}"
echo ""
echo "Required environment variables (set in your shell or settings.json env block):"
echo "  SYNAPSE_AGENT   — your agent name"
echo "  SYNAPSE_SECRET  — your agent secret"
echo "  SYNAPSE_HOST    — broker address (default: localhost:7777)"
echo "  SYNAPSE_CA      — CA cert path (default: /etc/synapse/ca.pem)"
echo "  SYNAPSE_CLI     — path to synapse binary (default: synapse in PATH)"
