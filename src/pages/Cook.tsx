import { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { ParamForm } from "../components/ParamForm";
import { resolveSteps, stepToCommands, type ResolvedStep } from "../lib/actions";
import { useApi } from "@/lib/use-api";
import { Card, CardContent } from "@/components/ui/card";
import { Button } from "@/components/ui/button";
import { cn } from "@/lib/utils";
import type { Recipe } from "../lib/types";
import { useInstance } from "@/lib/instance-context";


type Phase = "params" | "confirm" | "execute" | "done";
type StepStatus = "pending" | "running" | "done" | "failed" | "skipped";

export function Cook({
  recipeId,
  onDone,
  recipeSource,
}: {
  recipeId: string;
  onDone?: () => void;
  recipeSource?: string;
}) {
  const { t } = useTranslation();
  const ua = useApi();
  const { instanceId, isRemote } = useInstance();
  const [recipe, setRecipe] = useState<Recipe | null>(null);
  const [loading, setLoading] = useState(true);
  const [params, setParams] = useState<Record<string, string>>({});
  const [phase, setPhase] = useState<Phase>("params");
  const [resolvedStepList, setResolvedStepList] = useState<ResolvedStep[]>([]);
  const [stepStatuses, setStepStatuses] = useState<StepStatus[]>([]);
  const [stepErrors, setStepErrors] = useState<Record<number, string>>({});
  const [needsRestart, setNeedsRestart] = useState(false);

  useEffect(() => {
    setLoading(true);
    ua.listRecipes(recipeSource).then((recipes) => {
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
  }, [recipeId, recipeSource]);

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
          if (typeof cur === "string" && cur.length > 0) {
            setParams((prev) => ({ ...prev, [paramId]: prev[paramId] || cur as string }));
          }
        }
      } catch { /* ignore */ }
    }).catch(() => { /* ignore config read errors */ });
  }, [recipe, params.guild_id, params.channel_id, isRemote, instanceId]);

  if (loading) return <div className="flex items-center justify-center py-12"><div className="h-6 w-6 animate-spin rounded-full border-2 border-primary border-t-transparent" /></div>;
  if (!recipe) return <div>{t('cook.recipeNotFound')}</div>;

  const handleNext = () => {
    const steps = resolveSteps(recipe.steps, params);
    setResolvedStepList(steps);
    // Auto-skip steps whose template args resolved to empty
    setStepStatuses(steps.map((s) => (s.skippable ? "skipped" : "pending")));
    setStepErrors({});
    setNeedsRestart(steps.some((s) => !s.skippable));
    setPhase("confirm");
  };

  const runFrom = async (startIndex: number, statuses: StepStatus[]) => {
    for (let i = startIndex; i < resolvedStepList.length; i++) {
      if (statuses[i] === "skipped") continue;
      statuses[i] = "running";
      setStepStatuses([...statuses]);
      try {
        const commands = await stepToCommands(resolvedStepList[i], { instanceId, isRemote });
        for (const [label, cmd] of commands) {
          await ua.queueCommand(label, cmd);
        }
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
  };

  const handleExecute = () => {
    setPhase("execute");
    const statuses = [...stepStatuses];
    runFrom(0, statuses);
  };

  const handleRetry = (index: number) => {
    const statuses = [...stepStatuses];
    setStepErrors((prev) => {
      const next = { ...prev };
      delete next[index];
      return next;
    });
    runFrom(index, statuses);
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
    const nextIndex = statuses.findIndex((s, i) => i > index && s !== "skipped");
    if (nextIndex === -1) {
      setPhase("done");
    } else {
      runFrom(nextIndex, statuses);
    }
  };

  const statusIcon = (s: StepStatus) => {
    switch (s) {
      case "pending": return "\u25CB";
      case "running": return "\u25C9";
      case "done": return "\u2713";
      case "failed": return "\u2717";
      case "skipped": return "\u2013";
    }
  };

  const statusColor = (s: StepStatus) => {
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
        <ParamForm
          recipe={recipe}
          values={params}
          onChange={(id, value) => setParams((prev) => ({ ...prev, [id]: value }))}
          onSubmit={handleNext}
          submitLabel={t('cook.next')}
        />
      )}

      {(phase === "confirm" || phase === "execute") && (
        <Card>
          <CardContent>
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
                    {stepErrors[i] && (
                      <div className="text-xs text-destructive mt-1">{stepErrors[i]}</div>
                    )}
                    {stepStatuses[i] === "failed" && (
                      <div className="flex gap-2 mt-1.5">
                        <Button size="sm" variant="outline" onClick={() => handleRetry(i)}>
                          {t('cook.retry')}
                        </Button>
                        <Button size="sm" variant="ghost" onClick={() => handleSkip(i)}>
                          {t('cook.skip')}
                        </Button>
                      </div>
                    )}
                  </div>
                </div>
              ))}
            </div>
            {phase === "confirm" && (
              <div className="flex gap-2 mt-4">
                <Button onClick={handleExecute}>{t('cook.execute')}</Button>
                <Button variant="outline" onClick={() => setPhase("params")}>{t('cook.back')}</Button>
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
            {needsRestart && (
              <p className="text-sm text-muted-foreground mt-1">
                {t('cook.applyHint')}
              </p>
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
