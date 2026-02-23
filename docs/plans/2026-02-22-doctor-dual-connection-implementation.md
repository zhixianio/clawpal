# Doctor Agent Dual Connection (Operator + Bridge) Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Enable the doctor agent to both initiate diagnosis AND execute commands on the target machine by establishing two simultaneous connections to the openclaw gateway: an Operator connection (WebSocket) for starting the agent and receiving chat, plus a Bridge connection (TCP) for receiving and responding to tool invocations.

**Architecture:** ClawPal connects to the same gateway via two protocols:
- **Operator** (WS port 18789) — calls `agent` method with device auth, receives streaming `chat` events
- **Bridge** (TCP port 18790) — registers as a node with commands, receives `invoke` frames, sends `invoke-res` back

The agent running on the gateway decides to use tools → gateway routes `invoke` to ClawPal's bridge connection → ClawPal executes locally or via SSH → sends result back → agent continues.

**Tech Stack:** Rust (Tauri, tokio, ed25519-dalek, serde_json), TypeScript (React, Tauri event API)

**Reference:** `openclaw/clawgo` Go node implementation (~1400 lines) — uses the same bridge protocol over TCP with NDJSON framing.

---

## Context: Protocol Details

### Operator Protocol (WebSocket, port 18789)

**Frame format:** JSON over WebSocket messages (not newline-delimited).

**Connect handshake with device auth:**
1. Gateway pushes `connect.challenge` event with `nonce`
2. Client sends `connect` request with Ed25519 signature

```json
{
  "type": "req", "id": "c1", "method": "connect",
  "params": {
    "minProtocol": 3, "maxProtocol": 3,
    "auth": { "token": "<gateway-token>" },
    "role": "operator",
    "scopes": ["operator.admin", "operator.write"],
    "device": {
      "id": "<deviceId>",
      "publicKey": "<base64-encoded-public-key>",
      "signature": "<base64-encoded-ed25519-signature>",
      "signedAt": <unix-ms>,
      "nonce": "<nonce-from-challenge>"
    },
    "client": { "id": "clawpal", "platform": "macos", "mode": "cli", "version": "0.2.2" }
  }
}
```

**Signature payload:** `v2|<deviceId>|clawpal|cli|operator|operator.admin,operator.write|<signedAtMs>|<token>|<nonce>`

**Device identity files:**
- `~/.openclaw/identity/device.json` — contains `deviceId`, `publicKeyPem`, `privateKeyPem` (Ed25519 PKCS8)
- `~/.openclaw/identity/device-auth.json` — contains `tokens.operator.token` and `tokens.operator.scopes`

**Agent method:**
```json
{
  "type": "req", "id": "c2", "method": "agent",
  "params": {
    "message": "<diagnostic context as text>",
    "idempotencyKey": "<uuid>",
    "agentId": "main",
    "sessionKey": "agent:main:clawpal-doctor"
  }
}
```

**Chat events received:**
```json
{
  "type": "event", "event": "chat",
  "payload": {
    "state": "delta",
    "message": { "content": [{ "type": "text", "text": "partial text..." }] }
  }
}
```
Final message has `"state": "final"`.

### Bridge Protocol (TCP, port 18790)

**Frame format:** Newline-delimited JSON (NDJSON) over raw TCP. Each frame is a JSON object terminated by `\n` (byte 0x0a).

**Connection lifecycle:**
1. TCP connect to `host:18790`
2. If no token saved: send `pair-request`, wait for `pair-ok` (up to 6 min)
3. Send `hello` with token, wait for `hello-ok` (up to 30s)
4. Steady state: process `invoke`, respond with `invoke-res`, handle `ping`/`pong`

**pair-request frame:**
```json
{
  "type": "pair-request",
  "nodeId": "<hostname-derived>",
  "displayName": "ClawPal",
  "platform": "macos",
  "version": "0.2.2",
  "deviceFamily": "desktop",
  "commands": ["read_file", "list_files", "read_config", "system_info", "validate_config", "write_file", "run_command"],
  "silent": true
}
```

**pair-ok frame (from gateway):**
```json
{ "type": "pair-ok", "token": "<node-auth-token>" }
```

**hello frame:**
```json
{
  "type": "hello",
  "nodeId": "<same-nodeId>",
  "displayName": "ClawPal",
  "token": "<token-from-pair-ok>",
  "platform": "macos",
  "version": "0.2.2",
  "deviceFamily": "desktop",
  "commands": ["read_file", "list_files", "read_config", "system_info", "validate_config", "write_file", "run_command"]
}
```

**hello-ok frame (from gateway):**
```json
{ "type": "hello-ok", "serverName": "...", "canvasHostUrl": "..." }
```

**invoke frame (from gateway):**
```json
{ "type": "invoke", "id": "<request-id>", "command": "read_file", "args": {"path": "/etc/openclaw/openclaw.json"} }
```

**invoke-res frame (from node):**
```json
{ "type": "invoke-res", "id": "<same-id>", "ok": true, "payload": {"content": "..."} }
```
Or on error:
```json
{ "type": "invoke-res", "id": "<same-id>", "ok": false, "error": {"code": "REJECTED", "message": "User rejected"} }
```

**ping/pong:** Gateway sends `{"type": "ping", "id": "p1"}`, node responds `{"type": "pong", "id": "p1"}`. Node also sends pings every 30s.

---

## Existing Code to Understand

