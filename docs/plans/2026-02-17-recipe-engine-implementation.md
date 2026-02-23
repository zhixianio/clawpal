# Recipe Engine Redesign Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Replace the single-patch recipe system with composable multi-step recipes executed via a wizard UI.

**Architecture:** Recipes declare ordered `steps` (each referencing a registered action type). The frontend action registry maps each action to an API call + human-readable description. Cook page becomes a 4-phase wizard: fill params → confirm steps → execute → done. A new `apply_config_patch` backend command handles inline merge patches for the `config_patch` action (decoupled from recipe ID lookup).

**Tech Stack:** Rust (Tauri backend), TypeScript/React (frontend), shadcn/ui components

---

### Task 1: Update Rust Recipe struct and recipes.json

**Files:**
- Modify: `src-tauri/src/recipe.rs`
- Modify: `src-tauri/recipes.json`

**Step 1:** Update `Recipe` struct in `recipe.rs`. Remove `patch_template`, `impact_category`, `impact_summary`. Add `steps` field.

Replace the current `Recipe` struct (lines 33-46) with:

```rust
#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct RecipeStep {
    pub action: String,
    pub label: String,
    pub args: Map<String, Value>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct Recipe {
    pub id: String,
    pub name: String,
    pub description: String,
    pub version: String,
    pub tags: Vec<String>,
    pub difficulty: String,
    pub params: Vec<RecipeParam>,
    pub steps: Vec<RecipeStep>,
}
```

Also update `build_candidate_config` to accept a raw template string instead of a `Recipe`:

```rust
pub fn build_candidate_config_from_template(
    current: &Value,
    template: &str,
    params: &Map<String, Value>,
) -> Result<(Value, Vec<ChangeItem>), String> {
    let rendered = render_patch_template(template, params);
    let patch: Value = json5::from_str(&rendered).map_err(|e| e.to_string())?;
    let mut merged = current.clone();
    let mut changes = Vec::new();
    apply_merge_patch(&mut merged, &patch, "", &mut changes);
    Ok((merged, changes))
}
```

Remove the old `build_candidate_config` function that takes `&Recipe`. Remove `impact_category` usage in that function. Keep `render_patch_template`, `apply_merge_patch`, `format_diff`, `ChangeItem`, `PreviewResult`, `ApplyResult` unchanged.

**Step 2:** Update `recipes.json` with the new format — two recipes:

```json
{
  "recipes": [
    {
      "id": "dedicated-channel-agent",
      "name": "Create dedicated Agent for Channel",
      "description": "Create an independent agent, set its identity, bind it to a Discord channel, and configure persona",
      "version": "1.0.0",
      "tags": ["discord", "agent", "persona"],
      "difficulty": "easy",
      "params": [
        { "id": "agent_id", "label": "Agent ID", "type": "string", "required": true, "placeholder": "e.g. my-bot" },
        { "id": "guild_id", "label": "Guild", "type": "discord_guild", "required": true },
        { "id": "channel_id", "label": "Channel", "type": "discord_channel", "required": true },
        { "id": "name", "label": "Display Name", "type": "string", "required": true, "placeholder": "e.g. MyBot" },
        { "id": "emoji", "label": "Emoji", "type": "string", "required": false, "placeholder": "e.g. 🤖" },
        { "id": "persona", "label": "Persona", "type": "textarea", "required": true, "placeholder": "You are..." }
      ],
      "steps": [
        { "action": "create_agent", "label": "Create independent agent", "args": { "agentId": "{{agent_id}}", "independent": true } },
        { "action": "setup_identity", "label": "Set agent identity", "args": { "agentId": "{{agent_id}}", "name": "{{name}}", "emoji": "{{emoji}}" } },
        { "action": "bind_channel", "label": "Bind channel to agent", "args": { "channelType": "discord", "peerId": "{{channel_id}}", "agentId": "{{agent_id}}" } },
        { "action": "config_patch", "label": "Set channel persona", "args": { "patchTemplate": "{\"channels\":{\"discord\":{\"guilds\":{\"{{guild_id}}\":{\"channels\":{\"{{channel_id}}\":{\"systemPrompt\":\"{{persona}}\"}}}}}}}" } }
      ]
    },
    {
      "id": "discord-channel-persona",
      "name": "Channel Persona",
      "description": "Set a custom persona for a Discord channel",
      "version": "1.0.0",
      "tags": ["discord", "persona", "beginner"],
      "difficulty": "easy",
      "params": [
        { "id": "guild_id", "label": "Guild", "type": "discord_guild", "required": true },
        { "id": "channel_id", "label": "Channel", "type": "discord_channel", "required": true },
        { "id": "persona", "label": "Persona", "type": "textarea", "required": true, "placeholder": "You are..." }
      ],
      "steps": [
        { "action": "config_patch", "label": "Set channel persona", "args": { "patchTemplate": "{\"channels\":{\"discord\":{\"guilds\":{\"{{guild_id}}\":{\"channels\":{\"{{channel_id}}\":{\"systemPrompt\":\"{{persona}}\"}}}}}}}" } }
      ]
    }
  ]
}
```

