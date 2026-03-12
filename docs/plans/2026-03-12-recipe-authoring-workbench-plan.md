# Recipe Authoring Workbench Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** 给 ClawPal 的 Recipe 系统补齐“作者态工作台”，支持 fork 内置 recipe、编辑结构化 source、保存到本地 workspace、校验、预览、试跑，以及把运行记录关联回 recipe source。

**Architecture:** 以结构化 recipe source JSON 作为唯一真相，后端负责 parse、validate、plan、save 和 runtime traceability，前端只维护 draft 编辑状态和工作流 UI。内置 recipe 保持只读，通过 `Fork to workspace` 进入工作区；workspace recipe 采用“一文件一个 recipe”的本地模型，默认落到 `~/.clawpal/recipes/workspace/`，保存使用现有原子写入能力。

**Tech Stack:** Tauri 2, Rust, React 18, TypeScript, Bun, Cargo, JSON/JSON5 parsing, current RecipeBundle + ExecutionSpec pipeline

**Deferred / Not in this plan:** 不做远端 recipe 文件编辑，不支持直接写回 HTTP URL source，不做多人协作或云端同步，不做 AST 级 merge/rebase，不做可视化拖拽 builder。

## Delivered Notes

- Status: delivered on branch `chore/recipe-plan-test-fix`
- Task 1 delivered in `d321e81 feat: add recipe workspace storage commands`
- Task 1 test temp-root cleanup follow-up landed in `f4685d4 chore: clean recipe workspace test temp roots`
- Task 2 delivered in `ed17efd feat: add recipe source validation and draft planning`
- Task 3 delivered in `ccb9436 feat: add recipe studio source editor`
- Task 4 delivered in `697c73c feat: add recipe workspace save flows`
- Task 5 delivered in `d0c044e feat: add recipe studio validation and plan sandbox`
- Task 6 delivered in `8268928 feat: execute recipe drafts from studio`
- Task 7 delivered in `b9124bc feat: track recipe source metadata in runtime history`
- Task 8 delivered in `5eff6ad feat: add recipe studio form mode`

## Final Verification

- `cargo test recipe_ --lib`: PASS
- `bun test src/pages/__tests__/RecipeStudio.test.tsx src/pages/__tests__/Recipes.test.tsx src/pages/__tests__/cook-execution.test.ts src/pages/__tests__/Orchestrator.test.tsx src/pages/__tests__/History.test.tsx`: PASS
- `bun run typecheck`: PASS

---

### Task 1: 建立 workspace recipe 文件模型与后端命令

**Files:**
- Create: `src-tauri/src/recipe_workspace.rs`
- Modify: `src-tauri/src/models.rs`
- Modify: `src-tauri/src/config_io.rs`
- Modify: `src-tauri/src/commands/mod.rs`
- Modify: `src-tauri/src/lib.rs`
- Modify: `src/lib/types.ts`
- Modify: `src/lib/api.ts`
- Modify: `src/lib/use-api.ts`
- Test: `src-tauri/src/recipe_workspace_tests.rs`

**Step 1: Write the failing tests**

```rust
#[test]
fn workspace_recipe_save_writes_under_clawpal_recipe_workspace() {
    let store = RecipeWorkspace::for_test();
    let result = store.save_recipe_source("channel-persona", SAMPLE_SOURCE).unwrap();
    assert!(result.path.ends_with("recipes/workspace/channel-persona.recipe.json"));
}

#[test]
fn workspace_recipe_save_rejects_parent_traversal() {
    let store = RecipeWorkspace::for_test();
    assert!(store.save_recipe_source("../escape", SAMPLE_SOURCE).is_err());
}
```

**Step 2: Run tests to verify they fail**

Run: `cargo test recipe_workspace_tests --lib`  
Expected: FAIL because the workspace module and commands do not exist.

**Step 3: Write the minimal implementation**

- 定义 workspace root：`resolve_paths().clawpal_dir.join("recipes").join("workspace")`
- 增加 `RecipeWorkspace` 负责：
  - 规范化 recipe slug
  - 解析 recipe 文件路径
  - 原子读写 source text
  - 列出 workspace recipe 文件
