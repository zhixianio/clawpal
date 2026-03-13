import { useEffect, useMemo, useState } from "react";
import { ChevronDownIcon } from "lucide-react";
import { useTranslation } from "react-i18next";
import { ParamForm } from "../components/ParamForm";
import { resolveSteps, type ResolvedStep } from "../lib/actions";
import { useApi } from "@/lib/use-api";
import { Card, CardContent, CardDescription, CardHeader, CardTitle } from "@/components/ui/card";
import { Button } from "@/components/ui/button";
import { Badge } from "@/components/ui/badge";
import { InlineProgressBar } from "@/components/InlineProgressBar";
import { cn } from "@/lib/utils";
import type {
  ExecuteRecipeResult,
  PrecheckIssue,
  Recipe,
  RecipePlan,
  RecipeWorkspaceEntry,
} from "../lib/types";
import { useInstance } from "@/lib/instance-context";
import { RecipePlanPreview } from "@/components/RecipePlanPreview";
import { formatRecipeClaimForPeople } from "@/lib/recipe-run-copy";
import {
  buildCookPhaseItems,
  buildCookExecuteRequest,
  getCookExecutionProgress,
  getCookPlanningProgress,
  markCookFailure,
  markCookStatuses,
  type CookPhase,
  type CookPlanningStage,
  type CookStepStatus,
} from "./cook-execution";
import {
  buildCookAuthProfileScope,
  buildCookContextWarnings,
  buildCookRouteSummary,
  filterCookAuthIssues,
  hasBlockingAuthIssues,
} from "./cook-plan-context";
type Phase = "params" | "confirm" | "execute" | "done";

async function waitForNextPaint(): Promise<void> {
  await new Promise<void>((resolve) => {
    if (
      typeof window !== "undefined" &&
      typeof window.requestAnimationFrame === "function"
    ) {
      window.requestAnimationFrame(() => resolve());
      return;
    }
    setTimeout(resolve, 0);
  });
}