**Step 3:** Verify Rust compiles.

Run: `cargo check --manifest-path src-tauri/Cargo.toml`
Expected: compiles with possible warnings about unused `build_candidate_config` (now renamed)

**Step 4:** Commit.

```bash
git add src-tauri/src/recipe.rs src-tauri/recipes.json
git commit -m "refactor: update Recipe struct to use steps, remove patchTemplate"
```

---

### Task 2: Add apply_config_patch backend command

**Files:**
- Modify: `src-tauri/src/commands.rs`
- Modify: `src-tauri/src/lib.rs`

The `config_patch` step action needs a backend command that applies an inline JSON merge patch template to the config, without going through recipe ID lookup.

**Step 1:** Add `apply_config_patch` command to `commands.rs`. Put it near the existing `apply_recipe` command:

```rust
#[tauri::command]
pub fn apply_config_patch(
    patch_template: String,
    params: Map<String, Value>,
) -> Result<ApplyResult, String> {
    let paths = resolve_paths();
    ensure_dirs(&paths)?;
    let current = read_openclaw_config(&paths)?;
    let current_text = serde_json::to_string_pretty(&current).map_err(|e| e.to_string())?;
    let snapshot = add_snapshot(
        &paths.history_dir,
        &paths.metadata_path,
        Some("config-patch".into()),
        "apply",
        true,
        &current_text,
        None,
    )?;
    let (candidate, _changes) = build_candidate_config_from_template(&current, &patch_template, &params)?;
    write_json(&paths.config_path, &candidate)?;
    Ok(ApplyResult {
        ok: true,
        snapshot_id: Some(snapshot.id),
        config_path: paths.config_path.to_string_lossy().to_string(),
        backup_path: Some(snapshot.config_path),
        warnings: Vec::new(),
        errors: Vec::new(),
    })
}
```

Update the import in `commands.rs` to use `build_candidate_config_from_template` instead of `build_candidate_config`.

**Step 2:** Update the existing `preview_apply` and `apply_recipe` commands — these still reference `build_candidate_config` and `recipe.impact_category` / `recipe.patch_template`. Since recipes no longer have `patch_template`, these commands are no longer needed. Remove them from `commands.rs`.

Also remove `preview_apply` and `apply_recipe` from the handler list in `lib.rs` and add `apply_config_patch`.

**Step 3:** Register `apply_config_patch` in `lib.rs`:
- Add to imports: `apply_config_patch`
- Add to `generate_handler![]`: `apply_config_patch`
- Remove `preview_apply` and `apply_recipe` from both imports and handler

**Step 4:** Verify Rust compiles.

Run: `cargo check --manifest-path src-tauri/Cargo.toml`

**Step 5:** Commit.

```bash
git add src-tauri/src/commands.rs src-tauri/src/lib.rs
git commit -m "feat: add apply_config_patch command, remove preview_apply/apply_recipe"
```

---

### Task 3: Update TypeScript types and API bindings

**Files:**
- Modify: `src/lib/types.ts`
- Modify: `src/lib/api.ts`
- Delete: `src/lib/recipe_catalog.ts`