- 新增 Tauri commands：
  - `list_recipe_workspace_entries`
  - `read_recipe_workspace_source`
  - `save_recipe_workspace_source`
  - `delete_recipe_workspace_source`
- 先不做 rename，使用 `Save As` 覆盖 rename 需求
- 前端 types 里增加：
  - `RecipeWorkspaceEntry`
  - `RecipeSourceSaveResult`

**Step 4: Run tests to verify they pass**

Run: `cargo test recipe_workspace_tests --lib`  
Expected: PASS

**Step 5: Commit**

```bash
git add src-tauri/src/recipe_workspace.rs src-tauri/src/models.rs src-tauri/src/config_io.rs src-tauri/src/commands/mod.rs src-tauri/src/lib.rs src/lib/types.ts src/lib/api.ts src/lib/use-api.ts src-tauri/src/recipe_workspace_tests.rs
git commit -m "feat: add recipe workspace storage commands"
```

### Task 2: 增加 raw source 校验、解析和 draft planning API

**Files:**
- Modify: `src-tauri/src/recipe.rs`
- Modify: `src-tauri/src/recipe_adapter.rs`
- Modify: `src-tauri/src/recipe_planner.rs`
- Modify: `src-tauri/src/commands/mod.rs`
- Modify: `src-tauri/src/lib.rs`
- Modify: `src/lib/types.ts`
- Modify: `src/lib/api.ts`
- Modify: `src/lib/use-api.ts`
- Test: `src-tauri/src/recipe_adapter_tests.rs`
- Test: `src-tauri/src/recipe_planner_tests.rs`

**Step 1: Write the failing tests**

```rust
#[test]
fn exported_recipe_source_validates_as_structured_document() {
    let source = export_recipe_source(&builtin_recipe()).unwrap();
    let diagnostics = validate_recipe_source(&source).unwrap();
    assert!(diagnostics.errors.is_empty());
}

#[test]
fn plan_recipe_source_uses_unsaved_draft_text() {
    let plan = plan_recipe_source("channel-persona", SAMPLE_DRAFT_SOURCE, sample_params()).unwrap();
    assert_eq!(plan.summary.recipe_id, "channel-persona");
}
```

**Step 2: Run tests to verify they fail**

Run: `cargo test recipe_adapter_tests recipe_planner_tests --lib`  
Expected: FAIL because raw source validation and draft planning commands do not exist.

**Step 3: Write the minimal implementation**

- 增加基于 source text 的后端入口：
  - `validate_recipe_source`
  - `list_recipes_from_source_text`
  - `plan_recipe_source`
- 诊断结构分三层：
  - parse/schema error
  - bundle/spec consistency error
  - `steps` 与 `actions` 对齐 error
- `plan_recipe_source` 必须支持“未保存 draft”直接预览
- `export_recipe_source` 继续作为 canonicalization 入口
- diagnostics 返回结构化位置和消息，不只是一条字符串

**Step 4: Run tests to verify they pass**

Run: `cargo test recipe_adapter_tests recipe_planner_tests --lib`  
Expected: PASS

**Step 5: Commit**

```bash
git add src-tauri/src/recipe.rs src-tauri/src/recipe_adapter.rs src-tauri/src/recipe_planner.rs src-tauri/src/commands/mod.rs src-tauri/src/lib.rs src/lib/types.ts src/lib/api.ts src/lib/use-api.ts src-tauri/src/recipe_adapter_tests.rs src-tauri/src/recipe_planner_tests.rs
git commit -m "feat: add recipe source validation and draft planning"
```

### Task 3: 建立 Recipe Studio 路由和 Source Mode 编辑器

**Files:**
- Create: `src/pages/RecipeStudio.tsx`
- Create: `src/components/RecipeSourceEditor.tsx`
- Create: `src/components/RecipeValidationPanel.tsx`
- Modify: `src/App.tsx`
- Modify: `src/pages/Recipes.tsx`
- Modify: `src/components/RecipeCard.tsx`
- Modify: `src/lib/types.ts`
- Modify: `src/locales/en.json`
- Modify: `src/locales/zh.json`
- Test: `src/pages/__tests__/RecipeStudio.test.tsx`
- Test: `src/pages/__tests__/Recipes.test.tsx`

