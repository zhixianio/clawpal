use std::collections::HashMap;
use std::sync::Arc;

use base64::Engine;
use ed25519_dalek::pkcs8::DecodePrivateKey;
use ed25519_dalek::{Signer, SigningKey};
use futures_util::stream::SplitSink;
use futures_util::{SinkExt, StreamExt};
use serde_json::{json, Value};
use tauri::{AppHandle, Emitter};
use crate::models::resolve_paths;
use tokio::net::TcpStream;
use tokio::sync::{oneshot, Mutex};
use tokio_tungstenite::{
    connect_async,
    tungstenite::Message,
    MaybeTlsStream, WebSocketStream,
};

type WsSink = SplitSink<WebSocketStream<MaybeTlsStream<TcpStream>>, Message>;

/// Credentials for authenticating with a remote gateway.
/// When connecting to a non-local gateway (via SSH tunnel), we need the
/// remote host's auth token and device identity instead of the local ones.
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GatewayCredentials {
    pub token: String,
    pub device_id: String,
    pub private_key_pem: String,
}

struct NodeClientInner {
    tx: WsSink,
    req_counter: u64,
    pending: HashMap<String, oneshot::Sender<Value>>,
    challenge_nonce: Option<String>,
}

/// WebSocket operator client — connects to the gateway with `role: "operator"`.
/// Used for sending agent requests and receiving chat streaming events.
/// Tool invocations are handled by BridgeClient (node connection).
pub struct NodeClient {
    inner: Arc<Mutex<Option<NodeClientInner>>>,
    credentials: Arc<Mutex<Option<GatewayCredentials>>>,
}

impl NodeClient {
    pub fn new() -> Self {
        Self {
            inner: Arc::new(Mutex::new(None)),
            credentials: Arc::new(Mutex::new(None)),
        }
    }

    pub async fn connect(&self, url: &str, app: AppHandle, creds: Option<GatewayCredentials>) -> Result<(), String> {
        // Disconnect existing connection if any
        self.disconnect().await?;

        // Store credentials for use in handshake
        *self.credentials.lock().await = creds;

        let (ws_stream, _) = connect_async(url)
            .await
            .map_err(|e| format!("WebSocket connection failed: {e}"))?;

        let (tx, mut rx) = ws_stream.split();

        let inner = NodeClientInner {
            tx,
            req_counter: 0,
            pending: HashMap::new(),
            challenge_nonce: None,
        };

        {
            let mut guard = self.inner.lock().await;
            *guard = Some(inner);
        }

        // Spawn reader task
        let inner_ref = Arc::clone(&self.inner);
        let app_clone = app.clone();

        tokio::spawn(async move {
            while let Some(msg) = rx.next().await {
                match msg {
                    Ok(Message::Text(text)) => {
                        if let Ok(frame) = serde_json::from_str::<Value>(&text) {
                            Self::handle_frame(
                                frame,
                                &inner_ref,
                                &app_clone,
                            )
                            .await;
                        }
                    }
                    Ok(Message::Close(_)) => {
                        let _ = app_clone.emit("doctor:disconnected", json!({"reason": "server closed"}));
                        let mut guard = inner_ref.lock().await;
                        *guard = None;
                        break;
                    }
                    Err(e) => {
                        let _ = app_clone.emit("doctor:error", json!({"message": format!("WebSocket error: {e}")}));
                        let _ = app_clone.emit("doctor:disconnected", json!({"reason": format!("{e}")}));
                        let mut guard = inner_ref.lock().await;
                        *guard = None;
                        break;
                    }
                    _ => {}
                }
            }
        });

        // Do handshake
        self.do_handshake(&app).await?;

        let _ = app.emit("doctor:connected", json!({}));
        Ok(())
    }

    pub async fn disconnect(&self) -> Result<(), String> {
        let mut guard = self.inner.lock().await;
        if let Some(mut inner) = guard.take() {
            let _ = inner.tx.close().await;
        }
        Ok(())
    }

    pub async fn is_connected(&self) -> bool {
        self.inner.lock().await.is_some()
    }