**Step 1:** Update `types.ts`. Replace the `Recipe` interface and add `RecipeStep`:

```typescript
export interface RecipeStep {
  action: string;
  label: string;
  args: Record<string, unknown>;
}

export interface Recipe {
  id: string;
  name: string;
  description: string;
  version: string;
  tags: string[];
  difficulty: "easy" | "normal" | "advanced";
  params: RecipeParam[];
  steps: RecipeStep[];
}
```

Remove `action?: string`, `patchTemplate: string`, `impactCategory: string`, `impactSummary: string` from the old Recipe interface. Remove `PreviewResult` and `ChangeItem` types (no longer used by the frontend — preview_apply is removed).

**Step 2:** Update `api.ts`:

Remove `previewApply` and `applyRecipe` methods. Add `applyConfigPatch`:

```typescript
applyConfigPatch: (patchTemplate: string, params: Record<string, string>): Promise<ApplyResult> =>
  invoke("apply_config_patch", { patchTemplate, params }),
```

**Step 3:** Delete `src/lib/recipe_catalog.ts` (dead code).

**Step 4:** Verify TypeScript compiles.

Run: `npx tsc --noEmit`
Expected: errors in Cook.tsx, RecipeCard.tsx (they reference removed fields) — that's OK, we'll fix those in the next tasks.

**Step 5:** Commit.

```bash
git add src/lib/types.ts src/lib/api.ts
git rm src/lib/recipe_catalog.ts
git commit -m "refactor: update TS types for steps-based recipes, remove dead code"
```

---

### Task 4: Create action registry

**Files:**
- Create: `src/lib/actions.ts`

**Step 1:** Create `src/lib/actions.ts` with the action registry:

```typescript
import { api } from "./api";

export interface ActionDef {
  execute: (args: Record<string, unknown>) => Promise<unknown>;
  describe: (args: Record<string, unknown>) => string;
}

function renderArgs(
  args: Record<string, unknown>,
  params: Record<string, string>,
): Record<string, unknown> {
  const result: Record<string, unknown> = {};
  for (const [key, value] of Object.entries(args)) {
    if (typeof value === "string") {
      let rendered = value;
      for (const [paramId, paramValue] of Object.entries(params)) {
        rendered = rendered.replaceAll(`{{${paramId}}}`, paramValue);
      }
      result[key] = rendered;
    } else {
      result[key] = value;
    }
  }
  return result;
}

const registry: Record<string, ActionDef> = {
  create_agent: {
    execute: (args) =>
      api.createAgent(
        args.agentId as string,
        args.modelProfileId as string | undefined,
        args.independent as boolean | undefined,
      ),
    describe: (args) =>
      `Create ${args.independent ? "independent " : ""}agent "${args.agentId}"`,
  },
  setup_identity: {
    execute: (args) =>
      api.setupAgentIdentity(
        args.agentId as string,
        args.name as string,
        args.emoji as string | undefined,
      ),
    describe: (args) => {
      const emoji = args.emoji ? ` ${args.emoji}` : "";
      return `Set identity: ${args.name}${emoji}`;
    },
  },
  bind_channel: {
    execute: (args) =>
      api.assignChannelAgent(
        args.channelType as string,
        args.peerId as string,
        args.agentId as string,
      ),
    describe: (args) =>
      `Bind ${args.channelType} channel → agent "${args.agentId}"`,
  },
  config_patch: {
    execute: (args) =>
      api.applyConfigPatch(args.patchTemplate as string, args.params as Record<string, string>),
    describe: (_args) => "", // Uses step.label instead
  },
  set_global_model: {
    execute: (args) =>
      api.setGlobalModel(args.profileId as string),
    describe: (args) =>
      `Set default model to ${args.profileId}`,
  },
};

export function getAction(actionType: string): ActionDef | undefined {
  return registry[actionType];
}

export interface ResolvedStep {
  index: number;
  action: string;
  label: string;
  args: Record<string, unknown>;
  description: string;
}

export function resolveSteps(
  steps: { action: string; label: string; args: Record<string, unknown> }[],
  params: Record<string, string>,
): ResolvedStep[] {
  return steps.map((step, index) => {
    const resolved = renderArgs(step.args, params);
    // For config_patch, inject the params so the backend can do template substitution
    if (step.action === "config_patch") {
      resolved.params = params;
    }
    const actionDef = getAction(step.action);
    const description = actionDef?.describe(resolved) || step.label;
    return {
      index,
      action: step.action,
      label: step.label,
      args: resolved,
      description: description || step.label,
    };
  });
}

export async function executeStep(step: ResolvedStep): Promise<void> {
  const actionDef = getAction(step.action);
  if (!actionDef) {
    throw new Error(`Unknown action type: ${step.action}`);
  }
  await actionDef.execute(step.args);
}
```

