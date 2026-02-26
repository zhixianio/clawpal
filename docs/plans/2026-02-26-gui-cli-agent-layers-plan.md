# Implementation Plan: GUI-CLI-Agent Three-Layer Refactor

Reference design: `docs/plans/2026-02-26-gui-cli-agent-layers-design.md`

## Current State

- Single Rust crate `clawpal` in `src-tauri/`
- `commands.rs` is a 9400-line monolith containing all business logic
- SSH uses `openssh` crate (Unix-only, process-mux based)
- No workspace, no standalone CLI
- Agent tools: arbitrary `system.run` shell commands

## Target State

- Rust workspace with 3 members: `clawpal-core` (lib), `clawpal-cli` (bin), `src-tauri` (Tauri app)
- Business logic in `clawpal-core`, both CLI and Tauri consume it
- SSH uses `russh` (cross-platform, stateless per-call)
- `clawpal` CLI independently runnable
- Agent tools: structured `clawpal` + `openclaw` CLI calls

## Phases

### Phase 1: Workspace Setup + Core Crate Skeleton

**Goal:** Create the workspace structure. Core crate compiles but is mostly empty. Tauri app still works as before.

**Steps:**

1. Create `clawpal-core/Cargo.toml` as a library crate with basic deps (`serde`, `serde_json`, `thiserror`, `tokio`, `chrono`, `uuid`)
2. Create `clawpal-core/src/lib.rs` exporting empty modules matching the design: `instance`, `install`, `connect`, `health`, `ssh`, `profile`, `openclaw`
3. Create `clawpal-cli/Cargo.toml` as a binary crate depending on `clawpal-core` + `clap`
4. Create `clawpal-cli/src/main.rs` with clap subcommand skeleton (all subcommands print "not yet implemented")
5. Convert to workspace: create root `Cargo.toml` with `members = ["clawpal-core", "clawpal-cli", "src-tauri"]`
6. Update `src-tauri/Cargo.toml` to add `clawpal-core` as a path dependency
7. Verify: `cargo build` succeeds for all 3 crates, Tauri app still launches

**Files to create:**
- `/Cargo.toml` (workspace root)
- `/clawpal-core/Cargo.toml`
- `/clawpal-core/src/lib.rs`
- `/clawpal-cli/Cargo.toml`
- `/clawpal-cli/src/main.rs`

**Files to modify:**
- `/src-tauri/Cargo.toml` (add workspace member + core dep)

### Phase 2: Extract Instance Registry

**Goal:** Move instance management to core. First module to prove the extraction pattern works.

**Steps:**

1. Define `clawpal-core/src/instance.rs`:
   - `Instance` struct: `{ id, instance_type, label, openclaw_home, clawpal_data_dir, ssh_host_config }`
   - `InstanceType` enum: `Local | Docker | RemoteSsh`
   - `InstanceRegistry` struct with `load()`, `save()`, `list()`, `add()`, `remove()`, `get()`
   - Reads/writes `~/.clawpal/instances.json`
   - Currently instances are stored in frontend localStorage (`clawpal_docker_instances` key in App.tsx) and SSH hosts in a separate Tauri-managed store. Unify into one registry.

2. Wire into `clawpal-cli`:
   - `clawpal instance list` calls `InstanceRegistry::load()?.list()` and prints JSON
   - `clawpal instance remove <id>` calls `InstanceRegistry::load()?.remove(id)?.save()`

3. Wire into `src-tauri`: add a Tauri command that calls `clawpal_core::instance::InstanceRegistry` instead of the current localStorage-based approach. Frontend migration can happen later.

4. Verify: `clawpal instance list` works from terminal, Tauri app still compiles

### Phase 3: Extract OpenClaw CLI Runner

**Goal:** Move `resolve_openclaw_bin()`, `run_openclaw()`, `parse_json_output()` to core.

**Steps:**

1. Create `clawpal-core/src/openclaw.rs`:
   - Move `resolve_openclaw_bin()` from `commands.rs:26-86` -- binary resolution with PATH, homebrew, npm-global, nvm probing
   - Move `run_openclaw()` and `run_openclaw_with_env()` from `cli_runner.rs:65-138`
   - Move `parse_json_output()` from `cli_runner.rs:140-186`
   - Public API: `OpenclawCli::new() -> Self`, `run(&self, args) -> Result<CliOutput>`, `run_with_env(&self, args, env) -> Result<CliOutput>`
   - `CliOutput` struct: `{ stdout, stderr, exit_code }`

2. Update `src-tauri/src/commands.rs` and `cli_runner.rs` to call `clawpal_core::openclaw` instead of local functions. Remove duplicated code.

