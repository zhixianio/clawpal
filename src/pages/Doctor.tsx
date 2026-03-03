import { useCallback, useEffect, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { useTranslation } from "react-i18next";
import { FileTextIcon, DownloadIcon } from "lucide-react";
import { useApi, hasGuidanceEmitted } from "@/lib/use-api";
import { useInstance } from "@/lib/instance-context";
import { useDoctorAgent } from "@/lib/use-doctor-agent";
import type {
  RescuePrimaryDiagnosisResult,
  RescuePrimaryIssue,
  RescuePrimaryRepairResult,
} from "@/lib/types";
import {
  Card,
  CardHeader,
  CardTitle,
  CardContent,
} from "@/components/ui/card";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import {
  Dialog,
  DialogContent,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
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
import { Skeleton } from "@/components/ui/skeleton";
import { DoctorChat } from "@/components/DoctorChat";
import { TokenBadge } from "@/components/TokenBadge";
import { ModelSwitcher } from "@/components/ModelSwitcher";
import { SessionAnalysisPanel } from "@/components/SessionAnalysisPanel";
import type { BackupInfo } from "@/lib/types";
import { formatTime, formatBytes } from "@/lib/utils";

type RescueMessageTone = "info" | "success" | "error";

interface RescueUiState {
  activating: boolean;
  deactivating: boolean;
  unsetting: boolean;
  statusChecking: boolean;
  configured: boolean | null;
  profile: string;
  port: number | null;
  message: string | null;
  messageTone: RescueMessageTone;
}

interface PrimaryRecoveryState {
  checkLoading: boolean;
  checkResult: RescuePrimaryDiagnosisResult | null;
  checkError: string | null;
  repairing: boolean;
  repairingIssueId: string | null;
  repairResult: RescuePrimaryRepairResult | null;
  repairError: string | null;
}

interface DoctorLaunchGuidance {
  message: string;
  summary: string;
  actions: string[];
  operation: string;
  instanceId: string;
  transport: string;
  rawError: string;
  createdAt: number;
}

interface DoctorProps {
  active?: boolean;
  launchGuidance?: DoctorLaunchGuidance | null;
  onLaunchGuidanceConsumed?: (instanceId: string) => void;
  connectRemoteHost?: (hostId: string) => Promise<void>;
}

const createInitialRescueUiState = (): RescueUiState => ({
  activating: false,
  deactivating: false,
  unsetting: false,
  statusChecking: false,
  configured: null,
  profile: "rescue",
  port: null,
  message: null,
  messageTone: "info",
});

const createInitialPrimaryRecoveryState = (): PrimaryRecoveryState => ({
  checkLoading: false,
  checkResult: null,
  checkError: null,
  repairing: false,
  repairingIssueId: null,
  repairResult: null,
  repairError: null,
});

function buildLaunchGuidanceContext(guidance: DoctorLaunchGuidance): string {
  const lines: string[] = [
    "[Escalated App Error Context]",
    `instance: ${guidance.instanceId}`,
    `transport: ${guidance.transport}`,
    `operation: ${guidance.operation}`,
    `error: ${guidance.rawError}`,
  ];
  const summary = (guidance.summary || guidance.message || "").trim();
  if (summary) lines.push(`assistant_summary: ${summary}`);
  if (guidance.actions.length > 0) {
    lines.push("assistant_suggested_actions:");
    for (const action of guidance.actions) {
      lines.push(`- ${action}`);
    }
  }
  lines.push("Please prioritize diagnosing and fixing this exact failure path first.");
  return lines.join("\n");
}

export function Doctor({
  active = false,
  launchGuidance = null,
  onLaunchGuidanceConsumed,
  connectRemoteHost,
}: DoctorProps) {
  const { t } = useTranslation();
  const ua = useApi();
  const { instanceId, isDocker, isRemote, isConnected } = useInstance();
  const doctor = useDoctorAgent();
  const [runtimeModel, setRuntimeModel] = useState<string | undefined>(undefined);
  const [sessionModelOverride, setSessionModelOverride] = useState<string | undefined>(undefined);
  const [remoteConnState, setRemoteConnState] = useState<"checking" | "connected" | "disconnected">("checking");

  const [diagnosing, setDiagnosing] = useState(false);
  const [startupStage, setStartupStage] = useState<"idle" | "connecting" | "collecting" | "starting">("idle");
  const [startError, setStartError] = useState<string | null>(null);

  // Backups state
  const [backups, setBackups] = useState<BackupInfo[] | null>(null);
  const [backingUp, setBackingUp] = useState(false);
  const [backupMessage, setBackupMessage] = useState("");

  // Full-auto confirmation dialog
  const [fullAutoConfirmOpen, setFullAutoConfirmOpen] = useState(false);

  // Logs state
  const [logsOpen, setLogsOpen] = useState(false);
  const [logsSource, setLogsSource] = useState<"clawpal" | "gateway">("clawpal");
  const [logsTab, setLogsTab] = useState<"app" | "error">("app");
  const [logsContent, setLogsContent] = useState("");
  const [logsLoading, setLogsLoading] = useState(false);
  const logsContentRef = useRef<HTMLPreElement>(null);
  const [rescueState, setRescueState] = useState<RescueUiState>(createInitialRescueUiState);
  const [primaryState, setPrimaryState] = useState<PrimaryRecoveryState>(createInitialPrimaryRecoveryState);
  const lastAutoLaunchKeyRef = useRef<string | null>(null);

  const {
    activating: rescueActivating,
    deactivating: rescueDeactivating,
    unsetting: rescueUnsetting,
    statusChecking: rescueStatusChecking,
    configured: rescueConfigured,
    profile: rescueProfile,
    port: rescuePort,
    message: rescueMessage,
    messageTone: rescueMessageTone,
  } = rescueState;
  const {
    checkLoading: primaryCheckLoading,
    checkResult: primaryCheckResult,
    checkError: primaryCheckError,
    repairing: primaryRepairing,
    repairingIssueId: primaryRepairingIssueId,
    repairResult: primaryRepairResult,
    repairError: primaryRepairError,
  } = primaryState;

  const updateRescueState = (patch: Partial<RescueUiState>) => {
    setRescueState((prev) => ({ ...prev, ...patch }));
  };

  const updatePrimaryState = (patch: Partial<PrimaryRecoveryState>) => {
    setPrimaryState((prev) => ({ ...prev, ...patch }));
  };

  // Keep execution target synced with current instance tab:
  // - local/docker: execute on local machine
  // - remote ssh: execute on selected remote host
  useEffect(() => {
    doctor.reset();
    doctor.disconnect();
    setRescueState(createInitialRescueUiState());
    setPrimaryState(createInitialPrimaryRecoveryState());
    doctor.setTarget(isRemote ? instanceId : "local");
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [doctor.setTarget, instanceId, isRemote]);

  // Fetch runtime target model for TokenBadge / ModelSwitcher.
  useEffect(() => {
    invoke<{ model?: string }>("get_zeroclaw_runtime_target")
      .then((target) => {
        if (target?.model) setRuntimeModel(target.model);
      })
      .catch(() => {});
  }, []);

  // Use instanceId as the stable session key for model override / usage tracking.
  // This matches the backend which looks up overrides by instance_id.
  const doctorSessionId = instanceId || "local";

  // Track session model override so TokenBadge uses the effective model for cost.
  useEffect(() => {
    if (!doctorSessionId) return;
    invoke<string | null>("get_session_model_override", { sessionId: doctorSessionId })
      .then((m) => setSessionModelOverride(m ?? undefined))
      .catch(() => {});
  }, [doctorSessionId]);

  // Effective model: session override takes priority over global runtime model.
  const effectiveModel = sessionModelOverride ?? runtimeModel;

  const handleStartDiagnosis = async (extraContext?: string) => {
    setStartError(null);
    setDiagnosing(true);
    setStartupStage("connecting");
    try {
      if (isRemote && !instanceId.trim()) {
        throw new Error(t("doctor.targetUnavailable"));
      }
      const diagnosisScope = isRemote
        ? instanceId
        : isDocker
          ? instanceId
          : "local";

      if (isRemote) {
        setRemoteConnState("checking");
        const status = await ua.sshStatus(instanceId);
        if (status !== "connected") {
          if (connectRemoteHost) {
            await connectRemoteHost(instanceId);
          } else {
            await ua.sshConnect(instanceId);
          }
        }
        setRemoteConnState("connected");
      }

      await doctor.connect();
      setStartupStage("collecting");
      const baseContext = isRemote
        ? await ua.collectDoctorContextRemote(instanceId)
        : await ua.collectDoctorContext();
      const context = extraContext
        ? `${baseContext}\n\n${extraContext}`
        : baseContext;
      const diagnosisTransport: "local" | "docker_local" | "remote_ssh" = isRemote
        ? "remote_ssh"
        : isDocker
          ? "docker_local"
          : "local";
      setStartupStage("starting");
      await doctor.startDiagnosis(context, "main", diagnosisScope, diagnosisTransport);
    } catch (err) {
      const msg = String(err);
      setStartError(msg);
      if (isRemote) {
        setRemoteConnState("disconnected");
      }
    } finally {
      setDiagnosing(false);
      setStartupStage("idle");
    }
  };

  const startupHint = diagnosing && doctor.messages.length === 0
    ? (startupStage === "collecting"
      ? t("doctor.startupCollecting")
      : startupStage === "starting"
        ? t("doctor.startupStarting")
        : t("doctor.startupConnecting"))
    : null;

  const handleStopDiagnosis = async () => {
    await doctor.disconnect();
    doctor.reset();
  };

  // Logs helpers
  const fetchLog = (source: "clawpal" | "gateway", which: "app" | "error") => {
    setLogsLoading(true);
    const fn = source === "clawpal"
      ? (which === "app" ? ua.readAppLog : ua.readErrorLog)
      : (which === "app" ? ua.readGatewayLog : ua.readGatewayErrorLog);
    fn(200)
      .then((text) => {
        setLogsContent(text);
        setTimeout(() => {
          if (logsContentRef.current) {
            logsContentRef.current.scrollTop = logsContentRef.current.scrollHeight;
          }
        }, 50);
      })
      .catch(() => setLogsContent(""))
      .finally(() => setLogsLoading(false));
  };

  const openLogs = (source: "clawpal" | "gateway") => {
    setLogsSource(source);
    setLogsTab("app");
    setLogsOpen(true);
  };

  const exportLogs = () => {
    if (!logsContent) return;
    const timestamp = new Date().toISOString().replace(/[:.]/g, "-");
    const filename = `${logsSource}-${logsTab}-${timestamp}.log`;
    const blob = new Blob([logsContent], { type: "text/plain" });
    const url = URL.createObjectURL(blob);
    const a = document.createElement("a");
    a.href = url;
    a.download = filename;
    document.body.appendChild(a);
    a.click();
    document.body.removeChild(a);
    URL.revokeObjectURL(url);
  };

  const refreshRescueStatus = async (isCancelled?: () => boolean) => {
    const cancelled = () => isCancelled?.() ?? false;
    if (isRemote && !isConnected) {
      if (cancelled()) return;
      updateRescueState({
        configured: null,
        port: null,
        message: t("doctor.rescueBotConnectRequired"),
        messageTone: "info",
      });
      return;
    }

    updateRescueState({ statusChecking: true });
    try {
      const result = await ua.manageRescueBot("status");
      if (cancelled()) return;
      updateRescueState({
        configured: result.wasAlreadyConfigured,
        profile: result.profile,
        port: result.wasAlreadyConfigured ? result.rescuePort : null,
        message: result.wasAlreadyConfigured
          ? t("doctor.rescueBotAlreadyConfiguredState", {
            profile: result.profile,
            port: result.rescuePort,
          })
          : t("doctor.rescueBotNotConfigured"),
        messageTone: "info",
      });
    } catch (error) {
      const text = error instanceof Error ? error.message : String(error);
      if (cancelled()) return;
      updateRescueState({
        configured: null,
        port: null,
        message: t("doctor.rescueBotStatusCheckFailed", { error: text }),
        messageTone: "error",
      });
    } finally {
      if (cancelled()) return;
      updateRescueState({ statusChecking: false });
    }
  };

  const handleActivateRescueBot = async () => {
    if (isRemote && !isConnected) {
      updateRescueState({
        message: t("doctor.rescueBotConnectRequired"),
        messageTone: "error",
      });
      return;
    }
    updateRescueState({
      activating: true,
      message: null,
      messageTone: "info",
    });
    try {
      const result = await ua.manageRescueBot("activate");
      updateRescueState({
        configured: true,
        profile: result.profile,
        port: result.rescuePort,
        message: t("doctor.rescueBotActivated", {
          profile: result.profile,
          port: result.rescuePort,
        }),
        messageTone: "success",
      });
    } catch (error) {
      const text = error instanceof Error ? error.message : String(error);
      if (text.includes("Gateway restart timed out")) {
        updateRescueState({
          message: t("doctor.rescueBotFailedTimeout", { error: text }),
          messageTone: "error",
        });
      } else {
        updateRescueState({
          message: t("doctor.rescueBotFailed", { error: text }),
          messageTone: "error",
        });
      }
    } finally {
      updateRescueState({ activating: false });
    }
  };

  const handleDeactivateRescueBot = async () => {
    if (isRemote && !isConnected) {
      updateRescueState({
        message: t("doctor.rescueBotConnectRequired"),
        messageTone: "error",
      });
      return;
    }
    updateRescueState({
      deactivating: true,
      message: null,
      messageTone: "info",
    });
    try {
      const result = await ua.manageRescueBot("deactivate");
      if (result.wasAlreadyConfigured) {
        updateRescueState({
          profile: result.profile,
          configured: true,
          port: result.rescuePort,
          message: t("doctor.rescueBotDeactivated", { profile: result.profile }),
          messageTone: "success",
        });
      } else {
        updateRescueState({
          profile: result.profile,
          configured: false,
          port: null,
          message: t("doctor.rescueBotAlreadyNotConfigured"),
          messageTone: "info",
        });
      }
    } catch (error) {
      const text = error instanceof Error ? error.message : String(error);
      updateRescueState({
        message: t("doctor.rescueBotDeactivateFailed", { error: text }),
        messageTone: "error",
      });
    } finally {
      updateRescueState({ deactivating: false });
    }
  };

  const handleUnsetRescueBot = async () => {
    if (isRemote && !isConnected) {
      updateRescueState({
        message: t("doctor.rescueBotConnectRequired"),
        messageTone: "error",
      });
      return;
    }
    updateRescueState({
      unsetting: true,
      message: null,
      messageTone: "info",
    });
    try {
      const result = await ua.manageRescueBot("unset");
      if (result.wasAlreadyConfigured) {
        updateRescueState({
          profile: result.profile,
          configured: false,
          port: null,
          message: t("doctor.rescueBotUnset", { profile: result.profile }),
          messageTone: "success",
        });
      } else {
        updateRescueState({
          profile: result.profile,
          configured: false,
          port: null,
          message: t("doctor.rescueBotAlreadyNotConfigured"),
          messageTone: "info",
        });
      }
    } catch (error) {
      const text = error instanceof Error ? error.message : String(error);
      updateRescueState({
        message: t("doctor.rescueBotUnsetFailed", { error: text }),
        messageTone: "error",
      });
    } finally {
      updateRescueState({ unsetting: false });
    }
  };

  const handleCheckPrimaryViaRescue = async () => {
    if (isRemote && !isConnected) {
      updatePrimaryState({ checkError: t("doctor.rescueBotConnectRequired") });
      return;
    }
    updatePrimaryState({
      checkLoading: true,
      checkError: null,
      repairError: null,
      repairResult: null,
    });
    try {
      const result = await ua.diagnosePrimaryViaRescue("primary", rescueProfile);
      updatePrimaryState({ checkResult: result });
    } catch (error) {
      const text = error instanceof Error ? error.message : String(error);
      updatePrimaryState({
        checkResult: null,
        checkError: t("doctor.primaryCheckFailed", { error: text }),
      });
    } finally {
      updatePrimaryState({ checkLoading: false });
    }
  };

  const primaryStatusLabel = (status: RescuePrimaryDiagnosisResult["status"]) => {
    if (status === "healthy") return t("doctor.primaryStatusHealthy");
    if (status === "degraded") return t("doctor.primaryStatusDegraded");
    return t("doctor.primaryStatusBroken");
  };

  const formatCheckedAt = (checkedAt: string) => {
    const value = new Date(checkedAt);
    if (Number.isNaN(value.getTime())) return checkedAt;
    return value.toLocaleString();
  };

  const countSafeFixableIssues = (result: RescuePrimaryDiagnosisResult | null) =>
    result?.issues.filter((issue) => issue.source === "primary" && issue.autoFixable).length ?? 0;

  const handleRepairPrimaryViaRescue = async () => {
    if (isRemote && !isConnected) {
      updatePrimaryState({ repairError: t("doctor.rescueBotConnectRequired") });
      return;
    }
    updatePrimaryState({
      repairing: true,
      repairingIssueId: null,
      repairError: null,
      repairResult: null,
    });
    try {
      const selectedIssueIds =
        primaryCheckResult?.issues
          .filter((issue) => issue.source === "primary" && issue.autoFixable)
          .map((issue) => issue.id) ?? [];
      const result = await ua.repairPrimaryViaRescue(
        "primary",
        rescueProfile,
        selectedIssueIds.length > 0 ? selectedIssueIds : undefined,
      );
      updatePrimaryState({
        repairResult: result,
        checkResult: result.after,
        checkError: null,
      });
    } catch (error) {
      const text = error instanceof Error ? error.message : String(error);
      updatePrimaryState({
        repairResult: null,
        repairError: t("doctor.primaryRepairFailed", { error: text }),
      });
    } finally {
      updatePrimaryState({
        repairing: false,
        repairingIssueId: null,
      });
    }
  };

  const handleRepairPrimaryIssue = async (issue: RescuePrimaryIssue) => {
    if (!issue.autoFixable || issue.source !== "primary") {
      return;
    }
    if (isRemote && !isConnected) {
      updatePrimaryState({ repairError: t("doctor.rescueBotConnectRequired") });
      return;
    }
    updatePrimaryState({
      repairing: true,
      repairingIssueId: issue.id,
      repairError: null,
      repairResult: null,
    });
    try {
      const result = await ua.repairPrimaryViaRescue("primary", rescueProfile, [issue.id]);
      updatePrimaryState({
        repairResult: result,
        checkResult: result.after,
        checkError: null,
      });
    } catch (error) {
      const text = error instanceof Error ? error.message : String(error);
      updatePrimaryState({
        repairResult: null,
        repairError: t("doctor.primaryRepairFailed", { error: text }),
      });
    } finally {
      updatePrimaryState({
        repairing: false,
        repairingIssueId: null,
      });
    }
  };

  useEffect(() => {
    let cancelled = false;
    void refreshRescueStatus(() => cancelled);
    return () => {
      cancelled = true;
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [instanceId, isRemote, isConnected]);

  useEffect(() => {
    if (!active || !launchGuidance) return;
    const launchKey = `${launchGuidance.instanceId}:${launchGuidance.operation}:${launchGuidance.createdAt}`;
    if (lastAutoLaunchKeyRef.current === launchKey) return;
    lastAutoLaunchKeyRef.current = launchKey;
    onLaunchGuidanceConsumed?.(launchGuidance.instanceId);
    void handleStartDiagnosis(buildLaunchGuidanceContext(launchGuidance));
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [active, launchGuidance, onLaunchGuidanceConsumed]);

  useEffect(() => {
    if (logsOpen) fetchLog(logsSource, logsTab);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [logsOpen, logsSource, logsTab]);

  useEffect(() => {
    if (!isRemote) {
      setRemoteConnState("connected");
      return;
    }
    let cancelled = false;
    setRemoteConnState("checking");
    ua.sshStatus(instanceId)
      .then((status) => {
        if (!cancelled) {
          setRemoteConnState(status === "connected" ? "connected" : "disconnected");
        }
      })
      .catch(() => {
        if (!cancelled) {
          setRemoteConnState("disconnected");
        }
      });
    return () => {
      cancelled = true;
    };
  }, [ua, instanceId, isRemote]);

  // Backups
  const refreshBackups = useCallback(() => {
    ua.listBackups().then(setBackups).catch((e) => console.error("Failed to load backups:", e));
  }, [ua]);
  useEffect(refreshBackups, [refreshBackups]);
  const showLegacyRecoveryCards = false;
  const isWsl2 = instanceId.startsWith("wsl2:");
  const displayedDoctorTarget = isRemote || isDocker || isWsl2 ? instanceId : "local";
  const instanceTypeLabel = isRemote
    ? t("doctor.targetTypeSsh")
    : isDocker
      ? t("doctor.targetTypeDocker")
      : isWsl2
        ? t("doctor.targetTypeWsl2")
        : t("doctor.targetTypeLocal");
  const isPureLocal = !isRemote && !isDocker && !isWsl2;

  return (
    <section>
      <h2 className="text-2xl font-bold mb-4">{t("doctor.title")}</h2>

      {showLegacyRecoveryCards && <Card className="mb-4 gap-2 py-4">
        <CardHeader className="pb-0">
          <CardTitle className="text-base">{t("doctor.rescueBotTitle")}</CardTitle>
        </CardHeader>
        <CardContent>
          <div className="flex items-center justify-between gap-3 flex-wrap">
            <p className="text-sm text-muted-foreground">{t("doctor.rescueBotHint")}</p>
            <div className="flex items-center gap-2">
              <Button
                variant="default"
                size="sm"
                onClick={handleActivateRescueBot}
                disabled={
                  rescueActivating
                  || rescueDeactivating
                  || rescueUnsetting
                  || rescueStatusChecking
                  || (isRemote && !isConnected)
                }
              >
                {rescueActivating ? t("doctor.activatingRescueBot") : t("doctor.activateRescueBot")}
              </Button>
              <Button
                variant="secondary"
                size="sm"
                onClick={handleDeactivateRescueBot}
                disabled={
                  rescueActivating
                  || rescueDeactivating
                  || rescueUnsetting
                  || rescueStatusChecking
                  || rescueConfigured !== true
                  || (isRemote && !isConnected)
                }
              >
                {rescueDeactivating ? t("doctor.deactivatingRescueBot") : t("doctor.deactivateRescueBot")}
              </Button>
              <Button
                variant="outline"
                size="sm"
                onClick={handleUnsetRescueBot}
                disabled={
                  rescueActivating
                  || rescueDeactivating
                  || rescueUnsetting
                  || rescueStatusChecking
                  || rescueConfigured !== true
                  || (isRemote && !isConnected)
                }
              >
                {rescueUnsetting ? t("doctor.unsettingRescueBot") : t("doctor.unsetRescueBot")}
              </Button>
              <Button
                variant="ghost"
                size="sm"
                onClick={() => {
                  void refreshRescueStatus();
                }}
                disabled={
                  rescueActivating
                  || rescueDeactivating
                  || rescueUnsetting
                  || rescueStatusChecking
                  || (isRemote && !isConnected)
                }
              >
                {rescueStatusChecking ? t("doctor.rescueBotChecking") : t("doctor.refresh")}
              </Button>
            </div>
          </div>
          {rescueMessage && (
            <div
              className={`mt-3 rounded-md border px-3 py-2 text-sm ${
                rescueMessageTone === "error"
                  ? "border-destructive/40 bg-destructive/10 text-destructive"
                  : rescueMessageTone === "success"
                    ? "border-emerald-500/40 bg-emerald-500/10 text-emerald-700 dark:text-emerald-300"
                    : "border-border/50 bg-muted/40 text-muted-foreground"
              }`}
            >
              <div>{rescueMessage}</div>
              {rescueMessageTone === "error" && (
                <div className="mt-2">
                  <Button variant="outline" size="sm" onClick={() => openLogs("gateway")}>
                    {t("doctor.viewGatewayLogs")}
                  </Button>
                </div>
              )}
            </div>
          )}
        </CardContent>
      </Card>}

      {showLegacyRecoveryCards && <Card className="mb-4 gap-2 py-4">
        <CardHeader className="pb-0">
          <CardTitle className="text-base">{t("doctor.primaryRecoveryTitle")}</CardTitle>
        </CardHeader>
        <CardContent>
          <div className="flex items-center justify-between gap-3 flex-wrap">
            <p className="text-sm text-muted-foreground">{t("doctor.primaryRecoveryHint")}</p>
            <div className="flex items-center gap-2">
              <Button
                variant="default"
                size="sm"
                onClick={handleCheckPrimaryViaRescue}
                disabled={primaryCheckLoading || primaryRepairing || (isRemote && !isConnected)}
              >
                {primaryCheckLoading
                  ? t("doctor.primaryChecking")
                  : t("doctor.primaryCheckNow")}
              </Button>
              <Button
                variant="secondary"
                size="sm"
                onClick={handleRepairPrimaryViaRescue}
                disabled={
                  primaryCheckLoading
                  || primaryRepairing
                  || !primaryCheckResult
                  || (isRemote && !isConnected)
                }
              >
                {primaryRepairing
                  ? t("doctor.primaryRepairing")
                  : t("doctor.primaryRepairNow", { count: countSafeFixableIssues(primaryCheckResult) })}
              </Button>
            </div>
          </div>
          {primaryCheckError && (
            <div className="mt-3 rounded-md border border-destructive/40 bg-destructive/10 px-3 py-2 text-sm text-destructive">
              <div>{primaryCheckError}</div>
              <div className="mt-2">
                <Button variant="outline" size="sm" onClick={() => openLogs("gateway")}>
                  {t("doctor.viewGatewayLogs")}
                </Button>
              </div>
            </div>
          )}
          {primaryRepairError && (
            <div className="mt-3 rounded-md border border-destructive/40 bg-destructive/10 px-3 py-2 text-sm text-destructive">
              <div>{primaryRepairError}</div>
              <div className="mt-2">
                <Button variant="outline" size="sm" onClick={() => openLogs("gateway")}>
                  {t("doctor.viewGatewayLogs")}
                </Button>
              </div>
            </div>
          )}
          {primaryCheckResult && (
            <div className="mt-3 rounded-md border border-border/60 bg-muted/20 px-3 py-3">
              <div className="flex items-center justify-between gap-2 flex-wrap">
                <div className="text-sm">
                  {t("doctor.primaryCheckedAt", { time: formatCheckedAt(primaryCheckResult.checkedAt) })}
                </div>
                <Badge
                  variant={primaryCheckResult.status === "healthy" ? "outline" : "destructive"}
                  className={primaryCheckResult.status === "healthy" ? "border-emerald-500/40 text-emerald-700 dark:text-emerald-300" : undefined}
                >
                  {primaryStatusLabel(primaryCheckResult.status)}
                </Badge>
              </div>
              <div className="mt-3 text-xs font-medium uppercase tracking-wide text-muted-foreground">
                {t("doctor.primaryChecks")}
              </div>
              <div className="mt-2 grid gap-2">
                {primaryCheckResult.checks.map((check) => (
                  <div key={check.id} className="rounded-md border border-border/50 bg-background/60 p-2">
                    <div className="flex items-center justify-between gap-2">
                      <div className="flex items-center gap-2">
                        <div className="text-sm">{check.title}</div>
                        {!check.ok && check.id === "rescue.profile.configured" && (
                          <Button
                            variant="outline"
                            size="sm"
                            className="h-6 px-2 text-[11px]"
                            onClick={handleActivateRescueBot}
                            disabled={
                              rescueActivating
                              || rescueDeactivating
                              || rescueUnsetting
                              || rescueStatusChecking
                              || (isRemote && !isConnected)
                            }
                          >
                            {rescueActivating ? t("doctor.activatingRescueBot") : t("doctor.activateRescueBot")}
                          </Button>
                        )}
                        {!check.ok && check.id.startsWith("primary.") && countSafeFixableIssues(primaryCheckResult) > 0 && (
                          <Button
                            variant="outline"
                            size="sm"
                            className="h-6 px-2 text-[11px]"
                            onClick={handleRepairPrimaryViaRescue}
                            disabled={primaryCheckLoading || primaryRepairing || (isRemote && !isConnected)}
                          >
                            {primaryRepairing ? t("doctor.primaryRepairing") : t("doctor.primaryQuickFix")}
                          </Button>
                        )}
                      </div>
                      <Badge variant={check.ok ? "outline" : "destructive"} className="text-[10px]">
                        {check.ok ? t("doctor.primaryCheckPass") : t("doctor.primaryCheckFail")}
                      </Badge>
                    </div>
                    <div className="mt-1 text-xs text-muted-foreground">{check.detail}</div>
                  </div>
                ))}
              </div>
              <div className="mt-3 text-xs font-medium uppercase tracking-wide text-muted-foreground">
                {t("doctor.primaryIssues")}
              </div>
              {primaryCheckResult.issues.length === 0 ? (
                <div className="mt-2 text-sm text-emerald-700 dark:text-emerald-300">
                  {t("doctor.primaryNoIssues")}
                </div>
              ) : (
                <div className="mt-2 grid gap-2">
                  {primaryCheckResult.issues.map((issue) => (
                    <div key={issue.id} className="rounded-md border border-destructive/30 bg-destructive/5 p-2">
                      <div className="flex items-center justify-between gap-2">
                        <div className="flex items-center gap-2">
                          <div className="text-sm">{issue.message}</div>
                          {issue.source === "primary" && issue.autoFixable && (
                            <Button
                              variant="outline"
                              size="sm"
                              className="h-6 px-2 text-[11px]"
                              onClick={() => {
                                void handleRepairPrimaryIssue(issue);
                              }}
                              disabled={primaryCheckLoading || primaryRepairing || (isRemote && !isConnected)}
                            >
                              {primaryRepairing && primaryRepairingIssueId === issue.id
                                ? t("doctor.primaryIssueFixing")
                                : t("doctor.primaryIssueFix")}
                            </Button>
                          )}
                        </div>
                        <div className="flex items-center gap-1">
                          <Badge variant="outline" className="text-[10px]">
                            {issue.source === "rescue"
                              ? t("doctor.primaryIssueSourceRescue")
                              : t("doctor.primaryIssueSourcePrimary")}
                          </Badge>
                          <Badge variant={issue.severity === "error" ? "destructive" : "outline"} className="text-[10px]">
                            {issue.severity}
                          </Badge>
                        </div>
                      </div>
                      {issue.fixHint && (
                        <div className="mt-1 text-xs text-muted-foreground">{issue.fixHint}</div>
                      )}
                    </div>
                  ))}
                </div>
              )}
              {primaryRepairResult && (
                <div className="mt-4 rounded-md border border-border/60 bg-background/70 p-3">
                  <div className="text-xs font-medium uppercase tracking-wide text-muted-foreground">
                    {t("doctor.primaryRepairSummary")}
                  </div>
                  <div className="mt-2 flex flex-wrap items-center gap-2 text-xs">
                    <Badge variant="outline">
                      {t("doctor.primaryRepairSelected", { count: primaryRepairResult.selectedIssueIds.length })}
                    </Badge>
                    <Badge variant="outline" className="border-emerald-500/40 text-emerald-700 dark:text-emerald-300">
                      {t("doctor.primaryRepairApplied", { count: primaryRepairResult.appliedIssueIds.length })}
                    </Badge>
                    <Badge variant="outline">
                      {t("doctor.primaryRepairSkipped", { count: primaryRepairResult.skippedIssueIds.length })}
                    </Badge>
                    <Badge variant={primaryRepairResult.failedIssueIds.length > 0 ? "destructive" : "outline"}>
                      {t("doctor.primaryRepairFailedCount", { count: primaryRepairResult.failedIssueIds.length })}
                    </Badge>
                  </div>
                  <div className="mt-2 text-xs text-muted-foreground">
                    {t("doctor.primaryRecheckedAt", { time: formatCheckedAt(primaryRepairResult.after.checkedAt) })}
                  </div>
                  <div className="mt-3 grid gap-2">
                    {primaryRepairResult.steps.map((step) => (
                      <div key={step.id} className="rounded-md border border-border/50 bg-muted/20 p-2">
                        <div className="flex items-center justify-between gap-2">
                          <div className="text-sm">{step.title}</div>
                          <Badge variant={step.ok ? "outline" : "destructive"} className="text-[10px]">
                            {step.ok ? t("doctor.primaryCheckPass") : t("doctor.primaryCheckFail")}
                          </Badge>
                        </div>
                        <div className="mt-1 text-xs text-muted-foreground">{step.detail}</div>
                      </div>
                    ))}
                  </div>
                </div>
              )}
            </div>
          )}
        </CardContent>
      </Card>}

      <Card className="gap-2 py-4">
        <CardHeader className="pb-0">
          <div className="flex items-center justify-between gap-3 flex-wrap">
            <CardTitle className="text-base">{t("doctor.agentSource")}</CardTitle>
            <div className="flex items-center gap-2 flex-wrap justify-end">
              <span className="text-xs text-muted-foreground">{t("doctor.targetExecutionLabel")}</span>
              <code className="rounded bg-muted px-1.5 py-0.5 text-xs">{displayedDoctorTarget}</code>
              <Badge variant="outline" className="text-[10px]">{instanceTypeLabel}</Badge>
              {isRemote && (
                <Badge
                  variant={remoteConnState === "disconnected" ? "destructive" : "outline"}
                  className="text-[10px]"
                >
                  {remoteConnState === "checking"
                    ? t("doctor.connecting")
                    : remoteConnState === "connected"
                      ? t("doctor.connected")
                      : t("doctor.disconnected")}
                </Badge>
              )}
              {isPureLocal && (
                <span className="text-[11px] text-amber-700 dark:text-amber-300">
                  {t("doctor.targetExecutionLocalWarning")}
                </span>
              )}
              <Button variant="outline" size="sm" onClick={() => openLogs("clawpal")}>
                <FileTextIcon className="h-3.5 w-3.5 mr-1.5" />
                {t("doctor.clawpalLogs")}
              </Button>
              <Button variant="outline" size="sm" onClick={() => openLogs("gateway")}>
                <FileTextIcon className="h-3.5 w-3.5 mr-1.5" />
                {t("doctor.gatewayLogs")}
              </Button>
            </div>
          </div>
        </CardHeader>
        <CardContent>
          {!doctor.connected && doctor.messages.length === 0 ? (
            <>
              {startError && (
                <div className="mb-3 text-sm text-destructive">{startError}</div>
              )}
              {doctor.error && (
                <div className="mb-3 text-sm text-destructive">{doctor.error}</div>
              )}
              {startupHint && (
                <div className="mb-3 text-sm text-muted-foreground animate-pulse">
                  {startupHint}
                </div>
              )}
              <Button onClick={() => { void handleStartDiagnosis(); }} disabled={diagnosing}>
                {diagnosing ? t("doctor.connecting") : t("doctor.startDiagnosis")}
              </Button>
            </>
          ) : !doctor.connected && doctor.messages.length > 0 ? (
            <>
              {/* Disconnected mid-session — show chat with reconnect banner */}
              <div className="flex items-center justify-between mb-3 p-2 rounded-md bg-destructive/10 border border-destructive/20">
                <span className="text-sm text-destructive">
                  {doctor.error || t("doctor.disconnected")}
                </span>
                <div className="flex items-center gap-2">
                  <Button size="sm" onClick={() => doctor.reconnect()}>
                    {t("doctor.reconnect")}
                  </Button>
                  <Button variant="outline" size="sm" onClick={handleStopDiagnosis}>
                    {t("doctor.stopDiagnosis")}
                  </Button>
                </div>
              </div>
              <DoctorChat
                messages={doctor.messages}
                loading={false}
                error={null}
                connected={false}
                onSendMessage={doctor.sendMessage}
                onApproveInvoke={doctor.approveInvoke}
                onRejectInvoke={doctor.rejectInvoke}
              />
            </>
          ) : (
            <>
              {startupHint && (
                <div className="mb-3 text-sm text-muted-foreground animate-pulse">
                  {startupHint}
                </div>
              )}
              <div className="flex items-center justify-between mb-3">
                <div className="flex items-center gap-2 flex-wrap">
                  <Badge variant="outline" className="text-xs">
                    {t("doctor.engineZeroclaw")}
                  </Badge>
                  <Badge variant="outline" className="text-xs flex items-center gap-1.5">
                    <span className={`inline-block w-1.5 h-1.5 rounded-full ${doctor.bridgeConnected ? "bg-emerald-500" : "bg-muted-foreground/40"}`} />
                    {doctor.bridgeConnected ? t("doctor.bridgeConnected") : t("doctor.bridgeDisconnected")}
                  </Badge>
                  <TokenBadge sessionId={doctorSessionId} model={effectiveModel} />
                  <ModelSwitcher sessionId={doctorSessionId} defaultModel={runtimeModel} onModelChange={setSessionModelOverride} />
                </div>
                <div className="flex items-center gap-2">
                  <label className="flex items-center gap-1.5 text-xs cursor-pointer select-none">
                    <input
                      type="checkbox"
                      checked={doctor.fullAuto}
                      onChange={(e) => {
                        if (e.target.checked) {
                          setFullAutoConfirmOpen(true);
                        } else {
                          doctor.setFullAuto(false);
                        }
                      }}
                      className="accent-primary"
                    />
                    {t("doctor.fullAuto")}
                  </label>
                  <Button variant="outline" size="sm" onClick={handleStopDiagnosis}>
                    {t("doctor.stopDiagnosis")}
                  </Button>
                </div>
              </div>
              <DoctorChat
                messages={doctor.messages}
                loading={doctor.loading}
                error={doctor.error}
                connected={doctor.connected}
                onSendMessage={doctor.sendMessage}
                onApproveInvoke={doctor.approveInvoke}
                onRejectInvoke={doctor.rejectInvoke}
              />
            </>
          )}
        </CardContent>
      </Card>

      {/* Full-Auto Confirmation */}
      <Dialog open={fullAutoConfirmOpen} onOpenChange={setFullAutoConfirmOpen}>
        <DialogContent className="sm:max-w-md">
          <DialogHeader>
            <DialogTitle>{t("doctor.fullAutoTitle")}</DialogTitle>
          </DialogHeader>
          <p className="text-sm text-muted-foreground">{t("doctor.fullAutoWarning")}</p>
          <div className="flex justify-end gap-2 mt-4">
            <Button variant="outline" size="sm" onClick={() => setFullAutoConfirmOpen(false)}>
              {t("doctor.cancel")}
            </Button>
            <Button variant="destructive" size="sm" onClick={() => {
              doctor.setFullAuto(true);
              setFullAutoConfirmOpen(false);
            }}>
              {t("doctor.fullAutoConfirm")}
            </Button>
          </div>
        </DialogContent>
      </Dialog>

      {/* Logs Dialog */}
      <Dialog open={logsOpen} onOpenChange={setLogsOpen}>
        <DialogContent className="sm:max-w-2xl max-h-[80vh] flex flex-col">
          <DialogHeader>
            <DialogTitle>
              {logsSource === "clawpal" ? t("doctor.clawpalLogs") : t("doctor.gatewayLogs")}
            </DialogTitle>
          </DialogHeader>
          <div className="flex items-center gap-2 mb-2">
            <Button
              variant={logsTab === "app" ? "default" : "outline"}
              size="sm"
              onClick={() => setLogsTab("app")}
            >
              {t("doctor.appLog")}
            </Button>
            <Button
              variant={logsTab === "error" ? "default" : "outline"}
              size="sm"
              onClick={() => setLogsTab("error")}
            >
              {t("doctor.errorLog")}
            </Button>
            <Button
              variant="ghost"
              size="sm"
              onClick={() => fetchLog(logsSource, logsTab)}
              disabled={logsLoading}
            >
              {t("doctor.refreshLogs")}
            </Button>
            <Button
              variant="outline"
              size="sm"
              onClick={exportLogs}
              disabled={!logsContent}
            >
              <DownloadIcon className="h-3.5 w-3.5 mr-1.5" />
              {t("doctor.exportLogs")}
            </Button>
          </div>
          <pre
            ref={logsContentRef}
            className="flex-1 min-h-[300px] max-h-[60vh] overflow-auto rounded-md border bg-muted p-3 text-xs font-mono whitespace-pre-wrap break-all"
          >
            {logsContent || t("doctor.noLogs")}
          </pre>
        </DialogContent>
      </Dialog>

      {/* Sessions */}
      <div className="mt-8">
        <h3 className="text-lg font-semibold mb-4">{t("doctor.sessions")}</h3>
        <SessionAnalysisPanel />
      </div>

      {/* Backups */}
      <div className="mt-8">
        <div className="flex items-center justify-between mb-4">
          <h3 className="text-lg font-semibold">{t("doctor.backups")}</h3>
          <Button
            size="sm"
            variant="outline"
            disabled={backingUp}
            onClick={() => {
              setBackingUp(true);
              setBackupMessage("");
              ua.backupBeforeUpgrade()
                .then((info) => {
                  setBackupMessage(t("home.backupCreated", { name: info.name }));
                  refreshBackups();
                })
                .catch((e) => { if (!hasGuidanceEmitted(e)) setBackupMessage(t("home.backupFailed", { error: String(e) })); })
                .finally(() => setBackingUp(false));
            }}
          >
            {backingUp ? t("home.creating") : t("home.createBackup")}
          </Button>
        </div>
        {backupMessage && (
          <p className="text-sm text-muted-foreground mb-2">{backupMessage}</p>
        )}
        {backups === null ? (
          <div className="space-y-2">
            <Skeleton className="h-16 w-full" />
            <Skeleton className="h-16 w-full" />
          </div>
        ) : backups.length === 0 ? (
          <p className="text-muted-foreground text-sm">{t("doctor.noBackups")}</p>
        ) : (
          <div className="space-y-2">
            {backups.map((backup) => (
              <Card key={backup.name}>
                <CardContent className="flex items-center justify-between">
                  <div>
                    <div className="font-medium text-sm">{backup.name}</div>
                    <div className="text-xs text-muted-foreground">
                      {formatTime(backup.createdAt)} — {formatBytes(backup.sizeBytes)}
                    </div>
                    {ua.isRemote && backup.path && (
                      <div className="text-xs text-muted-foreground mt-0.5 font-mono">{backup.path}</div>
                    )}
                  </div>
                  <div className="flex gap-1.5">
                    {!ua.isRemote && (
                      <Button
                        size="sm"
                        variant="outline"
                        onClick={() => ua.openUrl(backup.path)}
                      >
                        {t("home.show")}
                      </Button>
                    )}
                    <AlertDialog>
                      <AlertDialogTrigger asChild>
                        <Button size="sm" variant="outline">
                          {t("home.restore")}
                        </Button>
                      </AlertDialogTrigger>
                      <AlertDialogContent>
                        <AlertDialogHeader>
                          <AlertDialogTitle>{t("home.restoreTitle")}</AlertDialogTitle>
                          <AlertDialogDescription>
                            {t("home.restoreDescription", { name: backup.name })}
                          </AlertDialogDescription>
                        </AlertDialogHeader>
                        <AlertDialogFooter>
                          <AlertDialogCancel>{t("config.cancel")}</AlertDialogCancel>
                          <AlertDialogAction
                            onClick={() => {
                              ua.restoreFromBackup(backup.name)
                                .then((msg) => setBackupMessage(msg))
                                .catch((e) => { if (!hasGuidanceEmitted(e)) setBackupMessage(t("home.restoreFailed", { error: String(e) })); });
                            }}
                          >
                            {t("home.restore")}
                          </AlertDialogAction>
                        </AlertDialogFooter>
                      </AlertDialogContent>
                    </AlertDialog>
                    <AlertDialog>
                      <AlertDialogTrigger asChild>
                        <Button size="sm" variant="destructive">
                          {t("home.delete")}
                        </Button>
                      </AlertDialogTrigger>
                      <AlertDialogContent>
                        <AlertDialogHeader>
                          <AlertDialogTitle>{t("home.deleteBackupTitle")}</AlertDialogTitle>
                          <AlertDialogDescription>
                            {t("home.deleteBackupDescription", { name: backup.name })}
                          </AlertDialogDescription>
                        </AlertDialogHeader>
                        <AlertDialogFooter>
                          <AlertDialogCancel>{t("config.cancel")}</AlertDialogCancel>
                          <AlertDialogAction
                            className="bg-destructive text-destructive-foreground hover:bg-destructive/90"
                            onClick={() => {
                              ua.deleteBackup(backup.name)
                                .then(() => {
                                  setBackupMessage(t("home.deletedBackup", { name: backup.name }));
                                  refreshBackups();
                                })
                                .catch((e) => { if (!hasGuidanceEmitted(e)) setBackupMessage(t("home.deleteBackupFailed", { error: String(e) })); });
                            }}
                          >
                            {t("home.delete")}
                          </AlertDialogAction>
                        </AlertDialogFooter>
                      </AlertDialogContent>
                    </AlertDialog>
                  </div>
                </CardContent>
              </Card>
            ))}
          </div>
        )}
      </div>
    </section>
  );
}