**Step 2:** Verify TypeScript compiles (may still have errors in Cook.tsx/RecipeCard.tsx — OK).

Run: `npx tsc --noEmit 2>&1 | grep -v 'Cook\|RecipeCard\|Recipes\|Home'`

**Step 3:** Commit.

```bash
git add src/lib/actions.ts
git commit -m "feat: add action registry for step-based recipe execution"
```

---

### Task 5: Rewrite Cook.tsx as wizard

**Files:**
- Modify: `src/pages/Cook.tsx`

**Step 1:** Rewrite Cook.tsx with the 4-phase wizard:

```typescript
import { useEffect, useState } from "react";
import { api } from "../lib/api";
import { ParamForm } from "../components/ParamForm";
import { resolveSteps, executeStep, type ResolvedStep } from "../lib/actions";
import { Card, CardContent } from "@/components/ui/card";
import { Button } from "@/components/ui/button";
import { cn } from "@/lib/utils";
import type { DiscordGuildChannel, Recipe } from "../lib/types";

type Phase = "params" | "confirm" | "execute" | "done";
type StepStatus = "pending" | "running" | "done" | "failed" | "skipped";

export function Cook({
  recipeId,
  onDone,
  recipeSource,
  discordGuildChannels,
}: {
  recipeId: string;
  onDone?: () => void;
  recipeSource?: string;
  discordGuildChannels: DiscordGuildChannel[];
}) {
  const [recipe, setRecipe] = useState<Recipe | null>(null);
  const [params, setParams] = useState<Record<string, string>>({});
  const [phase, setPhase] = useState<Phase>("params");
  const [resolvedSteps, setResolvedSteps] = useState<ResolvedStep[]>([]);
  const [stepStatuses, setStepStatuses] = useState<StepStatus[]>([]);
  const [stepErrors, setStepErrors] = useState<Record<number, string>>({});
  const [hasConfigPatch, setHasConfigPatch] = useState(false);

  useEffect(() => {
    api.listRecipes(recipeSource).then((recipes) => {
      const found = recipes.find((it) => it.id === recipeId);
      setRecipe(found || null);
      if (found) {
        const defaults: Record<string, string> = {};
        for (const p of found.params) {
          defaults[p.id] = "";
        }
        setParams(defaults);
      }
    });
  }, [recipeId, recipeSource]);

  if (!recipe) return <div>Recipe not found</div>;

  const handleNext = () => {
    const steps = resolveSteps(recipe.steps, params);
    setResolvedSteps(steps);
    setStepStatuses(steps.map(() => "pending"));
    setStepErrors({});
    setHasConfigPatch(steps.some((s) => s.action === "config_patch"));
    setPhase("confirm");
  };

  const handleExecute = async () => {
    setPhase("execute");
    const statuses: StepStatus[] = resolvedSteps.map(() => "pending");
    setStepStatuses([...statuses]);

    for (let i = 0; i < resolvedSteps.length; i++) {
      if (statuses[i] === "skipped") continue;
      statuses[i] = "running";
      setStepStatuses([...statuses]);

      try {
        await executeStep(resolvedSteps[i]);
        statuses[i] = "done";
      } catch (err) {
        statuses[i] = "failed";
        setStepErrors((prev) => ({ ...prev, [i]: String(err) }));
        setStepStatuses([...statuses]);
        // Stop on failure — user can retry or skip
        return;
      }
      setStepStatuses([...statuses]);
    }
    setPhase("done");
  };

  const handleRetry = async (index: number) => {
    const statuses = [...stepStatuses];
    statuses[index] = "running";
    setStepStatuses(statuses);
    setStepErrors((prev) => {
      const next = { ...prev };
      delete next[index];
      return next;
    });

    try {
      await executeStep(resolvedSteps[index]);
      statuses[index] = "done";
      setStepStatuses([...statuses]);

      // Continue executing remaining steps
      for (let i = index + 1; i < resolvedSteps.length; i++) {
        if (statuses[i] === "skipped") continue;
        statuses[i] = "running";
        setStepStatuses([...statuses]);
        try {
          await executeStep(resolvedSteps[i]);
          statuses[i] = "done";
        } catch (err) {
          statuses[i] = "failed";
          setStepErrors((prev) => ({ ...prev, [i]: String(err) }));
          setStepStatuses([...statuses]);
          return;
        }
        setStepStatuses([...statuses]);
      }
      setPhase("done");
    } catch (err) {
      statuses[index] = "failed";
      setStepErrors((prev) => ({ ...prev, [index]: String(err) }));
      setStepStatuses([...statuses]);
    }
  };

  const handleSkip = (index: number) => {
    const statuses = [...stepStatuses];
    statuses[index] = "skipped";
    setStepStatuses(statuses);
    setStepErrors((prev) => {
      const next = { ...prev };
      delete next[index];
      return next;
    });

    // Continue with next steps
    const remaining = resolvedSteps.slice(index + 1);
    if (remaining.length === 0 || remaining.every((_, i) => statuses[index + 1 + i] === "skipped")) {
      setPhase("done");
      return;
    }
    // Trigger execution of remaining steps
    (async () => {
      for (let i = index + 1; i < resolvedSteps.length; i++) {
        if (statuses[i] === "skipped") continue;
        statuses[i] = "running";
        setStepStatuses([...statuses]);
        try {
          await executeStep(resolvedSteps[i]);
          statuses[i] = "done";
        } catch (err) {
          statuses[i] = "failed";
          setStepErrors((prev) => ({ ...prev, [i]: String(err) }));
          setStepStatuses([...statuses]);
          return;
        }
        setStepStatuses([...statuses]);
      }
      setPhase("done");
    })();
  };

  const statusIcon = (s: StepStatus) => {
    switch (s) {
      case "pending": return "○";
      case "running": return "◉";
      case "done": return "✓";
      case "failed": return "✗";
      case "skipped": return "–";
    }
  };

  const statusColor = (s: StepStatus) => {
    switch (s) {
      case "done": return "text-green-600";
      case "failed": return "text-destructive";
      case "running": return "text-primary";
      case "skipped": return "text-muted-foreground";
      default: return "text-muted-foreground";
    }
  };

  const doneCount = stepStatuses.filter((s) => s === "done").length;
  const skippedCount = stepStatuses.filter((s) => s === "skipped").length;

  return (
    <section>
      <h2 className="text-2xl font-bold mb-4">{recipe.name}</h2>

      {phase === "params" && (
        <ParamForm
          recipe={recipe}
          values={params}
          onChange={(id, value) => setParams((prev) => ({ ...prev, [id]: value }))}
          onSubmit={handleNext}
          submitLabel="Next"
          discordGuildChannels={discordGuildChannels}
        />
      )}

      {(phase === "confirm" || phase === "execute") && (
        <Card>
          <CardContent>
            <div className="space-y-3">
              {resolvedSteps.map((step, i) => (
                <div key={i} className="flex items-start gap-3">
                  <span className={cn("text-lg font-mono w-5 text-center", statusColor(stepStatuses[i]))}>
                    {statusIcon(stepStatuses[i])}
                  </span>
                  <div className="flex-1">
                    <div className="text-sm font-medium">{step.label}</div>
                    {step.description !== step.label && (
                      <div className="text-xs text-muted-foreground">{step.description}</div>
                    )}
                    {stepErrors[i] && (
                      <div className="text-xs text-destructive mt-1">{stepErrors[i]}</div>
                    )}
                    {stepStatuses[i] === "failed" && (
                      <div className="flex gap-2 mt-1.5">
                        <Button size="sm" variant="outline" onClick={() => handleRetry(i)}>
                          Retry
                        </Button>
                        <Button size="sm" variant="ghost" onClick={() => handleSkip(i)}>
                          Skip
                        </Button>
                      </div>
                    )}
                  </div>
                </div>
              ))}
            </div>
            {phase === "confirm" && (
              <div className="flex gap-2 mt-4">
                <Button onClick={handleExecute}>Execute</Button>
                <Button variant="outline" onClick={() => setPhase("params")}>Back</Button>
              </div>
            )}
          </CardContent>
        </Card>
      )}

      {phase === "done" && (
        <Card>
          <CardContent className="py-8 text-center">
            <div className="text-2xl mb-2">&#10003;</div>
            <p className="text-lg font-medium">
              {doneCount} step{doneCount !== 1 ? "s" : ""} completed
              {skippedCount > 0 && `, ${skippedCount} skipped`}
            </p>
            {hasConfigPatch && (
              <p className="text-sm text-muted-foreground mt-1">
                Use "Apply Changes" in the sidebar to restart the gateway and activate config changes.
              </p>
            )}
            <Button className="mt-4" onClick={onDone}>
              Back to Recipes
            </Button>
          </CardContent>
        </Card>
      )}
    </section>
  );
}
```