3. Verify: `cargo build`, existing Tauri app functionality unchanged

### Phase 4: Extract Health Check

**Goal:** Move health check logic to core so both CLI and Tauri can use it.

**Steps:**

1. Create `clawpal-core/src/health.rs`:
   - `check_instance(instance: &Instance) -> Result<HealthStatus>`
   - For local/docker: calls openclaw CLI with appropriate `OPENCLAW_HOME`
   - For SSH: connects via SSH, runs openclaw remotely
   - `HealthStatus` struct: `{ healthy: bool, active_agents: u32, version: Option<String> }`

2. Wire into CLI: `clawpal health check <id>` and `clawpal health check --all`

3. Wire into Tauri: replace health polling in `commands.rs` (`get_status_light`, `get_status_extra`) with calls to core

4. Verify: `clawpal health check local` works from terminal

### Phase 5: Replace SSH with russh

**Goal:** Replace `openssh` crate with `russh` for cross-platform SSH. Stateless model.

**Steps:**

1. Add `russh`, `russh-keys`, `russh-sftp` to `clawpal-core/Cargo.toml`
2. Remove `openssh` from `src-tauri/Cargo.toml` (and the `cfg(unix)` gate)

3. Create `clawpal-core/src/ssh/mod.rs`:
   - `SshSession` struct -- single connection, not a pool
   - `SshSession::connect(config: &SshHostConfig) -> Result<Self>`
     - Supports key auth and ssh-agent auth
     - Handles passphrase-protected keys
   - `session.exec(cmd: &str) -> Result<ExecResult>`
   - `session.sftp_read(path) -> Result<Vec<u8>>`
   - `session.sftp_write(path, content) -> Result<()>`
   - `Drop` impl disconnects automatically

4. Create `clawpal-core/src/ssh/config.rs`:
   - Parse `~/.ssh/config` for host aliases, port, user, identity file
   - Reuse existing parsing logic from `ssh.rs`

5. Create `clawpal-core/src/ssh/registry.rs`:
   - SSH host CRUD backed by the instance registry (SSH hosts are instances with type `RemoteSsh`)
   - Move `list_ssh_hosts`, `upsert_ssh_host`, `delete_ssh_host` logic from `commands.rs`

6. Update `src-tauri/` to use `clawpal_core::ssh` instead of the old `ssh.rs` module:
   - For Tauri (GUI), wrap `SshSession` in an optional connection cache for responsiveness
   - The cache is Tauri-side only, not in core

7. Wire into CLI:
   - `clawpal ssh connect <host-id>` -- connect and verify, then exit
   - `clawpal ssh list` -- list registered SSH hosts from registry

8. Delete `src-tauri/src/ssh.rs` once migration is complete

9. Verify: SSH operations work on macOS. Cross-platform testing (Windows/Linux) can follow.

### Phase 6: Extract Profile Management

**Goal:** Move model profile CRUD to core.

**Steps:**

1. Create `clawpal-core/src/profile.rs`:
   - Move profile types and logic from `commands.rs:549-741`
   - `list_profiles(openclaw: &OpenclawCli) -> Result<Vec<ModelProfile>>`
   - `upsert_profile(openclaw, profile) -> Result<()>`
   - `delete_profile(openclaw, id) -> Result<()>`
   - `test_profile(openclaw, id) -> Result<TestResult>`

2. Wire into CLI: `clawpal profile list`, `add`, `remove`, `test`

3. Update Tauri commands to delegate to core

4. Verify: `clawpal profile list` works

### Phase 7: Extract Install Logic

**Goal:** Move install orchestration to core. This is the most complex extraction.

**Steps:**

1. Move `src-tauri/src/install/` contents into `clawpal-core/src/install/`:
   - `types.rs` -> `clawpal-core/src/install/types.rs` (already well-structured)
   - `runners/` -> `clawpal-core/src/install/runners/` (local, docker, remote_ssh, wsl2)
   - `commands.rs` state machine logic -> `clawpal-core/src/install/mod.rs`
   - Replace SSH runner's use of `SshConnectionPool` with `clawpal_core::ssh::SshSession`

2. Add coarse-grained functions in `clawpal-core/src/install/mod.rs`:
   - `install_docker(options: DockerInstallOptions) -> Result<InstallResult>`
     - Internally calls: `docker::pull() -> configure() -> up()` then `health::check_instance()`
     - On success, registers instance via `InstanceRegistry`
   - `install_local(options: LocalInstallOptions) -> Result<InstallResult>`