**Step 1: Write the failing tests**

```tsx
it("opens studio from recipes and shows editable source", async () => {
  render(<RecipeStudio initialSource={sampleSource} />);
  expect(screen.getByRole("textbox")).toHaveValue(expect.stringContaining('"kind": "ExecutionSpec"'));
});
```

```tsx
it("shows fork button for builtin recipe cards", async () => {
  render(<Recipes initialRecipes={[sampleBuiltinRecipe]} />);
  expect(screen.getByText(/view source/i)).toBeInTheDocument();
  expect(screen.getByText(/fork to workspace/i)).toBeInTheDocument();
});
```

**Step 2: Run tests to verify they fail**

Run: `bun test src/pages/__tests__/RecipeStudio.test.tsx src/pages/__tests__/Recipes.test.tsx`  
Expected: FAIL because studio route and source editor do not exist.

**Step 3: Write the minimal implementation**

- 新增 `RecipeStudio` 页面，支持：
  - source textarea/editor
  - dirty state
  - current recipe label
  - validation summary panel
- `Recipes` 页面增加入口：
  - `View source`
  - `Edit`
  - `Fork to workspace`
- `App.tsx` 增加 recipe studio route 和所需状态：
  - `recipeEditorSource`
  - `recipeEditorRecipeId`
  - `recipeEditorOrigin`
- 内置 recipe 在 studio 中默认只读，fork 后切换为可编辑

**Step 4: Run tests to verify they pass**

Run: `bun test src/pages/__tests__/RecipeStudio.test.tsx src/pages/__tests__/Recipes.test.tsx`  
Expected: PASS

**Step 5: Commit**

```bash
git add src/pages/RecipeStudio.tsx src/components/RecipeSourceEditor.tsx src/components/RecipeValidationPanel.tsx src/App.tsx src/pages/Recipes.tsx src/components/RecipeCard.tsx src/lib/types.ts src/locales/en.json src/locales/zh.json src/pages/__tests__/RecipeStudio.test.tsx src/pages/__tests__/Recipes.test.tsx
git commit -m "feat: add recipe studio source editor"
```

### Task 4: 打通 Save / Save As / New / Delete / Fork 工作流

**Files:**
- Modify: `src/pages/RecipeStudio.tsx`
- Create: `src/components/RecipeSaveDialog.tsx`
- Modify: `src/pages/Recipes.tsx`
- Modify: `src/lib/api.ts`
- Modify: `src/lib/use-api.ts`
- Modify: `src/lib/types.ts`
- Test: `src/pages/__tests__/RecipeStudio.test.tsx`
- Test: `src-tauri/src/recipe_workspace_tests.rs`

**Step 1: Write the failing tests**

```tsx
it("marks studio dirty and saves to workspace file", async () => {
  render(<RecipeStudio initialSource={sampleSource} />);
  await user.type(screen.getByRole("textbox"), "\n");
  await user.click(screen.getByRole("button", { name: /save/i }));
  expect(api.saveRecipeWorkspaceSource).toHaveBeenCalled();
});
```

```rust
#[test]
fn delete_workspace_recipe_removes_saved_file() {
    let store = RecipeWorkspace::for_test();
    let saved = store.save_recipe_source("persona", SAMPLE_SOURCE).unwrap();
    store.delete_recipe_source(saved.slug.as_str()).unwrap();
    assert!(!saved.path.exists());
}
```

**Step 2: Run tests to verify they fail**

Run: `bun test src/pages/__tests__/RecipeStudio.test.tsx`  
Run: `cargo test recipe_workspace_tests --lib`  
Expected: FAIL because save/delete/fork workflows are incomplete.

**Step 3: Write the minimal implementation**

- `RecipeStudio` 支持：
  - `New`
  - `Save`
  - `Save As`
  - `Delete`
  - `Fork builtin recipe`
