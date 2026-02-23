# Cron Jobs + Watchdog Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Add a Cron page to ClawPal that shows OpenClaw cron jobs, run history, and a watchdog process that monitors/enforces cron execution.

**Architecture:** Three layers — watchdog.js (Node script deployed to OpenClaw host), Tauri backend commands (read cron data + manage watchdog lifecycle), React page (display jobs + watchdog status). Local and remote instances supported via existing SSH connection pool.

**Tech Stack:** Node.js (watchdog), Rust/Tauri (backend), React + shadcn/ui + i18next (frontend)

---

### Task 1: TypeScript Types

**Files:**
- Modify: `src/lib/types.ts`

**Step 1: Add cron types to end of file**

```typescript
// Cron

export type WatchdogJobStatus = "ok" | "pending" | "triggered" | "retrying" | "escalated";

export interface CronSchedule {
  kind: "cron" | "every" | "at";
  expr?: string;
  tz?: string;
  everyMs?: number;
  at?: string;
}

export interface CronJob {
  jobId: string;
  name: string;
  schedule: CronSchedule;
  sessionTarget: "main" | "isolated";
  agentId?: string;
  enabled: boolean;
  description?: string;
}

export interface CronRun {
  jobId: string;
  startedAt: string;
  endedAt?: string;
  outcome: string;
  error?: string;
}

export interface WatchdogJobState {
  status: WatchdogJobStatus;
  lastScheduledAt?: string;
  lastRunAt?: string | null;
  retries: number;
  lastError?: string;
  escalatedAt?: string;
}

export interface WatchdogStatus {
  pid: number;
  startedAt: string;
  lastCheckAt: string;
  gatewayHealthy: boolean;
  jobs: Record<string, WatchdogJobState>;
}
```

**Step 2: Commit**

```bash
git add src/lib/types.ts
git commit -m "feat(cron): add TypeScript types for cron jobs and watchdog"
```

---

### Task 2: Rust Backend — Cron Data Commands

**Files:**
- Modify: `src-tauri/src/commands.rs`
- Modify: `src-tauri/src/lib.rs`

**Step 1: Add local cron commands at end of commands.rs (before closing)**

Add these after the existing upgrade commands section:

```rust
// ---------------------------------------------------------------------------
// Cron jobs
// ---------------------------------------------------------------------------

#[tauri::command]
pub fn list_cron_jobs() -> Result<Value, String> {
    let paths = resolve_paths();
    let jobs_path = paths.base_dir.join("cron").join("jobs.json");
    if !jobs_path.exists() {
        return Ok(Value::Array(vec![]));
    }
    let text = std::fs::read_to_string(&jobs_path).map_err(|e| e.to_string())?;
    let jobs: Value = serde_json::from_str(&text).unwrap_or(Value::Array(vec![]));
    // jobs.json can be an object with jobId keys or an array
    match jobs {
        Value::Object(map) => {
            let arr: Vec<Value> = map.into_iter().map(|(k, mut v)| {
                if let Value::Object(ref mut obj) = v {
                    obj.entry("jobId".to_string()).or_insert(Value::String(k));
                }
                v
            }).collect();
            Ok(Value::Array(arr))
        }
        Value::Array(_) => Ok(jobs),
        _ => Ok(Value::Array(vec![])),
    }
}

#[tauri::command]
pub fn get_cron_runs(job_id: String, limit: Option<usize>) -> Result<Vec<Value>, String> {
    let paths = resolve_paths();
    let runs_path = paths.base_dir.join("cron").join("runs").join(format!("{}.jsonl", job_id));
    if !runs_path.exists() {
        return Ok(vec![]);
    }
    let text = std::fs::read_to_string(&runs_path).map_err(|e| e.to_string())?;
    let mut runs: Vec<Value> = text.lines()
        .filter(|l| !l.trim().is_empty())
        .filter_map(|l| serde_json::from_str(l).ok())
        .collect();
    runs.reverse(); // newest first
    let limit = limit.unwrap_or(10);
    runs.truncate(limit);
    Ok(runs)
}

#[tauri::command]
pub fn trigger_cron_job(job_id: String) -> Result<String, String> {
    let output = std::process::Command::new("openclaw")
        .args(["cron", "run", &job_id])
        .output()
        .map_err(|e| format!("Failed to run openclaw: {e}"))?;
    let stdout = String::from_utf8_lossy(&output.stdout).to_string();
    let stderr = String::from_utf8_lossy(&output.stderr).to_string();
    if output.status.success() {
        Ok(stdout)
    } else {
        Err(format!("{stdout}\n{stderr}"))
    }
}
```

