# Code Review Notes (Claude → Codex)

Last updated: 2026-02-27

This file contains review findings and action items from architecture audits. Codex should check this file periodically and work through the items.

## Codex Feedback

Last run: 2026-02-27

| Action | Status | Result |
|--------|--------|--------|
| Action 1: Phase 5 SSH 收口 | PASS | `src-tauri/src/ssh.rs` 中 `SshHostConfig` 已是 core type alias；`SshExecResult` 仍为本地 UI 结果结构且用于连接池执行结果，不是 host registry 类型重复。`cargo update -p clawpal-core` 无变更，`Cargo.lock` 无 `openssh*` 残留。SSH host CRUD 走 `clawpal_core::ssh::registry::{list,upsert,delete}_ssh_host`，底层使用 `InstanceRegistry`。 |
| Action 2: Phase 6/7/8 核验 | PENDING | - |
| Action 3: Phase 9 Agent 工具链确认 | PENDING | - |
| Action 4: Phase 10 GUI 确认 | PENDING | - |
| Action 5: 质量检查 | PENDING | - |

---

## Outstanding Issues

### P1: Remote commands bypass core (long-term migration)

55 个 `remote_*` 函数仍在 `commands.rs`。其中：
- Profile 领域：已迁移到 core（`*_storage_json()` 纯函数），2 个边缘函数 `remote_resolve_api_keys` / `remote_extract_model_profiles_from_config` 仍有内联 Storage struct
- Config 领域：大部分 JSON 操作已通过 `clawpal_core::doctor` 共享（73 处 core 调用），Batch E1 已完成
- 剩余领域（sessions、cron、watchdog、discord、backup 等）：仍直接 SFTP+JSON

按领域逐批迁移，不急。

---

### P1: `commands.rs` 9,367 行

从 9,947 → 9,367（-580 行），随着迁移继续会自然缩减。

---

### P2: Doctor/Install prompt 结构重叠

~60% 内容重复。可考虑抽取 `prompts/common/tool-schema.md`。

---

## Resolved Issues

| Issue | Resolution | Commit |
|-------|-----------|--------|
| Remote profile CRUD bypass core (Phase A) | Core `*_storage_json()` pure functions | `e071d7c` |
| Docker instances localStorage dual-track (Phase B) | Registry-only, legacy migration + cleanup | `8f32491` |
| `extract_json_objects()` 3x duplication (Phase C) | `json_util.rs` shared module | `34d7d86` |
| `{probe:?}` Rust Debug format (Phase C) | `serde_json::to_string()` | `34d7d86` |
| Type duplication (ModelProfile, SshHostConfig) | Type aliases to core | `0b9b621`, `001d199` |
| Doctor commands duplicated in CLI and Tauri | `clawpal-core::doctor` module | `bb671a5` - `3e31a46` |
| `delete_json_path()` duplicated | Unified in core | `bb671a5` |
| Install prompt missing command enumeration | Allowlist + parity test | `54c26a8`, `fa2dd69` |
| Agent tool classification (read vs write) | `tool_intent.rs` | `f9bbf1b` |
| Doctor domain defaults | `doctor_domain_default_relpath()` | `ae23203` |
| `doctor-start.md` double identity | File removed | N/A |
| russh SSH migration (Phase D) | Native russh + legacy fallback | `8dcd0df` |
| Config domain migration (Phase E, Batch E1) | JSON ops → core doctor | `20f20d9` |
| Doctor/Rescue logic migration | Issue parsing, rescue planning, etc. → core | `da8bcdc` - `19563d8` |
| History-preamble strengthened | Tool format, allowlist, constraints re-stated | `68cd029` |
| 2 profile edge functions (`remote_resolve_api_keys`, `remote_extract_model_profiles_from_config`) | Use `list_profiles_from_storage_json()` | `84720c5` |

---

## Known Deferrals (not action items)

- **SSH deterministic install**: SSH/DigitalOcean targets still go through agent chat. Deferred.
- **Native LLM tool calling**: JSON-in-text format. Medium-term migration.

---

## Phase D Code Review Results (2026-02-27)

