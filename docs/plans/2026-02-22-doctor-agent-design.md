# Doctor Agent Design

Date: 2026-02-22

## Overview

Add AI-powered diagnostic and repair capabilities to ClawPal's Doctor feature. When static checks can't fix the problem (or openclaw is broken), an external AI agent steps in via a chat-based tool-use conversation.

Core idea: **ClawPal connects as an openclaw node** to a gateway running a doctor agent. The gateway sends tool calls via `node.invoke`, ClawPal executes locally (with user confirmation for writes), and returns results.

## Architecture

```
Doctor Gateway (local / SSH remote / hosted service)
    ↕ WebSocket (openclaw node protocol v3)
ClawPal (node client)
    → receives node.invoke → shows to user → executes locally → returns result
```

All three agent sources use the same protocol — only the WebSocket URL differs:

| Source | URL | Notes |
|---|---|---|
| Local openclaw | `ws://localhost:18789` | Current instance's gateway |
| SSH remote openclaw | `ws://localhost:<forwarded>` | SSH port forward to remote:18789 |
| Remote doctor service | `wss://doctor.openclaw.ai` | Hosted by us |

User selects the source manually on the Doctor page.

## Node Protocol

JSON over WebSocket. Three frame types: `req`, `res`, `event`. See `openclaw/clawgo` (~1400 lines Go) as reference implementation.

**Connection lifecycle:** `pair-request` → `pair-ok` (first time) → `hello` → `hello-ok` (every reconnect) → steady state.

**Tool-use cycle:**
1. ClawPal sends `agent` req with diagnostic context
2. Agent streams text replies via `chat` events (`delta` / `final`)
3. Agent tool calls arrive as `node.invoke` req
4. ClawPal shows to user → auto-executes reads, confirms writes → sends `res` back
5. Repeat 2-4 until agent is done

**Advertised commands** (in `hello.commands`):

| Command | Type | Description |
|---|---|---|
| `read_file` | read | Read file contents |
| `list_files` | read | List directory |
| `read_config` | read | Read openclaw.json |
| `system_info` | read | OS, PATH, version |
| `validate_config` | read | Run doctor checks |
| `write_file` | write | Write/overwrite file |
| `run_command` | write | Execute shell command |

All map to existing ClawPal Tauri commands (local fs ops or SSH equivalents).

## UI

- Doctor.tsx: add agent source selector + "Start Diagnosis" button
- Chat.tsx: extend with `mode: "doctor"` — adds tool-call cards with Execute/Skip buttons, streaming agent text
- Read ops auto-execute; write ops require user click

**Doctor context** auto-collected on start: openclaw version, config content, doctor report, error logs, system info.

## Rust Backend

New module `node_client.rs` using `tokio-tungstenite`. New Tauri commands:

- `doctor_connect(url)` / `doctor_disconnect()`
- `doctor_start(context)` / `doctor_send(message)`
- `doctor_approve_invoke(id)` / `doctor_reject_invoke(id, reason)`
- `collect_doctor_context(instance_id)`

Tauri events emitted to React: `doctor:connected`, `doctor:disconnected`, `doctor:chat-delta`, `doctor:chat-final`, `doctor:invoke`, `doctor:invoke-result`, `doctor:error`.

## Scope

**MVP:** node client + doctor mode in Chat + Doctor page integration. No auth (internal testing).

**Not MVP:** codex/claude code integration, diff preview for writes, diagnosis history, auto agent selection.