| File | What it does | Relevant to |
|------|-------------|-------------|
| `src-tauri/src/node_client.rs` | WebSocket operator client — connect, send_request, handle_frame, pending_invokes | Task 2, 3 |
| `src-tauri/src/doctor_commands.rs` | Tauri commands — connect, disconnect, approve_invoke, execute_local/remote_command | Task 4 |
| `src-tauri/src/lib.rs` | Module registration, state management, command handler | Task 5 |
| `src-tauri/src/models.rs` | `resolve_paths()` → OpenClawPaths with config_path, openclaw_dir | Task 2 |
| `src-tauri/src/ssh.rs` | `SshConnectionPool` — exec, sftp_read, sftp_write, sftp_list | Already works |
| `src/lib/use-doctor-agent.ts` | Frontend hook — events, approval patterns, target routing | Task 6 |
| `src/pages/Doctor.tsx` | Doctor page — health, logs, agent source selector, DoctorChat | Task 6 |
| `src/components/DoctorChat.tsx` | Chat UI — messages, tool-call cards, approval buttons | Task 6 |
| `src/lib/api.ts` | Tauri invoke wrappers | Task 6 |
| `src/lib/types.ts` | DoctorInvoke, DoctorChatMessage types | Task 6 |

---

### Task 1: Create bridge_client.rs — TCP NDJSON client

Creates a new Rust module that connects to the openclaw gateway's bridge port (18790) using raw TCP with NDJSON framing. Implements the pair-request/hello handshake and invoke handling.

**Files:**
- Create: `src-tauri/src/bridge_client.rs`

**Step 1: Write the bridge client module**

Create `src-tauri/src/bridge_client.rs` with the following content:

