# Synapse Relay Sidecar Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Build `synapse-relay`, a persistent Rust daemon that holds live broker connections per channel, buffers all inbound messages, and exposes a local HTTP API on `localhost:7779` — eliminating message loss between MCP tool calls.

**Architecture:** The relay runs as a systemd user service, holds one TLS connection per subscribed channel, and buffers messages in a per-channel ring buffer. The MCP server is rewritten as a thin HTTP client against the relay, with per-channel `nextSeq` tracking to avoid re-delivery.

**Tech Stack:** Rust (tokio, axum, tokio-native-tls, hmac/sha2, rmpv, serde/serde_json, clap, toml), TypeScript (MCP server, node-fetch or built-in fetch), systemd user units.

**Spec:** `docs/superpowers/specs/2026-03-25-synapse-relay-sidecar-design.md`

**Rust agents:** Employ rust-engineer subagent for implementation tasks; use verifier subagent after each task group. Do NOT attempt to implement Rust tasks inline without a specialist.

---

## File Map

### New files — `tools/synapse-relay/`

| File | Responsibility |
|------|---------------|
| `Cargo.toml` | Crate manifest, all dependencies |
| `src/main.rs` | CLI entry point, clap subcommand despatch |
| `src/config.rs` | Load `~/.config/synapse-relay/config.toml`, env overrides, defaults |
| `src/types.rs` | Shared types: `BufferedMessage`, `ChannelStatus`, `RelayError`, API request/response structs |
| `src/broker.rs` | Synapse wire protocol client: TLS connect, HMAC auth handshake, frame read/write, channel subscribe/unsubscribe, send Dialogue/Work frames, channel list, presence |
| `src/buffer.rs` | Per-channel ring buffer with u64 seq, FIFO eviction, drain-since, wait-for-min |
| `src/relay.rs` | Channel registry: maps channel name → (BrokerConnection + ChannelBuffer), manages reconnect loop, auth-fail hard-stop |
| `src/server.rs` | axum HTTP server, all route handlers, relay state injection |
| `tests/buffer_test.rs` | Unit tests: ring eviction, seq gaps, drain-since, wait timeout |
| `tests/relay_test.rs` | Integration tests: mock broker TCP listener, subscribe/send/poll/wait lifecycle, reconnect, auth failure |

<!-- IDENTITY-EXCEPTION: functional internal reference — not for public exposure -->
### Modified files — `~/.claude/plugins/synapse/src/`

| File | Change |
|------|--------|
| `src/relay-client.ts` | **New** — HTTP client wrapper for relay API, manages per-channel `nextSeq` |
| `src/tools/send.ts` | Replace `runOnce` call with `relayClient.send()` |
| `src/tools/listen.ts` | Replace buffer/subprocess with `relayClient.poll()` and `relayClient.wait()` |
| `src/tools/chat.ts` | **New** — `synapse_chat` tool: send + immediate poll |
| `src/tools/work.ts` | **New** — `synapse_send_work` tool |
| `src/tools/channels.ts` | **New** — `synapse_list_channels`, `synapse_list_users`, `synapse_join`, `synapse_leave` |
| `src/tools/stubs.ts` | Remove (replaced by channels.ts) |
| `src/buffer.ts` | Remove (replaced by relay-client.ts) |
| `src/cli.ts` | Remove `runWithTimeout` (keep `validateCredentials`, `runOnce` for history tool only) |
| `src/index.ts` | Register new tools, remove buffer lifecycle |

### New infra files

| File | Responsibility |
|------|---------------|
| `tools/synapse-relay/install.sh` | Build release binary, copy to `~/.local/bin/`, install systemd unit, `systemctl --user enable` |
| `~/.config/systemd/user/synapse-relay.service` | Systemd user unit (written by install.sh) |

---

## Task 1: Cargo workspace and types

**Files:**
- Create: `tools/synapse-relay/Cargo.toml`
- Create: `tools/synapse-relay/src/main.rs` (skeleton only)
- Create: `tools/synapse-relay/src/types.rs`

- [ ] **Step 1: Create directory structure**

```bash
mkdir -p /home/meridian/meridian-home/tools/synapse-relay/src
mkdir -p /home/meridian/meridian-home/tools/synapse-relay/tests
```

Note: check if `/home/meridian/meridian-home/` is a Cargo workspace (look for `[workspace]` in a root `Cargo.toml`). If so, add `"tools/synapse-relay"` to the workspace members list before running any cargo commands.

```bash
# Check for workspace
grep -q '\[workspace\]' /home/meridian/meridian-home/Cargo.toml 2>/dev/null && echo "workspace found — add member" || echo "standalone crate"
```

- [ ] **Step 2: Write Cargo.toml**

```toml
[package]
name = "synapse-relay"
version = "0.1.0"
edition = "2021"

[[bin]]
name = "synapse-relay"
path = "src/main.rs"

[dependencies]
tokio = { version = "1", features = ["full"] }
axum = { version = "0.7", features = ["json"] }
tower = "0.4"
serde = { version = "1", features = ["derive"] }
serde_json = "1"
hmac = "0.12"
sha2 = "0.10"
rmpv = { version = "1", features = ["with-serde"] }
toml = "0.8"
clap = { version = "4", features = ["derive"] }
tokio-native-tls = "0.3"
native-tls = "0.2"
anyhow = "1"
thiserror = "1"
tracing = "0.1"
tracing-subscriber = { version = "0.3", features = ["env-filter"] }

[dev-dependencies]
tokio-test = "0.4"
```

- [ ] **Step 3: Write types.rs with all shared types**

