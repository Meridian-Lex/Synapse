# Synapse MCP Server Implementation Plan

> **For Lex:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Build a TypeScript MCP server in `synapse/mcp/` that wraps the `synapse` CLI binary and exposes fleet communications as MCP tools usable natively in Meridian Lex sessions.

**Architecture:** stdio MCP server using `@modelcontextprotocol/sdk`. CLI calls are spawned as child processes. All credential configuration flows through `SYNAPSE_*` environment variables. Packaged as a plugin with `install.sh` / `install.ps1`.

**Tech Stack:** TypeScript 5, `@modelcontextprotocol/sdk`, `zod`, Node.js `child_process` + `readline`.

---

## Task 1: Scaffold project structure

**Files:**
- Create: `mcp/package.json`
- Create: `mcp/tsconfig.json`
- Create: `mcp/.gitignore`

**Step 1: Create `mcp/package.json`**

```json
{
  "name": "synapse-mcp",
  "version": "0.1.0",
  "description": "Synapse fleet communications MCP server",
  "type": "module",
  "main": "dist/index.js",
  "scripts": {
    "build": "tsc --noEmit && tsc",
    "dev": "tsc --watch"
  },
  "dependencies": {
    "@modelcontextprotocol/sdk": "^1.0.0",
    "zod": "^3.22.0"
  },
  "devDependencies": {
    "typescript": "^5.3.0",
    "@types/node": "^20.0.0"
  }
}
```

**Step 2: Create `mcp/tsconfig.json`**

```json
{
  "compilerOptions": {
    "target": "ES2022",
    "module": "Node16",
    "moduleResolution": "Node16",
    "outDir": "dist",
    "rootDir": "src",
    "strict": true,
    "esModuleInterop": true,
    "skipLibCheck": true,
    "declaration": true
  },
  "include": ["src/**/*"],
  "exclude": ["dist", "node_modules"]
}
```

**Step 3: Create `mcp/.gitignore`**

```text
dist/
node_modules/
```

**Step 4: Commit**

```bash
cd synapse/mcp
git add package.json tsconfig.json .gitignore
git commit -m "chore(mcp): scaffold TypeScript MCP project"
```

---

## Task 2: Write `src/cli.ts` — shared CLI spawner

**Files:**
- Create: `mcp/src/cli.ts`

This module is the only place in the codebase that touches `child_process`. All tools import from here. It validates credentials before spawning anything.

**Step 1: Create `mcp/src/cli.ts`**

```typescript
import { spawn } from "child_process";
import { createInterface } from "readline";

export interface RunResult {
  stdout: string;
  stderr: string;
  code: number;
}

export interface PollResult {
  messages: string[];
  timedOut: boolean;
}

/** Resolve synapse binary path. SYNAPSE_CLI overrides; falls back to "synapse" in PATH. */
function resolveBin(): string {
  return process.env.SYNAPSE_CLI ?? "synapse";
}

/** Build child process env — passes all SYNAPSE_* vars through. */
function buildEnv(): NodeJS.ProcessEnv {
  return { ...process.env };
}

/** Returns an error string if required SYNAPSE_* vars are missing, else null. */
export function validateCredentials(): string | null {
  const required = ["SYNAPSE_AGENT", "SYNAPSE_SECRET"];
  const missing = required.filter((k) => !process.env[k]);
  if (missing.length === 0) return null;
  return `Missing required environment variables: ${missing.join(", ")}. Set them before starting the MCP server.`;
}

/**
 * Spawn synapse with the given args, wait for exit.
 * Used by send_message.
 */
export async function runOnce(args: string[]): Promise<RunResult> {
  const bin = resolveBin();
  return new Promise((resolve, reject) => {
    const child = spawn(bin, args, {
      env: buildEnv(),
      stdio: ["ignore", "pipe", "pipe"],
    });

    let stdout = "";
    let stderr = "";

    child.stdout.on("data", (d: Buffer) => { stdout += d.toString(); });
    child.stderr.on("data", (d: Buffer) => { stderr += d.toString(); });

    child.on("error", (err: NodeJS.ErrnoException) => {
      if (err.code === "ENOENT") {
        reject(new Error(
          `synapse CLI not found. Set SYNAPSE_CLI or add synapse to PATH. ` +
          `See mcp/README.md or run mcp/install.sh to set up the Synapse CLI.`
        ));
      } else {
        reject(err);
      }
    });

    child.on("close", (code: number | null) => {
      resolve({ stdout: stdout.trim(), stderr: stderr.trim(), code: code ?? 1 });
    });
  });
}

/**
 * Spawn synapse with the given args, collect stdout lines for timeoutMs.
 * onLine is called for each line; return true to exit early (e.g. enough messages received).
 * Used by listen_poll and wait_for_reply.
 */
export async function runWithTimeout(
  args: string[],
  timeoutMs: number,
  onLine?: (line: string, collected: string[]) => boolean
): Promise<PollResult> {
  const bin = resolveBin();
  return new Promise((resolve, reject) => {
    const child = spawn(bin, args, {
      env: buildEnv(),
      stdio: ["ignore", "pipe", "pipe"],
    });

    const messages: string[] = [];
    let settled = false;

    child.on("error", (err: NodeJS.ErrnoException) => {
      if (err.code === "ENOENT") {
        reject(new Error(
          `synapse CLI not found. Set SYNAPSE_CLI or add synapse to PATH. ` +
          `See mcp/README.md or run mcp/install.sh to set up the Synapse CLI.`
        ));
      } else {
        reject(err);
      }
    });

    const rl = createInterface({ input: child.stdout, crlfDelay: Infinity });

    rl.on("line", (line: string) => {
      // Filter the initial status line from `synapse listen`
      if (line.startsWith("Listening on ")) return;
      if (line === "") return;
      messages.push(line);
      if (!settled && onLine && onLine(line, messages)) {
        settled = true;
        clearTimeout(timer);
        child.kill("SIGTERM");
        resolve({ messages, timedOut: false });
      }
    });

    const timer = setTimeout(() => {
      if (!settled) {
        settled = true;
        child.kill("SIGTERM");
        resolve({ messages, timedOut: true });
      }
    }, timeoutMs);

    child.on("close", () => {
      if (!settled) {
        settled = true;
        clearTimeout(timer);
        resolve({ messages, timedOut: false });
      }
    });
  });
}
```