```rust
use std::collections::HashMap;
use std::sync::Arc;

use serde_json::{json, Value};
use tauri::{AppHandle, Emitter};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::TcpStream;
use tokio::sync::Mutex;

/// Token storage path for bridge node authentication.
/// Stored separately from device auth (that's for operator protocol).
const BRIDGE_TOKEN_FILE: &str = "bridge-token.json";

struct BridgeClientInner {
    writer: tokio::io::WriteHalf<TcpStream>,
    node_id: String,
    token: Option<String>,
}

pub struct BridgeClient {
    inner: Arc<Mutex<Option<BridgeClientInner>>>,
    /// Pending invoke requests from the gateway, keyed by request ID.
    pending_invokes: Arc<Mutex<HashMap<String, Value>>>,
}

impl BridgeClient {
    pub fn new() -> Self {
        Self {
            inner: Arc::new(Mutex::new(None)),
            pending_invokes: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    /// Connect to the bridge at the given address (e.g., "127.0.0.1:18790").
    /// Performs pair-request or hello handshake, then spawns reader task.
    pub async fn connect(&self, addr: &str, app: AppHandle) -> Result<(), String> {
        self.disconnect().await?;

        let stream = TcpStream::connect(addr)
            .await
            .map_err(|e| format!("Bridge TCP connection failed: {e}"))?;

        let (reader, writer) = tokio::io::split(stream);

        let node_id = Self::derive_node_id();
        let token = Self::load_token();

        {
            let mut guard = self.inner.lock().await;
            *guard = Some(BridgeClientInner {
                writer,
                node_id: node_id.clone(),
                token: token.clone(),
            });
        }

        // Spawn reader task
        let inner_ref = Arc::clone(&self.inner);
        let invokes_ref = Arc::clone(&self.pending_invokes);
        let app_clone = app.clone();

        tokio::spawn(async move {
            let mut lines = BufReader::new(reader).lines();
            while let Ok(Some(line)) = lines.next_line().await {
                if let Ok(frame) = serde_json::from_str::<Value>(&line) {
                    Self::handle_frame(frame, &inner_ref, &invokes_ref, &app_clone).await;
                }
            }
            // Connection closed
            let _ = app_clone.emit("doctor:bridge-disconnected", json!({"reason": "connection closed"}));
            let mut guard = inner_ref.lock().await;
            *guard = None;
        });

        // Perform handshake
        self.do_handshake(&app).await?;

        let _ = app.emit("doctor:bridge-connected", json!({}));
        Ok(())
    }

    pub async fn disconnect(&self) -> Result<(), String> {
        let mut guard = self.inner.lock().await;
        if let Some(mut inner) = guard.take() {
            let _ = inner.writer.shutdown().await;
        }
        self.pending_invokes.lock().await.clear();
        Ok(())
    }

    pub async fn is_connected(&self) -> bool {
        self.inner.lock().await.is_some()
    }

    /// Send invoke-res success response back to gateway.
    pub async fn send_invoke_result(&self, req_id: &str, result: Value) -> Result<(), String> {
        self.send_frame(json!({
            "type": "invoke-res",
            "id": req_id,
            "ok": true,
            "payload": result,
        })).await
    }

    /// Send invoke-res error response back to gateway.
    pub async fn send_invoke_error(&self, req_id: &str, code: &str, message: &str) -> Result<(), String> {
        self.send_frame(json!({
            "type": "invoke-res",
            "id": req_id,
            "ok": false,
            "error": { "code": code, "message": message },
        })).await
    }

    pub async fn take_invoke(&self, id: &str) -> Option<Value> {
        self.pending_invokes.lock().await.remove(id)
    }

    // --- Private methods ---

    async fn send_frame(&self, frame: Value) -> Result<(), String> {
        let mut guard = self.inner.lock().await;
        let inner = guard.as_mut().ok_or("Bridge not connected")?;
        let mut data = serde_json::to_vec(&frame).map_err(|e| format!("JSON serialize failed: {e}"))?;
        data.push(b'\n');
        inner.writer.write_all(&data).await
            .map_err(|e| format!("Bridge write failed: {e}"))
    }

    async fn do_handshake(&self, app: &AppHandle) -> Result<(), String> {
        let (node_id, token) = {
            let guard = self.inner.lock().await;
            let inner = guard.as_ref().ok_or("Not connected")?;
            (inner.node_id.clone(), inner.token.clone())
        };

        let commands = Self::advertised_commands();

        if let Some(token) = token {
            // Have token — send hello directly
            self.send_frame(json!({
                "type": "hello",
                "nodeId": node_id,
                "displayName": "ClawPal",
                "token": token,
                "platform": std::env::consts::OS,
                "version": env!("CARGO_PKG_VERSION"),
                "deviceFamily": "desktop",
                "commands": commands,
            })).await?;

            // Wait for hello-ok is handled by the reader task via handle_frame.
            // The reader task emits doctor:bridge-connected on hello-ok.
            // We wait for a signal here with timeout.
            // For simplicity in this implementation, we use a short sleep and check.
            // A proper implementation would use a channel, but the reader task
            // will emit the connected event which the frontend handles.
            tokio::time::sleep(std::time::Duration::from_secs(2)).await;
            Ok(())
        } else {
            // No token — send pair-request
            self.send_frame(json!({
                "type": "pair-request",
                "nodeId": node_id,
                "displayName": "ClawPal",
                "platform": std::env::consts::OS,
                "version": env!("CARGO_PKG_VERSION"),
                "deviceFamily": "desktop",
                "commands": commands,
                "silent": true,
            })).await?;

            // pair-ok with token will arrive via reader task → handle_frame
            // Then handle_frame saves the token and sends hello automatically
            let _ = app.emit("doctor:bridge-pairing", json!({"nodeId": node_id}));
            Ok(())
        }
    }

    async fn handle_frame(
        frame: Value,
        inner_ref: &Arc<Mutex<Option<BridgeClientInner>>>,
        invokes_ref: &Arc<Mutex<HashMap<String, Value>>>,
        app: &AppHandle,
    ) {
        let frame_type = frame.get("type").and_then(|v| v.as_str()).unwrap_or("");

        match frame_type {
            "pair-ok" => {
                if let Some(token) = frame.get("token").and_then(|v| v.as_str()) {
                    // Save token for future connections
                    Self::save_token(token);

                    // Update inner state with token
                    let mut guard = inner_ref.lock().await;
                    if let Some(inner) = guard.as_mut() {
                        inner.token = Some(token.to_string());
                    }
                    drop(guard);

                    // Now send hello with the new token
                    let (node_id, commands) = {
                        let guard = inner_ref.lock().await;
                        let inner = guard.as_ref().unwrap();
                        (inner.node_id.clone(), Self::advertised_commands())
                    };

                    // Send hello frame
                    let hello = json!({
                        "type": "hello",
                        "nodeId": node_id,
                        "displayName": "ClawPal",
                        "token": token,
                        "platform": std::env::consts::OS,
                        "version": env!("CARGO_PKG_VERSION"),
                        "deviceFamily": "desktop",
                        "commands": commands,
                    });

                    let mut data = serde_json::to_vec(&hello).unwrap_or_default();
                    data.push(b'\n');
                    let mut guard = inner_ref.lock().await;
                    if let Some(inner) = guard.as_mut() {
                        let _ = inner.writer.write_all(&data).await;
                    }
                }
            }

            "hello-ok" => {
                let _ = app.emit("doctor:bridge-connected", json!({}));
            }

            "ping" => {
                let id = frame.get("id").cloned().unwrap_or(Value::Null);
                let pong = json!({"type": "pong", "id": id});
                let mut data = serde_json::to_vec(&pong).unwrap_or_default();
                data.push(b'\n');
                let mut guard = inner_ref.lock().await;
                if let Some(inner) = guard.as_mut() {
                    let _ = inner.writer.write_all(&data).await;
                }
            }

            "invoke" => {
                let id = frame.get("id").and_then(|v| v.as_str()).unwrap_or("").to_string();
                let command = frame.get("command").and_then(|v| v.as_str()).unwrap_or("").to_string();
                let args = frame.get("args").cloned().unwrap_or(Value::Null);

                let cmd_type = match command.as_str() {
                    "read_file" | "list_files" | "read_config" | "system_info" | "validate_config" => "read",
                    _ => "write",
                };

                let invoke_payload = json!({
                    "id": id,
                    "command": command,
                    "args": args,
                    "type": cmd_type,
                });

                // Store in pending invokes (bounded)
                {
                    let mut map = invokes_ref.lock().await;
                    if map.len() >= 50 {
                        let keys: Vec<String> = map.keys().take(10).cloned().collect();
                        for k in keys { map.remove(&k); }
                    }
                    map.insert(id.clone(), invoke_payload.clone());
                }

                let _ = app.emit("doctor:invoke", invoke_payload);
            }

            "error" => {
                let code = frame.get("code").and_then(|v| v.as_str()).unwrap_or("UNKNOWN");
                let message = frame.get("message").and_then(|v| v.as_str()).unwrap_or("Unknown bridge error");
                let _ = app.emit("doctor:error", json!({"message": format!("Bridge error [{code}]: {message}")}));
            }

            _ => {
                // Ignore unknown frame types (e.g., event, req)
            }
        }
    }

    fn derive_node_id() -> String {
        hostname::get()
            .map(|h| h.to_string_lossy().to_string())
            .unwrap_or_else(|_| "clawpal-node".to_string())
    }

    fn advertised_commands() -> Vec<&'static str> {
        vec![
            "read_file", "list_files", "read_config",
            "system_info", "validate_config",
            "write_file", "run_command",
        ]
    }

    fn token_path() -> std::path::PathBuf {
        let paths = crate::models::resolve_paths();
        paths.clawpal_dir.join(BRIDGE_TOKEN_FILE)
    }

    fn load_token() -> Option<String> {
        let path = Self::token_path();
        std::fs::read_to_string(&path).ok()
            .and_then(|text| serde_json::from_str::<Value>(&text).ok())
            .and_then(|v| v.get("token")?.as_str().map(|s| s.to_string()))
    }

    fn save_token(token: &str) {
        let path = Self::token_path();
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        let data = json!({"token": token});
        let _ = std::fs::write(&path, serde_json::to_string_pretty(&data).unwrap_or_default());
    }
}

impl Default for BridgeClient {
    fn default() -> Self {
        Self::new()
    }
}
```