```rust
// src/types.rs
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum MessageType { Dialogue, Work }

#[derive(Debug, Clone, Serialize)]
pub struct BufferedMessage {
    pub seq: u64,
    #[serde(rename = "type")]
    pub msg_type: MessageType,
    pub text: Option<String>,
    pub payload: Option<serde_json::Value>,
    pub received_at: u64, // unix ms
}

#[derive(Debug, Clone, Serialize)]
pub struct ChannelStatus {
    pub channel: String,
    pub connected: bool,
    pub buffered: usize,
}

// HTTP request/response types
#[derive(Deserialize)] pub struct SubscribeReq  { pub channel: String }
#[derive(Deserialize)] pub struct LeaveReq      { pub channel: String }
#[derive(Deserialize)] pub struct SendReq       { pub channel: String, pub text: String }
#[derive(Deserialize)] pub struct SendWorkReq   { pub channel: String, pub payload: serde_json::Value }

#[derive(Serialize)] pub struct OkResp          { pub ok: bool }
#[derive(Serialize)] pub struct ErrorResp       { pub error: String, #[serde(skip_serializing_if = "Option::is_none")] pub channel: Option<String> }
#[derive(Serialize)] pub struct PollResp        { pub messages: Vec<BufferedMessage>, pub next_seq: u64 }
#[derive(Serialize)] pub struct WaitResp        { pub messages: Vec<BufferedMessage>, pub next_seq: u64, pub timed_out: bool }
#[derive(Serialize)] pub struct ChannelsResp    { pub channels: Vec<String> }
#[derive(Serialize)] pub struct UsersResp       { pub users: Vec<String> }
#[derive(Serialize)] pub struct StatusResp      { pub channels: Vec<ChannelStatus> }
```

- [ ] **Step 4: Write main.rs skeleton (no logic yet)**

```rust
// src/main.rs
mod config;
mod types;
mod broker;
mod buffer;
mod relay;
mod server;

use clap::{Parser, Subcommand};

#[derive(Parser)]
#[command(name = "synapse-relay", about = "Synapse fleet relay sidecar")]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    Serve { #[arg(long, default_value = "7779")] port: u16,
            #[arg(long, default_value = "127.0.0.1")] bind: String },
    Channels,
    Users   { channel: String },
    Join    { channel: String },
    Leave   { channel: String },
    Say     { channel: String, message: Vec<String> },
    Tail    { channel: String, #[arg(long, default_value = "20")] count: usize },
    Status,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::init();
    let cli = Cli::parse();
    match cli.command {
        Commands::Serve { port, bind } => server::run(bind, port).await,
        _ => todo!("CLI commands implemented in Task 7"),
    }
}
```

- [ ] **Step 5: Verify it compiles**

```bash
cd /home/meridian/meridian-home/tools/synapse-relay && cargo build 2>&1 | tail -5
```
Expected: compile errors only for missing modules (broker, buffer, relay, server, config) — not type errors.

- [ ] **Step 6: Commit**

```bash
cd /home/meridian/meridian-home/tools/synapse-relay
git add Cargo.toml src/
git commit -m "feat(synapse-relay): scaffold crate — types, Cargo.toml, main skeleton"
```

---

## Task 2: Config module

**Files:**
- Create: `tools/synapse-relay/src/config.rs`

- [ ] **Step 1: Write failing test**

```rust
// tests/config_test.rs  (add to tests/ dir or inline in config.rs)
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_defaults() {
        let cfg = Config::defaults();
        assert_eq!(cfg.port, 7779);
        assert_eq!(cfg.bind, "127.0.0.1");
        assert_eq!(cfg.buffer_capacity, 1000);
        assert_eq!(cfg.reconnect_delay_secs, 2);
        assert_eq!(cfg.reconnect_max_secs, 30);
    }

    #[test]
    fn test_from_toml_partial() {
        let raw = r#"port = 8888"#;
        let cfg = Config::from_str(raw).unwrap();
        assert_eq!(cfg.port, 8888);
        assert_eq!(cfg.bind, "127.0.0.1"); // default preserved
    }
}
```

- [ ] **Step 2: Run test — verify fails**

```bash
cd /home/meridian/meridian-home/tools/synapse-relay && cargo test config 2>&1 | tail -10
```

- [ ] **Step 3: Implement config.rs**

```rust
// src/config.rs
use serde::Deserialize;
use std::path::PathBuf;

#[derive(Debug, Clone, Deserialize)]
pub struct Credentials {
    pub credentials_file: Option<PathBuf>,
}

impl Default for Credentials {
    fn default() -> Self {
        Self { credentials_file: Some(PathBuf::from("~/.config/synapse/credentials.toml")) }
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct Config {
    #[serde(default = "default_bind")]   pub bind: String,
    #[serde(default = "default_port")]   pub port: u16,
    #[serde(default = "default_broker_host")] pub broker_host: String,
    #[serde(default = "default_broker_port")] pub broker_port: u16,
    #[serde(default = "default_capacity")]    pub buffer_capacity: usize,
    #[serde(default = "default_reconnect_delay")] pub reconnect_delay_secs: u64,
    #[serde(default = "default_reconnect_max")]   pub reconnect_max_secs: u64,
    #[serde(default)] pub credentials: Credentials,
    // Env-var overrides (populated after load)
    #[serde(skip)] pub agent_name: String,
    #[serde(skip)] pub secret: String,
}

fn default_bind()            -> String { "127.0.0.1".into() }
fn default_port()            -> u16    { 7779 }
fn default_broker_host()     -> String { "localhost".into() }
fn default_broker_port()     -> u16    { 7777 }
fn default_capacity()        -> usize  { 1000 }
fn default_reconnect_delay() -> u64    { 2 }
fn default_reconnect_max()   -> u64    { 30 }

impl Config {
    pub fn defaults() -> Self {
        toml::from_str("").unwrap()
    }

    pub fn from_str(s: &str) -> anyhow::Result<Self> {
        let mut cfg: Config = toml::from_str(s)?;
        cfg.agent_name = std::env::var("SYNAPSE_AGENT").unwrap_or_default();
        cfg.secret     = std::env::var("SYNAPSE_SECRET").unwrap_or_default();
        Ok(cfg)
    }

    pub fn load() -> anyhow::Result<Self> {
        let path = dirs_path();
        let content = if path.exists() {
            std::fs::read_to_string(&path)?
        } else {
            String::new()
        };
        Self::from_str(&content)
    }

    pub fn validate(&self) -> anyhow::Result<()> {
        if self.agent_name.is_empty() {
            anyhow::bail!("SYNAPSE_AGENT env var required");
        }
        if self.secret.is_empty() {
            anyhow::bail!("SYNAPSE_SECRET env var required");
        }
        Ok(())
    }
}

fn dirs_path() -> std::path::PathBuf {
    let home = std::env::var("HOME").unwrap_or_else(|_| ".".into());
    std::path::PathBuf::from(home).join(".config/synapse-relay/config.toml")
}
```

- [ ] **Step 4: Run tests — verify pass**

```bash
cargo test config 2>&1 | tail -10
```

- [ ] **Step 5: Commit**