    pub async fn send_request(&self, method: &str, params: Value) -> Result<Value, String> {
        let (id, rx) = {
            let mut guard = self.inner.lock().await;
            let inner = guard.as_mut().ok_or("Not connected")?;
            inner.req_counter += 1;
            let id = format!("c{}", inner.req_counter);

            // Register the pending sender BEFORE sending the message to avoid
            // a race where the response arrives before the sender is registered.
            let (tx, rx) = oneshot::channel::<Value>();
            inner.pending.insert(id.clone(), tx);

            let frame = json!({
                "type": "req",
                "id": id,
                "method": method,
                "params": params,
            });

            let frame_str = frame.to_string();
            if let Err(e) = inner.tx.send(Message::Text(frame_str)).await {
                inner.pending.remove(&id);
                return Err(format!("Failed to send request: {e}"));
            }

            (id, rx)
        };

        // Wait for response with timeout
        match tokio::time::timeout(std::time::Duration::from_secs(120), rx).await {
            Ok(Ok(val)) => {
                // Protocol uses ok/payload format
                let ok = val.get("ok").and_then(|v| v.as_bool()).unwrap_or(false);
                if !ok {
                    if let Some(err) = val.get("error") {
                        Err(format!("Remote error: {}", err))
                    } else {
                        Err("Request failed".into())
                    }
                } else {
                    Ok(val.get("payload").cloned().unwrap_or(Value::Null))
                }
            }
            Ok(Err(_)) => {
                // oneshot dropped — connection lost
                let mut guard = self.inner.lock().await;
                if let Some(inner) = guard.as_mut() {
                    inner.pending.remove(&id);
                }
                Err("Connection lost while waiting for response".into())
            }
            Err(_) => {
                let mut guard = self.inner.lock().await;
                if let Some(inner) = guard.as_mut() {
                    inner.pending.remove(&id);
                }
                Err("Request timed out".into())
            }
        }
    }

    /// Send a request without waiting for the response.
    /// Used for agent requests where results arrive via streaming events.
    pub async fn send_request_fire(&self, method: &str, params: Value) -> Result<(), String> {
        let mut guard = self.inner.lock().await;
        let inner = guard.as_mut().ok_or("Not connected")?;
        inner.req_counter += 1;
        let id = format!("c{}", inner.req_counter);

        eprintln!("[operator] fire request: method={method}, id={id}");

        let frame = json!({
            "type": "req",
            "id": id,
            "method": method,
            "params": params,
        });

        let frame_str = frame.to_string();
        inner
            .tx
            .send(Message::Text(frame_str))
            .await
            .map_err(|e| format!("Failed to send request: {e}"))?;

        Ok(())
    }

    async fn do_handshake(&self, _app: &AppHandle) -> Result<(), String> {
        let creds = self.credentials.lock().await.clone();

        let (token, device_id, signing_key, public_key_b64) = if let Some(c) = creds {
            // Use remote gateway credentials (connecting via SSH tunnel)
            let signing_key = SigningKey::from_pkcs8_pem(&c.private_key_pem)
                .map_err(|e| format!("Failed to parse remote Ed25519 private key: {e}"))?;
            let raw_public = signing_key.verifying_key().to_bytes();
            let public_key_b64 = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(raw_public);
            (c.token, c.device_id, signing_key, public_key_b64)
        } else {
            // Use local credentials
            let paths = resolve_paths();
            let token = std::fs::read_to_string(&paths.config_path)
                .ok()
                .and_then(|text| serde_json::from_str::<Value>(&text).ok())
                .and_then(|config| {
                    config.get("gateway")?
                        .get("auth")?
                        .get("token")?
                        .as_str()
                        .map(|s| s.to_string())
                })
                .unwrap_or_default();
            let (device_id, signing_key, public_key_b64) =
                load_device_identity(&paths.openclaw_dir)?;
            (token, device_id, signing_key, public_key_b64)
        };

        // Scopes for operator role
        let scopes: Vec<String> = vec![
            "operator.admin".into(),
            "operator.read".into(),
            "operator.write".into(),
        ];

        // Wait for challenge nonce from the reader task (poll every 100ms, up to 3s)
        let mut nonce = None;
        for _ in 0..30 {
            {
                let mut guard = self.inner.lock().await;
                if let Some(inner) = guard.as_mut() {
                    if let Some(n) = inner.challenge_nonce.take() {
                        nonce = Some(n);
                        break;
                    }
                }
            }
            tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        }
        let nonce = nonce.unwrap_or_default();

        // Sign the challenge
        let signed_at = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_millis() as u64;

        let signature_b64 = sign_challenge(
            &signing_key,
            &device_id,
            &scopes,
            signed_at,
            &token,  // gateway auth token, same as connect.params.auth.token
            &nonce,
        );

        let version = env!("CARGO_PKG_VERSION");

        let mut device = json!({
            "id": device_id,
            "publicKey": public_key_b64,
            "signature": signature_b64,
            "signedAt": signed_at,
        });
        if !nonce.is_empty() {
            device["nonce"] = json!(nonce);
        }

        let result = self.send_request("connect", json!({
            "minProtocol": 3,
            "maxProtocol": 3,
            "auth": { "token": token },
            "role": "operator",
            "scopes": scopes,
            "device": device,
            "client": {
                "id": "cli",
                "displayName": "ClawPal",
                "platform": std::env::consts::OS,
                "mode": "cli",
                "version": version,
            },
        })).await?;

        eprintln!("[operator] connected, handshake ok");
        let _ = result;
        Ok(())
    }

