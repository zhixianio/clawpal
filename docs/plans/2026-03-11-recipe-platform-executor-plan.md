# Recipe Platform Executor Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** 把已编译的 `ExecutionSpec` 落到现有 local/remote 执行层，优先支持 systemd-backed `job/service/schedule/attachment`。

**Architecture:** 这一部分不引入独立的 `reciped` 守护进程，而是把 `ExecutionSpec` 物化成当前系统已经擅长的命令计划。local 复用 `install/runners/local.rs`，remote 复用 `install/runners/remote_ssh.rs` 和现有 SSH/SFTP 能力。

**Deferred / Not in phase 1:** 本计划只覆盖 `ExecutionSpec` 到现有 local/SSH runner 的直接物化和执行入口。phase 1 明确不包含远端 `reciped`、workflow engine、durable scheduler state、OPA/Rego policy plane、secret broker 或 lock manager；`schedule` 仅下发 systemd timer/unit，不承担持久调度控制面。

**Tech Stack:** Rust, systemd, systemd-run, SSH/SFTP, Tauri commands, Cargo tests

---

### Task 1: 新增 ExecutionSpec 执行计划物化层

**Files:**
- Create: `src-tauri/src/recipe_executor.rs`
- Create: `src-tauri/src/recipe_runtime/systemd.rs`
- Modify: `src-tauri/src/lib.rs`
- Test: `src-tauri/src/recipe_executor_tests.rs`

**Step 1: Write the failing tests**

```rust
#[test]
fn job_spec_materializes_to_systemd_run_command() {
    let spec = sample_job_spec();
    let plan = materialize_execution_plan(&spec).unwrap();
    assert!(plan.commands.iter().any(|cmd| cmd.join(" ").contains("systemd-run")));
}

#[test]
fn schedule_spec_references_job_launch_ref() {
    let spec = sample_schedule_spec();
    let plan = materialize_execution_plan(&spec).unwrap();
    assert!(plan.resources.iter().any(|ref_id| ref_id == "schedule/hourly"));
}
```

**Step 2: Run tests to verify they fail**

Run: `cargo test recipe_executor_tests`
Expected: FAIL because the executor layer does not exist.

**Step 3: Write the minimal implementation**

- `job` -> `systemd-run --unit clawpal-job-*`
- `service` -> 受控 unit 或 drop-in 文件
- `schedule` -> `systemd timer` + `job` launch target
- `attachment` -> 先只支持 `systemdDropIn` / `envPatch`

**Step 4: Run tests to verify they pass**

Run: `cargo test recipe_executor_tests`
Expected: PASS

**Step 5: Commit**

```bash
git add src-tauri/src/recipe_executor.rs src-tauri/src/recipe_runtime/systemd.rs src-tauri/src/recipe_executor_tests.rs src-tauri/src/lib.rs
git commit -m "feat: materialize recipe specs into systemd execution plans"
```

### Task 2: 接入 local / remote runner

**Files:**
- Modify: `src-tauri/src/install/runners/local.rs`
- Modify: `src-tauri/src/install/runners/remote_ssh.rs`
- Modify: `src-tauri/src/ssh.rs`
- Modify: `src-tauri/src/cli_runner.rs`
- Modify: `src-tauri/src/commands/mod.rs`
- Test: `src-tauri/src/recipe_executor_tests.rs`

**Step 1: Write the failing tests**

```rust
#[test]
fn local_target_uses_local_runner() {
    let route = route_execution(sample_target("local"));
    assert_eq!(route.runner, "local");
}

#[test]
fn remote_target_uses_remote_ssh_runner() {
    let route = route_execution(sample_target("remote"));
    assert_eq!(route.runner, "remote_ssh");
}
```

**Step 2: Run tests to verify they fail**

Run: `cargo test recipe_executor_tests`
Expected: FAIL because routing is not implemented.

**Step 3: Write the minimal implementation**

- 增加 target routing，把 `ExecutionSpec.target` 路由到 local 或 remote SSH
- 保留现有 command queue 能力，`ExecutionSpec` 只负责生成可执行命令列表
- 先不支持 workflow、人工审批恢复、后台持久调度

**Step 4: Run tests to verify they pass**

Run: `cargo test recipe_executor_tests`
Expected: PASS

**Step 5: Commit**

```bash
git add src-tauri/src/install/runners/local.rs src-tauri/src/install/runners/remote_ssh.rs src-tauri/src/ssh.rs src-tauri/src/cli_runner.rs src-tauri/src/commands/mod.rs src-tauri/src/recipe_executor_tests.rs
git commit -m "feat: route recipe execution through local and remote runners"
```

### Task 3: 暴露执行入口与最小回滚骨架

**Files:**
- Modify: `src-tauri/src/commands/mod.rs`
- Modify: `src/lib/api.ts`
- Modify: `src/lib/types.ts`
- Test: `src-tauri/src/recipe_executor_tests.rs`

**Step 1: Write the failing test**

```rust
#[test]
fn execute_recipe_returns_run_id_and_summary() {
    let result = execute_recipe(sample_execution_request()).unwrap();
    assert!(!result.run_id.is_empty());
}
```

**Step 2: Run test to verify it fails**

Run: `cargo test recipe_executor_tests`
Expected: FAIL because execute API is not exposed.

**Step 3: Write the minimal implementation**

- 增加 `execute_recipe` command
- 返回 `runId`, `instanceId`, `summary`, `warnings`
- 回滚只提供骨架入口，先复用现有 config snapshot / rollback 能力

**Step 4: Run test to verify it passes**

Run: `cargo test recipe_executor_tests`
Expected: PASS

**Step 5: Commit**

```bash
git add src-tauri/src/commands/mod.rs src/lib/api.ts src/lib/types.ts src-tauri/src/recipe_executor_tests.rs
git commit -m "feat: expose recipe execution api and rollback scaffold"
```