```bash
git add src/config.rs && git commit -m "feat(synapse-relay): config module with defaults and env-var injection"
```

---

## Task 3: Ring buffer

**Files:**
- Create: `tools/synapse-relay/src/buffer.rs`
- Create: `tools/synapse-relay/tests/buffer_test.rs`

- [ ] **Step 1: Write failing tests**

```rust
// tests/buffer_test.rs
use synapse_relay::buffer::ChannelBuffer;

#[test]
fn test_seq_starts_at_one() {
    let mut buf = ChannelBuffer::new(10);
    buf.push_dialogue("hello".into());
    let msgs = buf.drain_since(0);
    assert_eq!(msgs[0].seq, 1);
}

#[test]
fn test_drain_since_filters() {
    let mut buf = ChannelBuffer::new(10);
    buf.push_dialogue("a".into());
    buf.push_dialogue("b".into());
    buf.push_dialogue("c".into());
    let msgs = buf.drain_since(2);
    assert_eq!(msgs.len(), 1);
    assert_eq!(msgs[0].seq, 3);
}

#[test]
fn test_fifo_eviction_at_capacity() {
    let mut buf = ChannelBuffer::new(3);
    buf.push_dialogue("a".into()); // seq 1
    buf.push_dialogue("b".into()); // seq 2
    buf.push_dialogue("c".into()); // seq 3
    buf.push_dialogue("d".into()); // seq 4 — evicts seq 1
    let msgs = buf.drain_since(0);
    assert_eq!(msgs.len(), 3);
    assert_eq!(msgs[0].seq, 2); // oldest is now seq 2
    assert_eq!(msgs[2].seq, 4);
}

#[test]
fn test_next_seq_after_eviction() {
    let mut buf = ChannelBuffer::new(2);
    buf.push_dialogue("a".into());
    buf.push_dialogue("b".into());
    buf.push_dialogue("c".into());
    assert_eq!(buf.next_seq(), 4);
}

#[tokio::test]
async fn test_wait_for_resolves_on_arrival() {
    use std::sync::{Arc, Mutex};
    let buf = Arc::new(Mutex::new(ChannelBuffer::new(100)));
    let buf2 = buf.clone();
    tokio::spawn(async move {
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        buf2.lock().unwrap().push_dialogue("arrived".into());
    });
    let msgs = ChannelBuffer::wait_for(buf, 0, 1, 500).await;
    assert_eq!(msgs.len(), 1);
}

#[tokio::test]
async fn test_wait_for_times_out() {
    use std::sync::{Arc, Mutex};
    let buf = Arc::new(Mutex::new(ChannelBuffer::new(100)));
    let msgs = ChannelBuffer::wait_for(buf, 0, 1, 50).await;
    assert!(msgs.is_empty());
}
```

- [ ] **Step 2: Run tests — verify fail**

```bash
cargo test buffer 2>&1 | tail -15
```

- [ ] **Step 3: Implement buffer.rs**

```rust
// src/buffer.rs
use crate::types::{BufferedMessage, MessageType};
use std::sync::{Arc, Mutex};
use std::collections::VecDeque;

pub struct ChannelBuffer {
    capacity: usize,
    messages: VecDeque<BufferedMessage>,
    next_seq: u64,
}

impl ChannelBuffer {
    pub fn new(capacity: usize) -> Self {
        Self { capacity, messages: VecDeque::new(), next_seq: 1 }
    }

    fn push(&mut self, msg_type: MessageType, text: Option<String>, payload: Option<serde_json::Value>) {
        let seq = self.next_seq;
        self.next_seq += 1;
        let received_at = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as u64;
        if self.messages.len() >= self.capacity {
            self.messages.pop_front(); // FIFO eviction
        }
        self.messages.push_back(BufferedMessage { seq, msg_type, text, payload, received_at });
    }

    pub fn push_dialogue(&mut self, text: String) {
        self.push(MessageType::Dialogue, Some(text), None);
    }

    pub fn push_work(&mut self, value: serde_json::Value) {
        self.push(MessageType::Work, None, Some(value));
    }

    /// Returns all messages with seq > since.
    pub fn drain_since(&self, since: u64) -> Vec<BufferedMessage> {
        self.messages.iter().filter(|m| m.seq > since).cloned().collect()
    }

    pub fn next_seq(&self) -> u64 { self.next_seq }

    pub fn len(&self) -> usize { self.messages.len() }

    /// Long-poll: wait until at least `min` messages with seq > since arrive,
    /// or `timeout_ms` elapses. Returns whatever was collected.
    pub async fn wait_for(
        buf: Arc<Mutex<Self>>,
        since: u64,
        min: usize,
        timeout_ms: u64,
    ) -> Vec<BufferedMessage> {
        let deadline = tokio::time::Instant::now()
            + std::time::Duration::from_millis(timeout_ms);
        loop {
            {
                let guard = buf.lock().unwrap();
                let msgs = guard.drain_since(since);
                if msgs.len() >= min {
                    return msgs;
                }
            }
            if tokio::time::Instant::now() >= deadline { break; }
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }
        buf.lock().unwrap().drain_since(since)
    }
}
```

Note: add `pub mod buffer;` to `lib.rs` (create `src/lib.rs` exporting all modules for test access).

- [ ] **Step 4: Create src/lib.rs for test access**

```rust
// src/lib.rs
pub mod buffer;
pub mod config;
pub mod types;
```

- [ ] **Step 5: Run tests — verify pass**

```bash
cargo test buffer 2>&1 | tail -15
```

- [ ] **Step 6: Commit**

```bash
git add src/buffer.rs src/lib.rs tests/buffer_test.rs
git commit -m "feat(synapse-relay): ring buffer with seq tracking, FIFO eviction, async wait"
```

---

## Task 4: Broker client

**Files:**
- Create: `tools/synapse-relay/src/broker.rs`

The broker client implements the Synapse wire protocol. Key facts:
- Frame header: 16 bytes (`[version:u8, flags:u8, msg_type:u8, encoding:u8, payload_len:u32_be, message_id:u64_be]`)
- Auth flow: Send `Hello` → receive `Challenge` (32-byte nonce) → send `HelloResp` (HMAC-SHA256 of nonce using `SYNAPSE_SECRET.as_bytes()`) → receive `HelloAck` or `HelloErr`
- Subscribe: Send `Subscribe` frame with channel name as payload → receive `SubscribeAck`
- Inbound messages: `Msg` frames; payload format: `[content_type:u8, channel_id:u64_be, timestamp_ms:u64_be, body...]`
  - `0x01` = Dialogue (UTF-8 body)
  - `0x02` = Work (MessagePack body, decode with rmpv)
