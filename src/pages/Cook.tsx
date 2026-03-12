import { useEffect, useMemo, useState } from "react";
import { useTranslation } from "react-i18next";
import { ParamForm } from "../components/ParamForm";
import { resolveSteps, type ResolvedStep } from "../lib/actions";
import { useApi } from "@/lib/use-api";
import { Card, CardContent } from "@/components/ui/card";
import { Button } from "@/components/ui/button";
import { cn } from "@/lib/utils";
import type { ExecuteRecipeResult, PrecheckIssue, Recipe, RecipePlan } from "../lib/types";
import { useInstance } from "@/lib/instance-context";
import { RecipePlanPreview } from "@/components/RecipePlanPreview";
import {
  buildCookExecuteRequest,
  markCookFailure,
  markCookStatuses,
  type CookStepStatus,
} from "./cook-execution";
import {
  buildCookContextWarnings,
  buildCookRouteSummary,
  hasBlockingAuthIssues,
} from "./cook-plan-context";
type Phase = "params" | "confirm" | "execute" | "done";

export function Cook({
  recipeId,
  onDone,
  recipeSource,
  recipeSourceText,
  recipeSourceOrigin = "saved",
  recipeWorkspaceSlug,
}: {
  recipeId: string;
  onDone?: () => void;
  recipeSource?: string;
  recipeSourceText?: string;
  recipeSourceOrigin?: "saved" | "draft";
  recipeWorkspaceSlug?: string;
}) {
  const { t } = useTranslation();
  const ua = useApi();
  const { instanceId, isRemote, isDocker } = useInstance();
  const [recipe, setRecipe] = useState<Recipe | null>(null);
  const [loading, setLoading] = useState(true);
  const [params, setParams] = useState<Record<string, string>>({});
  const [phase, setPhase] = useState<Phase>("params");
  const [plan, setPlan] = useState<RecipePlan | null>(null);
  const [planError, setPlanError] = useState<string | null>(null);
  const [executionError, setExecutionError] = useState<string | null>(null);
  const [executionResult, setExecutionResult] = useState<ExecuteRecipeResult | null>(null);
  const [planning, setPlanning] = useState(false);
  const [resolvedStepList, setResolvedStepList] = useState<ResolvedStep[]>([]);
  const [stepStatuses, setStepStatuses] = useState<CookStepStatus[]>([]);
  const [authIssues, setAuthIssues] = useState<PrecheckIssue[]>([]);
  const [contextWarnings, setContextWarnings] = useState<string[]>([]);

  const routeSummary = useMemo(
    () => buildCookRouteSummary({ instanceId, isRemote, isDocker }),
    [instanceId, isRemote, isDocker],
  );
  const blockingAuthIssues = hasBlockingAuthIssues(authIssues);

  useEffect(() => {
    setLoading(true);
    const recipeLoader = recipeSourceText
      ? ua.listRecipesFromSourceText(recipeSourceText)
      : ua.listRecipes(recipeSource);
    recipeLoader.then((recipes) => {
      const found = recipes.find((it) => it.id === recipeId);
      setRecipe(found || null);
      if (found) {
        const defaults: Record<string, string> = {};
        for (const p of found.params) {
          defaults[p.id] = p.defaultValue ?? (p.type === "boolean" ? "false" : "");
        }
        setParams(defaults);
      }
    }).finally(() => setLoading(false));
  }, [recipeId, recipeSource, recipeSourceText]);

  // Pre-populate fields from existing config when channel is selected
  useEffect(() => {
    if (!recipe) return;
    const guildId = params.guild_id;
    const channelId = params.channel_id;
    if (!guildId || !channelId) return;

    // Find textarea params that map to config values via the recipe steps
    const configPaths: Record<string, string> = {};
    for (const step of recipe.steps) {
      if (step.action !== "config_patch" || typeof step.args?.patchTemplate !== "string") continue;
      try {
        const tpl = (step.args.patchTemplate as string)
          .replace(/\{\{guild_id\}\}/g, guildId)
          .replace(/\{\{channel_id\}\}/g, channelId);
        const parsed = JSON.parse(tpl);
        // Walk the parsed object to find {{param}} leaves
        const walk = (obj: Record<string, unknown>, path: string) => {
          for (const [k, v] of Object.entries(obj)) {
            const full = path ? `${path}.${k}` : k;
            if (typeof v === "string") {
              const m = v.match(/^\{\{(\w+)\}\}$/);
              if (m) configPaths[m[1]] = full;
            } else if (v && typeof v === "object") {
              walk(v as Record<string, unknown>, full);
            }
          }
        };
        walk(parsed, "");
      } catch { /* ignore parse errors */ }
    }

    if (Object.keys(configPaths).length === 0) return;

    const readConfig = ua.readRawConfig();

    readConfig.then((raw) => {
      try {
        const cfg = JSON.parse(raw);
        for (const [paramId, path] of Object.entries(configPaths)) {
          const parts = path.split(".");
          let cur: unknown = cfg;
          for (const part of parts) {
            if (cur && typeof cur === "object") cur = (cur as Record<string, unknown>)[part];
            else { cur = undefined; break; }
          }
          if (typeof cur === "string") {
            setParams((prev) => ({ ...prev, [paramId]: cur as string }));
          } else {
            // Clear param when config value is absent (e.g. channel has no persona)
            setParams((prev) => ({ ...prev, [paramId]: "" }));
          }
        }
      } catch { /* ignore */ }
    }).catch(() => { /* ignore config read errors */ });
  }, [recipe, params.guild_id, params.channel_id, isRemote, instanceId]);

  if (loading) return <div className="flex items-center justify-center py-12"><div className="h-6 w-6 animate-spin rounded-full border-2 border-primary border-t-transparent" /></div>;
  if (!recipe) return <div>{t('cook.recipeNotFound')}</div>;

  const handleNext = async () => {
    setPlanning(true);
    setPlanError(null);

    try {
      const nextPlan = recipeSourceText
        ? await ua.planRecipeSource(recipe.id, params, recipeSourceText)
        : await ua.planRecipe(recipe.id, params, recipeSource);
      const [authResult, configResult] = await Promise.allSettled([
        ua.precheckAuth(instanceId),
        ua.readRawConfig(),
      ]);
      const steps = resolveSteps(recipe.steps, params);
      const nextAuthIssues = authResult.status === "fulfilled" ? authResult.value : [];
      const nextContextWarnings =
        configResult.status === "fulfilled"
          ? buildCookContextWarnings(nextPlan, configResult.value)
          : [];
      setPlan(nextPlan);
      setExecutionError(null);
      setExecutionResult(null);
      setResolvedStepList(steps);
      setStepStatuses(steps.map((s) => (s.skippable ? "skipped" : "pending")));
      setAuthIssues(nextAuthIssues);
      setContextWarnings(nextContextWarnings);
      setPhase("confirm");
    } catch (error) {
      setPlan(null);
      setAuthIssues([]);
      setContextWarnings([]);
      setPlanError(String(error));
    } finally {
      setPlanning(false);
    }
  };

  const handleExecute = async () => {
    if (!plan) {
      setExecutionError(t("cook.missingExecutionSpec"));
      return;
    }
    if (blockingAuthIssues) {
      setExecutionError("Resolve auth precheck errors before execution.");
      return;
    }

    setPhase("execute");
    setExecutionError(null);
    setExecutionResult(null);
    setStepStatuses((current) => markCookStatuses(current, "running"));

    try {
      const result = await ua.executeRecipe({
        ...buildCookExecuteRequest(plan.executionSpec, {
          instanceId,
          isRemote,
          isDocker,
        }, recipeSourceOrigin, recipeSourceText, recipeWorkspaceSlug),
      });
      setExecutionResult(result);
      setStepStatuses((current) => markCookStatuses(current, "done"));
      setPhase("done");
    } catch (error) {
      setExecutionError(String(error));
      setExecutionResult(null);
      setStepStatuses((current) => markCookFailure(current));
    }
  };

  const statusIcon = (s: CookStepStatus) => {
    switch (s) {
      case "pending": return "\u25CB";
      case "running": return "\u25C9";
      case "done": return "\u2713";
      case "failed": return "\u2717";
      case "skipped": return "\u2013";
    }
  };

  const statusColor = (s: CookStepStatus) => {
    switch (s) {
      case "done": return "text-green-600";
      case "failed": return "text-destructive";
      case "running": return "text-primary";
      default: return "text-muted-foreground";
    }
  };

  const doneCount = stepStatuses.filter((s) => s === "done").length;
  const skippedCount = stepStatuses.filter((s) => s === "skipped").length;

  return (
    <section>
      <div className="flex items-center gap-2 mb-4">
        <Button variant="ghost" size="sm" className="px-2" onClick={onDone}>
          &larr;
        </Button>
        <h2 className="text-2xl font-bold">{recipe.name}</h2>
      </div>

      {phase === "params" && (
        <>
          {planError && (
            <div className="mb-3 rounded-md border border-destructive/30 bg-destructive/5 px-3 py-2 text-sm text-destructive">
              {planError}
            </div>
          )}
          <ParamForm
            recipe={recipe}
            values={params}
            onChange={(id, value) => setParams((prev) => ({ ...prev, [id]: value }))}
            onSubmit={handleNext}
            submitLabel={planning ? `${t('cook.next')}...` : t('cook.next')}
          />
        </>
      )}

      {(phase === "confirm" || phase === "execute") && (
        <Card>
          <CardContent>
            {plan && phase === "confirm" && (
              <RecipePlanPreview
                plan={plan}
                routeSummary={routeSummary}
                authIssues={authIssues}
                contextWarnings={contextWarnings}
              />
            )}
            <div className="space-y-3">
              {resolvedStepList.map((step, i) => (
                <div key={i} className={cn("flex items-start gap-3", stepStatuses[i] === "skipped" && "opacity-50")}>
                  <span className={cn("text-lg font-mono w-5 text-center", statusColor(stepStatuses[i]))}>
                    {statusIcon(stepStatuses[i])}
                  </span>
                  <div className="flex-1">
                    <div className="text-sm font-medium">
                      {step.label}
                      {stepStatuses[i] === "skipped" && phase === "confirm" && (
                        <span className="text-xs text-muted-foreground ml-2">{t('cook.skippedEmpty')}</span>
                      )}
                    </div>
                    {step.description !== step.label && stepStatuses[i] !== "skipped" && (
                      <div className="text-xs text-muted-foreground">{step.description}</div>
                    )}
                  </div>
                </div>
              ))}
            </div>
            {phase === "confirm" && blockingAuthIssues && (
              <div className="mt-4 rounded-md border border-destructive/30 bg-destructive/5 px-3 py-2 text-sm text-destructive">
                Resolve auth precheck errors before execution.
              </div>
            )}
            {phase === "confirm" && executionError && (
              <div className="mt-4 rounded-md border border-destructive/30 bg-destructive/5 px-3 py-2 text-sm text-destructive">
                {executionError}
              </div>
            )}
            {phase === "execute" && executionError && (
              <div className="mt-4 rounded-md border border-destructive/30 bg-destructive/5 px-3 py-2 text-sm text-destructive">
                <div className="font-medium">{t("cook.executionFailed")}</div>
                <div className="mt-1">{executionError}</div>
              </div>
            )}
            {phase === "confirm" && (
              <div className="flex gap-2 mt-4">
                <Button disabled={blockingAuthIssues} onClick={() => void handleExecute()}>{t('cook.execute')}</Button>
                <Button variant="outline" onClick={() => setPhase("params")}>{t('cook.back')}</Button>
              </div>
            )}
            {phase === "execute" && executionError && (
              <div className="flex gap-2 mt-4">
                <Button onClick={() => void handleExecute()}>{t('cook.retry')}</Button>
                <Button variant="outline" onClick={() => setPhase("confirm")}>{t('cook.back')}</Button>
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
              {t('cook.stepsCompleted', { done: doneCount })}
              {skippedCount > 0 && t('cook.stepsSkipped', { skipped: skippedCount })}
            </p>
            {executionResult && (
              <>
                <p className="text-sm text-muted-foreground mt-2">
                  {executionResult.summary}
                </p>
                <p className="text-xs text-muted-foreground mt-1">
                  {t("cook.runId")}: {executionResult.runId}
                </p>
              </>
            )}
            {(executionResult?.warnings.length ?? 0) > 0 && (
              <div className="mt-4 rounded-md border border-amber-500/30 bg-amber-500/10 p-3 text-left text-sm text-amber-950">
                <div className="font-medium">{t("cook.executionWarnings")}</div>
                {executionResult?.warnings.map((warning) => (
                  <div key={warning} className="mt-1">
                    {warning}
                  </div>
                ))}
              </div>
            )}
            <Button className="mt-4" onClick={onDone}>
              {t('cook.done')}
            </Button>
          </CardContent>
        </Card>
      )}
    </section>
  );
}