**Verdict**: ✅ APPROVED with minor recommendations

| Priority | Item | Details |
|----------|------|---------|
| P2 | Host key verification | `check_server_key()` accepts all keys. Implement `~/.ssh/known_hosts` check later |
| P2 | Error detail loss in fallback | `Err(_) => exec_legacy()` drops russh error. Add `tracing::debug!` |
| P3 | Test coverage | Add: auth failure without key, ssh_config parse path |
| P3 | Connection reuse | Per-call model is fine for now |

---

## Next Actions (for Codex)

### Action 1: Phase 5 SSH 收口

1. **确认 ssh.rs 类型无重复**：`src-tauri/src/ssh.rs` 的 `SshHostConfig` 和 `SshExecResult` 应该是 core 的 type alias。如果已经是，打勾跳过。**不要删除 ssh.rs** — Tauri 需要连接状态缓存。
2. **清理 Cargo.lock openssh 残留**：`cargo update -p clawpal-core` 确认无旧 openssh 依赖。
3. **SSH host CRUD 确认只走 core registry**：检查 `ssh/registry.rs` 操作的是 `InstanceRegistry`，无其他存储路径。
4. 以上都是确认项，没问题不需要改代码。有问题才 commit 修复。

### Action 2: Phase 6/7/8 核验

1. CLI 子命令 JSON 输出一致性：`cargo test --test cli_json_contract` 通过即可。
2. `install docker` 端到端：dry-run 测试通过即可（`cargo test -p clawpal-core install`）。
3. `connect docker/ssh` 错误路径：检查错误信息是否有意义（路径不存在、连接失败）。
4. `profile test_profile`：确认不是占位行为（`cargo test -p clawpal-core profile`）。
5. 同样是验证项，测试通过就打勾，有 bug 才 commit。

### Action 3: Phase 9 Agent 工具链确认

1. 全仓搜索 `system.run` 可执行路径：`grep -r "system.run\|system_run" src-tauri/src/ --include="*.rs"` 排除文档和测试字符串。应为零结果。
2. 确认 `doctor_approve_invoke` 的允许集与 prompt 白名单一致：已有 parity test（`fa2dd69`），跑 `cargo test -p clawpal supported_commands` 通过即可。

### Action 4: Phase 10 GUI 确认

**注意：Phase B（`8f32491`）已完成 localStorage 迁移。以下只需确认，不需要重做：**
1. 确认 `LEGACY_DOCKER_INSTANCES_KEY` 仅用于一次性迁移后 `removeItem`，无常驻 fallback。
2. 确认 StartPage/Tab 显示以 `listRegisteredInstances()` 为唯一数据源。
3. 确认 InstallHub 是 deterministic-first（Docker/Local 直接执行，agent 仅失败后触发）。
4. 全部是确认项。

### Action 5: 质量检查

1. `cargo build --workspace` 通过
2. `cargo test --workspace --all-targets` 通过（remote_api 失败忽略，需要 vm1）
3. `npx tsc --noEmit` 通过
4. 清理未提交改动：`git status` 检查，有意义的改动分组 commit，临时文件清理

每个 Action 完成后在 cc.md 的 Codex Feedback 区域更新状态。验证项只需报告结果（PASS/FAIL + 原因），代码改动才需要 commit。

---

## Execution History

| Phase | Status | Commits | Review Notes |
|-------|--------|---------|-------------|
| Phase A: Remote profile → core | **Done** | `e071d7c` | String in/out, 5 new tests |
| Phase B: Docker localStorage → registry | **Done** | `8f32491` | Clean migration |
| Phase C: Runtime hygiene | **Done** | `34d7d86` | json_util.rs, probe serialization |
| Phase D: russh migration | **Done** | `8dcd0df` | Native SSH + fallback. P2 recommendations pending |
| Phase E: Config domain migration | **Done** | `20f20d9` | Batch E1 complete |
| Doctor/Rescue migration | **Done** | `da8bcdc`-`19563d8` | 12 commits, 27 new core tests |
| History-preamble | **Done** | `68cd029` | Both doctor and install strengthened |