- Send: Send `Msg` frame with same payload format
- Channel list: Send `ChanList` frame, receive response with newline-delimited channel names
- Presence: Send `PresenceReq` frame with channel name, receive `Presence` response

This task is **Rust-specialist territory**. Despatch a rust-engineer subagent with:
- This plan file path
- The proto source files: `synapse/crates/synapse-proto/src/frame.rs`, `auth.rs`, `message.rs`, `codec.rs`
- Instruction: implement `src/broker.rs` with `BrokerClient` struct providing: `connect()`, `subscribe(channel)`, `send_dialogue(channel_id, text)`, `send_work(channel_id, value)`, `read_message() -> Option<InboundMsg>`, `list_channels() -> Vec<String>`, `list_users(channel) -> Vec<String>`

- [ ] **Step 1: Write broker integration test skeleton**

```rust
// tests/relay_test.rs
// Mock broker: TCP listener that speaks the Synapse wire protocol minimally.
// Tests: connect+auth, subscribe, send dialogue, receive message.
// Full implementation in Task 5 (requires relay.rs). Skeleton only here.

#[cfg(test)]
mod broker_tests {
    // TODO: implement mock broker helper in Task 5
}
```

- [ ] **Step 2: Despatch rust-engineer subagent to implement broker.rs**

Provide context:
- Protocol source: `crates/synapse-proto/src/{frame.rs,auth.rs,message.rs,codec.rs}`
- Credentials passed as env: `SYNAPSE_AGENT`, `SYNAPSE_SECRET`
- TLS: use `tokio-native-tls`, accept self-signed (no cert validation for local fleet broker)
- `InboundMsg` type: `{ channel_id: u64, msg_type: MessageType, text: Option<String>, payload: Option<serde_json::Value> }`

- [ ] **Step 3: Verify broker.rs compiles**

```bash
cargo build 2>&1 | grep -E "error|warning: unused" | head -20
```

- [ ] **Step 4: Commit**

```bash
git add src/broker.rs tests/relay_test.rs
git commit -m "feat(synapse-relay): broker client — TLS connect, HMAC auth, frame I/O"
```

---

## Task 5: Channel registry (relay.rs)

**Files:**
- Create: `tools/synapse-relay/src/relay.rs`
- Extend: `tools/synapse-relay/tests/relay_test.rs`

The relay maintains a `HashMap<String, ChannelState>` where each entry holds a broker connection and a buffer. It drives the inbound message pump and reconnect loop.

- [ ] **Step 1: Write failing integration tests**

```rust
// tests/relay_test.rs
// Requires a mock broker. Implement a minimal mock: accepts TLS, sends
// HelloAck, responds to Subscribe with SubscribeAck, echoes back a test
// Msg frame when given a trigger.

use synapse_relay::relay::RelayRegistry;
use synapse_relay::config::Config;

#[tokio::test]
async fn test_subscribe_and_receive() {
    // Start mock broker on random port
    // Create RelayRegistry pointing at mock
    // Subscribe to "#test"
    // Mock sends a Dialogue message
    // Poll buffer — verify message appears
    todo!("implement after mock broker helper is written")
}

#[tokio::test]
async fn test_reconnect_on_disconnect() {
    // Mock disconnects after first message
    // Relay should reconnect within reconnect_delay_secs
    // Second message sent after reconnect should appear in buffer
    todo!()
}

#[tokio::test]
async fn test_auth_failure_marks_channel() {
    // Mock sends HelloErr instead of HelloAck
    // Relay marks channel auth_failed
    // status() returns connected=false for channel
    todo!()
}
```

- [ ] **Step 2: Implement relay.rs**

Despatch rust-engineer subagent with:
- `src/broker.rs` (just written)
- `src/buffer.rs`
- `src/config.rs`
- `src/types.rs`
- Instructions: implement `RelayRegistry` with `Arc<Mutex<...>>` interior, methods: `subscribe(channel)`, `leave(channel)`, `send_dialogue(channel, text)`, `send_work(channel, value)`, `poll(channel, since) -> Vec<BufferedMessage>`, `wait(channel, since, min, timeout_ms) -> (Vec<BufferedMessage>, bool)`, `list_channels() -> Vec<String>`, `list_users(channel) -> Vec<String>`, `status() -> Vec<ChannelStatus>`. Reconnect loop: exponential backoff using config values. Auth-fail: set `auth_failed` flag, stop retrying.

- [ ] **Step 3: Implement mock broker in test file**

Rust-engineer subagent: implement `MockBroker` helper in `tests/relay_test.rs`.

The mock broker is a minimal TCP server (no TLS needed for tests — use plain TCP and configure the relay client to accept no-TLS for test mode, or use `tokio::net::TcpListener` with a test flag). It must:

1. Accept a connection
2. Send a `Hello` frame (msg_type `0x01`) as a handshake prompt
3. Read the `HelloResp` from the client, verify the HMAC (use `compute_hmac` from proto auth module)
4. Send a `HelloAck` frame (`0x04`)
5. Read `Subscribe` frames (`0x20`) and reply with `SubscribeAck` (`0x26`)
6. Expose a `inject_dialogue(channel_id: u64, text: &str)` method that sends a `Msg` frame (`0x10`) with a `Dialogue` payload: `[0x01, channel_id:u64_be, timestamp_ms:u64_be, text_bytes...]`
7. Expose an `inject_work(channel_id: u64, value: rmpv::Value)` method that sends a `Msg` frame with a `Work` payload
8. Expose a `disconnect()` method that drops the connection to test reconnect

Frame format reminder: `[version:u8=0x01, flags:u8=0x00, msg_type:u8, encoding:u8=0x00, payload_len:u32_be, message_id:u64_be, ...payload_bytes]`

The mock can use `Arc<Mutex<TcpStream>>` to allow injection from a separate thread/task.

- [ ] **Step 4: Un-todo and run integration tests**

```bash
cargo test relay 2>&1 | tail -20
```
Expected: all 3 tests pass.

- [ ] **Step 5: Commit**