export function Cook({
  recipeId,
  onDone,
  onOpenHistory,
  onOpenRuntimeDashboard,
  recipeSource,
  recipeSourceText,
  recipeSourceOrigin = "saved",
  recipeWorkspaceSlug,
}: {
  recipeId: string;
  onDone?: () => void;
  onOpenHistory?: () => void;
  onOpenRuntimeDashboard?: () => void;
  recipeSource?: string;
  recipeSourceText?: string;
  recipeSourceOrigin?: "saved" | "draft";
  recipeWorkspaceSlug?: string;
}) {
  const { t } = useTranslation();
  const ua = useApi();
  const { instanceId, instanceLabel, isRemote, isDocker } = useInstance();
  const [recipe, setRecipe] = useState<Recipe | null>(null);
  const [loading, setLoading] = useState(true);
  const [params, setParams] = useState<Record<string, string>>({});
  const [phase, setPhase] = useState<Phase>("params");
  const [plan, setPlan] = useState<RecipePlan | null>(null);
  const [planError, setPlanError] = useState<string | null>(null);
  const [executionError, setExecutionError] = useState<string | null>(null);
  const [executionResult, setExecutionResult] = useState<ExecuteRecipeResult | null>(null);
  const [planning, setPlanning] = useState(false);
  const [planningStage, setPlanningStage] = useState<CookPlanningStage | null>(null);
  const [resolvedStepList, setResolvedStepList] = useState<ResolvedStep[]>([]);
  const [stepStatuses, setStepStatuses] = useState<CookStepStatus[]>([]);
  const [authIssues, setAuthIssues] = useState<PrecheckIssue[]>([]);
  const [contextWarnings, setContextWarnings] = useState<string[]>([]);
  const [doneSupportDetailsOpen, setDoneSupportDetailsOpen] = useState(false);
  const [workspaceEntry, setWorkspaceEntry] = useState<RecipeWorkspaceEntry | null>(null);
  const [approvalSubmitting, setApprovalSubmitting] = useState(false);

  const routeSummary = useMemo(
    () => buildCookRouteSummary({ instanceId, instanceLabel, isRemote, isDocker }),
    [instanceId, instanceLabel, isRemote, isDocker],
  );
  const blockingAuthIssues = hasBlockingAuthIssues(authIssues);
  const approvalRequired = Boolean(recipeWorkspaceSlug && workspaceEntry?.approvalRequired);
  const planningProgress = planningStage ? getCookPlanningProgress(planningStage) : null;
  const phaseItems = useMemo(
    () => buildCookPhaseItems(phase as CookPhase),
    [phase],
  );
  const executionProgress =
    phase === "execute"
      ? getCookExecutionProgress(executionError ? "failed" : "running", stepStatuses)
      : phase === "done"
        ? getCookExecutionProgress("done", stepStatuses)
        : null;
  const routeKindLabel =
    routeSummary.kind === "ssh"
      ? t("cook.routeKindSsh")
      : routeSummary.kind === "docker"
        ? t("cook.routeKindDocker")
        : t("cook.routeKindLocal");

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

  useEffect(() => {
    if (!recipeWorkspaceSlug) {
      setWorkspaceEntry(null);
      return;
    }

    let cancelled = false;
    void ua
      .listRecipeWorkspaceEntries()
      .then((entries) => {
        if (cancelled) {
          return;
        }
        setWorkspaceEntry(entries.find((entry) => entry.slug === recipeWorkspaceSlug) ?? null);
      })
      .catch((error) => {
        console.error("Failed to load recipe workspace entry:", error);
        if (!cancelled) {
          setWorkspaceEntry(null);
        }
      });

    return () => {
      cancelled = true;
    };
  }, [recipeWorkspaceSlug]);

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
    setPlanningStage("validate");
    setPlanError(null);
    await waitForNextPaint();

    try {
      setPlanningStage("build");
      const nextPlan = recipeSourceText
        ? await ua.planRecipeSource(recipe.id, params, recipeSourceText)
        : await ua.planRecipe(recipe.id, params, recipeSource);
      setPlanningStage("checks");
      await waitForNextPaint();
      const [authResult, configResult] = await Promise.allSettled([
        ua.precheckAuth(instanceId),
        ua.readRawConfig(),
      ]);
      const steps = resolveSteps(recipe.steps, params);
      const authScope = buildCookAuthProfileScope(nextPlan);
      const nextAuthIssues =
        authResult.status === "fulfilled"
          ? filterCookAuthIssues(authResult.value, authScope)
          : [];
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
      setPlanningStage(null);
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
    if (approvalRequired && recipeWorkspaceSlug) {
      setExecutionError(t("cook.approvalBlocked"));
      return;
    }

    setPhase("execute");
    setExecutionError(null);
    setExecutionResult(null);

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
      setDoneSupportDetailsOpen(false);
      setPhase("done");
    } catch (error) {
      setExecutionError(String(error));
      setExecutionResult(null);
      setStepStatuses((current) => markCookFailure(current));
    }
  };

  const handleApprove = async () => {
    if (!recipeWorkspaceSlug) {
      return;
    }
    setApprovalSubmitting(true);
    setExecutionError(null);
    try {
      await ua.approveRecipeWorkspaceSource(recipeWorkspaceSlug);
      const entries = await ua.listRecipeWorkspaceEntries();
      setWorkspaceEntry(entries.find((entry) => entry.slug === recipeWorkspaceSlug) ?? null);
    } catch (error) {
      console.error("Failed to approve recipe execution:", error);
      setExecutionError(String(error));
    } finally {
      setApprovalSubmitting(false);
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
    <section className="space-y-5">
      <div className="flex items-center gap-2 mb-4">
        <Button variant="ghost" size="sm" className="px-2" onClick={onDone}>
          &larr;
        </Button>
        <div className="min-w-0">
          <h2 className="truncate text-2xl font-bold">{recipe.name}</h2>
          <div className="mt-1 flex flex-wrap items-center gap-2 text-sm text-muted-foreground">
            <Badge variant="outline">{routeKindLabel}</Badge>
            <span>{routeSummary.targetLabel}</span>
          </div>
        </div>
      </div>

      <div className="grid gap-2 md:grid-cols-4">
        {phaseItems.map((item, index) => (
          <div
            key={item.key}
            className={cn(
              "rounded-xl border px-3 py-3 transition-colors",
              item.state === "current" && "border-primary/40 bg-primary/5",
              item.state === "complete" && "border-border/70 bg-muted/20",
              item.state === "upcoming" && "border-border/60 bg-background",
            )}
          >
            <div className="flex items-center gap-2">
              <div
                className={cn(
                  "flex h-6 w-6 items-center justify-center rounded-full text-xs font-semibold",
                  item.state === "current" && "bg-primary text-primary-foreground",
                  item.state === "complete" && "bg-foreground text-background",
                  item.state === "upcoming" && "bg-muted text-muted-foreground",
                )}
              >
                {index + 1}
              </div>
              <div className="min-w-0">
                <div className="text-sm font-medium">{t(item.labelKey)}</div>
              </div>
            </div>
          </div>
        ))}
      </div>

      {phase === "params" && (
        <>
          {planError && (
            <div className="mb-3 rounded-md border border-destructive/30 bg-destructive/5 px-3 py-2 text-sm text-destructive">
              {planError}
            </div>
          )}
          {planningProgress && (
            <div className="mb-3">
              <InlineProgressBar
                title={t("cook.planningProgressTitle")}
                detail={t(planningProgress.labelKey)}
                value={planningProgress.value}
                animated={planningProgress.value < 100}
              />
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

      {phase === "confirm" && (
        <Card>
          <CardHeader className="space-y-1">
            <CardTitle>{t("cook.reviewTitle")}</CardTitle>
            <CardDescription>{t("cook.reviewDescription")}</CardDescription>
          </CardHeader>
          <CardContent className="space-y-6">
            {plan && (
              <RecipePlanPreview
                plan={plan}
                routeSummary={routeSummary}
                authIssues={authIssues}
                contextWarnings={contextWarnings}
                workspaceEntry={workspaceEntry}
              />
            )}
            <div>
              <div className="text-xs font-medium uppercase tracking-[0.16em] text-muted-foreground">
                {t("cook.plannedChangesTitle")}
              </div>
              <div className="mt-3 space-y-3">
                {resolvedStepList.map((step, i) => (
                  <div key={i} className={cn("flex items-start gap-3", stepStatuses[i] === "skipped" && "opacity-50")}>
                    <span className={cn("text-lg font-mono w-5 text-center", statusColor(stepStatuses[i]))}>
                      {statusIcon(stepStatuses[i])}
                    </span>
                    <div className="flex-1">
                      <div className="text-sm font-medium">
                        {step.label}
                        {stepStatuses[i] === "skipped" && (
                          <span className="ml-2 text-xs text-muted-foreground">{t('cook.skippedEmpty')}</span>
                        )}
                      </div>
                      {step.description !== step.label && stepStatuses[i] !== "skipped" && (
                        <div className="text-xs text-muted-foreground">{step.description}</div>
                      )}
                    </div>
                  </div>
                ))}
              </div>
            </div>
            {blockingAuthIssues && (
              <div className="rounded-md border border-destructive/30 bg-destructive/5 px-3 py-2 text-sm text-destructive">
                {t("cook.authBlocker")}
              </div>
            )}
            {approvalRequired && (
              <div className="rounded-md border border-amber-500/30 bg-amber-500/10 px-3 py-3 text-sm text-amber-950">
                <div className="font-medium">{t("cook.approvalTitle")}</div>
                <div className="mt-1">{t("cook.approvalDescription")}</div>
                <div className="mt-3">
                  <Button
                    variant="outline"
                    onClick={() => void handleApprove()}
                    disabled={approvalSubmitting}
                  >
                    {approvalSubmitting ? t("cook.approving") : t("cook.approveToContinue")}
                  </Button>
                </div>
              </div>
            )}
            {executionError && (
              <div className="rounded-md border border-destructive/30 bg-destructive/5 px-3 py-2 text-sm text-destructive">
                {executionError}
              </div>
            )}
            <div className="flex flex-wrap gap-2">
              <Button disabled={blockingAuthIssues || approvalRequired} onClick={() => void handleExecute()}>{t('cook.execute')}</Button>
              <Button variant="outline" onClick={() => setPhase("params")}>{t('cook.backToConfiguration')}</Button>
            </div>
          </CardContent>
        </Card>
      )}

      {phase === "execute" && executionProgress && (
        <div className="space-y-4">
          <Card>
            <CardHeader className="space-y-1">
              <CardTitle>{t("cook.executionActiveTitle")}</CardTitle>
              <CardDescription>{t("cook.executionActiveDescription")}</CardDescription>
            </CardHeader>
            <CardContent className="space-y-4">
              <InlineProgressBar
                title={t("cook.executionProgressTitle")}
                detail={t(executionProgress.detailKey, executionProgress.detailArgs)}
                value={executionProgress.value}
                tone={executionProgress.failed ? "destructive" : "primary"}
                animated={executionProgress.animated}
              />
              <div className="rounded-md border border-border/70 bg-muted/20 px-3 py-2 text-sm text-muted-foreground">
                {t("cook.executionApplyNote")}
              </div>
              {executionError && (
                <div className="rounded-md border border-destructive/30 bg-destructive/5 px-3 py-2 text-sm text-destructive">
                  <div className="font-medium">{t("cook.executionFailed")}</div>
                  <div className="mt-1">{executionError}</div>
                </div>
              )}
              {executionError && (
                <div className="flex flex-wrap gap-2">
                  <Button variant="outline" onClick={() => setPhase("confirm")}>{t("cook.backToReview")}</Button>
                  <Button variant="outline" onClick={() => setPhase("params")}>{t("cook.backToConfiguration")}</Button>
                </div>
              )}
            </CardContent>
          </Card>

          <Card>
            <CardHeader className="space-y-1">
              <CardTitle>{t("cook.plannedChangesTitle")}</CardTitle>
              <CardDescription>{t("cook.plannedChangesDescription")}</CardDescription>
            </CardHeader>
            <CardContent className="space-y-3">
              {resolvedStepList.map((step, i) => (
                <div key={i} className={cn("flex items-start gap-3", stepStatuses[i] === "skipped" && "opacity-50")}>
                  <span className={cn("text-lg font-mono w-5 text-center", statusColor(stepStatuses[i]))}>
                    {statusIcon(stepStatuses[i])}
                  </span>
                  <div className="flex-1">
                    <div className="text-sm font-medium">
                      {step.label}
                      {stepStatuses[i] === "skipped" && (
                        <span className="ml-2 text-xs text-muted-foreground">{t('cook.skippedEmpty')}</span>
                      )}
                    </div>
                    {step.description !== step.label && stepStatuses[i] !== "skipped" && (
                      <div className="text-xs text-muted-foreground">{step.description}</div>
                    )}
                  </div>
                </div>
              ))}
            </CardContent>
          </Card>
        </div>
      )}

      {phase === "done" && (
        <Card>
          <CardHeader className="space-y-1">
            <CardTitle>{t("cook.doneTitle")}</CardTitle>
            <CardDescription>{t("cook.doneDescription")}</CardDescription>
          </CardHeader>
          <CardContent className="space-y-5">
            <div className="rounded-xl border border-green-500/20 bg-green-500/5 px-4 py-4">
              <div className="text-sm font-medium text-green-700">
                {t("cook.doneResultTitle")}
              </div>
              {executionResult && (
                <>
                  <p className="mt-2 text-sm text-foreground">
                    {executionResult.summary}
                  </p>
                  <p className="mt-2 text-xs text-muted-foreground">
                    {t("cook.doneEnvironment", { name: routeSummary.targetLabel })}
                    {" · "}
                    {t("cook.stepsCompleted", { done: doneCount })}
                    {skippedCount > 0 ? t("cook.stepsSkipped", { skipped: skippedCount }) : ""}
                  </p>
                </>
              )}
            </div>
            {executionResult && (
              <div>
                <div className="text-xs font-medium uppercase tracking-[0.16em] text-muted-foreground">
                  {t("cook.doneWhatChangedTitle")}
                </div>
                <ul className="mt-3 space-y-2 text-sm text-foreground">
                  {plan?.concreteClaims.map((claim, index) => (
                    <li key={`${claim.kind}-${claim.id ?? claim.path ?? index}`}>
                      {formatRecipeClaimForPeople(t, claim)}
                    </li>
                  ))}
                </ul>
              </div>
            )}
            {(executionResult?.warnings.length ?? 0) > 0 && (
              <div className="rounded-md border border-amber-500/30 bg-amber-500/10 p-3 text-left text-sm text-amber-950">
                <div className="font-medium">{t("cook.doneNeedsAttentionTitle")}</div>
                {executionResult?.warnings.map((warning) => (
                  <div key={warning} className="mt-1">
                    {warning}
                  </div>
                ))}
              </div>
            )}
            {executionResult && (
              <div className="rounded-md border border-border/70 bg-background/80 px-3 py-2">
                <button
                  type="button"
                  className="flex w-full items-center justify-between gap-3 text-left text-sm font-medium text-foreground"
                  onClick={() => setDoneSupportDetailsOpen((open) => !open)}
                >
                  <span>{t("cook.doneSupportDetailsTitle")}</span>
                  <ChevronDownIcon
                    className={cn(
                      "size-4 text-muted-foreground transition-transform",
                      doneSupportDetailsOpen && "rotate-180",
                    )}
                    aria-hidden="true"
                  />
                </button>
                {doneSupportDetailsOpen && (
                  <div className="mt-3 space-y-2 text-sm text-muted-foreground">
                    <div>
                      <span className="font-medium text-foreground">{t("cook.runId")}:</span>{" "}
                      {executionResult.runId}
                    </div>
                  </div>
                )}
              </div>
            )}
            <div className="flex flex-wrap gap-2">
              <Button onClick={onDone}>{t("cook.return")}</Button>
              {onOpenHistory && (
                <Button variant="outline" onClick={onOpenHistory}>{t("cook.viewHistory")}</Button>
              )}
              {onOpenRuntimeDashboard && (
                <Button variant="outline" onClick={onOpenRuntimeDashboard}>{t("cook.viewRuntime")}</Button>
              )}
            </div>
          </CardContent>
        </Card>
      )}
    </section>
  );
}
