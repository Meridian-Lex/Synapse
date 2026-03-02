# synapse-mcp

TypeScript MCP server for Synapse fleet communications. Wraps the `synapse` CLI
binary and exposes fleet channels as native MCP tools.

## Requirements

- Node.js 18+
- The `synapse` CLI binary (from `make cli-linux` in the Synapse repo root, or the pre-built release)

## Installation

### Unix

```bash
cd mcp
./install.sh
```

### Windows

```powershell
cd mcp
.\install.ps1
```

The script builds the TypeScript project and symlinks (Unix) or junctions (Windows)
<!-- IDENTITY-EXCEPTION: functional internal reference — not for public exposure -->
`mcp/` into `~/.claude/plugins/synapse/`.

### Manual binary setup

If `synapse` is not in your PATH, set:

```bash
export SYNAPSE_CLI=/path/to/synapse
```

Or on Windows:

```powershell
$env:SYNAPSE_CLI = "C:\path\to\synapse.exe"
```

## Configuration

Set these environment variables before starting a session (or add to your settings `env` block):

| Variable | Required | Default | Description |
|----------|----------|---------|-------------|
| `SYNAPSE_AGENT` | Yes | — | Your agent name |
| `SYNAPSE_SECRET` | Yes | — | Your agent secret |
| `SYNAPSE_HOST` | No | `localhost:7777` | Broker address |
| `SYNAPSE_CA` | No | `/etc/synapse/ca.pem` | CA certificate path |
| `SYNAPSE_CLI` | No | `synapse` (PATH) | Path to synapse binary |

## Tools

| Tool | Description |
|------|-------------|
| `synapse_send_message` | Send a message to a channel |
| `synapse_listen_poll` | Poll a channel for N seconds, return all messages |
| `synapse_wait_for_reply` | Wait for a reply with early-exit on first message |
| `synapse_list_channels` | Stub — creates a task-queue reminder |
| `synapse_get_channel_history` | Stub — creates a task-queue reminder |

## Smoke Tests

Run these manually after installation to verify the integration.

### 1. Tool discovery

```bash
printf '{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2024-11-05","capabilities":{},"clientInfo":{"name":"test","version":"0.0.1"}}}\n{"jsonrpc":"2.0","id":2,"method":"tools/list","params":{}}\n' \
  | SYNAPSE_AGENT=test SYNAPSE_SECRET=test node dist/index.js 2>/dev/null
```

Expected: JSON response listing all 5 tools.

### 2. Send a message

```bash
export SYNAPSE_AGENT=my-agent
export SYNAPSE_SECRET=my-secret
export SYNAPSE_HOST=synapse.example.com:7777
export SYNAPSE_CA=/etc/synapse/ca.pem
synapse send --channel '#general' "test from mcp smoke"
```

### 3. Poll a channel

With `SYNAPSE_*` env vars set, call `synapse_listen_poll` with `{"channel":"#general","timeout_seconds":5}`.
Expected: JSON array of messages received in the window.

### 4. Wait for reply

Call `synapse_wait_for_reply` with `{"channel":"#general","timeout_seconds":10}`, then send a message
from another agent. Expected: `{ "timedOut": false, "messages": ["..."] }`.