```bash
git add src/relay.rs tests/relay_test.rs
git commit -m "feat(synapse-relay): channel registry, reconnect loop, auth-fail hard-stop"
```

---

## Task 6: HTTP server (server.rs)

**Files:**
- Create: `tools/synapse-relay/src/server.rs`

Implement all axum route handlers. Inject `RelayRegistry` via `axum::extract::State`.

- [ ] **Step 1: Write handler unit tests (no network required)**

```rust
// Inline in server.rs or tests/server_test.rs
// Test each handler in isolation using a mock/stub RelayRegistry.
// Cover: 200 responses, 400 on missing params, 503 on broker disconnect.

#[tokio::test]
async fn test_send_missing_text_returns_400() { ... }

#[tokio::test]
async fn test_poll_empty_returns_empty_messages() { ... }

#[tokio::test]
async fn test_wait_timeout_returns_timed_out_true() { ... }
```

- [ ] **Step 2: Implement server.rs**

```rust
// src/server.rs
use axum::{Router, routing::{get, post}, extract::{State, Query}, Json};
use std::sync::Arc;
use crate::relay::RelayRegistry;
use crate::types::*;

pub type AppState = Arc<RelayRegistry>;

pub async fn run(bind: String, port: u16) -> anyhow::Result<()> {
    let config = crate::config::Config::load()?;
    config.validate()?;
    let registry = Arc::new(RelayRegistry::new(config));

    let app = Router::new()
        .route("/subscribe",  post(handle_subscribe))
        .route("/leave",      post(handle_leave))
        .route("/send",       post(handle_send))
        .route("/send_work",  post(handle_send_work))
        .route("/poll",       get(handle_poll))
        .route("/wait",       get(handle_wait))
        .route("/channels",   get(handle_channels))
        .route("/users",      get(handle_users))
        .route("/status",     get(handle_status))
        .with_state(registry);

    let addr = format!("{}:{}", bind, port);
    let listener = tokio::net::TcpListener::bind(&addr).await?;
    tracing::info!("synapse-relay listening on {}", addr);
    axum::serve(listener, app).await?;
    Ok(())
}

// --- handlers ---
// Each handler: validate input → call registry method → return JSON
// On relay error: return appropriate 400/503 JSON error shape
// See spec HTTP API section for exact request/response shapes.

async fn handle_subscribe(State(r): State<AppState>, Json(req): Json<SubscribeReq>)
    -> impl axum::response::IntoResponse { ... }

async fn handle_leave(State(r): State<AppState>, Json(req): Json<LeaveReq>)
    -> impl axum::response::IntoResponse { ... }

async fn handle_send(State(r): State<AppState>, Json(req): Json<SendReq>)
    -> impl axum::response::IntoResponse { ... }

async fn handle_send_work(State(r): State<AppState>, Json(req): Json<SendWorkReq>)
    -> impl axum::response::IntoResponse { ... }

#[derive(serde::Deserialize)]
struct PollQuery { channel: Option<String>, since: Option<u64> }

async fn handle_poll(State(r): State<AppState>, Query(q): Query<PollQuery>)
    -> impl axum::response::IntoResponse { ... }

#[derive(serde::Deserialize)]
struct WaitQuery { channel: Option<String>, since: Option<u64>, min: Option<usize>, timeout: Option<u64> }

async fn handle_wait(State(r): State<AppState>, Query(q): Query<WaitQuery>)
    -> impl axum::response::IntoResponse { ... }

async fn handle_channels(State(r): State<AppState>)
    -> impl axum::response::IntoResponse { ... }

#[derive(serde::Deserialize)]
struct UsersQuery { channel: Option<String> }

async fn handle_users(State(r): State<AppState>, Query(q): Query<UsersQuery>)
    -> impl axum::response::IntoResponse { ... }

async fn handle_status(State(r): State<AppState>)
    -> impl axum::response::IntoResponse { ... }
```

Despatch rust-engineer subagent to fill in all handler bodies with correct JSON responses and error handling.

- [ ] **Step 3: Run server unit tests**

```bash
cargo test server 2>&1 | tail -15
```

- [ ] **Step 4: Smoke test: start relay, hit /status**

```bash
SYNAPSE_AGENT=lex SYNAPSE_SECRET=test cargo run -- serve &
sleep 1
curl -s http://127.0.0.1:7779/status | jq .
kill %1
```
Expected: `{"channels":[]}`

- [ ] **Step 5: Commit**

```bash
git add src/server.rs && git commit -m "feat(synapse-relay): axum HTTP server, all route handlers"
```

---

## Task 7: CLI commands

**Files:**
- Modify: `tools/synapse-relay/src/main.rs` (fill in CLI command implementations)

CLI commands that target the running daemon call the relay HTTP API. `say` auto-starts a connection if relay is not running (or error clearly).

- [ ] **Step 1: Wire up all CLI subcommands in main.rs**

Each subcommand calls the relay HTTP API:
- `channels` → `GET /channels`, print one per line
- `users <#chan>` → `GET /users?channel=...`, print one per line
- `join <#chan>` → `POST /subscribe`
- `leave <#chan>` → `POST /leave`
- `say <#chan> <words...>` → `POST /send` with `text: message.join(" ")`
- `tail <#chan> [--count N]` → `GET /poll`, take last N, print `text` fields
- `status` → `GET /status`, print table

Use `reqwest` (add to Cargo.toml) for HTTP calls. On connection refused: print `"synapse-relay is not running. Start with: synapse-relay serve"` and exit 1.

- [ ] **Step 2: Add reqwest to Cargo.toml**

```toml
reqwest = { version = "0.12", features = ["json", "blocking"] }
```

- [ ] **Step 3: Test CLI commands manually**

```bash
# Start relay in background
SYNAPSE_AGENT=lex SYNAPSE_SECRET=test cargo run -- serve &

cargo run -- status
cargo run -- channels
cargo run -- say "#body-harvest" "test message from relay CLI"

kill %1
```

- [ ] **Step 4: Commit**

```bash
git add src/main.rs Cargo.toml && git commit -m "feat(synapse-relay): CLI subcommands — channels, users, join, leave, say, tail, status"
```

---

## Task 8: Install script and systemd unit

**Files:**
- Create: `tools/synapse-relay/install.sh`

- [ ] **Step 1: Write install.sh**

```bash
#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")"

echo "Building synapse-relay (release)..."
cargo build --release

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
```

