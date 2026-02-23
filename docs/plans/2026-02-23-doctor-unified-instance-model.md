# Doctor Unified Instance Model

Date: 2026-02-23

> Supersedes: `2026-02-22-doctor-2x2-matrix-design.md`

## TL;DR

**Current tab = Target. Pick another instance as Doctor.**

No more local/remote distinction. All instances are peers.

## Previous Model (2x2 Matrix)

The previous design treated "Agent Source" and "Execution Target" as independent axes, creating a 2x2 matrix:

```
Agent Source × Execution Target = 4 combinations
- Local Gateway → Local
- Local Gateway → Remote SSH
- Remote Gateway → Local
- Remote Gateway → Remote SSH
```

This was conceptually clean but created unnecessary complexity in the UI and code.

## New Model (Unified Instances)

ClawPal already manages multiple OpenClaw instances through its Instance tabs (Local + SSH hosts). The new model leverages this directly:

```
┌──────────────────────────────────────────────────────────────────┐
│  ClawPal                                                         │
│                                                                  │
│  Instance Tabs:  [Local]  [server-1]  [server-2]                 │
│                     │                                            │
│                     ▼                                            │
│              Current Tab = Target                                │
│              (machine being diagnosed)                           │
│                                                                  │
│  ─────────────────────────────────────────────────────────────   │
│                                                                  │
│  Select Doctor:                                                  │
│  ○ server-1          ← excludes current target                   │
│  ○ server-2                                                      │
│  ○ Remote Doctor Service   (coming soon)                         │
│  ○ Codex / Claude Code     (future)                              │
│                                                                  │
└──────────────────────────────────────────────────────────────────┘
```

### Core Principles

1. **All instances are peers** — no special "local" vs "remote" treatment
2. **Current tab = Target** — auto-inferred, no separate selector needed
3. **Doctor = any other instance** — the source selector excludes the current target
4. **Self-diagnosis impossible** — you can't pick the same instance as both target and doctor

### User Mental Model

> "I'm looking at a broken machine. I pick a working machine to fix it."

That's it. No matrix, no axes, no cognitive overhead.

## Implementation

### Target Selection (Auto-Inferred)

```typescript
// From Doctor.tsx
useEffect(() => {
  if (isRemote) {
    doctor.setTarget(instanceId);  // SSH host ID
  } else {
    doctor.setTarget("local");
  }
}, [instanceId, isRemote, doctor.setTarget]);
```

### Doctor Source Selection (Exclude Target)

```typescript
// From Doctor.tsx — agent source radio buttons
{sshHosts
  .filter((h) => h.id !== doctor.target)  // exclude current target
  .map((h) => (
    <label key={h.id}>
      <input type="radio" value={h.id} ... />
      {h.label || h.host}
    </label>
  ))}
```

### Connection Flow

Regardless of which instance is target vs doctor, the connection flow is identical:

1. **Connect to Doctor's gateway** (Operator WS + Bridge TCP)
2. **Collect context from Target** (via SSH if remote, local exec if local)
3. **Start diagnosis** — agent runs on Doctor, commands execute on Target
4. **Route tool calls** — ClawPal receives invokes via Bridge, executes on Target

```
Doctor Gateway (any instance)
    ↕ WebSocket (Operator) + TCP (Bridge)
ClawPal
    ↕ Local exec or SSH
Target Machine (current tab)
```

## Doctor Source Types

| Type | Description | Status |
|------|-------------|--------|
| **Instance Gateway** | Another OpenClaw instance (local or SSH) | ✅ Implemented |
| **Remote Doctor Service** | Hosted at `doctor.openclaw.ai` | 🚧 Coming soon |
| **Coding Agent** | Codex, Claude Code, Gemini CLI | 📋 Future |

### Instance Gateway (Implemented)

Any instance except the current target can be a doctor:

```typescript
const availableDoctors = [
  ...(doctor.target !== "local" ? [{ id: "local", label: "Local" }] : []),
  ...sshHosts.filter((h) => h.id !== doctor.target),
];
```

### Remote Doctor Service (Coming Soon)

A hosted OpenClaw instance at `doctor.openclaw.ai`:

- No user-owned gateway required
- Read-only + suggestions (doesn't modify directly)
- Config sanitization before upload
- Returns repair scripts that ClawPal executes locally

### Coding Agents (Future)

Local or remote Codex / Claude Code / Gemini CLI:

- Detected via `which codex`, `which claude`, etc.
- Launched as subprocess with diagnostic prompt
- Full shell access (with user approval)
- More powerful but requires local installation

## Security Model

Unchanged from previous design:

| Command Type | Approval |
|--------------|----------|
| Read (first time) | User clicks "Allow" |
| Read (pattern approved) | Auto-execute |
| Write | Always requires confirmation |
| Full-Auto mode | Everything auto-executes (opt-in) |

Sensitive paths (`~/.ssh/`, `~/.aws/`, etc.) are **always blocked**.

## UI Changes from Previous Design

### Removed

- Separate "Target" selector — now auto-inferred from current tab
- "Local" vs "Remote" distinction in source selector

### Simplified

- Source selector only shows "other instances" + future options
- Target displayed as read-only badge (from current tab)

### UI Flow

```
1. User navigates to broken instance's tab
2. Opens Doctor page
3. Target auto-set to current instance
4. User picks a working instance as Doctor
5. Clicks "Start Diagnosis"
6. Chat-based diagnosis with tool approval
```

## Migration Notes

### Code Changes

The implementation already follows this model. The 2x2 matrix was a design-phase concept that simplified during implementation.

Key files reflecting the unified model:
- `src/pages/Doctor.tsx` — auto-infer target, filter source
- `src/lib/use-doctor-agent.ts` — target/source agnostic
- `src-tauri/src/doctor_commands.rs` — unified local/remote exec

### Documentation

This document supersedes:
- `2026-02-22-doctor-2x2-matrix-design.md`
- `2026-02-22-doctor-2x2-matrix-implementation.md`

The dual-connection architecture docs remain valid:
- `2026-02-22-doctor-agent-design.md`
- `2026-02-22-doctor-dual-connection-implementation.md`

## Summary

| Aspect | Old (2x2 Matrix) | New (Unified) |
|--------|------------------|---------------|
| Target selection | Manual selector | Auto from current tab |
| Source selection | All instances | Excludes current target |
| Local/Remote | Distinct categories | All instances are peers |
| Mental model | 4 combinations | "Pick another to fix this" |
| Implementation | Conditional logic | Unified flow |

The unified model is simpler to understand, simpler to implement, and matches how users naturally think about multi-machine management.
