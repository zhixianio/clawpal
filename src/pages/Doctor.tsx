import { listen } from "@tauri-apps/api/event";
import { useCallback, useEffect, useMemo, useState } from "react";
import { useTranslation } from "react-i18next";
import {
  FileTextIcon,
  LoaderCircleIcon,
  StethoscopeIcon,
  WrenchIcon,
} from "lucide-react";

import { DoctorLogsDialog } from "@/components/DoctorLogsDialog";
import { DoctorRecoveryOverview } from "@/components/DoctorRecoveryOverview";
import { DoctorTempProviderDialog } from "@/components/DoctorTempProviderDialog";
import { RescueAsciiHeader } from "@/components/RescueAsciiHeader";
import { Button } from "@/components/ui/button";
import { Card, CardContent, CardHeader } from "@/components/ui/card";
import { useInstance } from "@/lib/instance-context";
import { localizeDoctorReportText } from "@/lib/doctor-report-i18n";
import {
  createDataLoadRequestId,
  emitDataLoadMetric,
} from "@/lib/data-load-log";
import type {
  ModelProfile,
  RescueBotRuntimeState,
  RescuePrimaryDiagnosisResult,
  RescuePrimaryRepairResult,
} from "@/lib/types";
import { useApi } from "@/lib/use-api";

interface DoctorProgressPayload {
  runId?: string;
  phase?: string;
  line?: string;
  progress?: number;
  attempt?: number;
  resolvedIssueId?: string | null;
  resolvedIssueLabel?: string | null;
}

interface DoctorProps {
}

function diagnosisNeedsRepair(result: RescuePrimaryDiagnosisResult | null): boolean {
  if (!result) return false;
  if ((result.summary.selectedFixIssueIds?.length ?? 0) > 0) return true;
  return result.sections.some(
    (section) =>
      section.key === "gateway"
      && (section.status === "broken" || section.status === "degraded"),
  );
}

function resolveBotState(
  busy: boolean,
  diagnosis: RescuePrimaryDiagnosisResult | null,
  error: string | null,
): RescueBotRuntimeState {
  if (busy) return "checking";
  if (error) return "error";
  if (!diagnosis) return "configured_inactive";
  return diagnosisNeedsRepair(diagnosis) ? "error" : "active";
}

