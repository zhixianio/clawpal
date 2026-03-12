import { useEffect, useMemo, useState } from "react";
import { useTranslation } from "react-i18next";
import { Card, CardContent } from "@/components/ui/card";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { useInstance } from "@/lib/instance-context";
import { useApi } from "@/lib/use-api";
import type { RecipeRuntimeRun } from "@/lib/types";
import {
  clearOrchestratorEvents,
  readOrchestratorEvents,
  type OrchestratorEvent,
} from "@/lib/orchestrator-log";
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

function formatClaim(claim: RecipeRuntimeRun["resourceClaims"][number]): string {
  return [claim.kind, claim.id, claim.target, claim.path]
    .filter((value): value is string => !!value && value.trim().length > 0)
    .join(" · ");
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
  const { instanceId } = useInstance();
  const [events, setEvents] = useState<OrchestratorEvent[]>(() => initialEvents ?? []);
  const [runs, setRuns] = useState<RecipeRuntimeRun[]>(() => initialRuns ?? []);
  const [scope, setScope] = useState<"current" | "all">("current");

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

  const onClear = () => {
    if (scope === "current") {
      clearOrchestratorEvents(instanceId);
    } else {
      clearOrchestratorEvents();
    }
    setEvents(readOrchestratorEvents());
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

      <div className="mb-6">
        <h3 className="text-sm font-semibold mb-2">{t("orchestrator.runtimeTitle")}</h3>
        <p className="text-sm text-muted-foreground mb-3">{t("orchestrator.runtimeDescription")}</p>
        {visibleRuns.length === 0 ? (
          <p className="text-muted-foreground">{t("orchestrator.runtimeEmpty")}</p>
        ) : (
          <div className="space-y-3">
            {visibleRuns.map((run) => (
              <Card key={run.id}>
                <CardContent className="space-y-3">
                  <div className="flex items-center justify-between gap-2">
                    <div>
                      <div className="font-medium">{run.summary}</div>
                      <div className="text-xs text-muted-foreground">
                        {formatTime(run.startedAt)} · {run.instanceId} · {run.runner} · {run.executionKind}
                      </div>
                    </div>
                    <Badge className={runStatusClass(run.status)}>{run.status}</Badge>
                  </div>

                  {run.artifacts.length > 0 && (
                    <div className="space-y-1">
                      <div className="text-xs font-medium text-muted-foreground">
                        {t("orchestrator.artifacts")}
                      </div>
                      <div className="flex flex-wrap gap-2">
                        {run.artifacts.map((artifact) => (
                          <Badge key={artifact.id} variant="outline">
                            {artifact.label}
                          </Badge>
                        ))}
                      </div>
                    </div>
                  )}

                  {run.resourceClaims.length > 0 && (
                    <div className="space-y-1">
                      <div className="text-xs font-medium text-muted-foreground">
                        {t("orchestrator.resourceClaims")}
                      </div>
                      <div className="flex flex-wrap gap-2">
                        {run.resourceClaims.map((claim, index) => (
                          <Badge
                            key={`${run.id}-${claim.kind}-${claim.id ?? claim.path ?? index}`}
                            variant="outline"
                          >
                            {formatClaim(claim)}
                          </Badge>
                        ))}
                      </div>
                    </div>
                  )}

                  {run.warnings.length > 0 && (
                    <div className="rounded border bg-muted/30 p-2 text-xs whitespace-pre-wrap break-all">
                      {run.warnings.join("\n")}
                    </div>
                  )}

                  {formatSourceTrace(run) && (
                    <div className="rounded border bg-muted/20 p-2 text-xs whitespace-pre-wrap break-all">
                      {t("orchestrator.sourceTrace")}: {formatSourceTrace(run)}
                    </div>
                  )}
                </CardContent>
              </Card>
            ))}
          </div>
        )}
      </div>

      <div>
        <h3 className="text-sm font-semibold mb-2">{t("orchestrator.eventLogTitle")}</h3>
      {visible.length === 0 ? (
        <p className="text-muted-foreground">{t("orchestrator.empty")}</p>
      ) : (
        <div className="space-y-3">
          {visible.map((event) => (
            <Card key={event.id}>
              <CardContent className="space-y-1">
                <div className="flex items-center justify-between gap-2">
                  <div className="font-medium">{event.message}</div>
                  <Badge className={levelClass(event.level)}>{event.level}</Badge>
                </div>
                <div className="text-xs text-muted-foreground">
                  {event.at} · {event.instanceId}
                  {event.sessionId ? ` · ${event.sessionId}` : ""}
                  {event.step ? ` · step=${event.step}` : ""}
                  {event.state ? ` · state=${event.state}` : ""}
                  {event.source ? ` · source=${event.source}` : ""}
                </div>
                {event.details && (
                  <div className="text-xs rounded border bg-muted/30 p-2 whitespace-pre-wrap break-all">
                    {event.details}
                  </div>
                )}
              </CardContent>
            </Card>
          ))}
        </div>
      )}
      </div>
    </div>
  );
}