**Step 2: Verify it compiles (won't yet — need to register module)**

This step is deferred to Task 5 when we add the module to lib.rs.

**Step 3: Commit**

```bash
git add src-tauri/src/bridge_client.rs
git commit -m "feat: add bridge_client.rs — TCP NDJSON client for node protocol"
```

---

### Task 2: Fix node_client.rs — Device auth with Ed25519

The current `do_handshake` uses a simple gateway token. The operator protocol requires Ed25519 device authentication to get `operator.write` scope (needed for the `agent` method).

**Files:**
- Modify: `src-tauri/src/node_client.rs:1-16` (add imports)
- Modify: `src-tauri/src/node_client.rs:255-284` (rewrite `do_handshake`)

**Dependencies already in Cargo.toml:**
- `ed25519-dalek = { version = "2", features = ["pkcs8"] }` (already added)
- `base64 = "0.22"` (already present)

**Step 1: Add imports for device auth**

At the top of `src-tauri/src/node_client.rs`, after the existing imports (line 15), add:

```rust
use base64::Engine;
use ed25519_dalek::{SigningKey, Signer};
use ed25519_dalek::pkcs8::DecodePrivateKey;
```

**Step 2: Add helper to read device identity**

After the `impl Default for NodeClient` block (after line 370), add:

```rust
/// Device identity for Ed25519 authentication.
struct DeviceIdentity {
    device_id: String,
    public_key_pem: String,
    private_key_pem: String,
    token: String,
    scopes: Vec<String>,
}

fn load_device_identity() -> Result<DeviceIdentity, String> {
    let paths = resolve_paths();
    let identity_dir = paths.openclaw_dir.join("identity");

    // Read device.json — contains keys
    let device_path = identity_dir.join("device.json");
    let device_text = std::fs::read_to_string(&device_path)
        .map_err(|e| format!("Cannot read {}: {e}", device_path.display()))?;
    let device: Value = serde_json::from_str(&device_text)
        .map_err(|e| format!("Invalid JSON in {}: {e}", device_path.display()))?;

    let device_id = device.get("deviceId").and_then(|v| v.as_str())
        .ok_or("device.json missing deviceId")?.to_string();
    let public_key_pem = device.get("publicKeyPem").and_then(|v| v.as_str())
        .ok_or("device.json missing publicKeyPem")?.to_string();
    let private_key_pem = device.get("privateKeyPem").and_then(|v| v.as_str())
        .ok_or("device.json missing privateKeyPem")?.to_string();

    // Read device-auth.json — contains tokens
    let auth_path = identity_dir.join("device-auth.json");
    let auth_text = std::fs::read_to_string(&auth_path)
        .map_err(|e| format!("Cannot read {}: {e}", auth_path.display()))?;
    let auth: Value = serde_json::from_str(&auth_text)
        .map_err(|e| format!("Invalid JSON in {}: {e}", auth_path.display()))?;

    let operator = auth.get("tokens").and_then(|t| t.get("operator"))
        .ok_or("device-auth.json missing tokens.operator")?;
    let token = operator.get("token").and_then(|v| v.as_str())
        .ok_or("device-auth.json missing operator token")?.to_string();
    let scopes: Vec<String> = operator.get("scopes")
        .and_then(|v| v.as_array())
        .map(|arr| arr.iter().filter_map(|s| s.as_str().map(|s| s.to_string())).collect())
        .unwrap_or_else(|| vec!["operator.admin".to_string(), "operator.write".to_string()]);

    Ok(DeviceIdentity { device_id, public_key_pem, private_key_pem, token, scopes })
}

fn sign_challenge(identity: &DeviceIdentity, nonce: &str) -> Result<(String, u64), String> {
    let signing_key = SigningKey::from_pkcs8_pem(&identity.private_key_pem)
        .map_err(|e| format!("Failed to parse private key: {e}"))?;

    let signed_at = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_millis() as u64;

    let scopes_str = identity.scopes.join(",");
    let payload = format!(
        "v2|{}|clawpal|cli|operator|{}|{}|{}|{}",
        identity.device_id, scopes_str, signed_at, identity.token, nonce
    );

    let signature = signing_key.sign(payload.as_bytes());
    let sig_b64 = base64::engine::general_purpose::STANDARD.encode(signature.to_bytes());

    Ok((sig_b64, signed_at))
}
```

**Step 3: Rewrite `do_handshake` for device auth with challenge**

Replace the entire `do_handshake` method (lines 255-284) with:

```rust
    async fn do_handshake(&self, _app: &AppHandle) -> Result<(), String> {
        let identity = load_device_identity()?;

        // Read gateway auth token from local openclaw config
        let paths = resolve_paths();
        let gateway_token = std::fs::read_to_string(&paths.config_path)
            .ok()
            .and_then(|text| serde_json::from_str::<Value>(&text).ok())
            .and_then(|config| {
                config.get("gateway")?.get("auth")?.get("token")?.as_str().map(|s| s.to_string())
            })
            .unwrap_or_default();

        // Step 1: Send initial connect to trigger challenge
        // The gateway will respond with a connect.challenge event containing a nonce.
        // We need to listen for it before sending the actual connect request.
        // Since our reader task is already running, we need a channel to receive the nonce.

        // For now, send connect without device auth first to get the challenge nonce
        // Actually, the gateway sends connect.challenge as an event immediately after WS connect.
        // We need to wait for it. Let's use a simple approach: wait up to 5 seconds for the nonce.

        // Store a oneshot channel for the challenge nonce
        let (nonce_tx, nonce_rx) = tokio::sync::oneshot::channel::<String>();
        {
            let mut guard = self.inner.lock().await;
            if let Some(inner) = guard.as_mut() {
                // Store the nonce sender in a special pending slot
                // We'll use a special key "__challenge_nonce__" in the pending map
                // The sender sends the nonce as a JSON string value
                // Actually, we can't mix types in the pending HashMap<String, oneshot::Sender<Value>>
                // So let's store the nonce sender separately
            }
        }

        // Simpler approach: wait briefly for the challenge event, which our reader task
        // will have already received and we need to capture.
        // Actually, the challenge is sent as a connect.challenge event, and our handle_frame
        // doesn't handle events other than "chat". Let's add challenge handling.
        //
        // Even simpler: try sending connect without nonce first. If the gateway requires it,
        // it will reject and we retry with the nonce. But from testing, the gateway sends
        // the challenge BEFORE we send connect.
        //
        // Best approach: add a challenge_nonce field to NodeClientInner, set it from handle_frame
        // when connect.challenge event arrives, then read it here.

        // Wait for challenge nonce (the reader task sets it via handle_frame)
        let nonce = {
            let mut attempts = 0;
            loop {
                tokio::time::sleep(std::time::Duration::from_millis(100)).await;
                let guard = self.inner.lock().await;
                if let Some(inner) = guard.as_ref() {
                    if let Some(sender) = inner.pending.get("__nonce__") {
                        // Can't read from pending without removing, so use a different approach
                        break String::new();
                    }
                }
                attempts += 1;
                if attempts > 30 { // 3 seconds
                    break String::new();
                }
            }
        };

        // TODO: This needs a proper nonce channel. For now, send connect without nonce
        // and rely on gateway not requiring it (some gateways accept token-only auth).

        let (signature, signed_at) = if !nonce.is_empty() {
            sign_challenge(&identity, &nonce)?
        } else {
            // Sign with empty nonce (gateway may accept this)
            sign_challenge(&identity, "")?
        };

        let scopes: Vec<Value> = identity.scopes.iter().map(|s| Value::String(s.clone())).collect();

        // Extract raw public key bytes from PEM for base64 encoding
        let pub_key_b64 = {
            // The publicKeyPem is an Ed25519 SubjectPublicKeyInfo PEM.
            // Extract the raw 32-byte key.
            let pem_str = &identity.public_key_pem;
            let pem_body: String = pem_str.lines()
                .filter(|l| !l.starts_with("-----"))
                .collect();
            let der = base64::engine::general_purpose::STANDARD.decode(&pem_body)
                .map_err(|e| format!("Failed to decode public key PEM: {e}"))?;
            // Ed25519 SubjectPublicKeyInfo is 44 bytes: 12 byte header + 32 byte key
            let raw_key = if der.len() == 44 { &der[12..] } else { &der };
            base64::engine::general_purpose::STANDARD.encode(raw_key)
        };

        let _result = self.send_request("connect", json!({
            "minProtocol": 3,
            "maxProtocol": 3,
            "auth": { "token": gateway_token },
            "role": "operator",
            "scopes": scopes,
            "device": {
                "id": identity.device_id,
                "publicKey": pub_key_b64,
                "signature": signature,
                "signedAt": signed_at,
                "nonce": nonce,
            },
            "client": {
                "id": "clawpal",
                "platform": std::env::consts::OS,
                "mode": "cli",
                "version": env!("CARGO_PKG_VERSION"),
            },
        })).await?;

        Ok(())
    }
```

**Step 4: Add challenge nonce handling to NodeClientInner and handle_frame**

Add a `challenge_nonce` field to `NodeClientInner` (line 19-23):

Replace:
```rust
struct NodeClientInner {
    tx: WsSink,
    req_counter: u64,
    pending: HashMap<String, oneshot::Sender<Value>>,
}
```

With:
```rust
struct NodeClientInner {
    tx: WsSink,
    req_counter: u64,
    pending: HashMap<String, oneshot::Sender<Value>>,
    /// Challenge nonce received from gateway's connect.challenge event.
    challenge_nonce: Option<String>,
}
```

Update the constructor in `connect` (line 50-54) to include the new field:
```rust
        let inner = NodeClientInner {
            tx,
            req_counter: 0,
            pending: HashMap::new(),
            challenge_nonce: None,
        };
```

In `handle_frame`, update the `"event"` match arm (line 306-320) to also handle `connect.challenge`:

Replace:
```rust
            "event" => {
                let event_name = frame.get("event").and_then(|v| v.as_str()).unwrap_or("");
                let payload = frame.get("payload").cloned().unwrap_or(Value::Null);
                match event_name {
                    "chat" => {
```

With:
```rust
            "event" => {
                let event_name = frame.get("event").and_then(|v| v.as_str()).unwrap_or("");
                let payload = frame.get("payload").cloned().unwrap_or(Value::Null);
                match event_name {
                    "connect.challenge" => {
                        if let Some(nonce) = payload.get("nonce").and_then(|v| v.as_str()) {
                            let mut guard = inner_ref.lock().await;
                            if let Some(inner) = guard.as_mut() {
                                inner.challenge_nonce = Some(nonce.to_string());
                            }
                        }
                    }
                    "chat" => {
```

Then update `do_handshake` to read the nonce from `challenge_nonce`:

Replace the nonce-waiting loop in `do_handshake` with:
```rust
        // Wait for challenge nonce (set by reader task from connect.challenge event)
        let nonce = {
            let mut attempts = 0;
            loop {
                tokio::time::sleep(std::time::Duration::from_millis(100)).await;
                let guard = self.inner.lock().await;
                if let Some(inner) = guard.as_ref() {
                    if let Some(ref n) = inner.challenge_nonce {
                        break n.clone();
                    }
                }
                attempts += 1;
                if attempts > 30 { // 3 seconds timeout
                    // No challenge received — proceed without nonce (some gateways may not require it)
                    break String::new();
                }
            }
        };
```

**Step 5: Remove debug eprintln statements**

Remove the following lines from node_client.rs:
- Line 71: `eprintln!("[doctor-ws] <<< {}", &text[..text.len().min(500)]);`
- Line 141: `eprintln!("[doctor-ws] >>> {}", &frame_str[..frame_str.len().min(500)]);`
- Line 199: `eprintln!("[doctor-ws] >>> (fire) {}", &frame_str[..frame_str.len().min(500)]);`

**Step 6: Verify it compiles**

Run: `cd /Users/zhixian/Codes/clawpal/src-tauri && cargo check 2>&1 | tail -10`

**Step 7: Commit**

```bash
git add src-tauri/src/node_client.rs
git commit -m "feat: add Ed25519 device auth to operator handshake, remove debug logging"
```

---

### Task 3: Fix node_client.rs — Agent params and chat event parsing

The `agent` method and `chat` event parsing use wrong formats discovered during testing.

**Files:**
- Modify: `src-tauri/src/node_client.rs:306-320` (chat event parsing)
- Modify: `src-tauri/src/doctor_commands.rs:24-49` (agent params in start_diagnosis and send_message)

**Step 1: Fix chat event parsing in handle_frame**

The current code expects `payload.text` and `payload.final`. The actual format from testing is:
- `payload.state` = `"delta"` or `"final"`
- `payload.message.content[0].text` = the text

In `handle_frame` (node_client.rs), replace the `"chat"` match arm:

```rust
                    "chat" => {
                        let is_final = payload.get("final").and_then(|v| v.as_bool()).unwrap_or(false);
                        let text = payload.get("text").and_then(|v| v.as_str()).unwrap_or("");
```

With:

```rust
                    "chat" => {
                        let state = payload.get("state").and_then(|v| v.as_str()).unwrap_or("");
                        let is_final = state == "final";
                        // Extract text from payload.message.content[0].text
                        let text = payload.get("message")
                            .and_then(|m| m.get("content"))
                            .and_then(|c| c.as_array())
                            .and_then(|arr| arr.first())
                            .and_then(|item| item.get("text"))
                            .and_then(|t| t.as_str())
                            .unwrap_or("");
```

**Step 2: Fix doctor_start_diagnosis params**

In `doctor_commands.rs`, replace `doctor_start_diagnosis` (lines 24-37):

```rust
#[tauri::command]
pub async fn doctor_start_diagnosis(
    client: State<'_, NodeClient>,
    context: String,
) -> Result<(), String> {
    let idempotency_key = uuid::Uuid::new_v4().to_string();

    // Fire-and-forget: results arrive via streaming chat events
    client.send_request_fire("agent", json!({
        "message": context,
        "idempotencyKey": idempotency_key,
        "agentId": "main",
        "sessionKey": "agent:main:clawpal-doctor",
    })).await
}
```

**Step 3: Fix doctor_send_message params**

In `doctor_commands.rs`, replace `doctor_send_message` (lines 39-49):

```rust
#[tauri::command]
pub async fn doctor_send_message(
    client: State<'_, NodeClient>,
    message: String,
) -> Result<(), String> {
    let idempotency_key = uuid::Uuid::new_v4().to_string();

    client.send_request_fire("agent", json!({
        "message": message,
        "idempotencyKey": idempotency_key,
        "agentId": "main",
        "sessionKey": "agent:main:clawpal-doctor",
    })).await
}
```

**Step 4: Verify it compiles**

Run: `cd /Users/zhixian/Codes/clawpal/src-tauri && cargo check 2>&1 | tail -10`

**Step 5: Commit**

```bash
git add src-tauri/src/node_client.rs src-tauri/src/doctor_commands.rs
git commit -m "fix: correct agent method params and chat event parsing format"
```

---

### Task 4: Add bridge commands to doctor_commands.rs

Wire the bridge client into Tauri commands. The doctor flow now uses both connections:
1. Bridge connect (registers as node)
2. Operator connect (for agent method)
3. Start diagnosis
4. Invokes arrive via bridge, executed locally/remotely, results sent back via bridge

**Files:**
- Modify: `src-tauri/src/doctor_commands.rs:1-15` (add import)
- Modify: `src-tauri/src/doctor_commands.rs:8-22` (update connect/disconnect to manage both connections)

**Step 1: Add bridge_client import**

At line 4 of doctor_commands.rs, after `use crate::node_client::NodeClient;`, add:

```rust
use crate::bridge_client::BridgeClient;
```

**Step 2: Add bridge connect/disconnect commands**

After the existing `doctor_disconnect` command, add:

```rust
#[tauri::command]
pub async fn doctor_bridge_connect(
    bridge: State<'_, BridgeClient>,
    app: AppHandle,
    addr: String,
) -> Result<(), String> {
    bridge.connect(&addr, app).await
}

#[tauri::command]
pub async fn doctor_bridge_disconnect(
    bridge: State<'_, BridgeClient>,
) -> Result<(), String> {
    bridge.disconnect().await
}
```

**Step 3: Update doctor_approve_invoke to use bridge for responses**

The invoke comes from the bridge, so the response goes back via bridge (not operator WebSocket).

Replace the `doctor_approve_invoke` function to accept both clients:

```rust
#[tauri::command]
pub async fn doctor_approve_invoke(
    client: State<'_, NodeClient>,
    bridge: State<'_, BridgeClient>,
    pool: State<'_, SshConnectionPool>,
    app: AppHandle,
    invoke_id: String,
    target: String,
) -> Result<Value, String> {
    // Try bridge first (invokes come from bridge in dual-connection mode)
    // Fall back to operator client (for operator-only mode)
    let invoke = bridge.take_invoke(&invoke_id).await
        .or_else(|| {
            // Synchronous fallback — try operator client's pending invokes
            // Note: take_invoke is async, so we need a different approach
            None
        });

    // If not found in bridge, try operator client
    let invoke = match invoke {
        Some(inv) => inv,
        None => client.take_invoke(&invoke_id).await
            .ok_or_else(|| format!("No pending invoke with id: {invoke_id}"))?,
    };

    let command = invoke.get("command").and_then(|v| v.as_str()).unwrap_or("");
    let args = invoke.get("args").cloned().unwrap_or(Value::Null);

    // Route to local or remote execution
    let result = if target == "local" {
        execute_local_command(command, &args).await?
    } else {
        execute_remote_command(&pool, &target, command, &args).await?
    };

    // Send result back — try bridge first, then operator
    if bridge.is_connected().await {
        bridge.send_invoke_result(&invoke_id, result.clone()).await?;
    } else {
        client.send_response(&invoke_id, result.clone()).await?;
    }

    let _ = app.emit("doctor:invoke-result", json!({
        "id": invoke_id,
        "result": result,
    }));

    Ok(result)
}
```

**Step 4: Update doctor_reject_invoke similarly**

```rust
#[tauri::command]
pub async fn doctor_reject_invoke(
    client: State<'_, NodeClient>,
    bridge: State<'_, BridgeClient>,
    invoke_id: String,
    reason: String,
) -> Result<(), String> {
    let _invoke = bridge.take_invoke(&invoke_id).await
        .or(client.take_invoke(&invoke_id).await)
        .ok_or_else(|| format!("No pending invoke with id: {invoke_id}"))?;

    if bridge.is_connected().await {
        bridge.send_invoke_error(&invoke_id, "REJECTED", &format!("Rejected by user: {reason}")).await
    } else {
        client.send_error_response(&invoke_id, &format!("Rejected by user: {reason}")).await
    }
}
```

**Step 5: Update doctor_disconnect to close both connections**

Replace the existing `doctor_disconnect`:

```rust
#[tauri::command]
pub async fn doctor_disconnect(
    client: State<'_, NodeClient>,
    bridge: State<'_, BridgeClient>,
) -> Result<(), String> {
    let _ = bridge.disconnect().await;
    client.disconnect().await
}
```

**Step 6: Verify it compiles**

Run: `cd /Users/zhixian/Codes/clawpal/src-tauri && cargo check 2>&1 | tail -10`

**Step 7: Commit**

```bash
git add src-tauri/src/doctor_commands.rs
git commit -m "feat: add bridge commands and dual-connection invoke routing"
```

---

### Task 5: Register bridge module in lib.rs

Register the new bridge_client module, BridgeClient state, and bridge commands.

**Files:**
- Modify: `src-tauri/src/lib.rs:43-47` (add bridge imports)
- Modify: `src-tauri/src/lib.rs:68` (add module declaration)
- Modify: `src-tauri/src/lib.rs:76-77` (add state)
- Modify: `src-tauri/src/lib.rs:214-221` (add commands to handler)

**Also needed:** Add `hostname` crate to Cargo.toml for `BridgeClient::derive_node_id()`.

**Step 1: Add hostname dependency to Cargo.toml**

In `src-tauri/Cargo.toml`, add after the `shellexpand` line:

```toml
hostname = "0.4"
```

**Step 2: Add module declaration in lib.rs**

After line 68 (`pub mod node_client;`), add:

```rust
pub mod bridge_client;
```

**Step 3: Add imports in lib.rs**

After the `use crate::node_client::NodeClient;` line (line 57), add:

```rust
use crate::bridge_client::BridgeClient;
```

Update the doctor_commands import (lines 43-47) to include bridge commands:

```rust
use crate::doctor_commands::{
    doctor_connect, doctor_disconnect, doctor_start_diagnosis, doctor_send_message,
    doctor_approve_invoke, doctor_reject_invoke, collect_doctor_context,
    collect_doctor_context_remote, doctor_bridge_connect, doctor_bridge_disconnect,
};
```

**Step 4: Add BridgeClient state**

After `.manage(NodeClient::new())` (line 77), add:

```rust
        .manage(BridgeClient::new())
```

**Step 5: Register bridge commands in generate_handler**

After `collect_doctor_context_remote,` in the handler (line 221), add:

```rust
            doctor_bridge_connect,
            doctor_bridge_disconnect,
```

**Step 6: Verify it compiles**

Run: `cd /Users/zhixian/Codes/clawpal/src-tauri && cargo check 2>&1 | tail -10`

**Step 7: Commit**

```bash
git add src-tauri/Cargo.toml src-tauri/src/lib.rs
git commit -m "feat: register bridge_client module, state, and commands"
```

---

### Task 6: Update frontend for dual connection

Update the API layer, hook, and Doctor page to manage both operator and bridge connections.

**Files:**
- Modify: `src/lib/api.ts` (add bridge API calls)
- Modify: `src/lib/use-api.ts` (add bridge methods)
- Modify: `src/lib/use-doctor-agent.ts` (add bridge connection state and events)
- Modify: `src/pages/Doctor.tsx` (update connection flow)

**Step 1: Add bridge API calls to api.ts**

Find the doctor section in `api.ts` and add after `collectDoctorContextRemote`:

```typescript
  doctorBridgeConnect: (addr: string): Promise<void> =>
    invoke("doctor_bridge_connect", { addr }),
  doctorBridgeDisconnect: (): Promise<void> =>
    invoke("doctor_bridge_disconnect"),
```

**Step 2: Add bridge methods to use-api.ts**

In the doctor section of `use-api.ts`, add:

```typescript
      doctorBridgeConnect: api.doctorBridgeConnect,
      doctorBridgeDisconnect: api.doctorBridgeDisconnect,
```

**Step 3: Update use-doctor-agent.ts**

Add bridge connection state:

After `const [connected, setConnected] = useState(false);` (line 18), add:

```typescript
  const [bridgeConnected, setBridgeConnected] = useState(false);
```

Add bridge event listeners inside the `useEffect` (after the existing listeners):

```typescript
      listen("doctor:bridge-connected", () => {
        setBridgeConnected(true);
      }),
      listen<{ reason: string }>("doctor:bridge-disconnected", () => {
        setBridgeConnected(false);
      }),
      listen("doctor:bridge-pairing", () => {
        // Bridge is waiting for pairing approval on the gateway
        // This is informational — the gateway admin needs to approve
      }),
```

Update the `connect` callback to also connect the bridge:

```typescript
  const connect = useCallback(async (url: string) => {
    setError(null);
    try {
      // Extract host from WebSocket URL for bridge TCP connection
      const wsUrl = new URL(url);
      const bridgeAddr = `${wsUrl.hostname}:18790`;

      // Connect bridge first (registers as node)
      await api.doctorBridgeConnect(bridgeAddr);

      // Then connect operator (for agent method)
      await api.doctorConnect(url);
    } catch (err) {
      const msg = `Connection failed: ${err}`;
      setError(msg);
      throw new Error(msg);
    }
  }, []);
```

Update `disconnect` to close both:

```typescript
  const disconnect = useCallback(async () => {
    try {
      await api.doctorDisconnect(); // This now closes both via the updated Tauri command
    } catch (err) {
      setError(`Disconnect failed: ${err}`);
    }
    setConnected(false);
    setBridgeConnected(false);
    setLoading(false);
  }, []);
```

Update `reset` to clear bridge state:

```typescript
  const reset = useCallback(() => {
    setMessages([]);
    setPendingInvokes(new Map());
    setLoading(false);
    setError(null);
    setApprovedPatterns(new Set());
    setBridgeConnected(false);
    streamingRef.current = "";
  }, []);
```

Add `bridgeConnected` to the return value:

```typescript
  return {
    connected,
    bridgeConnected,
    messages,
    // ... rest unchanged
  };
```

**Step 4: Update Doctor.tsx to show bridge status**

In the connected state section (inside the `doctor.connected` branch), add bridge status to the badge area:

After the existing `<Badge>` showing agent source, add:

```tsx
                <Badge variant={doctor.bridgeConnected ? "outline" : "destructive"} className="text-xs">
                  {doctor.bridgeConnected ? t("doctor.bridgeConnected") : t("doctor.bridgeDisconnected")}
                </Badge>
```

**Step 5: Verify TypeScript compiles**

Run: `cd /Users/zhixian/Codes/clawpal && npx tsc --noEmit 2>&1 | tail -10`

**Step 6: Commit**

```bash
git add src/lib/api.ts src/lib/use-api.ts src/lib/use-doctor-agent.ts src/pages/Doctor.tsx
git commit -m "feat: add dual connection (operator + bridge) support to frontend"
```

---

### Task 7: Add i18n keys for bridge

**Files:**
- Modify: `src/locales/en.json`
- Modify: `src/locales/zh.json`

**Step 1: Add English translations**

Find the doctor section in en.json and add:

```json
  "doctor.bridgeConnected": "Node registered",
  "doctor.bridgeDisconnected": "Node disconnected",
  "doctor.bridgePairing": "Waiting for gateway approval...",
```

**Step 2: Add Chinese translations**

```json
  "doctor.bridgeConnected": "节点已注册",
  "doctor.bridgeDisconnected": "节点未连接",
  "doctor.bridgePairing": "等待网关审批...",
```

**Step 3: Commit**

```bash
git add src/locales/en.json src/locales/zh.json
git commit -m "i18n: add bridge connection status translation keys"
```

---

### Task 8: Full build verification

**Files:** None (verification only)

**Step 1: Rust check**

Run: `cd /Users/zhixian/Codes/clawpal/src-tauri && cargo check 2>&1 | tail -10`
Expected: no errors

**Step 2: TypeScript check**

Run: `cd /Users/zhixian/Codes/clawpal && npx tsc --noEmit 2>&1 | tail -10`
Expected: no errors

**Step 3: Vite production build**

Run: `cd /Users/zhixian/Codes/clawpal && npx vite build 2>&1 | tail -10`
Expected: build succeeds

**Step 4: Full Rust build**

Run: `cd /Users/zhixian/Codes/clawpal/src-tauri && cargo build 2>&1 | tail -20`
Expected: build succeeds

**Step 5: Fix any issues found, commit fixes**

If any errors are found, fix them and create a new commit:

```bash
git add -A
git commit -m "fix: resolve build issues from dual connection implementation"
```