- `Save` 仅对 workspace recipe 可用
- `Save As` 让用户输入 slug；slug 校验在后端做最终裁决
- 保存成功后重新拉取 `Recipes` 列表，并保持当前 editor 打开的就是保存后的 workspace recipe
- 对未保存离开增加确认

**Step 4: Run tests to verify they pass**

Run: `bun test src/pages/__tests__/RecipeStudio.test.tsx`  
Run: `cargo test recipe_workspace_tests --lib`  
Expected: PASS

**Step 5: Commit**

```bash
git add src/pages/RecipeStudio.tsx src/components/RecipeSaveDialog.tsx src/pages/Recipes.tsx src/lib/api.ts src/lib/use-api.ts src/lib/types.ts src/pages/__tests__/RecipeStudio.test.tsx src-tauri/src/recipe_workspace_tests.rs
git commit -m "feat: add recipe workspace save flows"
```

### Task 5: 在 Studio 中加入 live validation 和 sample params sandbox

**Files:**
- Modify: `src/pages/RecipeStudio.tsx`
- Modify: `src/components/RecipeValidationPanel.tsx`
- Create: `src/components/RecipeSampleParamsForm.tsx`
- Modify: `src/components/RecipePlanPreview.tsx`
- Modify: `src/lib/types.ts`
- Modify: `src/lib/api.ts`
- Modify: `src/lib/use-api.ts`
- Test: `src/pages/__tests__/RecipeStudio.test.tsx`

**Step 1: Write the failing tests**

```tsx
it("shows planner warnings for unsaved draft source", async () => {
  render(<RecipeStudio initialSource={sampleSourceWithOptionalSteps} />);
  await user.type(screen.getByLabelText(/persona/i), "Keep answers concise");
  await user.click(screen.getByRole("button", { name: /preview plan/i }));
  expect(await screen.findByText(/optional step/i)).toBeInTheDocument();
});
```

**Step 2: Run tests to verify they fail**

Run: `bun test src/pages/__tests__/RecipeStudio.test.tsx`  
Expected: FAIL because studio cannot preview draft plans yet.

**Step 3: Write the minimal implementation**

- 增加 sample params form，优先复用现有 `ParamForm` 的字段渲染逻辑
- 调用 `validate_recipe_source` 实时显示 diagnostics
- 调用 `plan_recipe_source` 预览 unsaved draft 的结构化 plan
- 复用现有 `RecipePlanPreview`
- 把 parse error、schema error、plan error 分开展示

**Step 4: Run tests to verify they pass**

Run: `bun test src/pages/__tests__/RecipeStudio.test.tsx`  
Expected: PASS

**Step 5: Commit**

```bash
git add src/pages/RecipeStudio.tsx src/components/RecipeValidationPanel.tsx src/components/RecipeSampleParamsForm.tsx src/components/RecipePlanPreview.tsx src/lib/types.ts src/lib/api.ts src/lib/use-api.ts src/pages/__tests__/RecipeStudio.test.tsx
git commit -m "feat: add recipe studio validation and plan sandbox"
```

### Task 6: 支持 draft recipe 直接进入 Cook 并执行

**Files:**
- Modify: `src/App.tsx`
- Modify: `src/pages/Cook.tsx`
- Modify: `src/pages/cook-execution.ts`
- Modify: `src/pages/cook-plan-context.ts`
- Modify: `src/lib/api.ts`
- Modify: `src/lib/use-api.ts`
- Modify: `src/lib/types.ts`
- Modify: `src-tauri/src/commands/mod.rs`
- Test: `src/pages/__tests__/cook-execution.test.ts`
- Test: `src/pages/__tests__/RecipeStudio.test.tsx`

**Step 1: Write the failing tests**

```tsx
it("can open cook from studio with unsaved draft source", async () => {
  render(<RecipeStudio initialSource={sampleSource} />);
  await user.click(screen.getByRole("button", { name: /cook draft/i }));
  expect(mockNavigate).toHaveBeenCalledWith("cook");
});
```