**Step 2:** Verify TypeScript compiles.

Run: `npx tsc --noEmit`

**Step 3:** Commit.

```bash
git add src/pages/Cook.tsx
git commit -m "feat: rewrite Cook as step-by-step wizard"
```

---

### Task 6: Update RecipeCard, Recipes page, Home page

**Files:**
- Modify: `src/components/RecipeCard.tsx`
- Modify: `src/pages/Recipes.tsx`
- Modify: `src/pages/Home.tsx`

**Step 1:** Update `RecipeCard.tsx` — remove `impactCategory` display, show step count instead:

```typescript
import type { Recipe } from "../lib/types";
import { Card, CardHeader, CardTitle, CardDescription, CardContent, CardFooter } from "@/components/ui/card";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";

export function RecipeCard({ recipe, onCook }: { recipe: Recipe; onCook: (id: string) => void }) {
  return (
    <Card>
      <CardHeader>
        <CardTitle>{recipe.name}</CardTitle>
        <CardDescription>{recipe.description}</CardDescription>
      </CardHeader>
      <CardContent>
        <div className="flex flex-wrap gap-1.5 mb-2">
          {recipe.tags.map((t) => (
            <Badge key={t} variant="secondary">
              {t}
            </Badge>
          ))}
        </div>
        <p className="text-sm text-muted-foreground">
          {recipe.steps.length} step{recipe.steps.length !== 1 ? "s" : ""} &middot; {recipe.difficulty}
        </p>
      </CardContent>
      <CardFooter>
        <Button onClick={() => onCook(recipe.id)}>
          Cook
        </Button>
      </CardFooter>
    </Card>
  );
}
```