export function Doctor(_: DoctorProps) {
  const { t, i18n } = useTranslation();
  const ua = useApi();
  const { isRemote, isConnected } = useInstance();

  const [logsOpen, setLogsOpen] = useState(false);
  const [logsSource, setLogsSource] = useState<"clawpal" | "gateway" | "helper">("gateway");
  const [diagnosisLoading, setDiagnosisLoading] = useState(false);
  const [repairing, setRepairing] = useState(false);
  const [diagnosis, setDiagnosis] = useState<RescuePrimaryDiagnosisResult | null>(null);
  const [repairResult, setRepairResult] = useState<RescuePrimaryRepairResult | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [statusLine, setStatusLine] = useState<string | null>(null);
  const [statusProgress, setStatusProgress] = useState(0.16);
  const [tempProviderDialogOpen, setTempProviderDialogOpen] = useState(false);
  const [tempProviderProfileId, setTempProviderProfileId] = useState<string | null>(null);

  const busy = diagnosisLoading || repairing;
  const liveReadsReady = ua.instanceToken !== 0;
  const needsRepair = diagnosisNeedsRepair(diagnosis);
  const pendingTempProviderSetup =
    repairResult?.status === "needsTempProviderSetup" ? repairResult.pendingAction ?? null : null;
  const repairableCount = useMemo(() => {
    const summaryCount = diagnosis?.summary.fixableIssueCount ?? 0;
    const selectedCount = diagnosis?.summary.selectedFixIssueIds.length ?? 0;
    return Math.max(summaryCount, selectedCount, needsRepair ? 1 : 0);
  }, [diagnosis, needsRepair]);
  const botState = resolveBotState(busy, diagnosis, error);
  const localizedStatusLine = useMemo(
    () => (statusLine ? localizeDoctorReportText(statusLine, i18n.language) : null),
    [i18n.language, statusLine],
  );
  const localizedRecommendedAction = useMemo(
    () => (
      diagnosis
        ? localizeDoctorReportText(diagnosis.summary.recommendedAction, i18n.language)
        : null
    ),
    [diagnosis, i18n.language],
  );
  const localizedPendingReason = useMemo(
    () => (
      pendingTempProviderSetup?.reason
        ? localizeDoctorReportText(pendingTempProviderSetup.reason, i18n.language)
        : null
    ),
    [i18n.language, pendingTempProviderSetup],
  );

  useEffect(() => {
    let disposed = false;
    let unlisten: (() => void) | null = null;
    void listen<DoctorProgressPayload>("doctor:assistant-progress", (event) => {
      if (disposed) return;
      const payload = event.payload;
      setStatusLine(payload.line?.trim() || null);
      if (typeof payload.progress === "number" && !Number.isNaN(payload.progress)) {
        setStatusProgress(Math.max(0, Math.min(1, payload.progress)));
      }
    }).then((fn) => {
      if (disposed) {
        fn();
        return;
      }
      unlisten = fn;
    });
    return () => {
      disposed = true;
      if (unlisten) unlisten();
    };
  }, []);

  useEffect(() => {
    if (!busy && diagnosis && !needsRepair && !error && !pendingTempProviderSetup) {
      setStatusLine(null);
    }
  }, [busy, diagnosis, error, needsRepair, pendingTempProviderSetup]);

  const openLogs = useCallback((source: "clawpal" | "gateway" | "helper" = "gateway") => {
    setLogsSource(source);
    setLogsOpen(true);
  }, []);

  const runDiagnosis = useCallback(async () => {
    if (!liveReadsReady || busy) return;
    if (isRemote && !isConnected) {
      setError(t("doctor.rescueBotConnectRequired", { defaultValue: "Connect to SSH first." }));
      return;
    }
    setDiagnosisLoading(true);
    setRepairResult(null);
    setError(null);
    setStatusLine(t("doctor.analyzing", { defaultValue: "Diagnosing..." }));
    setStatusProgress(0.08);
    const requestId = createDataLoadRequestId("diagnoseDoctorAssistant");
    const startedAt = Date.now();
    emitDataLoadMetric({
      requestId,
      resource: "diagnoseDoctorAssistant",
      page: "doctor",
      instanceId: ua.instanceId,
      instanceToken: ua.instanceToken,
      source: "cli",
      phase: "start",
      elapsedMs: 0,
      cacheHit: false,
    });
    try {
      const result = await ua.diagnoseDoctorAssistant();
      setDiagnosis(result);
      setStatusProgress(1);
      setStatusLine(
        diagnosisNeedsRepair(result)
          ? result.summary.recommendedAction
          : null,
      );
      emitDataLoadMetric({
        requestId,
        resource: "diagnoseDoctorAssistant",
        page: "doctor",
        instanceId: ua.instanceId,
        instanceToken: ua.instanceToken,
        source: "cli",
        phase: "success",
        elapsedMs: Date.now() - startedAt,
        cacheHit: false,
      });
    } catch (cause) {
      const text = cause instanceof Error ? cause.message : String(cause);
      setError(text);
      setStatusProgress(0.84);
      setStatusLine(text);
      emitDataLoadMetric({
        requestId,
        resource: "diagnoseDoctorAssistant",
        page: "doctor",
        instanceId: ua.instanceId,
        instanceToken: ua.instanceToken,
        source: "cli",
        phase: "error",
        elapsedMs: Date.now() - startedAt,
        cacheHit: false,
        errorSummary: text,
      });
    } finally {
      setDiagnosisLoading(false);
    }
  }, [busy, isConnected, isRemote, liveReadsReady, t, ua]);

  const runRepair = useCallback(async (overrideTempProviderProfileId?: string) => {
    if (!liveReadsReady || busy || !diagnosis) return;
    if (isRemote && !isConnected) {
      setError(t("doctor.rescueBotConnectRequired", { defaultValue: "Connect to SSH first." }));
      return;
    }
    setRepairing(true);
    setError(null);
    setStatusLine(
      t("doctor.fixSafeIssues", {
        count: repairableCount,
        defaultValue: repairableCount === 1 ? "Fixing 1 issue" : `Fixing ${repairableCount} issues`,
      }),
    );
    setStatusProgress(0.18);
    const requestId = createDataLoadRequestId("repairDoctorAssistant");
    const startedAt = Date.now();
    emitDataLoadMetric({
      requestId,
      resource: "repairDoctorAssistant",
      page: "doctor",
      instanceId: ua.instanceId,
      instanceToken: ua.instanceToken,
      source: "cli",
      phase: "start",
      elapsedMs: 0,
      cacheHit: false,
    });
    try {
      const result = await ua.repairDoctorAssistant(
        overrideTempProviderProfileId ?? tempProviderProfileId ?? undefined,
        diagnosis,
      );
      setRepairResult(result);
      setDiagnosis(result.after);
      if (result.pendingAction?.tempProviderProfileId) {
        setTempProviderProfileId(result.pendingAction.tempProviderProfileId);
      } else if (result.status === "completed" && !diagnosisNeedsRepair(result.after)) {
        setTempProviderProfileId(null);
      }
      setStatusProgress(1);
      setStatusLine(
        result.status === "needsTempProviderSetup" && result.pendingAction
          ? result.pendingAction.reason
          : diagnosisNeedsRepair(result.after)
            ? result.after.summary.recommendedAction
          : t("doctor.primaryRepairSuccess", {
              count: result.appliedIssueIds.length,
              defaultValue:
                result.appliedIssueIds.length === 1
                  ? "Applied 1 fix."
                  : `Applied ${result.appliedIssueIds.length} fixes.`,
            }),
      );
      emitDataLoadMetric({
        requestId,
        resource: "repairDoctorAssistant",
        page: "doctor",
        instanceId: ua.instanceId,
        instanceToken: ua.instanceToken,
        source: "cli",
        phase: "success",
        elapsedMs: Date.now() - startedAt,
        cacheHit: false,
      });
    } catch (cause) {
      const text = cause instanceof Error ? cause.message : String(cause);
      setError(text);
      setStatusProgress(0.9);
      setStatusLine(text);
      emitDataLoadMetric({
        requestId,
        resource: "repairDoctorAssistant",
        page: "doctor",
        instanceId: ua.instanceId,
        instanceToken: ua.instanceToken,
        source: "cli",
        phase: "error",
        elapsedMs: Date.now() - startedAt,
        cacheHit: false,
        errorSummary: text,
      });
    } finally {
      setRepairing(false);
    }
  }, [busy, diagnosis, isConnected, isRemote, liveReadsReady, repairableCount, t, tempProviderProfileId, ua]);

  const buttonLabel = useMemo(() => {
    if (diagnosisLoading) {
      return t("doctor.analyzing", { defaultValue: "Diagnosing..." });
    }
    if (repairing) {
      return t("doctor.repairing", { defaultValue: "Repairing..." });
    }
    if (pendingTempProviderSetup) {
      return t(
        tempProviderProfileId ? "doctor.editTempProvider" : "doctor.configureTempProvider",
        {
          defaultValue: tempProviderProfileId
            ? "Edit temp gateway provider"
            : "Configure temp gateway provider",
        },
      );
    }
    if (needsRepair) {
      return t("doctor.fixSafeIssues", {
        count: repairableCount,
        defaultValue: repairableCount === 1 ? "Fix 1 issue" : `Fix ${repairableCount} issues`,
      });
    }
    return t("doctor.diagnose", { defaultValue: "Diagnose" });
  }, [diagnosisLoading, needsRepair, pendingTempProviderSetup, repairableCount, repairing, t, tempProviderProfileId]);

  const buttonIcon = diagnosisLoading || repairing
    ? <LoaderCircleIcon className="size-3.5 animate-spin" />
    : pendingTempProviderSetup
      ? <WrenchIcon className="size-3.5" />
    : needsRepair
      ? <WrenchIcon className="size-3.5" />
      : <StethoscopeIcon className="size-3.5" />;

  const actionDisabled = busy || (isRemote && !isConnected);
  const helperText = localizedStatusLine ?? (
    localizedPendingReason
    ?? (needsRepair
      ? localizedRecommendedAction
      : diagnosis
        ? null
        : t("doctor.primaryRecoveryHint", {
            defaultValue: "Run OpenClaw Doctor first, then merge the current checklist into one report.",
          }))
  );

  const handlePrimaryAction = useCallback(() => {
    if (pendingTempProviderSetup) {
      setTempProviderDialogOpen(true);
      return;
    }
    void (needsRepair ? runRepair() : runDiagnosis());
  }, [needsRepair, pendingTempProviderSetup, runDiagnosis, runRepair]);

  const handleTempProviderSaved = useCallback((profile: ModelProfile) => {
    setTempProviderProfileId(profile.id);
    setTempProviderDialogOpen(false);
    setError(null);
    setRepairResult(null);
    setStatusLine(
      t("doctor.tempProviderSaved", {
        defaultValue: "Temporary gateway provider saved. Resuming repair...",
      }),
    );
    void runRepair(profile.id);
  }, [runRepair, t]);

  return (
    <section>
      <h2 className="mb-4 text-2xl font-bold">{t("doctor.title")}</h2>
      <Card className="mb-4 gap-2 py-4">
        <CardHeader className="pb-0">
          <div className="flex flex-col items-center gap-3 text-center">
            <RescueAsciiHeader
              state={botState}
              title={buttonLabel}
              progress={busy ? statusProgress : diagnosis ? (needsRepair ? 0.86 : 1) : 0.16}
              animateProgress={busy}
              animateFace={busy}
            />
            <div className="flex items-center justify-center gap-2">
              <Button
                variant={pendingTempProviderSetup || needsRepair ? "default" : "outline"}
                size="sm"
                onClick={handlePrimaryAction}
                disabled={actionDisabled}
                className="gap-2"
              >
                {buttonIcon}
                <span>{buttonLabel}</span>
              </Button>
              <Button
                variant="ghost"
                size="icon-sm"
                onClick={() => openLogs("gateway")}
                aria-label={t("doctor.openLogs", { defaultValue: "Open logs" })}
                title={t("doctor.openLogs", { defaultValue: "Open logs" })}
                className="text-muted-foreground hover:text-foreground"
              >
                <FileTextIcon className="size-3.5" />
              </Button>
            </div>
            <div className="h-5 max-w-md overflow-hidden text-sm text-muted-foreground">
              {helperText ? (
                <span key={helperText} className="inline-block whitespace-nowrap transition-opacity duration-300 animate-pulse">
                  {helperText}
                </span>
              ) : null}
            </div>
          </div>
        </CardHeader>
        <CardContent className="space-y-4">
          {error ? (
            <div className="rounded-md border border-destructive/40 bg-destructive/10 px-3 py-2 text-sm text-destructive">
              <div>{error}</div>
              <div className="mt-2">
                <Button variant="outline" size="sm" onClick={() => openLogs("gateway")}>
                  {t("doctor.viewGatewayLogs", { defaultValue: "View Gateway Logs" })}
                </Button>
              </div>
            </div>
          ) : null}

          {pendingTempProviderSetup ? (
            <div className="rounded-md border border-amber-500/30 bg-amber-500/10 px-3 py-3 text-sm">
              <div className="font-medium text-foreground">
                {t("doctor.tempProviderActionRequired", {
                  defaultValue: "Temporary gateway needs a provider before repair can continue.",
                })}
              </div>
              <div className="mt-1 text-muted-foreground">
                {localizedPendingReason ?? pendingTempProviderSetup.reason}
              </div>
              <div className="mt-3">
                <Button size="sm" onClick={() => setTempProviderDialogOpen(true)}>
                  {t(
                    tempProviderProfileId ? "doctor.editTempProvider" : "doctor.configureTempProvider",
                    {
                      defaultValue: tempProviderProfileId
                        ? "Edit temp gateway provider"
                        : "Configure temp gateway provider",
                    },
                  )}
                </Button>
              </div>
            </div>
          ) : null}

          {diagnosis ? (
            <DoctorRecoveryOverview
              diagnosis={diagnosis}
              checkLoading={diagnosisLoading}
              repairing={repairing}
              progressLine={null}
              repairResult={repairResult}
              repairError={null}
              onRepairAll={() => void runRepair()}
              onRepairIssue={(_issueId) => void runRepair()}
              showRepairActions={false}
            />
          ) : null}
        </CardContent>
      </Card>

      <DoctorLogsDialog
        open={logsOpen}
        onOpenChange={setLogsOpen}
        source={logsSource}
        onSourceChange={setLogsSource}
      />
      <DoctorTempProviderDialog
        open={tempProviderDialogOpen}
        onOpenChange={setTempProviderDialogOpen}
        initialProfileId={tempProviderProfileId}
        onSaved={handleTempProviderSaved}
      />
    </section>
  );
}
