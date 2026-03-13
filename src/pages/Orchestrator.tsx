import { useEffect, useMemo, useState } from "react";
import { ChevronDownIcon } from "lucide-react";
import { useTranslation } from "react-i18next";
import { Card, CardContent } from "@/components/ui/card";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import {
  AlertDialog,
  AlertDialogAction,
  AlertDialogCancel,
  AlertDialogContent,
  AlertDialogDescription,
  AlertDialogFooter,
  AlertDialogHeader,
  AlertDialogTitle,
  AlertDialogTrigger,
} from "@/components/ui/alert-dialog";
import { useInstance } from "@/lib/instance-context";
import { useApi } from "@/lib/use-api";
import type { RecipeRuntimeRun } from "@/lib/types";
import {
  clearOrchestratorEvents,
  readOrchestratorEvents,
  type OrchestratorEvent,
} from "@/lib/orchestrator-log";
import {
  formatRecipeClaimForPeople,
  formatRecipeRunStatusLabel,
  resolveRecipeEnvironmentLabel,
} from "@/lib/recipe-run-copy";
import { formatTime } from "@/lib/utils";

function levelClass(level: OrchestratorEvent["level"]): string {
  if (level === "success") return "bg-emerald-500/10 text-emerald-600";
  if (level === "error") return "bg-red-500/10 text-red-600";
  return "bg-muted text-muted-foreground";
}

function runStatusClass(status: string): string {
  if (status === "succeeded") return "bg-emerald-500/10 text-emerald-600";
  if (status === "failed") return "bg-red-500/10 text-red-600";
  return "bg-muted text-muted-foreground";
}

function formatSourceTrace(run: RecipeRuntimeRun): string | null {
  const parts = [run.sourceOrigin, run.sourceDigest, run.workspacePath]
    .filter((value): value is string => !!value && value.trim().length > 0);
  return parts.length > 0 ? parts.join(" · ") : null;
}

