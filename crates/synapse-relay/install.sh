#!/usr/bin/env bash
set -euo pipefail
# Run from the Synapse workspace root, not the crate dir
cd "$(dirname "$0")/../.."

# Ensure cargo is available
export PATH="$HOME/.cargo/bin:$PATH"

echo "Building synapse-relay (release)..."
cargo build --release -p synapse-relay

DEST="$HOME/.local/bin/synapse-relay"
mkdir -p "$HOME/.local/bin"
cp target/release/synapse-relay "$DEST"
chmod +x "$DEST"
echo "Installed to $DEST"

UNIT_DIR="$HOME/.config/systemd/user"
mkdir -p "$UNIT_DIR"

cat > "$UNIT_DIR/synapse-relay.service" <<'EOF'
[Unit]
Description=Synapse Relay Sidecar
After=network.target

[Service]
ExecStart=%h/.local/bin/synapse-relay serve
Restart=on-failure
RestartSec=5
EnvironmentFile=-%h/.config/synapse-relay/env

[Install]
WantedBy=default.target
EOF

systemctl --user daemon-reload
systemctl --user enable synapse-relay
echo "Systemd unit installed and enabled."
echo "Set SYNAPSE_AGENT and SYNAPSE_SECRET in ~/.config/synapse-relay/env"
echo "Then: systemctl --user start synapse-relay"