**Step 2: Add remote cron commands**

```rust
// ---------------------------------------------------------------------------
// Remote cron jobs
// ---------------------------------------------------------------------------

#[tauri::command]
pub async fn remote_list_cron_jobs(pool: State<'_, SshConnectionPool>, host_id: String) -> Result<Value, String> {
    let raw = pool.sftp_read(&host_id, "~/.openclaw/cron/jobs.json").await;
    match raw {
        Ok(text) => {
            let jobs: Value = serde_json::from_str(&text).unwrap_or(Value::Array(vec![]));
            match jobs {
                Value::Object(map) => {
                    let arr: Vec<Value> = map.into_iter().map(|(k, mut v)| {
                        if let Value::Object(ref mut obj) = v {
                            obj.entry("jobId".to_string()).or_insert(Value::String(k));
                        }
                        v
                    }).collect();
                    Ok(Value::Array(arr))
                }
                Value::Array(_) => Ok(jobs),
                _ => Ok(Value::Array(vec![])),
            }
        }
        Err(_) => Ok(Value::Array(vec![])),
    }
}

#[tauri::command]
pub async fn remote_get_cron_runs(pool: State<'_, SshConnectionPool>, host_id: String, job_id: String, limit: Option<usize>) -> Result<Vec<Value>, String> {
    let path = format!("~/.openclaw/cron/runs/{}.jsonl", job_id);
    let raw = pool.sftp_read(&host_id, &path).await;
    match raw {
        Ok(text) => {
            let mut runs: Vec<Value> = text.lines()
                .filter(|l| !l.trim().is_empty())
                .filter_map(|l| serde_json::from_str(l).ok())
                .collect();
            runs.reverse();
            let limit = limit.unwrap_or(10);
            runs.truncate(limit);
            Ok(runs)
        }
        Err(_) => Ok(vec![]),
    }
}

#[tauri::command]
pub async fn remote_trigger_cron_job(pool: State<'_, SshConnectionPool>, host_id: String, job_id: String) -> Result<String, String> {
    let result = pool.exec_login(&host_id, &format!("openclaw cron run {}", job_id)).await?;
    if result.exit_code == 0 {
        Ok(result.stdout)
    } else {
        Err(format!("{}\n{}", result.stdout, result.stderr))
    }
}
```

**Step 3: Register commands in lib.rs**

Add imports at top of `src-tauri/src/lib.rs` (in the `use crate::commands::` block):

```rust
list_cron_jobs, get_cron_runs, trigger_cron_job,
remote_list_cron_jobs, remote_get_cron_runs, remote_trigger_cron_job,
```

Add to `tauri::generate_handler![]` macro:

```rust
list_cron_jobs,
get_cron_runs,
trigger_cron_job,
remote_list_cron_jobs,
remote_get_cron_runs,
remote_trigger_cron_job,
```

**Step 4: Verify compilation**

Run: `cd src-tauri && cargo check`
Expected: Compiles with no errors

**Step 5: Commit**

```bash
git add src-tauri/src/commands.rs src-tauri/src/lib.rs
git commit -m "feat(cron): add Tauri commands for cron job listing and triggering"
```

---

### Task 3: Rust Backend — Watchdog Management Commands

**Files:**
- Modify: `src-tauri/src/commands.rs`
- Modify: `src-tauri/src/lib.rs`

**Step 1: Add local watchdog commands in commands.rs**