**Step 2:** `Recipes.tsx` — remove `useReducer` since we only need recipes state. Simplify to `useState`:

Replace the full file with:

```typescript
import { useEffect, useState } from "react";
import type { FormEvent } from "react";
import { api } from "../lib/api";
import { RecipeCard } from "../components/RecipeCard";
import type { Recipe } from "../lib/types";
import { Input } from "@/components/ui/input";
import { Button } from "@/components/ui/button";
import { Label } from "@/components/ui/label";

export function Recipes({
  onCook,
}: {
  onCook: (id: string, source?: string) => void;
}) {
  const [recipes, setRecipes] = useState<Recipe[]>([]);
  const [source, setSource] = useState("");
  const [loadedSource, setLoadedSource] = useState<string | undefined>(undefined);
  const [isLoading, setIsLoading] = useState(false);

  const load = (nextSource: string) => {
    setIsLoading(true);
    const value = nextSource.trim();
    api
      .listRecipes(value || undefined)
      .then((r) => {
        setLoadedSource(value || undefined);
        setRecipes(r);
      })
      .catch(() => {})
      .finally(() => setIsLoading(false));
  };

  useEffect(() => {
    load("");
  }, []);

  const onLoadSource = (event: FormEvent) => {
    event.preventDefault();
    load(source);
  };

  return (
    <section>
      <h2 className="text-2xl font-bold mb-4">Recipes</h2>
      <form onSubmit={onLoadSource} className="mb-2 flex items-center gap-2">
        <Label>Recipe source (file path or URL)</Label>
        <Input
          value={source}
          onChange={(event) => setSource(event.target.value)}
          placeholder="/path/recipes.json or https://example.com/recipes.json"
          className="w-[380px]"
        />
        <Button type="submit" className="ml-2">
          {isLoading ? "Loading..." : "Load"}
        </Button>
      </form>
      <p className="text-sm text-muted-foreground mt-0">
        Loaded from: {loadedSource || "builtin / clawpal recipes"}
      </p>
      <div className="grid grid-cols-[repeat(auto-fit,minmax(220px,1fr))] gap-3">
        {recipes.map((recipe) => (
          <RecipeCard
            key={recipe.id}
            recipe={recipe}
            onCook={() => onCook(recipe.id, loadedSource)}
          />
        ))}
      </div>
    </section>
  );
}
```