**Step 2: Run tests to verify they fail**

Run: `bun test src/pages/__tests__/RecipeStudio.test.tsx src/pages/__tests__/cook-execution.test.ts`  
Expected: FAIL because Cook only accepts saved recipe source/path.

**Step 3: Write the minimal implementation**

- `Cook` 增加 `recipeSourceText` 可选输入
- `listRecipes` / `planRecipe` / `executeRecipe` 补 source-text 变体，允许对 draft 直接编译和执行
- 保持 Cook 文案和阶段不变，只扩输入来源
- 如果 draft 未保存，runtime 记录里标记 `sourceOrigin = draft`

**Step 4: Run tests to verify they pass**

Run: `bun test src/pages/__tests__/RecipeStudio.test.tsx src/pages/__tests__/cook-execution.test.ts`  
Expected: PASS

**Step 5: Commit**

```bash
git add src/App.tsx src/pages/Cook.tsx src/pages/cook-execution.ts src/pages/cook-plan-context.ts src/lib/api.ts src/lib/use-api.ts src/lib/types.ts src-tauri/src/commands/mod.rs src/pages/__tests__/cook-execution.test.ts src/pages/__tests__/RecipeStudio.test.tsx
git commit -m "feat: execute recipe drafts from studio"
```

### Task 7: 给 runtime run 补 recipe source traceability

**Files:**
- Modify: `src-tauri/src/recipe_store.rs`
- Modify: `src-tauri/src/commands/mod.rs`
- Modify: `src-tauri/src/history.rs`
- Modify: `src/lib/types.ts`
- Modify: `src/pages/Recipes.tsx`
- Modify: `src/pages/Orchestrator.tsx`
- Modify: `src/pages/History.tsx`
- Test: `src-tauri/src/recipe_store_tests.rs`
- Test: `src/pages/__tests__/Recipes.test.tsx`
- Test: `src/pages/__tests__/Orchestrator.test.tsx`
- Test: `src/pages/__tests__/History.test.tsx`

**Step 1: Write the failing tests**

```rust
#[test]
fn recorded_run_persists_source_digest_and_origin() {
    let store = RecipeStore::for_test();
    let run = sample_run_with_source();
    let recorded = store.record_run(run).unwrap();
    assert_eq!(recorded.source_digest.as_deref(), Some("digest-123"));
    assert_eq!(recorded.source_origin.as_deref(), Some("workspace"));
}
```

**Step 2: Run tests to verify they fail**

Run: `cargo test recipe_store_tests --lib`  
Expected: FAIL because run metadata does not contain source trace fields.

**Step 3: Write the minimal implementation**

- `RecipeRuntimeRun` 增加：
  - `sourceDigest`
  - `sourceVersion`
  - `sourceOrigin`
  - `workspacePath`
- `execute_recipe` 在 record run 前写入这些字段
- `History` / `Orchestrator` / `Recipes` 面板显示“这次运行来自哪份 recipe source”
- 如果 source 来自 workspace，提供“Open in studio”入口

**Step 4: Run tests to verify they pass**

Run: `cargo test recipe_store_tests --lib`  
Run: `bun test src/pages/__tests__/Recipes.test.tsx src/pages/__tests__/Orchestrator.test.tsx src/pages/__tests__/History.test.tsx`  
Expected: PASS

**Step 5: Commit**

```bash
git add src-tauri/src/recipe_store.rs src-tauri/src/commands/mod.rs src-tauri/src/history.rs src/lib/types.ts src/pages/Recipes.tsx src/pages/Orchestrator.tsx src/pages/History.tsx src-tauri/src/recipe_store_tests.rs src/pages/__tests__/Recipes.test.tsx src/pages/__tests__/Orchestrator.test.tsx src/pages/__tests__/History.test.tsx
git commit -m "feat: link runtime runs back to recipe source"
```

### Task 8: 增加 Form Mode，并与 canonical source 双向同步

