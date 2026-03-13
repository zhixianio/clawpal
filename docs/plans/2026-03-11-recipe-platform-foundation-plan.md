# Recipe Platform Foundation Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** 给 ClawPal 现有 recipe 体系补上 `RecipeBundle -> Runner Contract -> ExecutionSpec` 的基础模型、兼容编译层和 plan preview API。

**Architecture:** 第一部分只做“声明、编译、校验、预览”，不做真正的新执行器。现有 `step-based recipe` 继续可用，但后端会多一层 IR，把现有 recipe 编译成结构化 plan，供审批摘要、diff 和执行摘要复用。

**Deferred / Not in phase 1:** 本计划只覆盖 bundle/schema、兼容编译、静态校验和 plan preview。phase 1 明确不包含远端 `reciped`、workflow engine、durable scheduler state、OPA/Rego policy plane、secret broker 或 lock manager；`secrets` 在这一阶段只保留引用与校验，不引入集中密钥分发或并发协调能力。

**Tech Stack:** Tauri 2, Rust, React 18, TypeScript, Bun, Cargo, JSON Schema, YAML/JSON parsing

---

### Task 1: 新增 RecipeBundle 与 ExecutionSpec 核心模型

**Files:**
- Create: `src-tauri/src/recipe_bundle.rs`
- Create: `src-tauri/src/execution_spec.rs`
- Modify: `src-tauri/src/lib.rs`
- Modify: `src/lib/types.ts`
- Test: `src-tauri/src/recipe_bundle_tests.rs`
- Test: `src-tauri/src/execution_spec_tests.rs`

**Step 1: Write the failing tests**

```rust
#[test]
fn recipe_bundle_rejects_unknown_execution_kind() {
    let raw = r#"apiVersion: strategy.platform/v1
kind: StrategyBundle
execution: { supportedKinds: [workflow] }"#;
    assert!(parse_recipe_bundle(raw).is_err());
}

#[test]
fn execution_spec_rejects_inline_secret_value() {
    let raw = r#"apiVersion: strategy.platform/v1
kind: ExecutionSpec
secrets: { bindings: [{ id: "k", source: "plain://abc" }] }"#;
    assert!(parse_execution_spec(raw).is_err());
}
```

**Step 2: Run tests to verify they fail**

Run: `cargo test recipe_bundle_tests execution_spec_tests`
Expected: FAIL because the modules do not exist yet.

**Step 3: Write the minimal implementation**

- 定义 `RecipeBundle` 最小字段集：`metadata`, `compatibility`, `inputs`, `capabilities`, `resources`, `execution`, `runner`, `outputs`
- 定义 `ExecutionSpec` 最小字段集：`metadata`, `source`, `target`, `execution`, `capabilities`, `resources`, `secrets`, `desired_state`, `actions`, `outputs`
- 先实现 4 个硬约束：
  - `execution.kind` 仅允许 `job | service | schedule | attachment`
  - secret source 不允许明文协议
  - `usedCapabilities` 不得超出 bundle 上限
  - `claims` 不得出现未知 resource kind

**Step 4: Run tests to verify they pass**

Run: `cargo test recipe_bundle_tests execution_spec_tests`
Expected: PASS

**Step 5: Commit**

```bash
git add src-tauri/src/recipe_bundle.rs src-tauri/src/execution_spec.rs src-tauri/src/recipe_bundle_tests.rs src-tauri/src/execution_spec_tests.rs src-tauri/src/lib.rs src/lib/types.ts
git commit -m "feat: add recipe bundle and execution spec primitives"
```

### Task 2: 给现有 step-based recipe 增加兼容编译层

**Files:**
- Create: `src-tauri/src/recipe_adapter.rs`
- Modify: `src-tauri/src/recipe.rs`
- Modify: `src-tauri/src/commands/mod.rs`
- Test: `src-tauri/src/recipe_adapter_tests.rs`

**Step 1: Write the failing test**

```rust
#[test]
fn legacy_recipe_compiles_to_attachment_or_job_spec() {
    let recipe = builtin_recipes().into_iter().find(|r| r.id == "dedicated-channel-agent").unwrap();
    let spec = compile_legacy_recipe_to_spec(&recipe, sample_params()).unwrap();
    assert!(matches!(spec.execution.kind.as_str(), "attachment" | "job"));
}
```

**Step 2: Run test to verify it fails**

Run: `cargo test recipe_adapter_tests`
Expected: FAIL because the adapter does not exist.

**Step 3: Write the minimal implementation**

- 增加 `compile_legacy_recipe_to_spec(recipe, params)` 入口
- `config_patch` 映射到 `attachment` 或 `file` 资源
- `create_agent` / `bind_channel` / `setup_identity` 先映射到 `job` actions
- 保留当前 `recipes.json` 结构，先不引入新的 bundle 文件格式

**Step 4: Run test to verify it passes**

Run: `cargo test recipe_adapter_tests`
Expected: PASS

**Step 5: Commit**

```bash
git add src-tauri/src/recipe_adapter.rs src-tauri/src/recipe.rs src-tauri/src/commands/mod.rs src-tauri/src/recipe_adapter_tests.rs
git commit -m "feat: compile legacy recipes into structured specs"
```

### Task 3: 增加 plan preview API 与确认摘要

**Files:**
- Create: `src-tauri/src/recipe_planner.rs`
- Modify: `src-tauri/src/commands/mod.rs`
- Modify: `src/lib/api.ts`
- Modify: `src/lib/types.ts`
- Create: `src/components/RecipePlanPreview.tsx`
- Modify: `src/pages/Cook.tsx`
- Test: `src-tauri/src/recipe_planner_tests.rs`
- Test: `src/components/__tests__/RecipePlanPreview.test.tsx`

**Step 1: Write the failing tests**

```rust
#[test]
fn plan_recipe_returns_capabilities_claims_and_digest() {
    let plan = build_recipe_plan(sample_bundle(), sample_inputs(), sample_facts()).unwrap();
    assert!(!plan.used_capabilities.is_empty());
    assert!(!plan.concrete_claims.is_empty());
    assert!(!plan.execution_spec_digest.is_empty());
}
```

```tsx
it("renders capability and resource summaries in the confirm phase", async () => {
  render(<RecipePlanPreview plan={samplePlan} />);
  expect(screen.getByText(/service.manage/i)).toBeInTheDocument();
  expect(screen.getByText(/path/i)).toBeInTheDocument();
});
```

**Step 2: Run tests to verify they fail**

Run: `cargo test recipe_planner_tests`
Run: `bun test src/components/__tests__/RecipePlanPreview.test.tsx`
Expected: FAIL because no planning API or preview component exists.

**Step 3: Write the minimal implementation**

- 新增 `plan_recipe` Tauri command
- 返回 `summary`, `usedCapabilities`, `concreteClaims`, `executionSpecDigest`, `warnings`
- `Cook.tsx` 确认阶段改为展示结构化计划，而不是只列 step label

**Step 4: Run tests to verify they pass**

Run: `cargo test recipe_planner_tests`
Run: `bun test src/components/__tests__/RecipePlanPreview.test.tsx`
Expected: PASS

**Step 5: Commit**

```bash
git add src-tauri/src/recipe_planner.rs src-tauri/src/recipe_planner_tests.rs src-tauri/src/commands/mod.rs src/lib/api.ts src/lib/types.ts src/components/RecipePlanPreview.tsx src/components/__tests__/RecipePlanPreview.test.tsx src/pages/Cook.tsx
git commit -m "feat: add recipe planning preview and approval summary"
```
