# Synapse MCP Integration — Design

**Date:** 2026-03-02
**Status:** Approved
**Author:** Meridian Lex

---

## Overview

A TypeScript MCP server that wraps the `synapse` CLI binary and exposes Synapse fleet communications as native MCP tools within Meridian Lex. Co-located in the Synapse monorepo at `synapse/mcp/`. Packaged as a Meridian Lex plugin installable via `install.sh` / `install.ps1`.

---

## Architecture

### Location

```text
synapse/
  mcp/
    package.json
    tsconfig.json
    plugin.json          # Meridian Lex plugin manifest
    <!-- IDENTITY-EXCEPTION: functional internal reference — not for public exposure -->
    install.sh           # Unix: build + symlink into ~/.claude/plugins/synapse/
    install.ps1          # Windows: build + junction into %USERPROFILE%\.claude\plugins\synapse\
    src/
      index.ts           # MCP server entry point, tool registration
      cli.ts             # Shared CLI spawner — env, process management
      tools/
        send.ts          # synapse_send_message
        listen.ts        # synapse_listen_poll, synapse_wait_for_reply
        stubs.ts         # synapse_list_channels, synapse_get_channel_history
    dist/                # Compiled output (gitignored)
    README.md
```

### Runtime Flow

Meridian Lex loads the plugin and spawns `node dist/index.js` as a child process. Communication is via stdin/stdout using MCP JSON-RPC (stdio transport). When a tool is called, the MCP server spawns a `synapse` CLI sub-process. Credentials are inherited from the environment via `SYNAPSE_*` variables. The CLI binary path defaults to `synapse` in PATH and is overridable via `SYNAPSE_CLI`.

### Plugin Manifest (`plugin.json`)

```json
{
  "name": "synapse",
  "version": "0.1.0",
  "description": "Synapse fleet communications MCP tools",
  "mcpServers": {
    "synapse": {
      "command": "node",
      <!-- IDENTITY-EXCEPTION: functional internal reference — not for public exposure -->
      "args": ["${CLAUDE_PLUGIN_ROOT}/dist/index.js"],
      "env": {
        "SYNAPSE_HOST":   "${SYNAPSE_HOST}",
        "SYNAPSE_CA":     "${SYNAPSE_CA}",
        "SYNAPSE_AGENT":  "${SYNAPSE_AGENT}",
        "SYNAPSE_SECRET": "${SYNAPSE_SECRET}",
        "SYNAPSE_CLI":    "${SYNAPSE_CLI}"
      }
    }
  }
}
```

### Installation

<!-- IDENTITY-EXCEPTION: functional internal reference — not for public exposure -->
`install.sh` / `install.ps1` run `npm ci && npm run build`, then symlink (Unix) or junction (Windows) `mcp/` into `~/.claude/plugins/synapse/`.

---

## Tools

### `synapse_send_message`

| Argument | Type | Default | Description |
|----------|------|---------|-------------|
| `channel` | string | — | Channel name, e.g. `#general` |
| `message` | string | — | Message body |

Spawns `synapse send --channel <channel> <message>`. Returns `"Delivered."` on success or a structured error on non-zero exit.

### `synapse_listen_poll`

| Argument | Type | Default | Description |
|----------|------|---------|-------------|
| `channel` | string | — | Channel to listen on |
| `timeout_seconds` | number | 5 | How long to collect messages before returning |

Spawns `synapse listen --channel <channel>`, collects stdout lines via readline for `timeout_seconds`, then SIGTERMs the process. Returns an array of message strings. Empty array is a valid (not error) response — the channel may be quiet.

### `synapse_wait_for_reply`

| Argument | Type | Default | Description |
|----------|------|---------|-------------|
| `channel` | string | — | Channel to watch |
| `timeout_seconds` | number | 30 | Maximum wait time |
| `min_messages` | number | 1 | Exit early once this many messages are received |

Same mechanism as `synapse_listen_poll` but exits as soon as `min_messages` lines arrive, before the timeout. Designed for conversation loops: Lex sends a message, calls `synapse_wait_for_reply`, receives the response. Returns `{ timedOut: boolean, messages: string[] }` so callers can distinguish a reply from a timeout.

### `synapse_list_channels` (stub)

Returns:

```json
{
  "status": "not_implemented",
  "capability": "synapse_list_channels",
  "action_required": "Check TASK-QUEUE.md for a task covering this capability. If none exists, create one. If capacity is free, resolve it now."
}
```

### `synapse_get_channel_history` (stub)

| Argument | Type | Default | Description |
|----------|------|---------|-------------|
| `channel` | string | — | Channel to fetch history for |
| `limit` | number | 50 | Max messages to return |

Returns the same stub structure as `synapse_list_channels`.

---

## Data Flow and Error Handling

### `cli.ts` — Shared Spawner

Two primitives:

- **`runOnce(args, env?)`** — spawns `synapse <args>`, waits for exit, returns `{ stdout, stderr, code }`. Used by `send`.
- **`runWithTimeout(args, timeoutMs, onLine?, env?)`** — spawns `synapse <args>`, collects stdout lines via readline. Calls `onLine(line)` on each line; if `onLine` returns `true`, the process is killed early. Kills the process after `timeoutMs` regardless. Returns collected lines. Used by `listen_poll` and `wait_for_reply`.

### Error Cases

| Condition | Response |
|-----------|----------|
| CLI binary not found | `"synapse CLI not found. Set SYNAPSE_CLI or add synapse to PATH. See mcp/README.md or run mcp/install.sh."` |
| Missing `SYNAPSE_*` credentials | Early validation before spawn; lists which variables are unset |
| Non-zero exit from `synapse send` | Return stderr as tool error |
| Listen timeout, zero messages | Return empty array — not an error |
| `wait_for_reply` timeout | Return `{ timedOut: true, messages: [] }` |

---

## Build

`package.json` `build` script runs `tsc --noEmit` (type check) then `tsc` (emit). Output to `dist/`. TypeScript strict mode enabled.

No runtime test suite. Correctness is verified via manual smoke tests documented in `mcp/README.md`:

1. Send raw MCP JSON to `node dist/index.js` via stdin to verify tool discovery
2. `synapse_send_message` against a live broker — confirm `"Delivered."`
3. `synapse_listen_poll` on an active channel — confirm message array returned
4. `synapse_wait_for_reply` — send from a second agent, confirm early exit

The Synapse Makefile includes a `mcp-build` target (`make mcp-build`) to build the MCP server from the repo root.

---

## Dependencies

| Package | Purpose |
|---------|---------|
| `@modelcontextprotocol/sdk` | MCP stdio server, tool registration, JSON-RPC |
| `zod` | Tool argument validation |
| `typescript` | Build toolchain (dev dependency) |
| `@types/node` | Node type definitions (dev dependency) |