```rust
// ---------------------------------------------------------------------------
// Watchdog management
// ---------------------------------------------------------------------------

#[tauri::command]
pub fn get_watchdog_status() -> Result<Value, String> {
    let paths = resolve_paths();
    let wd_dir = paths.base_dir.join("watchdog");
    let status_path = wd_dir.join("status.json");
    let pid_path = wd_dir.join("watchdog.pid");

    let mut status = if status_path.exists() {
        let text = std::fs::read_to_string(&status_path).map_err(|e| e.to_string())?;
        serde_json::from_str::<Value>(&text).unwrap_or(Value::Null)
    } else {
        Value::Null
    };

    // Verify PID is alive
    let alive = if pid_path.exists() {
        let pid_str = std::fs::read_to_string(&pid_path).unwrap_or_default();
        if let Ok(pid) = pid_str.trim().parse::<u32>() {
            std::process::Command::new("kill")
                .args(["-0", &pid.to_string()])
                .output()
                .map(|o| o.status.success())
                .unwrap_or(false)
        } else {
            false
        }
    } else {
        false
    };

    if let Value::Object(ref mut map) = status {
        map.insert("alive".into(), Value::Bool(alive));
        map.insert("deployed".into(), Value::Bool(wd_dir.join("watchdog.js").exists()));
    } else {
        let mut map = serde_json::Map::new();
        map.insert("alive".into(), Value::Bool(alive));
        map.insert("deployed".into(), Value::Bool(wd_dir.join("watchdog.js").exists()));
        status = Value::Object(map);
    }

    Ok(status)
}

#[tauri::command]
pub fn deploy_watchdog(app_handle: tauri::AppHandle) -> Result<bool, String> {
    let paths = resolve_paths();
    let wd_dir = paths.base_dir.join("watchdog");
    std::fs::create_dir_all(&wd_dir).map_err(|e| e.to_string())?;

    let resource_path = app_handle.path()
        .resolve("resources/watchdog.js", tauri::path::BaseDirectory::Resource)
        .map_err(|e| format!("Failed to resolve watchdog resource: {e}"))?;

    let content = std::fs::read_to_string(&resource_path)
        .map_err(|e| format!("Failed to read watchdog resource: {e}"))?;

    std::fs::write(wd_dir.join("watchdog.js"), content).map_err(|e| e.to_string())?;
    Ok(true)
}

#[tauri::command]
pub fn start_watchdog() -> Result<bool, String> {
    let paths = resolve_paths();
    let wd_dir = paths.base_dir.join("watchdog");
    let script = wd_dir.join("watchdog.js");
    let pid_path = wd_dir.join("watchdog.pid");
    let log_path = wd_dir.join("watchdog.log");

    if !script.exists() {
        return Err("Watchdog not deployed. Deploy first.".into());
    }

    // Check if already running
    if pid_path.exists() {
        let pid_str = std::fs::read_to_string(&pid_path).unwrap_or_default();
        if let Ok(pid) = pid_str.trim().parse::<u32>() {
            let alive = std::process::Command::new("kill")
                .args(["-0", &pid.to_string()])
                .output()
                .map(|o| o.status.success())
                .unwrap_or(false);
            if alive {
                return Ok(true); // Already running
            }
        }
    }

    let log_file = std::fs::OpenOptions::new()
        .create(true).append(true)
        .open(&log_path)
        .map_err(|e| e.to_string())?;
    let log_err = log_file.try_clone().map_err(|e| e.to_string())?;

    let child = std::process::Command::new("node")
        .arg(&script)
        .current_dir(&wd_dir)
        .stdout(log_file)
        .stderr(log_err)
        .stdin(std::process::Stdio::null())
        .spawn()
        .map_err(|e| format!("Failed to start watchdog: {e}"))?;

    std::fs::write(&pid_path, child.id().to_string()).map_err(|e| e.to_string())?;
    Ok(true)
}

#[tauri::command]
pub fn stop_watchdog() -> Result<bool, String> {
    let paths = resolve_paths();
    let pid_path = paths.base_dir.join("watchdog").join("watchdog.pid");

    if !pid_path.exists() {
        return Ok(true);
    }

    let pid_str = std::fs::read_to_string(&pid_path).unwrap_or_default();
    if let Ok(pid) = pid_str.trim().parse::<u32>() {
        let _ = std::process::Command::new("kill")
            .arg(pid.to_string())
            .output();
    }

    let _ = std::fs::remove_file(&pid_path);
    Ok(true)
}
```

**Step 2: Add remote watchdog commands**