export function Orchestrator({
  initialRuns,
  initialEvents,
}: {
  initialRuns?: RecipeRuntimeRun[];
  initialEvents?: OrchestratorEvent[];
}) {
  const { t } = useTranslation();
  const ua = useApi();
  const { instanceId, instanceLabel } = useInstance();
  const [events, setEvents] = useState<OrchestratorEvent[]>(() => initialEvents ?? []);
  const [runs, setRuns] = useState<RecipeRuntimeRun[]>(() => initialRuns ?? []);
  const [scope, setScope] = useState<"current" | "all">("current");
  const [clearingRuns, setClearingRuns] = useState(false);
  const [instanceLabels, setInstanceLabels] = useState<Record<string, string>>(() =>
    instanceLabel ? { [instanceId]: instanceLabel } : {},
  );
  const [showEventLog, setShowEventLog] = useState(false);
  const [openRunSupportDetails, setOpenRunSupportDetails] = useState<Record<string, boolean>>({});

  useEffect(() => {
    ua.listRegisteredInstances()
      .then((items) => {
        setInstanceLabels((prev) => {
          const next = { ...prev };
          for (const item of items) {
            next[item.id] = item.label;
          }
          if (instanceLabel) {
            next[instanceId] = instanceLabel;
          }
          return next;
        });
      })
      .catch(() => {});
  }, [instanceId, instanceLabel, ua]);

  useEffect(() => {
    if (initialRuns || initialEvents) {
      return;
    }

    const load = () => {
      setEvents(readOrchestratorEvents());
      ua.listRecipeRuns().then(setRuns).catch(() => {});
    };
    load();
    const timer = setInterval(load, 1000);
    return () => clearInterval(timer);
  }, [initialEvents, initialRuns, ua]);

  const visible = useMemo(() => {
    const list = scope === "current"
      ? events.filter((e) => e.instanceId === instanceId)
      : events;
    return [...list].sort((a, b) => b.at.localeCompare(a.at));
  }, [events, instanceId, scope]);

  const visibleRuns = useMemo(() => {
    const list = scope === "current"
      ? runs.filter((run) => run.instanceId === instanceId)
      : runs;
    return [...list].sort((a, b) => b.startedAt.localeCompare(a.startedAt));
  }, [instanceId, runs, scope]);

  const scopeDescription =
    scope === "current"
      ? t("orchestrator.scopeCurrentDescription", {
        name: resolveRecipeEnvironmentLabel(instanceId, {
          currentInstanceId: instanceId,
          currentInstanceLabel: instanceLabel,
          labelsById: instanceLabels,
        }),
      })
      : t("orchestrator.scopeAllDescription");

  const onClear = () => {
    if (scope === "current") {
      clearOrchestratorEvents(instanceId);
    } else {
      clearOrchestratorEvents();
    }
    setEvents(readOrchestratorEvents());
  };

  const onClearRuns = async () => {
    setClearingRuns(true);
    try {
      await ua.deleteRecipeRuns(scope === "current" ? instanceId : undefined);
      const nextRuns = await ua.listRecipeRuns();
      setRuns(nextRuns);
    } catch (error) {
      console.error("Failed to clear recipe runs:", error);
    } finally {
      setClearingRuns(false);
    }
  };

  return (
    <div>
      <h2 className="text-2xl font-bold mb-4">{t("orchestrator.title")}</h2>
      <p className="text-sm text-muted-foreground mb-4">{t("orchestrator.description")}</p>

      <div className="flex items-center gap-2 mb-4">
        <Button
          size="sm"
          variant={scope === "current" ? "default" : "outline"}
          onClick={() => setScope("current")}
        >
          {t("orchestrator.scope.current")}
        </Button>
        <Button
          size="sm"
          variant={scope === "all" ? "default" : "outline"}
          onClick={() => setScope("all")}
        >
          {t("orchestrator.scope.all")}
        </Button>
        <Button size="sm" variant="outline" onClick={onClear}>
          {t("orchestrator.clear")}
        </Button>
      </div>
      <p className="mb-5 text-sm text-muted-foreground">{scopeDescription}</p>

      <div className="mb-6">
        <div className="mb-2 flex items-center justify-between gap-2">
          <h3 className="text-sm font-semibold">{t("orchestrator.runtimeTitle")}</h3>
          <AlertDialog>
            <AlertDialogTrigger asChild>
              <Button size="sm" variant="outline" disabled={clearingRuns || visibleRuns.length === 0}>
                {t("orchestrator.clearRuns")}
              </Button>
            </AlertDialogTrigger>
            <AlertDialogContent>
              <AlertDialogHeader>
                <AlertDialogTitle>{t("orchestrator.clearRunsTitle")}</AlertDialogTitle>
                <AlertDialogDescription>
                  {t(
                    scope === "current"
                      ? "orchestrator.clearRunsCurrentDescription"
                      : "orchestrator.clearRunsAllDescription",
                  )}
                </AlertDialogDescription>
              </AlertDialogHeader>
              <AlertDialogFooter>
                <AlertDialogCancel>{t("settings.cancel")}</AlertDialogCancel>
                <AlertDialogAction
                  className="bg-destructive text-white hover:bg-destructive/90"
                  onClick={() => {
                    void onClearRuns();
                  }}
                >
                  {t("orchestrator.clearRuns")}
                </AlertDialogAction>
              </AlertDialogFooter>
            </AlertDialogContent>
          </AlertDialog>
        </div>
        <p className="text-sm text-muted-foreground mb-3">{t("orchestrator.runtimeDescription")}</p>
        {visibleRuns.length === 0 ? (
          <p className="text-muted-foreground">{t("orchestrator.runtimeEmpty")}</p>
        ) : (
          <div className="space-y-3">
            {visibleRuns.map((run) => {
              const environmentLabel = resolveRecipeEnvironmentLabel(run.instanceId, {
                currentInstanceId: instanceId,
                currentInstanceLabel: instanceLabel,
                labelsById: instanceLabels,
              });
              const supportOpen = !!openRunSupportDetails[run.id];
              return (
              <Card key={run.id}>
                <CardContent className="space-y-3">
                  <div className="flex items-center justify-between gap-2">
                    <div>
                      <div className="font-medium">{run.summary}</div>
                      <div className="text-xs text-muted-foreground">
                        {formatTime(run.startedAt)} · {t("orchestrator.environmentSummary", { name: environmentLabel })}
                      </div>
                    </div>
                    <Badge className={runStatusClass(run.status)}>
                      {formatRecipeRunStatusLabel(t, run.status)}
                    </Badge>
                  </div>

                  {run.resourceClaims.length > 0 && (
                    <div className="space-y-1">
                      <div className="text-xs font-medium text-muted-foreground">
                        {t("orchestrator.whatChangedTitle")}
                      </div>
                      <ul className="space-y-2 text-sm text-foreground">
                        {run.resourceClaims.map((claim, index) => (
                          <li
                            key={`${run.id}-${claim.kind}-${claim.id ?? claim.path ?? index}`}
                            className="flex gap-2"
                          >
                            <span className="mt-0.5 text-muted-foreground">•</span>
                            <span>{formatRecipeClaimForPeople(t, claim)}</span>
                          </li>
                        ))}
                      </ul>
                    </div>
                  )}

                  {run.warnings.length > 0 && (
                    <div className="rounded border border-amber-500/30 bg-amber-500/10 p-3 text-sm text-amber-950">
                      <div className="font-medium">{t("orchestrator.needsAttentionTitle")}</div>
                      {run.warnings.map((warning) => (
                        <div key={warning} className="mt-1">
                          {warning}
                        </div>
                      ))}
                    </div>
                  )}

                  {(formatSourceTrace(run) || run.artifacts.length > 0) && (
                    <div className="rounded-md border border-border/70 bg-background/80 px-3 py-2">
                      <button
                        type="button"
                        className="flex w-full items-center justify-between gap-3 text-left text-sm font-medium text-foreground"
                        onClick={() =>
                          setOpenRunSupportDetails((prev) => ({
                            ...prev,
                            [run.id]: !prev[run.id],
                          }))
                        }
                      >
                        <span>{t("orchestrator.supportDetailsTitle")}</span>
                        <ChevronDownIcon
                          className={`size-4 text-muted-foreground transition-transform ${
                            supportOpen ? "rotate-180" : ""
                          }`}
                          aria-hidden="true"
                        />
                      </button>
                      {supportOpen && (
                        <div className="mt-3 space-y-3 text-sm text-muted-foreground">
                          <div>
                            <span className="font-medium text-foreground">{t("orchestrator.supportRunId")}:</span>{" "}
                            {run.id}
                          </div>
                          {run.artifacts.length > 0 && (
                            <div>
                              <div className="font-medium text-foreground">{t("orchestrator.supportArtifacts")}:</div>
                              <div className="mt-1 flex flex-wrap gap-2">
                                {run.artifacts.map((artifact) => (
                                  <Badge key={artifact.id} variant="outline">
                                    {artifact.label}
                                  </Badge>
                                ))}
                              </div>
                            </div>
                          )}
                          {formatSourceTrace(run) && (
                            <div className="rounded border bg-muted/20 p-2 text-xs whitespace-pre-wrap break-all">
                              {t("orchestrator.sourceTrace")}: {formatSourceTrace(run)}
                            </div>
                          )}
                        </div>
                      )}
                    </div>
                  )}
                </CardContent>
              </Card>
            );
            })}
          </div>
        )}
      </div>

      <div>
        <button
          type="button"
          className="mb-2 flex w-full items-center justify-between gap-3 rounded-md border border-border/70 bg-background/80 px-3 py-2 text-left"
          onClick={() => setShowEventLog((open) => !open)}
        >
          <div>
            <div className="text-sm font-semibold">{t("orchestrator.eventLogTitle")}</div>
            <div className="text-xs text-muted-foreground">{t("orchestrator.eventLogDescription")}</div>
          </div>
          <ChevronDownIcon
            className={`size-4 text-muted-foreground transition-transform ${
              showEventLog ? "rotate-180" : ""
            }`}
            aria-hidden="true"
          />
        </button>
      {showEventLog && (visible.length === 0 ? (
        <p className="text-muted-foreground">{t("orchestrator.empty")}</p>
      ) : (
        <div className="space-y-3">
          {visible.map((event) => {
            const environmentLabel = resolveRecipeEnvironmentLabel(event.instanceId, {
              currentInstanceId: instanceId,
              currentInstanceLabel: instanceLabel,
              labelsById: instanceLabels,
            });
            return (
            <Card key={event.id}>
              <CardContent className="space-y-1">
                <div className="flex items-center justify-between gap-2">
                  <div className="font-medium">{event.message}</div>
                  <Badge className={levelClass(event.level)}>{event.level}</Badge>
                </div>
                <div className="text-xs text-muted-foreground">
                  {event.at} · {t("orchestrator.environmentSummary", { name: environmentLabel })}
                </div>
                {event.details && (
                  <div className="text-xs rounded border bg-muted/30 p-2 whitespace-pre-wrap break-all">
                    {event.details}
                  </div>
                )}
              </CardContent>
            </Card>
            );
          })}
        </div>
      ))}
      </div>
    </div>
  );
}
