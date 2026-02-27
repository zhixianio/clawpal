# Code Review Notes (Claude → Codex)

Last updated: 2026-02-27

This file contains review findings and action items from architecture audits. Codex should check this file periodically and work through the items.

## Codex Feedback

Last run: 2026-02-27

| Action | Status | Result |
|--------|--------|--------|
| Action 1: Phase 5 SSH 收口 | PASS | `src-tauri/src/ssh.rs` 中 `SshHostConfig` 已是 core type alias；`SshExecResult` 仍为本地 UI 结果结构且用于连接池执行结果，不是 host registry 类型重复。`cargo update -p clawpal-core` 无变更，`Cargo.lock` 无 `openssh*` 残留。SSH host CRUD 走 `clawpal_core::ssh::registry::{list,upsert,delete}_ssh_host`，底层使用 `InstanceRegistry`。 |
| Action 2: Phase 6/7/8 核验 | PASS | `cargo test --test cli_json_contract` 4/4 通过；`cargo test -p clawpal-core install`（含 dry-run 相关）通过；`cargo test -p clawpal-core connect` 覆盖 docker/ssh 连接成功与失败路径通过；`cargo test -p clawpal-core profile` 13/13 通过，`test_profile` 非占位行为。错误文案包含 `remote ssh host not found`、`ssh connect failed`、`remote connectivity probe failed` 等可诊断信息。 |
| Action 3: Phase 9 Agent 工具链确认 | PASS | `grep -RIn \"system.run\\|system_run\" src-tauri/src/ --include=\"*.rs\"` 无结果（可执行路径为 0）；`cargo test -p clawpal supported_commands` 通过（doctor/install prompt allowlist parity tests 通过）。 |
| Action 4: Phase 10 GUI 确认 | PASS | `LEGACY_DOCKER_INSTANCES_KEY` 仅在迁移读取并在迁移成功后 `removeItem`；StartPage/Tab 展示已收口为 `listRegisteredInstances()`（`registeredInstances`）单一来源；`InstallHub` 为 deterministic-first（`docker/local` 直走 deterministic pipeline，`ssh/digitalocean` 先 `installDecideTarget`，仅在无法确定时进入 agent chat）。 |
| Action 5: 质量检查 | PASS (with noted env constraint) | `cargo build --workspace` 通过；`cargo test --workspace --all-targets` 除 `remote_api` 外通过。`remote_api` 失败原因为当前环境无法访问 `192.168.65.2:22`（`Operation not permitted`），按说明忽略。`install_history_preamble_contains_execution_guardrails` 断言漂移已修复并复测通过。`npx tsc --noEmit` 通过。`git status` 已检查，保留用户已有未提交改动（`src-tauri/src/runtime/zeroclaw/*`, `src/lib/use-api.ts`, `.claude/`, `.tmp/`, `scripts/review-loop.sh`）。 |

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
| Phase 5 SSH 收口验证 | Type alias confirmed, no openssh residue, CRUD via InstanceRegistry | `ff14eb7` (验证) |
| Phase 6/7/8 核验 | cli_json_contract 4/4, install dry-run, profile 13/13, connect error paths | `ff14eb7` (验证) |
| Phase 9 Agent 工具链 | No system.run paths, prompt allowlist parity tests pass | `ff14eb7` (验证) |
| Phase 10 GUI 确认 | Legacy key one-shot migration, listRegisteredInstances sole source, InstallHub deterministic-first | `ff14eb7` (验证) |
| Instance display fallback paths removed | Registry-only in App.tsx openTabs + StartPage instancesMap | `506661a` |
| Install history preamble test drift | Assertion aligned to current prompt content | `d327823` |

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

_所有验证 Action 已完成。无新任务。_

如有新一轮工作，Claude 会在此写入。

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
| Verification Actions 1-5 | **Done** | `ff14eb7`-`d327823` | All PASS. Test drift fixed, instance display fallback removed |