```rust
// ---------------------------------------------------------------------------
// Remote watchdog management
// ---------------------------------------------------------------------------

#[tauri::command]
pub async fn remote_get_watchdog_status(pool: State<'_, SshConnectionPool>, host_id: String) -> Result<Value, String> {
    let status_raw = pool.sftp_read(&host_id, "~/.openclaw/watchdog/status.json").await;
    let mut status = match status_raw {
        Ok(text) => serde_json::from_str::<Value>(&text).unwrap_or(Value::Null),
        Err(_) => Value::Null,
    };

    // Check PID alive
    let pid_raw = pool.sftp_read(&host_id, "~/.openclaw/watchdog/watchdog.pid").await;
    let alive = match pid_raw {
        Ok(pid_str) => {
            let cmd = format!("kill -0 {} 2>/dev/null && echo alive || echo dead", pid_str.trim());
            pool.exec(&host_id, &cmd).await
                .map(|r| r.stdout.trim() == "alive")
                .unwrap_or(false)
        }
        Err(_) => false,
    };

    // Check deployed
    let deployed = pool.sftp_read(&host_id, "~/.openclaw/watchdog/watchdog.js").await.is_ok();

    if let Value::Object(ref mut map) = status {
        map.insert("alive".into(), Value::Bool(alive));
        map.insert("deployed".into(), Value::Bool(deployed));
    } else {
        let mut map = serde_json::Map::new();
        map.insert("alive".into(), Value::Bool(alive));
        map.insert("deployed".into(), Value::Bool(deployed));
        status = Value::Object(map);
    }

    Ok(status)
}

#[tauri::command]
pub async fn remote_deploy_watchdog(pool: State<'_, SshConnectionPool>, host_id: String, script_content: String) -> Result<bool, String> {
    pool.exec(&host_id, "mkdir -p ~/.openclaw/watchdog").await?;
    pool.sftp_write(&host_id, "~/.openclaw/watchdog/watchdog.js", &script_content).await?;
    Ok(true)
}

#[tauri::command]
pub async fn remote_start_watchdog(pool: State<'_, SshConnectionPool>, host_id: String) -> Result<bool, String> {
    // Check if already running
    let pid_raw = pool.sftp_read(&host_id, "~/.openclaw/watchdog/watchdog.pid").await;
    if let Ok(pid_str) = pid_raw {
        let cmd = format!("kill -0 {} 2>/dev/null && echo alive || echo dead", pid_str.trim());
        if let Ok(r) = pool.exec(&host_id, &cmd).await {
            if r.stdout.trim() == "alive" {
                return Ok(true); // Already running
            }
        }
    }

    let cmd = "cd ~/.openclaw/watchdog && nohup node watchdog.js >> watchdog.log 2>&1 & echo $!";
    let result = pool.exec(&host_id, cmd).await?;
    let pid = result.stdout.trim();
    if !pid.is_empty() {
        pool.sftp_write(&host_id, "~/.openclaw/watchdog/watchdog.pid", pid).await?;
    }
    Ok(true)
}

#[tauri::command]
pub async fn remote_stop_watchdog(pool: State<'_, SshConnectionPool>, host_id: String) -> Result<bool, String> {
    let pid_raw = pool.sftp_read(&host_id, "~/.openclaw/watchdog/watchdog.pid").await;
    if let Ok(pid_str) = pid_raw {
        let _ = pool.exec(&host_id, &format!("kill {} 2>/dev/null", pid_str.trim())).await;
    }
    let _ = pool.exec(&host_id, "rm -f ~/.openclaw/watchdog/watchdog.pid").await;
    Ok(true)
}
```

**Step 3: Register all watchdog commands in lib.rs**

Add imports and handler entries for:
```
get_watchdog_status, deploy_watchdog, start_watchdog, stop_watchdog,
remote_get_watchdog_status, remote_deploy_watchdog, remote_start_watchdog, remote_stop_watchdog,
```

**Step 4: Verify compilation**

Run: `cd src-tauri && cargo check`
Expected: Compiles with no errors

**Step 5: Commit**

```bash
git add src-tauri/src/commands.rs src-tauri/src/lib.rs
git commit -m "feat(cron): add Tauri commands for watchdog management"
```

---

### Task 4: API Layer + i18n

**Files:**
- Modify: `src/lib/api.ts`
- Modify: `src/locales/en.json`
- Modify: `src/locales/zh.json`

**Step 1: Add API methods at end of api object in `src/lib/api.ts`**