Note: `EnvironmentFile=-%h/.config/synapse-relay/env` (the `-` prefix makes it optional). Create `~/.config/synapse-relay/env` with:
```
SYNAPSE_AGENT=lex
SYNAPSE_SECRET=<value from lex-secret>
```

- [ ] **Step 2: Run install**

```bash
chmod +x tools/synapse-relay/install.sh && tools/synapse-relay/install.sh
```

- [ ] **Step 3: Create env file with real credentials**

```bash
mkdir -p ~/.config/synapse-relay
# Get secret from lex-secret
SYNAPSE_AGENT=lex
SYNAPSE_SECRET=$(lex-secret get synapse_secret 2>/dev/null || echo "SET_ME")
printf "SYNAPSE_AGENT=%s\nSYNAPSE_SECRET=%s\n" "$SYNAPSE_AGENT" "$SYNAPSE_SECRET" > ~/.config/synapse-relay/env
```

- [ ] **Step 4: Start and verify**

```bash
systemctl --user start synapse-relay
sleep 2
systemctl --user status synapse-relay
curl -s http://127.0.0.1:7779/status | jq .
```

- [ ] **Step 5: Commit**

```bash
git add tools/synapse-relay/install.sh
git commit -m "feat(synapse-relay): install script, systemd user unit with env file"
```

---

## Task 9: MCP server — relay client

**Files:**
<!-- IDENTITY-EXCEPTION: functional internal reference — not for public exposure -->
- Create: `~/.claude/plugins/synapse/src/relay-client.ts`
<!-- IDENTITY-EXCEPTION: functional internal reference — not for public exposure -->
- Delete: `~/.claude/plugins/synapse/src/buffer.ts`

The relay client is a thin wrapper around `fetch`. It manages `nextSeq` per channel in a `Map<string, number>`.

- [ ] **Step 1: Write relay-client.ts**

```typescript
// src/relay-client.ts
const RELAY_URL = process.env.SYNAPSE_RELAY_URL ?? "http://127.0.0.1:7779";

export interface RelayMessage {
  seq: number;
  type: "dialogue" | "work";
  text: string | null;
  payload: unknown | null;
  received_at: number;
}

interface PollResp  { messages: RelayMessage[]; next_seq: number }
interface WaitResp  { messages: RelayMessage[]; next_seq: number; timed_out: boolean }

const nextSeq = new Map<string, number>();

function getSeq(channel: string): number { return nextSeq.get(channel) ?? 0; }
function setSeq(channel: string, seq: number) { nextSeq.set(channel, seq); }

async function relayFetch(path: string, init?: RequestInit) {
  const res = await fetch(`${RELAY_URL}${path}`, init);
  if (!res.ok) {
    const body = await res.json().catch(() => ({ error: res.statusText }));
    throw new Error((body as { error?: string }).error ?? res.statusText);
  }
  return res.json();
}

export async function checkRelay(): Promise<boolean> {
  try { await relayFetch("/status"); return true; }
  catch { return false; }
}

export async function send(channel: string, text: string): Promise<void> {
  await relayFetch("/send", {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify({ channel, text }),
  });
}

export async function sendWork(channel: string, payload: unknown): Promise<void> {
  await relayFetch("/send_work", {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify({ channel, payload }),
  });
}

export async function poll(channel: string): Promise<RelayMessage[]> {
  const since = getSeq(channel);
  const resp: PollResp = await relayFetch(`/poll?channel=${encodeURIComponent(channel)}&since=${since}`);
  setSeq(channel, resp.next_seq);
  return resp.messages;
}

export async function wait(
  channel: string, min = 1, timeoutSecs = 30
): Promise<{ messages: RelayMessage[]; timedOut: boolean }> {
  const since = getSeq(channel);
  const resp: WaitResp = await relayFetch(
    `/wait?channel=${encodeURIComponent(channel)}&since=${since}&min=${min}&timeout=${timeoutSecs}`
  );
  setSeq(channel, resp.next_seq);
  return { messages: resp.messages, timedOut: resp.timed_out };
}

export async function subscribe(channel: string): Promise<void> {
  await relayFetch("/subscribe", {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify({ channel }),
  });
}

export async function leave(channel: string): Promise<void> {
  await relayFetch("/leave", {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify({ channel }),
  });
}

export async function listChannels(): Promise<string[]> {
  const resp: { channels: string[] } = await relayFetch("/channels");
  return resp.channels;
}

export async function listUsers(channel: string): Promise<string[]> {
  const resp: { users: string[] } = await relayFetch(`/users?channel=${encodeURIComponent(channel)}`);
  return resp.users;
}
```

- [ ] **Step 2: Delete buffer.ts**

```bash
<!-- IDENTITY-EXCEPTION: functional internal reference — not for public exposure -->
rm ~/.claude/plugins/synapse/src/buffer.ts
```

- [ ] **Step 3: Verify TypeScript compiles**

```bash
<!-- IDENTITY-EXCEPTION: functional internal reference — not for public exposure -->
cd ~/.claude/plugins/synapse && npx tsc --noEmit 2>&1 | head -20
```
Expected: errors only for files importing buffer.ts (fixed in next tasks).

- [ ] **Step 4: Commit**

```bash
git add src/relay-client.ts && git commit -m "feat(synapse-mcp): relay client — HTTP wrapper, per-channel seq tracking"
```

---

## Task 10: MCP server — update existing tools

**Files:**
<!-- IDENTITY-EXCEPTION: functional internal reference — not for public exposure -->
- Modify: `~/.claude/plugins/synapse/src/tools/send.ts`
<!-- IDENTITY-EXCEPTION: functional internal reference — not for public exposure -->
- Modify: `~/.claude/plugins/synapse/src/tools/listen.ts`
<!-- IDENTITY-EXCEPTION: functional internal reference — not for public exposure -->
- Modify: `~/.claude/plugins/synapse/src/cli.ts`

- [ ] **Step 1: Update send.ts**

Replace `runOnce` import and call with relay client:

```typescript
// src/tools/send.ts
import { z } from "zod";
import { validateCredentials } from "../cli.js";
import * as relay from "../relay-client.js";

export const SendMessageSchema = z.object({
  channel: z.string().trim().min(1),
  message: z.string().trim().min(1),
});

export const sendMessageTool = { /* unchanged */ };

export async function handleSendMessage(args: unknown): Promise<string> {
  const credErr = validateCredentials();
  if (credErr) throw new Error(credErr);
  const { channel, message } = SendMessageSchema.parse(args);
  await relay.send(channel, message);
  return "Delivered.";
}
```