3. Expose fine-grained functions:
   - `docker::pull(options) -> Result<StepResult>`
   - `docker::configure(options) -> Result<StepResult>`
   - `docker::up(options) -> Result<StepResult>`

4. Wire into CLI:
   - `clawpal install docker [--home PATH]` -> coarse
   - `clawpal install docker pull` -> fine
   - `clawpal install docker configure` -> fine
   - `clawpal install docker up` -> fine
   - `clawpal install local` -> coarse

5. Wire into Tauri: replace `install_commands.rs` to call core

6. Verify: `clawpal install docker --home /tmp/test-install` runs the full pipeline

### Phase 8: Extract Connect Logic

**Goal:** Registering existing instances without installation.

**Steps:**

1. Create `clawpal-core/src/connect.rs`:
   - `connect_docker(home: &str, label: Option<&str>) -> Result<Instance>` -- validates path exists, registers in `InstanceRegistry`
   - `connect_ssh(host_config: SshHostConfig) -> Result<Instance>` -- validates connectivity, registers

2. Wire into CLI:
   - `clawpal connect docker --home PATH [--label NAME]`
   - `clawpal connect ssh --host H [--port P] [--user U]`

3. Wire into Tauri: replace InstallHub "Connect Existing" form handler

4. Verify: `clawpal connect docker --home ~/.clawpal/docker-local` registers and `clawpal instance list` shows it

### Phase 9: Update Agent Tool Set

**Goal:** Replace `system.run` with structured `clawpal` + `openclaw` tools in the zeroclaw adapter.

**Steps:**

1. Update `runtime/zeroclaw/adapter.rs` (doctor adapter):
   - Change system prompt to describe the two CLI tools instead of `system.run`
   - Tool schema: `clawpal(args: string)` and `openclaw(args: string, instance?: string)`

2. Update `runtime/zeroclaw/install_adapter.rs`:
   - Same tool set change

3. Update tool execution in `doctor_commands.rs` (`doctor_approve_invoke`):
   - When tool is `clawpal`: parse args, route to `clawpal_core` function directly (in-process)
   - When tool is `openclaw`: route to `clawpal_core::openclaw::run()` with optional `OPENCLAW_HOME` from instance

4. Remove the old `system.run` execution path and command whitelist/path blacklist security checks (no longer needed -- agent can only call bounded CLI commands)

5. Verify: doctor diagnosis works with new tool set, agent correctly calls `clawpal health check` instead of raw `curl`

### Phase 10: Update GUI

**Goal:** GUI uses core for deterministic ops, agent only for exception handling.

**Steps:**

1. Update `InstallHub.tsx`:
   - Default flow: call Tauri IPC -> `clawpal_core::install::install_docker()` directly
   - On success: close dialog, instance appears on StartPage
   - On failure: show error + "Let AI help" button -> launches agent chat with error context

2. Update `StartPage.tsx`:
   - Health polling: call Tauri IPC -> `clawpal_core::health::check_instance()`
   - SSH check: call Tauri IPC -> `clawpal_core::ssh::SshSession::connect()` then health check
   - Instance list: Tauri IPC -> `clawpal_core::instance::InstanceRegistry::list()`

3. Remove `InstallHub` agent chat as default mode (keep it as fallback behind "AI help" button)

4. Verify: full user flow works -- install docker -> instance appears -> health check green

## Ordering and Dependencies

```
Phase 1 (workspace skeleton)
  |
Phase 2 (instance registry) --- used by everything below
  |
Phase 3 (openclaw runner) ----- used by health, profile, install
  |
Phase 4 (health check)
Phase 5 (russh SSH) ----------- used by remote health, remote install
Phase 6 (profiles)
  | (all above complete)
Phase 7 (install) ------------- depends on openclaw, health, ssh, instance
Phase 8 (connect) ------------- depends on instance, ssh
  |
Phase 9 (agent tools) --------- depends on core being complete
Phase 10 (GUI update) --------- depends on agent tools + all core
```

Phases 4, 5, 6 can run in parallel once 2 and 3 are done.

## Verification Criteria

Each phase must pass:
- `cargo build` for all workspace members
- `cargo test` for clawpal-core (add unit tests for each module)
- Tauri app compiles and launches
- CLI subcommands for that phase produce correct output
- `npx tsc --noEmit` passes (frontend changes)

Final verification:
- `clawpal instance list` shows local + docker + ssh instances
- `clawpal install docker --home /tmp/test` runs full pipeline
- `clawpal health check --all` reports status for all instances
- `clawpal profile list` shows model profiles
- Doctor agent uses `clawpal`/`openclaw` tools instead of `system.run`
- InstallHub default path is deterministic (no agent), agent only on failure