```typescript
  // Cron
  listCronJobs: (): Promise<CronJob[]> =>
    invoke("list_cron_jobs", {}),
  getCronRuns: (jobId: string, limit?: number): Promise<CronRun[]> =>
    invoke("get_cron_runs", { jobId, limit }),
  triggerCronJob: (jobId: string): Promise<string> =>
    invoke("trigger_cron_job", { jobId }),

  // Watchdog
  getWatchdogStatus: (): Promise<WatchdogStatus & { alive: boolean; deployed: boolean }> =>
    invoke("get_watchdog_status", {}),
  deployWatchdog: (): Promise<boolean> =>
    invoke("deploy_watchdog", {}),
  startWatchdog: (): Promise<boolean> =>
    invoke("start_watchdog", {}),
  stopWatchdog: (): Promise<boolean> =>
    invoke("stop_watchdog", {}),

  // Remote cron
  remoteListCronJobs: (hostId: string): Promise<CronJob[]> =>
    invoke("remote_list_cron_jobs", { hostId }),
  remoteGetCronRuns: (hostId: string, jobId: string, limit?: number): Promise<CronRun[]> =>
    invoke("remote_get_cron_runs", { hostId, jobId, limit }),
  remoteTriggerCronJob: (hostId: string, jobId: string): Promise<string> =>
    invoke("remote_trigger_cron_job", { hostId, jobId }),

  // Remote watchdog
  remoteGetWatchdogStatus: (hostId: string): Promise<WatchdogStatus & { alive: boolean; deployed: boolean }> =>
    invoke("remote_get_watchdog_status", { hostId }),
  remoteDeployWatchdog: (hostId: string, scriptContent: string): Promise<boolean> =>
    invoke("remote_deploy_watchdog", { hostId, scriptContent }),
  remoteStartWatchdog: (hostId: string): Promise<boolean> =>
    invoke("remote_start_watchdog", { hostId }),
  remoteStopWatchdog: (hostId: string): Promise<boolean> =>
    invoke("remote_stop_watchdog", { hostId }),
```

Also add to the import line at top: `CronJob, CronRun, WatchdogStatus`

**Step 2: Add i18n keys to `src/locales/en.json`**

```json
"nav.cron": "Cron",
"cron.title": "Cron Jobs",
"cron.noJobs": "No cron jobs configured",
"cron.noJobsHint": "Use the OpenClaw CLI to add cron jobs: openclaw cron add",
"cron.name": "Name",
"cron.schedule": "Schedule",
"cron.agent": "Agent",
"cron.lastRun": "Last Run",
"cron.monitor": "Monitor",
"cron.actions": "Actions",
"cron.trigger": "Trigger",
"cron.triggering": "Triggering...",
"cron.triggerSuccess": "Job triggered successfully",
"cron.triggerFailed": "Failed to trigger job: {{error}}",
"cron.runHistory": "Run History",
"cron.noRuns": "No runs yet",
"cron.outcome": "Outcome",
"cron.duration": "Duration",
"cron.status.ok": "OK",
"cron.status.pending": "Pending",
"cron.status.triggered": "Triggered",
"cron.status.retrying": "Retrying",
"cron.status.escalated": "Escalated",
"cron.disabled": "Disabled",
"cron.every": "Every {{interval}}",
"cron.daily": "Daily at {{time}}",
"cron.oneShot": "One-shot: {{time}}",
"watchdog.title": "Watchdog",
"watchdog.running": "Running",
"watchdog.stopped": "Stopped",
"watchdog.crashed": "Crashed",
"watchdog.notDeployed": "Not Deployed",
"watchdog.lastCheck": "Last check: {{time}}",
"watchdog.gateway": "Gateway: {{status}}",
"watchdog.deploy": "Deploy",
"watchdog.deploying": "Deploying...",
"watchdog.start": "Start",
"watchdog.starting": "Starting...",
"watchdog.stop": "Stop",
"watchdog.stopping": "Stopping...",
"watchdog.deploySuccess": "Watchdog deployed",
"watchdog.startSuccess": "Watchdog started",
"watchdog.stopSuccess": "Watchdog stopped",
"watchdog.actionFailed": "Watchdog action failed: {{error}}"
```

**Step 3: Add corresponding Chinese translations to `src/locales/zh.json`**