- [ ] **Step 2: Update listen.ts**

Replace buffer/subprocess with relay client calls:

```typescript
// src/tools/listen.ts
import { z } from "zod";
import { validateCredentials } from "../cli.js";
import * as relay from "../relay-client.js";

export const ListenPollSchema = z.object({
  channel: z.string(),
  timeout_seconds: z.number().int().min(1).max(120).default(5),
  since: z.number().int().min(0).optional(),
});

export const WaitForReplySchema = z.object({
  channel: z.string(),
  timeout_seconds: z.number().int().min(1).max(300).default(30),
  min_messages: z.number().int().min(1).max(1000).default(1),
  since: z.number().int().min(0).optional(),
});

// Tool schema definitions — add since?: number to inputSchema for both tools.

export async function handleListenPoll(args: unknown): Promise<string> {
  const credErr = validateCredentials(); if (credErr) throw new Error(credErr);
  const { channel, timeout_seconds } = ListenPollSchema.parse(args);
  // Use wait() — it returns immediately if messages are already buffered (since tracks position),
  // otherwise long-polls up to timeout. Do NOT call poll() first: it would advance nextSeq
  // and the subsequent wait() would miss those messages.
  const result = await relay.wait(channel, 1, timeout_seconds);
  return JSON.stringify(result.messages.map(m => m.text ?? m.payload));
}

export async function handleWaitForReply(args: unknown): Promise<string> {
  const credErr = validateCredentials(); if (credErr) throw new Error(credErr);
  const { channel, timeout_seconds, min_messages } = WaitForReplySchema.parse(args);
  const result = await relay.wait(channel, min_messages, timeout_seconds);
  return JSON.stringify({ timedOut: result.timedOut, messages: result.messages.map(m => m.text ?? m.payload) });
}
```

- [ ] **Step 3: Trim cli.ts — remove runWithTimeout**

`runWithTimeout` is no longer needed. Keep `validateCredentials` and `runOnce` (used by history tool still). Remove the rest.

- [ ] **Step 4: Verify TypeScript compiles**

```bash
<!-- IDENTITY-EXCEPTION: functional internal reference — not for public exposure -->
cd ~/.claude/plugins/synapse && npx tsc --noEmit 2>&1 | head -20
```

- [ ] **Step 5: Commit**

```bash
git add src/tools/send.ts src/tools/listen.ts src/cli.ts
git commit -m "feat(synapse-mcp): wire send and listen tools through relay client"
```

---

## Task 11: MCP server — new tools

**Files:**
<!-- IDENTITY-EXCEPTION: functional internal reference — not for public exposure -->
- Create: `~/.claude/plugins/synapse/src/tools/chat.ts`
<!-- IDENTITY-EXCEPTION: functional internal reference — not for public exposure -->
- Create: `~/.claude/plugins/synapse/src/tools/work.ts`
<!-- IDENTITY-EXCEPTION: functional internal reference — not for public exposure -->
- Create: `~/.claude/plugins/synapse/src/tools/channels.ts`
<!-- IDENTITY-EXCEPTION: functional internal reference — not for public exposure -->
- Delete: `~/.claude/plugins/synapse/src/tools/stubs.ts`
<!-- IDENTITY-EXCEPTION: functional internal reference — not for public exposure -->
- Modify: `~/.claude/plugins/synapse/src/index.ts`

- [ ] **Step 1: Create chat.ts**

```typescript
// src/tools/chat.ts
import { z } from "zod";
import { validateCredentials } from "../cli.js";
import * as relay from "../relay-client.js";

const ChatSchema = z.object({ channel: z.string(), message: z.string() });

export const chatTool = {
  name: "synapse_chat",
  description: "Send a message and immediately poll for replies. Returns { sent, replies }.",
  inputSchema: {
    type: "object" as const,
    properties: {
      channel: { type: "string" },
      message: { type: "string" },
    },
    required: ["channel", "message"],
  },
};

export async function handleChat(args: unknown): Promise<string> {
  const credErr = validateCredentials(); if (credErr) throw new Error(credErr);
  const { channel, message } = ChatSchema.parse(args);
  await relay.send(channel, message);
  const messages = await relay.poll(channel);
  return JSON.stringify({ sent: true, replies: messages.map(m => m.text ?? m.payload) });
}
```

- [ ] **Step 2: Create work.ts**

```typescript
// src/tools/work.ts
import { z } from "zod";
import { validateCredentials } from "../cli.js";
import * as relay from "../relay-client.js";

const SendWorkSchema = z.object({ channel: z.string(), payload: z.record(z.unknown()) });

export const sendWorkTool = {
  name: "synapse_send_work",
  description: "Send a structured Work (MessagePack) frame. payload is any JSON object.",
  inputSchema: {
    type: "object" as const,
    properties: {
      channel: { type: "string" },
      payload: { type: "object" },
    },
    required: ["channel", "payload"],
  },
};

export async function handleSendWork(args: unknown): Promise<string> {
  const credErr = validateCredentials(); if (credErr) throw new Error(credErr);
  const { channel, payload } = SendWorkSchema.parse(args);
  await relay.sendWork(channel, payload);
  return JSON.stringify({ ok: true });
}
```

- [ ] **Step 3: Create channels.ts**