    async fn handle_frame(
        frame: Value,
        inner_ref: &Arc<Mutex<Option<NodeClientInner>>>,
        app: &AppHandle,
    ) {
        let frame_type = frame.get("type").and_then(|v| v.as_str()).unwrap_or("");

        match frame_type {
            "res" => {
                // Response to a request we sent
                if let Some(id) = frame.get("id").and_then(|v| v.as_str()) {
                    let ok = frame.get("ok").and_then(|v| v.as_bool()).unwrap_or(false);
                    let has_pending = {
                        let guard = inner_ref.lock().await;
                        guard.as_ref().map_or(false, |i| i.pending.contains_key(id))
                    };
                    if !has_pending {
                        // Response to fire-and-forget request — log it
                        if !ok {
                            let err = frame.get("error").cloned().unwrap_or(Value::Null);
                            eprintln!("[operator] fire-and-forget res FAILED: id={id}, error={err}");
                        } else {
                            eprintln!("[operator] fire-and-forget res OK: id={id}");
                        }
                    }
                    let mut guard = inner_ref.lock().await;
                    if let Some(inner) = guard.as_mut() {
                        if let Some(sender) = inner.pending.remove(id) {
                            let _ = sender.send(frame.clone());
                        }
                    }
                }
            }
            "event" => {
                let event_name = frame.get("event").and_then(|v| v.as_str()).unwrap_or("");
                let payload = frame.get("payload").cloned().unwrap_or(Value::Null);
                eprintln!("[operator] event: {event_name}");
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
                        let state = payload.get("state").and_then(|v| v.as_str()).unwrap_or("");
                        let is_final = state == "final";
                        let text = payload.get("message")
                            .and_then(|m| m.get("content"))
                            .and_then(|c| c.as_array())
                            .and_then(|arr| arr.first())
                            .and_then(|item| item.get("text"))
                            .and_then(|t| t.as_str())
                            .unwrap_or("");
                        if is_final {
                            let _ = app.emit("doctor:chat-final", json!({"text": text}));
                        } else {
                            let _ = app.emit("doctor:chat-delta", json!({"text": text}));
                        }
                    }
                    _ => {}
                }
            }
            "req" => {
                // Operator connection does not receive requests from the gateway.
                // Tool invocations go to the node connection (BridgeClient).
            }
            _ => {}
        }
    }
}

/// Load device identity from ~/.openclaw/identity/device.json.
/// Returns (device_id, signing_key, base64_raw_public_key).
fn load_device_identity(
    openclaw_dir: &std::path::Path,
) -> Result<(String, SigningKey, String), String> {
    let device_path = openclaw_dir.join("identity").join("device.json");
    let device_json: Value = std::fs::read_to_string(&device_path)
        .map_err(|e| format!("Failed to read device.json: {e}"))?
        .parse()
        .map_err(|e| format!("Failed to parse device.json: {e}"))?;

    let device_id = device_json
        .get("deviceId")
        .and_then(|v| v.as_str())
        .ok_or("Missing deviceId in device.json")?
        .to_string();

    let private_key_pem = device_json
        .get("privateKeyPem")
        .and_then(|v| v.as_str())
        .ok_or("Missing privateKeyPem in device.json")?;

    let signing_key = SigningKey::from_pkcs8_pem(private_key_pem)
        .map_err(|e| format!("Failed to parse Ed25519 private key: {e}"))?;

    // Extract raw 32-byte public key from the signing key and base64-encode it
    let raw_public = signing_key.verifying_key().to_bytes();
    let public_key_b64 = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(raw_public);

    Ok((device_id, signing_key, public_key_b64))
}

/// Sign the challenge payload using Ed25519.
/// Payload: `v2|<deviceId>|cli|cli|operator|<scopes>|<signedAt>|<token>|<nonce>`
fn sign_challenge(
    signing_key: &SigningKey,
    device_id: &str,
    scopes: &[String],
    signed_at: u64,
    operator_token: &str,
    nonce: &str,
) -> String {
    let scopes_str = scopes.join(",");
    let payload = format!(
        "v2|{device_id}|cli|cli|operator|{scopes_str}|{signed_at}|{operator_token}|{nonce}"
    );
    let signature = signing_key.sign(payload.as_bytes());
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(signature.to_bytes())
}

impl Default for NodeClient {
    fn default() -> Self {
        Self::new()
    }
}