```json
"nav.cron": "定时任务",
"cron.title": "定时任务",
"cron.noJobs": "未配置定时任务",
"cron.noJobsHint": "使用 OpenClaw CLI 添加定时任务：openclaw cron add",
"cron.name": "名称",
"cron.schedule": "调度",
"cron.agent": "智能体",
"cron.lastRun": "上次执行",
"cron.monitor": "监控",
"cron.actions": "操作",
"cron.trigger": "触发",
"cron.triggering": "触发中...",
"cron.triggerSuccess": "任务已触发",
"cron.triggerFailed": "触发失败：{{error}}",
"cron.runHistory": "执行记录",
"cron.noRuns": "暂无执行记录",
"cron.outcome": "结果",
"cron.duration": "耗时",
"cron.status.ok": "正常",
"cron.status.pending": "等待中",
"cron.status.triggered": "已触发",
"cron.status.retrying": "重试中",
"cron.status.escalated": "需干预",
"cron.disabled": "已禁用",
"cron.every": "每 {{interval}}",
"cron.daily": "每天 {{time}}",
"cron.oneShot": "一次性：{{time}}",
"watchdog.title": "看门狗",
"watchdog.running": "运行中",
"watchdog.stopped": "已停止",
"watchdog.crashed": "异常退出",
"watchdog.notDeployed": "未部署",
"watchdog.lastCheck": "上次检查：{{time}}",
"watchdog.gateway": "网关：{{status}}",
"watchdog.deploy": "部署",
"watchdog.deploying": "部署中...",
"watchdog.start": "启动",
"watchdog.starting": "启动中...",
"watchdog.stop": "停止",
"watchdog.stopping": "停止中...",
"watchdog.deploySuccess": "看门狗已部署",
"watchdog.startSuccess": "看门狗已启动",
"watchdog.stopSuccess": "看门狗已停止",
"watchdog.actionFailed": "看门狗操作失败：{{error}}"
```

**Step 4: Commit**

```bash
git add src/lib/api.ts src/lib/types.ts src/locales/en.json src/locales/zh.json
git commit -m "feat(cron): add API layer and i18n for cron + watchdog"
```

---

### Task 5: Watchdog Script

**Files:**
- Create: `src-tauri/resources/watchdog.js`
- Modify: `src-tauri/tauri.conf.json` (add to bundle resources)

**Step 1: Create `src-tauri/resources/watchdog.js`**

Write the full watchdog script. Zero dependencies, pure Node.js stdlib. Key modules:
- `fs` for file I/O
- `net` for TCP probe
- `child_process` for `openclaw` CLI calls
- Inline cron parser (5-field)

The script must:
1. Self-check for duplicate instance via PID file
2. Read `~/.openclaw/openclaw.json` for gateway port
3. Main loop every 60s:
   - Read `~/.openclaw/cron/jobs.json`
   - For each enabled job, compute if it should have run
   - Check `~/.openclaw/cron/runs/<jobId>.jsonl` for last run
   - If missed: probe gateway, optionally restart, trigger job
   - Backoff: 30s/60s/120s, max 3 retries per schedule cycle
4. Write `~/.openclaw/watchdog/status.json` after each check
5. Handle SIGTERM gracefully (clean PID file)
6. Log to stdout (caller redirects to log file)

The inline cron parser only needs `previousMatchBefore(date)` — given a cron expression and a reference time, return the most recent time it should have fired. For `every` schedule, compute from job creation or last run. For `at`, just parse the ISO date.

**Step 2: Add resource to Tauri bundle**

In `src-tauri/tauri.conf.json`, find the `bundle` section and add:
```json
"resources": ["resources/watchdog.js"]
```

**Step 3: Test locally**

Run: `node src-tauri/resources/watchdog.js --help` (or dry-run mode)
Expected: Script loads without errors

**Step 4: Commit**

```bash
git add src-tauri/resources/watchdog.js src-tauri/tauri.conf.json
git commit -m "feat(cron): add watchdog.js Node script"
```

---

### Task 6: Cron Page Component

**Files:**
- Create: `src/pages/Cron.tsx`

**Step 1: Create the Cron page**

Follow the pattern from `Doctor.tsx`:
- `useInstance()` for remote/local context
- `useTranslation()` for i18n
- `useState` for jobs, runs, watchdog status
- `useEffect` to fetch data on mount and poll every 10s