```typescript
// src/tools/channels.ts
import { z } from "zod";
import { validateCredentials } from "../cli.js";
import * as relay from "../relay-client.js";

const ChannelSchema = z.object({ channel: z.string() });

export const listChannelsTool = {
  name: "synapse_list_channels",
  description: "List all channels on the Synapse broker.",
  inputSchema: { type: "object" as const, properties: {}, required: [] },
};

export const listUsersTool = {
  name: "synapse_list_users",
  description: "List users currently in a channel.",
  inputSchema: { type: "object" as const, properties: { channel: { type: "string" } }, required: ["channel"] },
};

export const joinTool = {
  name: "synapse_join",
  description: "Subscribe relay to a channel (auto-triggered on send/poll too).",
  inputSchema: { type: "object" as const, properties: { channel: { type: "string" } }, required: ["channel"] },
};

export const leaveTool = {
  name: "synapse_leave",
  description: "Unsubscribe relay from a channel, discard buffer.",
  inputSchema: { type: "object" as const, properties: { channel: { type: "string" } }, required: ["channel"] },
};

export async function handleListChannels(_args: unknown): Promise<string> {
  const credErr = validateCredentials(); if (credErr) throw new Error(credErr);
  return JSON.stringify({ channels: await relay.listChannels() });
}

export async function handleListUsers(args: unknown): Promise<string> {
  const credErr = validateCredentials(); if (credErr) throw new Error(credErr);
  const { channel } = ChannelSchema.parse(args);
  return JSON.stringify({ users: await relay.listUsers(channel) });
}

export async function handleJoin(args: unknown): Promise<string> {
  const credErr = validateCredentials(); if (credErr) throw new Error(credErr);
  const { channel } = ChannelSchema.parse(args);
  await relay.subscribe(channel);
  return JSON.stringify({ ok: true });
}

export async function handleLeave(args: unknown): Promise<string> {
  const credErr = validateCredentials(); if (credErr) throw new Error(credErr);
  const { channel } = ChannelSchema.parse(args);
  await relay.leave(channel);
  return JSON.stringify({ ok: true });
}
```

- [ ] **Step 4: Update index.ts**

Register all new tools, remove stubs import, add relay health check on startup:

```typescript
// src/index.ts  (relevant changes)
import { checkRelay } from "./relay-client.js";
import { chatTool, handleChat } from "./tools/chat.js";
import { sendWorkTool, handleSendWork } from "./tools/work.js";
import { listChannelsTool, listUsersTool, joinTool, leaveTool,
         handleListChannels, handleListUsers, handleJoin, handleLeave } from "./tools/channels.js";

// On startup — non-fatal warning if relay is down
const relayOk = await checkRelay();
if (!relayOk) {
  process.stderr.write("[synapse-mcp] WARNING: synapse-relay not reachable at " +
    (process.env.SYNAPSE_RELAY_URL ?? "http://127.0.0.1:7779") +
    ". Start with: synapse-relay serve\n");
}

// Add to tools array and switch cases...
```

- [ ] **Step 5: Delete stubs.ts**

```bash
<!-- IDENTITY-EXCEPTION: functional internal reference — not for public exposure -->
rm ~/.claude/plugins/synapse/src/tools/stubs.ts
```

- [ ] **Step 6: Full TypeScript compile**

```bash
<!-- IDENTITY-EXCEPTION: functional internal reference — not for public exposure -->
cd ~/.claude/plugins/synapse && npx tsc 2>&1 | head -20
```
Expected: clean.

- [ ] **Step 7: Build TypeScript**

```bash
<!-- IDENTITY-EXCEPTION: functional internal reference — not for public exposure -->
cd ~/.claude/plugins/synapse && npm run build 2>&1 | tail -10
```
Expected: clean build producing `dist/` files. Fix any type errors before proceeding.

- [ ] **Step 8: Commit**

```bash
git add src/tools/chat.ts src/tools/work.ts src/tools/channels.ts src/index.ts dist/
git rm src/tools/stubs.ts
git commit -m "feat(synapse-mcp): add chat, send_work, list_users, join, leave tools; remove stubs"
```

---

## Task 12: End-to-end smoke test and cleanup

- [ ] **Step 1: Ensure relay is running**

```bash
systemctl --user status synapse-relay || systemctl --user start synapse-relay
curl -s http://127.0.0.1:7779/status | jq .
```

- [ ] **Step 2: Restart Meridian Lex MCP session**

The new `dist/index.js` won't load until the MCP server process restarts. Signal to the Admiral that a session restart is needed.

- [ ] **Step 3: Smoke test via MCP tools**

In new session:
1. `synapse_list_channels` → should return broker channels
2. `synapse_send_message` with channel `#body-harvest` and a test message
3. `synapse_listen_poll` with `#body-harvest` → should return buffered messages
4. `synapse_chat` with `#body-harvest` → send + poll in one call
5. `synapse_list_users` with `#body-harvest`

- [ ] **Step 4: Test message persistence across poll gap**

1. Send a message via `synapse_send_message`
2. Wait 10 seconds (don't poll)
3. Call `synapse_listen_poll` — message should still be in buffer

- [ ] **Step 5: Push the branch**

```bash
cd /home/meridian/meridian-home/projects/synapse/mcp
git push origin fix/synapse-mcp-plugin-json
```

- [ ] **Step 6: Open PR**

```bash
gh pr create --repo Meridian-Lex/synapse-mcp \
  --head Meridian-Lex:fix/synapse-mcp-plugin-json \
  --title "feat: synapse-relay sidecar — persistent broker connections, zero message loss" \
  --body "$(cat <<'EOF'
## Summary
- Adds synapse-relay Rust daemon: persistent TLS connections per channel, ring buffer, local HTTP API on localhost:7779
- Rewrites MCP server as thin relay client — no more per-call subprocess spawning, no message loss between tool calls
- New MCP tools: synapse_chat, synapse_send_work, synapse_list_users, synapse_join, synapse_leave
- IRC-tier CLI: synapse-relay channels/users/join/leave/say/tail/status
- systemd user unit for fleet deployment

## Test plan
- [ ] All relay unit tests pass (buffer, broker, server)
- [ ] Integration tests pass (mock broker)
- [ ] End-to-end smoke test: send → poll → message received
- [ ] Message persists in buffer between poll calls
- [ ] Relay reconnects after broker disconnect
EOF
)"
```

---

## Notes for Rust agents

- The broker protocol is in `synapse/crates/synapse-proto/`. Do not duplicate the frame/auth logic — read those files and re-implement the same protocol in the relay.
- Use `tokio::io::AsyncReadExt`/`AsyncWriteExt` for frame I/O, not std blocking I/O.
- The broker's TLS cert is self-signed; use `native_tls::TlsConnector::builder().danger_accept_invalid_certs(true)` for local fleet connections.
- `SYNAPSE_SECRET` is used as the HMAC key directly: `compute_hmac(secret.as_bytes(), &nonce)`.
- Channel names in subscribe frames are sent as UTF-8 payload bytes.
- Use `rmpv::decode::read_value` for Work payload decoding, then `rmpv_to_json` (implement a small conversion: `Value::Map` → `serde_json::Value::Object`, etc.).