**Files:**
- Create: `src/lib/recipe-editor-model.ts`
- Create: `src/components/RecipeFormEditor.tsx`
- Modify: `src/pages/RecipeStudio.tsx`
- Modify: `src/components/RecipeSourceEditor.tsx`
- Modify: `src/lib/types.ts`
- Test: `src/lib/__tests__/recipe-editor-model.test.ts`
- Test: `src/pages/__tests__/RecipeStudio.test.tsx`

**Step 1: Write the failing tests**

```ts
it("round-trips metadata params steps and execution template", () => {
  const doc = parseRecipeSource(sampleSource);
  const form = toRecipeEditorModel(doc);
  const nextDoc = fromRecipeEditorModel(form);
  expect(nextDoc.executionSpecTemplate.kind).toBe("ExecutionSpec");
});
```

**Step 2: Run tests to verify they fail**

Run: `bun test src/lib/__tests__/recipe-editor-model.test.ts src/pages/__tests__/RecipeStudio.test.tsx`  
Expected: FAIL because no form model exists.

**Step 3: Write the minimal implementation**

- 定义 canonical editor model，只覆盖：
  - top-level metadata
  - params
  - steps
  - action rows
  - bundle capability/resource lists
- `RecipeStudio` 增加 `Source / Form` 两个 tab
- 双向同步策略：
  - form 修改后重建 canonical source text
  - source 修改后重建 form model
- 任一方向 parse 失败时，保留另一侧最后一个有效快照，不做 silent overwrite

**Step 4: Run tests to verify they pass**

Run: `bun test src/lib/__tests__/recipe-editor-model.test.ts src/pages/__tests__/RecipeStudio.test.tsx`  
Expected: PASS

**Step 5: Commit**

```bash
git add src/lib/recipe-editor-model.ts src/components/RecipeFormEditor.tsx src/pages/RecipeStudio.tsx src/components/RecipeSourceEditor.tsx src/lib/types.ts src/lib/__tests__/recipe-editor-model.test.ts src/pages/__tests__/RecipeStudio.test.tsx
git commit -m "feat: add recipe studio form mode"
```

### Task 9: 文档、回归和收尾

**Files:**
- Modify: `docs/plans/2026-03-12-recipe-authoring-workbench-plan.md`
- Modify: `docs/mvp-checklist.md`
- Modify: `src/locales/en.json`
- Modify: `src/locales/zh.json`

**Step 1: Run full relevant verification**

Run:

```bash
cargo test recipe_ --lib
bun test src/pages/__tests__/RecipeStudio.test.tsx src/pages/__tests__/Recipes.test.tsx src/pages/__tests__/cook-execution.test.ts src/pages/__tests__/Orchestrator.test.tsx src/pages/__tests__/History.test.tsx
bun run typecheck
```

Expected: PASS

**Step 2: Fix any failing assertions and stale copy**

- 更新文案、空态、按钮标签
- 更新 plan 文档中的实际 commit hash
- 把已完成项从 plan 转为 delivered notes

**Step 3: Commit**

```bash
git add docs/plans/2026-03-12-recipe-authoring-workbench-plan.md docs/mvp-checklist.md src/locales/en.json src/locales/zh.json
git commit -m "docs: finalize recipe authoring workbench rollout notes"
```

---

## Recommended Execution Order

1. Task 1-2 先把 workspace source 和 draft validate/plan API 打通。
2. Task 3-4 再做 studio 和 save/fork 流程，形成真正 authoring 闭环。
3. Task 5-6 接上 live preview 和 draft execute，把 authoring 和 Cook 贯通。
4. Task 7 最后补 runtime traceability，保证运行记录可追溯。
5. Task 8 作为完整作者体验的最后一层，在 source mode 稳定后再做。

## Acceptance Criteria

- 可以从内置 recipe 一键 fork 到 workspace。
- 可以在 UI 中直接编辑 canonical recipe source 并保存到本地文件。
- 可以对未保存 draft 做 validate 和 plan preview。
- 可以从 draft 直接进入 Cook 并执行。
- Runtime run 可以追溯到 source digest / source origin / workspace path。
- 至少一个 workspace recipe 可以通过 Form Mode 与 Source Mode 来回切换而不丢关键字段。