Layout:
1. **Watchdog control bar** — Card at top with status indicator, last check time, gateway health, deploy/start/stop buttons
2. **Job list** — Table with name, schedule (human-readable), agent, last run, monitor status badge, trigger button
3. **Expanded row** — Collapsible section showing last 10 runs for a job

Status badge colors:
- `ok` → green (`bg-green-500/10 text-green-500`)
- `pending` → gray (`bg-muted text-muted-foreground`)
- `triggered` → blue (`bg-blue-500/10 text-blue-500`)
- `retrying` → yellow (`bg-yellow-500/10 text-yellow-500`)
- `escalated` → red (`bg-red-500/10 text-red-500`)

Helper function `formatSchedule(schedule: CronSchedule): string` to convert schedule to human-readable text.

Remote/local branching pattern for all API calls:
```typescript
const loadJobs = () => {
  const p = isRemote
    ? api.remoteListCronJobs(instanceId)
    : api.listCronJobs();
  p.then(setJobs).catch(() => {});
};
```

**Step 2: Commit**

```bash
git add src/pages/Cron.tsx
git commit -m "feat(cron): add Cron page component"
```

---

### Task 7: Wire Up Route + Navigation

**Files:**
- Modify: `src/App.tsx`

**Step 1: Add "cron" to Route type**

```typescript
type Route = "home" | "recipes" | "cook" | "history" | "channels" | "cron" | "doctor" | "settings";
```

**Step 2: Import Cron page**

```typescript
import { Cron } from "./pages/Cron";
```

**Step 3: Add nav button after Channels button**

Follow exact pattern of other nav buttons. Add between Channels and History:

```tsx
<Button
  variant="ghost"
  className={cn(
    "justify-start hover:bg-accent",
    (route === "cron") && "bg-accent text-accent-foreground border-l-[3px] border-primary"
  )}
  onClick={() => setRoute("cron")}
>
  {t('nav.cron')}
</Button>
```

**Step 4: Add route rendering**

After channels rendering block, add:

```tsx
{route === "cron" && <Cron key={`${activeInstance}`} />}
```

**Step 5: Verify build**

Run: `npm run build`
Expected: Builds with no errors

**Step 6: Commit**

```bash
git add src/App.tsx
git commit -m "feat(cron): wire up Cron page route and navigation"
```

---

### Task 8: Red Dot Badge for Escalated Jobs

**Files:**
- Modify: `src/App.tsx`

**Step 1: Add state for escalated jobs**

Add state in App component:
```typescript
const [hasEscalatedCron, setHasEscalatedCron] = useState(false);
```

**Step 2: Poll watchdog status for escalated jobs**

In the existing status polling useEffect (or create a new one), check watchdog status periodically:

```typescript
useEffect(() => {
  const check = () => {
    const p = isRemote
      ? api.remoteGetWatchdogStatus(activeInstance)
      : api.getWatchdogStatus();
    p.then((status) => {
      if (status?.jobs) {
        const escalated = Object.values(status.jobs).some((j: any) => j.status === "escalated");
        setHasEscalatedCron(escalated);
      } else {
        setHasEscalatedCron(false);
      }
    }).catch(() => setHasEscalatedCron(false));
  };
  check();
  const interval = setInterval(check, 30000);
  return () => clearInterval(interval);
}, [activeInstance, isRemote]);
```

**Step 3: Add red dot to Cron nav button**

Add a relative wrapper and red dot indicator:

```tsx
<Button ... onClick={() => setRoute("cron")}>
  {t('nav.cron')}
  {hasEscalatedCron && (
    <span className="ml-auto w-2 h-2 rounded-full bg-red-500" />
  )}
</Button>
```

**Step 4: Commit**

```bash
git add src/App.tsx
git commit -m "feat(cron): add red dot badge for escalated cron jobs"
```

---

### Task 9: Final Verification

**Step 1: Full build check**

Run: `npm run build`
Expected: No errors

**Step 2: Cargo check**

Run: `cd src-tauri && cargo check`
Expected: No errors

**Step 3: Manual smoke test**

1. Run `npm run dev` (Tauri dev mode)
2. Click "Cron" in sidebar — should show empty state or job list
3. Watchdog control bar should show "Not Deployed" state
4. If OpenClaw has cron jobs configured, they should appear in the table

**Step 4: Final commit with any fixes**

```bash
git add -A
git commit -m "feat(cron): final polish and fixes"
```