**Step 2: Verify it compiles**

```bash
cd synapse/mcp
npm install
npx tsc --noEmit
```

Expected: no errors (only missing src files — that is fine at this stage, we're building incrementally).

**Step 3: Commit**

```bash
git add src/cli.ts
git commit -m "feat(mcp): add CLI spawner with runOnce and runWithTimeout"
```

---

## Task 3: Write `src/tools/send.ts`

**Files:**
- Create: `mcp/src/tools/send.ts`

**Step 1: Create `mcp/src/tools/send.ts`**

```typescript
import { z } from "zod";
import { runOnce, validateCredentials } from "../cli.js";

export const SendMessageSchema = z.object({
  channel: z.string().describe("Channel name, e.g. #general"),
  message: z.string().describe("Message body to send"),
});

export const sendMessageTool = {
  name: "synapse_send_message",
  description: "Send a message to a Synapse fleet channel.",
  inputSchema: {
    type: "object" as const,
    properties: {
      channel: { type: "string", description: "Channel name, e.g. #general" },
      message: { type: "string", description: "Message body to send" },
    },
    required: ["channel", "message"],
  },
};

export async function handleSendMessage(args: unknown): Promise<string> {
  const credErr = validateCredentials();
  if (credErr) return credErr;

  const { channel, message } = SendMessageSchema.parse(args);
  const result = await runOnce(["send", "--channel", channel, message]);

  if (result.code !== 0) {
    return `Send failed (exit ${result.code}): ${result.stderr || result.stdout || "unknown error"}`;
  }

  return result.stdout || "Delivered.";
}
```

**Step 2: Verify compiles**

```bash
npx tsc --noEmit
```

**Step 3: Commit**

```bash
git add src/tools/send.ts
git commit -m "feat(mcp): add synapse_send_message tool"
```

---

## Task 4: Write `src/tools/listen.ts`

**Files:**
- Create: `mcp/src/tools/listen.ts`

**Step 1: Create `mcp/src/tools/listen.ts`**

```typescript
import { z } from "zod";
import { runWithTimeout, validateCredentials } from "../cli.js";

export const ListenPollSchema = z.object({
  channel: z.string().describe("Channel name, e.g. #general"),
  timeout_seconds: z.number().int().min(1).max(120).default(5)
    .describe("How long to collect messages before returning (default 5s)"),
});

export const WaitForReplySchema = z.object({
  channel: z.string().describe("Channel name, e.g. #general"),
  timeout_seconds: z.number().int().min(1).max(300).default(30)
    .describe("Maximum time to wait for replies (default 30s)"),
  min_messages: z.number().int().min(1).default(1)
    .describe("Exit early once this many messages are received (default 1)"),
});

export const listenPollTool = {
  name: "synapse_listen_poll",
  description:
    "Poll a Synapse channel for messages. Listens for timeout_seconds and returns all messages received. " +
    "Empty array means the channel was quiet — not an error.",
  inputSchema: {
    type: "object" as const,
    properties: {
      channel: { type: "string", description: "Channel name, e.g. #general" },
      timeout_seconds: {
        type: "number",
        description: "How long to collect messages (default 5s, max 120s)",
        default: 5,
      },
    },
    required: ["channel"],
  },
};

export const waitForReplyTool = {
  name: "synapse_wait_for_reply",
  description:
    "Wait for a reply on a Synapse channel. Exits as soon as min_messages arrive or timeout_seconds elapses. " +
    "Use after synapse_send_message to receive the response in a conversation loop. " +
    "Returns { timedOut, messages }.",
  inputSchema: {
    type: "object" as const,
    properties: {
      channel: { type: "string", description: "Channel name, e.g. #general" },
      timeout_seconds: {
        type: "number",
        description: "Maximum wait time in seconds (default 30s, max 300s)",
        default: 30,
      },
      min_messages: {
        type: "number",
        description: "Exit early once this many messages are received (default 1)",
        default: 1,
      },
    },
    required: ["channel"],
  },
};

export async function handleListenPoll(args: unknown): Promise<string> {
  const credErr = validateCredentials();
  if (credErr) return credErr;

  const { channel, timeout_seconds } = ListenPollSchema.parse(args);
  const result = await runWithTimeout(
    ["listen", "--channel", channel],
    timeout_seconds * 1000
  );

  return JSON.stringify(result.messages);
}

export async function handleWaitForReply(args: unknown): Promise<string> {
  const credErr = validateCredentials();
  if (credErr) return credErr;

  const { channel, timeout_seconds, min_messages } = WaitForReplySchema.parse(args);
  const result = await runWithTimeout(
    ["listen", "--channel", channel],
    timeout_seconds * 1000,
    (_line, collected) => collected.length >= min_messages
  );

  return JSON.stringify({ timedOut: result.timedOut, messages: result.messages });
}
```

**Step 2: Verify compiles**

```bash
npx tsc --noEmit
```

**Step 3: Commit**

```bash
git add src/tools/listen.ts
git commit -m "feat(mcp): add synapse_listen_poll and synapse_wait_for_reply tools"
```

---

## Task 5: Write `src/tools/stubs.ts`

**Files:**
- Create: `mcp/src/tools/stubs.ts`

**Step 1: Create `mcp/src/tools/stubs.ts`**

```typescript
import { z } from "zod";

const STUB_ACTION =
  "Check TASK-QUEUE.md for a task covering this capability. " +
  "If none exists, create one. If capacity is free, resolve it now.";

function stubResponse(capability: string): string {
  return JSON.stringify({
    status: "not_implemented",
    capability,
    action_required: STUB_ACTION,
  });
}

export const listChannelsTool = {
  name: "synapse_list_channels",
  description:
    "List available Synapse channels. NOT YET IMPLEMENTED — returns a task-queue reminder.",
  inputSchema: {
    type: "object" as const,
    properties: {},
    required: [],
  },
};

export const getChannelHistoryTool = {
  name: "synapse_get_channel_history",
  description:
    "Fetch message history from a Synapse channel. NOT YET IMPLEMENTED — returns a task-queue reminder.",
  inputSchema: {
    type: "object" as const,
    properties: {
      channel: { type: "string", description: "Channel name, e.g. #general" },
      limit: { type: "number", description: "Max messages to return (default 50)", default: 50 },
    },
    required: ["channel"],
  },
};

// eslint-disable-next-line @typescript-eslint/no-unused-vars
export async function handleListChannels(_args: unknown): Promise<string> {
  return stubResponse("synapse_list_channels");
}

// eslint-disable-next-line @typescript-eslint/no-unused-vars
export async function handleGetChannelHistory(_args: unknown): Promise<string> {
  return stubResponse("synapse_get_channel_history");
}
```

**Step 2: Verify compiles**

```bash
npx tsc --noEmit
```

**Step 3: Commit**

```bash
git add src/tools/stubs.ts
git commit -m "feat(mcp): add stub tools with task-queue reminders"
```

---

## Task 6: Write `src/index.ts` — MCP server entry point

**Files:**
- Create: `mcp/src/index.ts`

**Step 1: Create `mcp/src/index.ts`**

```typescript
import { Server } from "@modelcontextprotocol/sdk/server/index.js";
import { StdioServerTransport } from "@modelcontextprotocol/sdk/server/stdio.js";
import {
  CallToolRequestSchema,
  ListToolsRequestSchema,
} from "@modelcontextprotocol/sdk/types.js";

import { sendMessageTool, handleSendMessage } from "./tools/send.js";
import { listenPollTool, waitForReplyTool, handleListenPoll, handleWaitForReply } from "./tools/listen.js";
import { listChannelsTool, getChannelHistoryTool, handleListChannels, handleGetChannelHistory } from "./tools/stubs.js";

const server = new Server(
  { name: "synapse", version: "0.1.0" },
  { capabilities: { tools: {} } }
);

const tools = [
  sendMessageTool,
  listenPollTool,
  waitForReplyTool,
  listChannelsTool,
  getChannelHistoryTool,
];

server.setRequestHandler(ListToolsRequestSchema, async () => ({ tools }));

server.setRequestHandler(CallToolRequestSchema, async (request) => {
  const { name, arguments: args } = request.params;

  let text: string;
  try {
    switch (name) {
      case "synapse_send_message":       text = await handleSendMessage(args); break;
      case "synapse_listen_poll":        text = await handleListenPoll(args); break;
      case "synapse_wait_for_reply":     text = await handleWaitForReply(args); break;
      case "synapse_list_channels":      text = await handleListChannels(args); break;
      case "synapse_get_channel_history": text = await handleGetChannelHistory(args); break;
      default:
        return {
          content: [{ type: "text" as const, text: `Unknown tool: ${name}` }],
          isError: true,
        };
    }
  } catch (err) {
    const message = err instanceof Error ? err.message : String(err);
    return {
      content: [{ type: "text" as const, text: `Tool error: ${message}` }],
      isError: true,
    };
  }

  return { content: [{ type: "text" as const, text }] };
});

const transport = new StdioServerTransport();
await server.connect(transport);
```

**Step 2: Build**

```bash
npm run build
```

Expected: `dist/index.js` and supporting files created, no TypeScript errors.

**Step 3: Commit**

```bash
git add src/index.ts
git commit -m "feat(mcp): add MCP server entry point with all tool registrations"
```

---

## Task 7: Write `plugin.json`

**Files:**
- Create: `mcp/plugin.json`

**Step 1: Create `mcp/plugin.json`**

```json
{
  "name": "synapse",
  "version": "0.1.0",
  "description": "Synapse fleet communications MCP tools — send, listen, poll channels",
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

**Step 2: Commit**

```bash
git add plugin.json
git commit -m "feat(mcp): add plugin manifest"
```

---

## Task 8: Write `install.sh` and `install.ps1`

**Files:**
- Create: `mcp/install.sh`
- Create: `mcp/install.ps1`

**Step 1: Create `mcp/install.sh`**

```bash
#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

<!-- IDENTITY-EXCEPTION: functional internal reference — not for public exposure -->
PLUGIN_DIR="${HOME}/.claude/plugins/synapse"

echo "Building synapse-mcp..."
cd "$SCRIPT_DIR"
npm install
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

<!-- IDENTITY-EXCEPTION: functional internal reference — not for public exposure -->
echo "Required environment variables (set in your shell or ~/.claude/settings.json):"
echo "  SYNAPSE_AGENT   — your agent name"
echo "  SYNAPSE_SECRET  — your agent secret"
echo "  SYNAPSE_HOST    — broker address (default: localhost:7777)"
echo "  SYNAPSE_CA      — CA cert path (default: /etc/synapse/ca.pem)"
echo "  SYNAPSE_CLI     — path to synapse binary (default: synapse in PATH)"
```

**Step 2: Make executable**

```bash
chmod +x install.sh
```

**Step 3: Create `mcp/install.ps1`**

```powershell
#Requires -Version 5.1
$ErrorActionPreference = "Stop"

$ScriptDir = Split-Path -Parent $MyInvocation.MyCommand.Path
$PluginDir = Join-Path $env:USERPROFILE ".claude\plugins\synapse"

Write-Host "Building synapse-mcp..."
Set-Location $ScriptDir
npm install
npm run build

Write-Host "Installing plugin to $PluginDir ..."
$PluginsParent = Split-Path -Parent $PluginDir
if (-not (Test-Path $PluginsParent)) {
    New-Item -ItemType Directory -Path $PluginsParent | Out-Null
}

if (Test-Path $PluginDir) {
    $item = Get-Item $PluginDir
    if ($item.Attributes -band [System.IO.FileAttributes]::ReparsePoint) {
        Write-Host "Removing existing junction at $PluginDir"
        Remove-Item $PluginDir -Force
    } else {
        Write-Error "ERROR: $PluginDir exists and is not a junction. Remove it manually before installing."
        exit 1
    }
}

cmd /c "mklink /J `"$PluginDir`" `"$ScriptDir`"" | Out-Null
Write-Host "Done. Plugin installed at $PluginDir -> $ScriptDir"
Write-Host ""
Write-Host "Required environment variables:"
Write-Host "  SYNAPSE_AGENT   -- your agent name"
Write-Host "  SYNAPSE_SECRET  -- your agent secret"
Write-Host "  SYNAPSE_HOST    -- broker address (default: localhost:7777)"
Write-Host "  SYNAPSE_CA      -- CA cert path (default: /etc/synapse/ca.pem)"
Write-Host "  SYNAPSE_CLI     -- path to synapse binary (default: synapse in PATH)"
```

**Step 4: Commit**

```bash
git add install.sh install.ps1
git commit -m "feat(mcp): add install scripts for Unix and Windows"
```

---

## Task 9: Write `README.md`

**Files:**
- Create: `mcp/README.md`

**Step 1: Create `mcp/README.md`**

```markdown
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

Set these environment variables before starting a session (or add to

<!-- IDENTITY-EXCEPTION: functional internal reference — not for public exposure -->
`~/.claude/settings.json` under `env`):

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
echo '{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2024-11-05","capabilities":{},"clientInfo":{"name":"test","version":"0.0.1"}}}
{"jsonrpc":"2.0","id":2,"method":"tools/list","params":{}}' \
  | SYNAPSE_AGENT=test SYNAPSE_SECRET=test node dist/index.js
```

Expected: JSON response listing all 5 tools.

### 2. Send a message

```bash
SYNAPSE_AGENT=my-agent SYNAPSE_SECRET=my-secret \
  SYNAPSE_HOST=synapse.example.com:7777 \
  SYNAPSE_CA=/etc/synapse/ca.pem \
  synapse send --channel '#general' "test from mcp smoke"
```

### 3. Poll a channel

Send a message on a second terminal, then on the first:

```bash
echo '...tools/call synapse_listen_poll {"channel":"#general","timeout_seconds":5}...' \
  | node dist/index.js
```

Expected: JSON array containing the message sent.

### 4. Wait for a reply

Call `synapse_wait_for_reply` with `timeout_seconds: 10`, then send a message
from another agent. Expected: early exit with `{ timedOut: false, messages: [...] }`.

**Step 2: Commit**

```bash
git add README.md
git commit -m "docs(mcp): add README with setup, config, and smoke test instructions"
```

---

## Task 10: Wire `mcp-build` into the Synapse Makefile

**Files:**
- Modify: `Makefile` (Synapse repo root)

**Step 1: Read the current Makefile targets**

```bash
grep -n "^[a-z]" Makefile | head -20
```

**Step 2: Add `mcp-build` target**

Append after the existing `test` target (or at the end of the file):

```makefile
.PHONY: mcp-build
mcp-build:
	cd mcp && npm install && npm run build
```

**Step 3: Add `mcp-build` to the `all` target**

Find the line:

```makefile
all: broker cli-linux cli-windows
```

Change to:

```makefile
all: broker cli-linux cli-windows mcp-build
```

**Step 4: Commit**

```bash
git add Makefile
git commit -m "chore: add mcp-build target to Makefile"
```

---

## Task 11: Final build verification and push

**Step 1: Clean build from scratch**

```bash
cd mcp
rm -rf dist node_modules
npm install
npm run build
```

Expected: `dist/index.js` exists, no TypeScript errors.

**Step 2: Verify tool listing works**

```bash
printf '{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2024-11-05","capabilities":{},"clientInfo":{"name":"test","version":"0.0.1"}}}\n{"jsonrpc":"2.0","id":2,"method":"tools/list","params":{}}\n' \
  | SYNAPSE_AGENT=test SYNAPSE_SECRET=test node dist/index.js 2>/dev/null
```

Expected: response contains all 5 tool names.

**Step 3: Push branch**

```bash
git push origin fix/broker-decompress-before-decode
```

**Step 4: Done — open PR if this is the right branch, or note that mcp/ work can be cherry-picked to its own branch.**
