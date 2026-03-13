# Recipe Platform Runtime Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** 在不引入远端守护进程的前提下，先把 `RecipeInstance / Run / Artifact / ResourceClaim` 做成本地可追踪运行时，并接入现有页面。

**Architecture:** runtime 数据先落在本地 `.clawpal/recipe-runtime/` 的 JSON index 中，作为 phase 1 临时状态层。这样可以先打通实例列表、运行记录、产物视图和资源占用展示，后续再平滑迁到 VPS 侧 SQLite。

**Deferred / Not in phase 1:** 本计划只覆盖本地 `.clawpal/recipe-runtime/` JSON store、实例/运行/产物索引和页面展示。phase 1 明确不包含远端 `reciped`、workflow engine、durable scheduler state、OPA/Rego policy plane、secret broker 或 lock manager；任何远端常驻控制面、集中策略决策、集中密钥分发和分布式锁统一留到 phase 2。

**Tech Stack:** Rust, Tauri, React 18, TypeScript, JSON persistence, Bun, Cargo

---

### Task 1: 增加运行时 store 与索引模型

**Files:**
- Create: `src-tauri/src/recipe_store.rs`
- Modify: `src-tauri/src/models.rs`
- Modify: `src-tauri/src/lib.rs`
- Test: `src-tauri/src/recipe_store_tests.rs`

**Step 1: Write the failing tests**

```rust
#[test]
fn record_run_persists_instance_and_artifacts() {
    let store = RecipeStore::for_test();
    let run = store.record_run(sample_run()).unwrap();
    assert_eq!(store.list_runs("inst_01").unwrap()[0].id, run.id);
}
```

**Step 2: Run tests to verify they fail**

Run: `cargo test recipe_store_tests`
Expected: FAIL because the runtime store does not exist.

**Step 3: Write the minimal implementation**

- 定义 `RecipeInstance`, `Run`, `Artifact`, `ResourceClaim`
- 在 `.clawpal/recipe-runtime/` 下保存最小 JSON index
- 支持 `record_run`, `list_runs`, `list_instances`

**Step 4: Run tests to verify they pass**

Run: `cargo test recipe_store_tests`
Expected: PASS

**Step 5: Commit**

```bash
git add src-tauri/src/recipe_store.rs src-tauri/src/recipe_store_tests.rs src-tauri/src/models.rs src-tauri/src/lib.rs
git commit -m "feat: add recipe runtime store for instances and runs"
```

### Task 2: 把 runtime 数据接到现有页面

**Files:**
- Modify: `src/pages/Recipes.tsx`
- Modify: `src/pages/Orchestrator.tsx`
- Modify: `src/pages/History.tsx`
- Modify: `src/lib/api.ts`
- Modify: `src/lib/types.ts`
- Test: `src/pages/__tests__/Recipes.test.tsx`
- Test: `src/pages/__tests__/Orchestrator.test.tsx`

**Step 1: Write the failing tests**

```tsx
it("shows recipe instance status and recent run summary", async () => {
  render(<Recipes onCook={() => {}} />);
  expect(await screen.findByText(/recent run/i)).toBeInTheDocument();
});
```

```tsx
it("shows artifacts and resource claims in orchestrator", async () => {
  render(<Orchestrator />);
  expect(await screen.findByText(/resource claims/i)).toBeInTheDocument();
});
```

**Step 2: Run tests to verify they fail**

Run: `bun test src/pages/__tests__/Recipes.test.tsx src/pages/__tests__/Orchestrator.test.tsx`
Expected: FAIL because the pages do not render runtime data yet.

**Step 3: Write the minimal implementation**

- `Recipes.tsx` 增加实例状态、最近运行、进入 dashboard 的入口
- `Orchestrator.tsx` 展示 run timeline、artifact 列表、resource claims
- `History.tsx` 只补最小链接，不复制一套新的历史系统

**Step 4: Run tests to verify they pass**

Run: `bun test src/pages/__tests__/Recipes.test.tsx src/pages/__tests__/Orchestrator.test.tsx`
Expected: PASS

**Step 5: Commit**

```bash
git add src/pages/Recipes.tsx src/pages/Orchestrator.tsx src/pages/History.tsx src/lib/api.ts src/lib/types.ts src/pages/__tests__/Recipes.test.tsx src/pages/__tests__/Orchestrator.test.tsx
git commit -m "feat: surface recipe runtime state in recipes and orchestrator pages"
```

### Task 3: 记录 phase 2 迁移边界，避免 phase 1 过度设计

**Files:**
- Modify: `docs/plans/2026-03-11-recipe-platform-foundation-plan.md`
- Modify: `docs/plans/2026-03-11-recipe-platform-executor-plan.md`
- Modify: `docs/plans/2026-03-11-recipe-platform-runtime-plan.md`

**Step 1: Write the failing check**

创建一个人工 checklist，逐条确认这 3 份计划没有把以下内容混进 phase 1：
- 远端 `reciped`
- workflow engine
- scheduler durable state
- OPA/Rego policy plane
- secret broker / lock manager

**Step 2: Run the check**

Run: `rg -n "reciped|workflow|scheduler|OPA|Rego|secret broker|lock manager" docs/plans/2026-03-11-recipe-platform-*-plan.md`
Expected: only deferred or explicitly excluded references remain.

**Step 3: Write the minimal implementation**

- 在 3 份计划中补 “Deferred / Not in phase 1” 边界说明
- 确保后续执行不会误把第二阶段内容拉进第一阶段

**Step 4: Run the check again**

Run: `rg -n "reciped|workflow|scheduler|OPA|Rego|secret broker|lock manager" docs/plans/2026-03-11-recipe-platform-*-plan.md`
Expected: only deferred references remain.

**Step 5: Commit**

```bash
git add docs/plans/2026-03-11-recipe-platform-foundation-plan.md docs/plans/2026-03-11-recipe-platform-executor-plan.md docs/plans/2026-03-11-recipe-platform-runtime-plan.md
git commit -m "docs: clarify phase boundaries for recipe runtime rollout"
```
