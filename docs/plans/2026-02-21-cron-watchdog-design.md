# Cron Jobs Management + Watchdog

## Problem

OpenClaw's built-in cron scheduler is unreliable — jobs silently fail to trigger. Users have no visibility into what ran and what didn't. There is no fallback mechanism when the scheduler misses a job.

## Solution

Three components:

1. **Cron page in ClawPal** — read-only view of all cron jobs, run history, and watchdog status
2. **Watchdog process** — lightweight Node.js script running on the OpenClaw host, monitors cron execution and force-triggers missed jobs
3. **Tauri backend commands** — bridge between UI and data (cron files + watchdog status)

## Architecture

```
ClawPal UI (Cron page)
  ├── reads → cron/jobs.json (job definitions)
  ├── reads → cron/runs/*.jsonl (execution history)
  ├── reads → watchdog/status.json (monitor state)
  └── controls → watchdog process (deploy/start/stop)

Watchdog process (Node.js, on OpenClaw host)
  ├── reads → cron/jobs.json
  ├── reads → cron/runs/*.jsonl
  ├── reads → openclaw.json (gateway port)
  ├── executes → openclaw cron run <jobId>
  ├── executes → openclaw up (if gateway down)
  └── writes → watchdog/status.json
```

All file paths relative to `~/.openclaw/`.

## Watchdog Process

### Implementation

Single file: `watchdog.js`. Zero external dependencies (pure Node.js stdlib). Stored in Tauri resources (`src-tauri/resources/watchdog.js`), deployed to target machine on demand.

### Deployment

- **Local**: ClawPal copies `watchdog.js` to `~/.openclaw/watchdog/`
- **Remote**: ClawPal pushes via SFTP through existing SSH connection pool

### Lifecycle

- Start: `nohup node ~/.openclaw/watchdog/watchdog.js &`
- PID written to `~/.openclaw/watchdog/watchdog.pid`
- Stop: read PID file, `kill <pid>`, clean up PID file
- Graceful shutdown on `SIGTERM`, cleans PID file on exit
- Logs to `~/.openclaw/watchdog/watchdog.log`, daily rotation, 7 day retention

### Main Loop (every 60 seconds)

1. Read `cron/jobs.json`, filter `enabled: true` jobs
2. For each job, compute last expected trigger time from `schedule` (cron/every/at)
3. Read `cron/runs/<jobId>.jsonl`, get last run record
4. If expected trigger time > last run time (or no run record) → job missed
5. Remediation for missed jobs:
   - TCP probe gateway port (read from `openclaw.json`, default 18789)
   - Port open → `openclaw cron run <jobId> --due`
   - Port closed → `openclaw up`, wait up to 15s for port, then trigger job
   - On failure: retry with backoff 30s → 60s → 120s, max 3 attempts
   - After 3 failures: mark `escalated`, stop retrying until next schedule cycle
6. Write `status.json`

### Cron Expression Parsing

Inline minimal 5-field cron parser. Only needs to compute "last expected trigger time", not a full scheduler. Supports `every` (interval) and `at` (one-shot) natively.

### Multi-instance Safety

- Check PID file before starting; abort if existing process is alive
- watchdog.js self-checks on startup, exits if duplicate detected

## status.json

```json
{
  "pid": 12345,
  "startedAt": "2026-02-21T10:00:00Z",
  "lastCheckAt": "2026-02-21T10:05:00Z",
  "gatewayHealthy": true,
  "jobs": {
    "daily-report": {
      "status": "ok",
      "lastScheduledAt": "2026-02-21T07:00:00Z",
      "lastRunAt": "2026-02-21T07:00:12Z",
      "retries": 0
    },
    "hourly-sync": {
      "status": "escalated",
      "lastScheduledAt": "2026-02-21T10:00:00Z",
      "lastRunAt": null,
      "retries": 3,
      "lastError": "gateway restart failed: port 18789 not responding after 15s",
      "escalatedAt": "2026-02-21T10:04:30Z"
    }
  }
}
```

### Job status values

| Status | Meaning | UI color |
|--------|---------|----------|
| `ok` | Ran on time | green |
| `pending` | Not yet due | gray |
| `triggered` | Watchdog just triggered remediation, awaiting result | blue |
| `retrying` | Trigger failed, backing off | yellow |
| `escalated` | 3 retries exhausted, needs human intervention | red |

Retries and status reset automatically at the next schedule cycle.

## ClawPal UI

### Navigation

Sidebar: new "Cron" item, same level as Agents/Channels. Red dot badge when any job is `escalated`.

### Page Layout

**Watchdog control bar** (top):
- Status indicator (green running / gray stopped / red crashed)
- Last check time (relative, e.g. "2 min ago")
- Gateway health
- Deploy / Start / Stop buttons

**Job list** (table):

| Column | Source |
|--------|--------|
| Name | `jobs.json` → `name` |
| Schedule | Human-readable from `schedule` (e.g. "daily at 7:00", "every 20m") |
| Agent | `agentId` or "main" |
| Last run | Latest entry from `runs/*.jsonl` |
| Monitor status | From `status.json`, colored badge |
| Actions | Manual trigger button, expand for history |

**Expanded row**: Last 10 run records (time, outcome, duration). Error details for `escalated` jobs.

### Not in scope

- No CRUD for cron jobs (use CLI or edit config directly)
- No cross-instance aggregation
- No log viewer (status.json provides structured data)

## Tauri Backend Commands

### Cron data (local + remote variants)

| Command | Description |
|---------|-------------|
| `list_cron_jobs()` | Read `~/.openclaw/cron/jobs.json` |
| `get_cron_runs(job_id, limit)` | Read `~/.openclaw/cron/runs/<jobId>.jsonl`, last N entries |
| `trigger_cron_job(job_id)` | Execute `openclaw cron run <jobId>` |

### Watchdog management (local + remote variants)

| Command | Description |
|---------|-------------|
| `get_watchdog_status()` | Read `status.json` + verify PID alive |
| `deploy_watchdog()` | Write `watchdog.js` to `~/.openclaw/watchdog/` |
| `start_watchdog()` | `nohup node watchdog.js &`, write PID file |
| `stop_watchdog()` | Kill process by PID, clean PID file |

Remote variants (e.g. `remote_list_cron_jobs(host_id)`) use SSH connection pool, same pattern as existing `remote_list_agents_overview`.

### watchdog.js packaging

Bundled in `src-tauri/resources/watchdog.js`. Updated automatically when ClawPal updates. Deploy command handles copying to target machine.

## Security

- Watchdog runs as current user, same permissions as OpenClaw
- Opens no network ports — pure file I/O + CLI invocation
- Remote deployment reuses ClawPal's existing SSH auth, no new credentials