**Step 3:** Update Home.tsx recipe card rendering — the Home page shows recommended recipes as clickable cards. Update to use `recipe.steps.length` and `recipe.difficulty` instead of `recipe.impactCategory`:

Find the recipe card rendering section (around the "Recommended Recipes" heading) and replace:
```typescript
<div className="text-xs text-muted-foreground mt-2">
  {recipe.difficulty} &middot; {recipe.impactCategory}
</div>
```
with:
```typescript
<div className="text-xs text-muted-foreground mt-2">
  {recipe.steps.length} step{recipe.steps.length !== 1 ? "s" : ""} &middot; {recipe.difficulty}
</div>
```

**Step 4:** Verify TypeScript compiles.

Run: `npx tsc --noEmit`

**Step 5:** Commit.

```bash
git add src/components/RecipeCard.tsx src/pages/Recipes.tsx src/pages/Home.tsx
git commit -m "refactor: update RecipeCard, Recipes, Home for steps-based recipes"
```

---

### Task 7: Clean up unused code

**Files:**
- Modify: `src/lib/state.ts` — check if `lastPreview` is still used anywhere
- Modify: `src-tauri/src/recipe.rs` — remove old `build_candidate_config` if not already removed

**Step 1:** Check if `PreviewResult` and `lastPreview` are still referenced anywhere. They were used by Cook.tsx (removed) and History.tsx (still uses `PreviewResult` for rollback preview). If History still uses it, keep `PreviewResult` in types.ts; otherwise remove it.

Run: `grep -r "PreviewResult\|lastPreview\|setPreview" src/ --include="*.ts" --include="*.tsx"`

Based on findings:
- If only History.tsx uses it: keep `PreviewResult` in types.ts, remove `lastPreview` from `state.ts` (History now uses local `useState`)
- Clean up `state.ts` to remove `lastPreview` and its action if no longer used

**Step 2:** Remove `DiffViewer` import from Cook.tsx if still present (it shouldn't be after the rewrite, but verify).

**Step 3:** Verify both TypeScript and Rust compile.

Run: `npx tsc --noEmit && cargo check --manifest-path src-tauri/Cargo.toml`

**Step 4:** Commit.

```bash
git add -A
git commit -m "chore: clean up unused preview/state code"
```

---

## Verification

After all tasks:

1. `cargo check --manifest-path src-tauri/Cargo.toml` — Rust compiles
2. `npx tsc --noEmit` — TypeScript compiles
3. `npm run dev` — manual test:
   - Go to Recipes → see 2 recipes with step counts
   - Click "Channel Persona" → fill guild/channel/persona → "Next" → see 1 step → "Execute" → completes
   - Click "Create dedicated Agent for Channel" → fill all params → "Next" → see 4 steps with descriptions → "Execute" → watch steps complete one by one
   - Home page recommended recipes still clickable
   - Load external recipe from URL still works
