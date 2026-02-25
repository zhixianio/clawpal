import { useEffect, useMemo, useState } from "react";
import { useTranslation } from "react-i18next";
import { Card, CardContent } from "@/components/ui/card";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import type {
  EnsureAccessResult,
  InstallMethod,
  InstallMethodCapability,
  InstallSession,
  InstallStep,
  InstallStepResult,
  SshHost,
} from "@/lib/types";
import { useApi } from "@/lib/use-api";
import { appendOrchestratorEvent } from "@/lib/orchestrator-log";

const METHOD_ORDER: InstallMethod[] = ["local", "wsl2", "docker", "remote_ssh"];
const STEP_ORDER: InstallStep[] = ["precheck", "install", "init", "verify"];

type StepStatus = "pending" | "running" | "success" | "failed";
type BlockerAction = "resume" | "settings" | "doctor" | "instances";

interface InstallAutoBlocker {
  code: string;
  message: string;
  details?: string;
  actions: BlockerAction[];
}

function classifyAutoBlocker(
  error: string,
  fallbackMessage: string,
  errorCode?: string | null,
  actionHint?: string | null,
): InstallAutoBlocker {
  if (actionHint === "open_settings_auth") {
    return {
      code: errorCode || "auth_missing",
      message: fallbackMessage,
      details: error,
      actions: ["settings", "resume"],
    };
  }
  if (actionHint === "open_instances") {
    return {
      code: errorCode || "remote_target_missing",
      message: fallbackMessage,
      details: error,
      actions: ["instances", "resume"],
    };
  }
  if (actionHint === "open_doctor") {
    return {
      code: errorCode || "diagnosis_required",
      message: fallbackMessage,
      details: error,
      actions: ["doctor", "resume"],
    };
  }
  if (errorCode === "permission_denied") {
    return {
      code: "permission_denied",
      message: fallbackMessage,
      details: error,
      actions: ["doctor", "resume"],
    };
  }
  if (errorCode === "network_error") {
    return {
      code: "network_error",
      message: fallbackMessage,
      details: error,
      actions: ["doctor", "resume"],
    };
  }
  if (errorCode === "env_missing") {
    return {
      code: "env_missing",
      message: fallbackMessage,
      details: error,
      actions: ["doctor", "resume"],
    };
  }
  const lower = error.toLowerCase();
  if (
    lower.includes("no compatible api key found")
    || lower.includes("no auth profile")
    || lower.includes("openrouter_api_key")
    || lower.includes("anthropic_api_key")
    || lower.includes("openai_api_key")
  ) {
    return {
      code: "auth_missing",
      message: fallbackMessage,
      details: error,
      actions: ["settings", "resume"],
    };
  }
  if (
    lower.includes("no ssh host config with id")
    || lower.includes("remote ssh host not found")
    || lower.includes("remote ssh target missing")
  ) {
    return {
      code: "remote_target_missing",
      message: fallbackMessage,
      details: error,
      actions: ["instances", "resume"],
    };
  }
  if (
    lower.includes("cannot connect to the docker daemon")
    || lower.includes("docker: command not found")
    || lower.includes("command failed: docker")
  ) {
    return {
      code: "docker_unavailable",
      message: fallbackMessage,
      details: error,
      actions: ["doctor", "resume"],
    };
  }
  if (lower.includes("permission denied") || lower.includes("operation not permitted")) {
    return {
      code: "permission_denied",
      message: fallbackMessage,
      details: error,
      actions: ["doctor", "resume"],
    };
  }
  if (lower.includes("network") || lower.includes("timed out") || lower.includes("failed to connect")) {
    return {
      code: "network_error",
      message: fallbackMessage,
      details: error,
      actions: ["doctor", "resume"],
    };
  }
  return {
    code: "unknown",
    message: fallbackMessage,
    details: error,
    actions: ["resume"],
  };
}

function sortMethods(methods: InstallMethodCapability[]): InstallMethodCapability[] {
  const rank = new Map(METHOD_ORDER.map((method, index) => [method, index]));
  return [...methods].sort((a, b) => (rank.get(a.method) ?? 99) - (rank.get(b.method) ?? 99));
}

function getStepStatus(state: string | null | undefined, step: InstallStep): StepStatus {
  if (!state) return "pending";
  if (step === "precheck") {
    if (state === "precheck_running") return "running";
    if (state === "precheck_failed") return "failed";
    if (["precheck_passed", "install_running", "install_passed", "init_running", "init_passed", "ready"].includes(state)) return "success";
    return "pending";
  }
  if (step === "install") {
    if (state === "install_running") return "running";
    if (state === "install_failed") return "failed";
    if (["install_passed", "init_running", "init_passed", "ready"].includes(state)) return "success";
    return "pending";
  }
  if (step === "init") {
    if (state === "init_running") return "running";
    if (state === "init_failed") return "failed";
    if (["init_passed", "ready"].includes(state)) return "success";
    return "pending";
  }
  if (step === "verify") {
    if (state === "ready") return "success";
    return "pending";
  }
  return "pending";
}

function canRunStep(state: string | null | undefined, step: InstallStep): boolean {
  if (!state) return false;
  if (step === "precheck") {
    return state === "selected_method" || state === "precheck_failed";
  }
  if (step === "install") {
    return state === "precheck_passed" || state === "install_failed";
  }
  if (step === "init") {
    return state === "install_passed" || state === "init_failed";
  }
  return state === "init_passed";
}

export function InstallHub({
  showToast,
  onNavigate,
  onReady,
}: {
  showToast?: (message: string, type?: "success" | "error") => void;
  onNavigate?: (route: string) => void;
  onReady?: (method: InstallMethod) => void;
}) {
  const { t } = useTranslation();
  const ua = useApi();
  const [methods, setMethods] = useState<InstallMethodCapability[]>([]);
  const [loadingMethods, setLoadingMethods] = useState(true);
  const [selectedMethod, setSelectedMethod] = useState<InstallMethod>("local");
  const [creating, setCreating] = useState(false);
  const [runningStep, setRunningStep] = useState<InstallStep | null>(null);
  const [autoRunning, setAutoRunning] = useState(false);
  const [session, setSession] = useState<InstallSession | null>(null);
  const [lastResult, setLastResult] = useState<InstallStepResult | null>(null);
  const [lastAccessResult, setLastAccessResult] = useState<EnsureAccessResult | null>(null);
  const [lastAccessError, setLastAccessError] = useState<string | null>(null);
  const [ensuringAccess, setEnsuringAccess] = useState(false);
  const [lastOrchestratorReason, setLastOrchestratorReason] = useState<string>("");
  const [lastOrchestratorSource, setLastOrchestratorSource] = useState<string>("");
  const [autoBlocker, setAutoBlocker] = useState<InstallAutoBlocker | null>(null);
  const [sshHosts, setSshHosts] = useState<SshHost[]>([]);
  const [selectedSshHostId, setSelectedSshHostId] = useState<string>("");

  useEffect(() => {
    setLoadingMethods(true);
    ua.listInstallMethods()
      .then((result) => {
        const sorted = sortMethods(result);
        setMethods(sorted);
        if (sorted.length > 0) {
          setSelectedMethod(sorted[0].method);
        }
      })
      .catch((e) => showToast?.(String(e), "error"))
      .finally(() => setLoadingMethods(false));
  }, [ua, showToast]);

  useEffect(() => {
    ua.listSshHosts()
      .then((hosts) => {
        setSshHosts(hosts);
        if (hosts.length > 0) {
          setSelectedSshHostId(hosts[0].id);
        }
      })
      .catch(() => {});
  }, [ua]);

  const selectedMeta = useMemo(
    () => methods.find((m) => m.method === selectedMethod) ?? null,
    [methods, selectedMethod],
  );

  const methodLabel = (method: InstallMethod): string => t(`home.install.method.${method}`);

  const ensureInstanceByMethod = (nextSession: InstallSession): { instanceId: string; transport: string } | null => {
    if (nextSession.method === "local") {
      return { instanceId: "local", transport: "local" };
    }
    if (nextSession.method === "docker") {
      return { instanceId: "docker:local", transport: "docker_local" };
    }
    if (nextSession.method === "wsl2") {
      return { instanceId: "wsl2:local", transport: "wsl2" };
    }
    if (nextSession.method === "remote_ssh") {
      const hostId = (nextSession.artifacts?.ssh_host_id as string | undefined) || selectedSshHostId;
      if (!hostId) return null;
      return { instanceId: hostId, transport: "remote_ssh" };
    }
    return null;
  };

  const runEnsureAccess = async (nextSession: InstallSession): Promise<EnsureAccessResult | null> => {
    const target = ensureInstanceByMethod(nextSession);
    if (!target) return null;
    setEnsuringAccess(true);
    setLastAccessError(null);
    try {
      const result = await ua.ensureAccessProfile(target.instanceId, target.transport);
      setLastAccessResult(result);
      showToast?.(
        t("home.install.access.ready", {
          chain: result.workingChain.join(" -> "),
        }),
        "success",
      );
      appendOrchestratorEvent({
        level: "success",
        message: "access discovery completed",
        instanceId: target.instanceId,
        sessionId: nextSession.id,
        goal: `install:${nextSession.method}`,
        source: "aad",
        state: nextSession.state,
        details: result.workingChain.join(" -> "),
      });
      return result;
    } catch (e) {
      const message = String(e);
      setLastAccessError(message);
      showToast?.(t("home.install.access.failed", { error: message }), "error");
      appendOrchestratorEvent({
        level: "error",
        message: "access discovery failed",
        instanceId: target.instanceId,
        sessionId: nextSession.id,
        goal: `install:${nextSession.method}`,
        source: "aad",
        state: nextSession.state,
        details: message,
      });
      return null;
    } finally {
      setEnsuringAccess(false);
    }
  };

  const runRecordExperience = async (nextSession: InstallSession) => {
    const target = ensureInstanceByMethod(nextSession);
    if (!target) return;
    try {
      const result = await ua.recordInstallExperience(
        nextSession.id,
        target.instanceId,
        `install:${nextSession.method}`,
      );
      showToast?.(
        t("home.install.access.experienceSaved", { count: result.totalCount }),
        "success",
      );
      appendOrchestratorEvent({
        level: "success",
        message: "experience saved",
        instanceId: target.instanceId,
        sessionId: nextSession.id,
        goal: `install:${nextSession.method}`,
        source: "experience-store",
        state: nextSession.state,
        details: `total=${result.totalCount}`,
      });
    } catch (e) {
      showToast?.(t("home.install.access.experienceFailed", { error: String(e) }), "error");
      appendOrchestratorEvent({
        level: "error",
        message: "experience save failed",
        instanceId: target.instanceId,
        sessionId: nextSession.id,
        goal: `install:${nextSession.method}`,
        source: "experience-store",
        state: nextSession.state,
        details: String(e),
      });
    }
  };

  const runStepAndRefresh = async (
    targetSession: InstallSession,
    step: InstallStep,
    quiet = false,
  ): Promise<{ result: InstallStepResult; session: InstallSession | null }> => {
    setRunningStep(step);
    try {
      const result = await ua.installRunStep(targetSession.id, step);
      setLastResult(result);
      if (!quiet) {
        showToast?.(result.summary, result.ok ? "success" : "error");
      }
      const next = await refreshSession(targetSession.id);
      if (result.ok) {
        setAutoBlocker(null);
      }
      const target = ensureInstanceByMethod(next);
      appendOrchestratorEvent({
        level: result.ok ? "success" : "error",
        message: result.summary,
        instanceId: target?.instanceId || "local",
        sessionId: targetSession.id,
        goal: `install:${targetSession.method}`,
        source: "step-runner",
        step,
        state: next.state,
        details: result.details,
      });
      if (next.state === "init_passed" || next.state === "ready") {
        await runEnsureAccess(next);
      }
      if (next.state === "ready") {
        await runRecordExperience(next);
        onReady?.(next.method);
      }
      return { result, session: next };
    } catch (e) {
      const message = String(e);
      if (!quiet) {
        showToast?.(message, "error");
      }
      return {
        result: {
          ok: false,
          summary: message,
          details: message,
          commands: [],
          artifacts: {},
          next_step: null,
          error_code: "runtime_error",
        },
        session: null,
      };
    } finally {
      setRunningStep(null);
    }
  };

  const runAutoInstall = async (startSession: InstallSession) => {
    setAutoRunning(true);
    setAutoBlocker(null);
    try {
      let current = startSession;
      const goal = `install:${startSession.method}`;
      const initialTarget = ensureInstanceByMethod(startSession);
      appendOrchestratorEvent({
        level: "info",
        message: "auto-install started",
        instanceId: initialTarget?.instanceId || "local",
        sessionId: startSession.id,
        goal,
        source: "orchestrator",
        state: startSession.state,
      });
      while (current.state !== "ready") {
        let step: InstallStep | null = null;
        try {
          const decision = await ua.installOrchestratorNext(current.id, goal);
          setLastOrchestratorReason(decision.reason || "");
          setLastOrchestratorSource(decision.source || "");
          if (decision.source !== "zeroclaw-sidecar") {
            const blocker = classifyAutoBlocker(
              decision.reason || "",
              t("home.install.blocked.orchestratorSource", { source: decision.source }),
              decision.errorCode,
              decision.actionHint,
            );
            setAutoBlocker(blocker);
            const target = ensureInstanceByMethod(current);
            appendOrchestratorEvent({
              level: "error",
              message: "orchestrator fallback blocked (strict mode)",
              instanceId: target?.instanceId || "local",
              sessionId: current.id,
              goal,
              source: decision.source,
              state: current.state,
              details: decision.reason,
            });
            showToast?.(t("home.install.orchestratorStrict", { source: decision.source }), "error");
            return;
          }
          step = decision.step as InstallStep | null;
          const target = ensureInstanceByMethod(current);
          appendOrchestratorEvent({
            level: "info",
            message: `orchestrator selected step: ${decision.step || "stop"}`,
            instanceId: target?.instanceId || "local",
            sessionId: current.id,
            goal,
            source: decision.source,
            state: current.state,
            details: decision.reason,
          });
        } catch (e) {
          const blocker = classifyAutoBlocker(
            String(e),
            t("home.install.blocked.orchestratorUnavailable"),
          );
          setAutoBlocker(blocker);
          setLastOrchestratorReason(String(e));
          setLastOrchestratorSource("error");
          const target = ensureInstanceByMethod(current);
          appendOrchestratorEvent({
            level: "error",
            message: "orchestrator decision failed",
            instanceId: target?.instanceId || "local",
            sessionId: current.id,
            goal,
            source: "error",
            state: current.state,
            details: String(e),
          });
          showToast?.(t("home.install.orchestratorUnavailable", { error: String(e) }), "error");
          return;
        }
        if (!step) break;
        const { result, session: refreshed } = await runStepAndRefresh(current, step, true);
        if (!result.ok || !refreshed) {
          const blocker = classifyAutoBlocker(
            result.details || result.summary,
            t("home.install.blocked.stepFailed", { step: t(`home.install.step.${step}`) }),
            result.error_code,
          );
          setAutoBlocker(blocker);
          showToast?.(result.summary, "error");
          return;
        }
        current = refreshed;
      }
      if (current.state === "ready") {
        setAutoBlocker(null);
        showToast?.(t("home.install.autoDone"), "success");
        const target = ensureInstanceByMethod(current);
        appendOrchestratorEvent({
          level: "success",
          message: "auto-install completed",
          instanceId: target?.instanceId || "local",
          sessionId: current.id,
          goal,
          source: "orchestrator",
          state: current.state,
        });
      }
    } finally {
      setAutoRunning(false);
    }
  };

  const handleCreateSession = () => {
    if (selectedMethod === "remote_ssh" && !selectedSshHostId) {
      showToast?.(t("home.install.remoteHostRequired"), "error");
      return;
    }
    setCreating(true);
    setLastResult(null);
    setLastAccessResult(null);
    setLastAccessError(null);
    setAutoBlocker(null);
    const options = selectedMethod === "remote_ssh"
      ? { ssh_host_id: selectedSshHostId }
      : undefined;
    ua.installCreateSession(selectedMethod, options)
      .then((next) => {
        setSession(next);
        showToast?.(t("home.install.sessionCreated"), "success");
        const target = ensureInstanceByMethod(next);
        appendOrchestratorEvent({
          level: "info",
          message: "install session created",
          instanceId: target?.instanceId || "local",
          sessionId: next.id,
          goal: `install:${next.method}`,
          source: "ui",
          state: next.state,
        });
        void runAutoInstall(next);
      })
      .catch((e) => showToast?.(String(e), "error"))
      .finally(() => setCreating(false));
  };

  const refreshSession = (sessionId: string) => {
    return ua.installGetSession(sessionId).then((next) => {
      setSession(next);
      return next;
    });
  };

  const runStep = (step: InstallStep) => {
    if (!session) return;
    void runStepAndRefresh(session, step);
  };

  const renderBlockerAction = (action: BlockerAction) => {
    if (!session) return null;
    if (action === "resume") {
      return (
        <Button size="xs" variant="outline" onClick={() => void runAutoInstall(session)}>
          {t("home.install.resumeAuto")}
        </Button>
      );
    }
    if (action === "settings") {
      return (
        <Button size="xs" variant="outline" onClick={() => onNavigate?.("settings")}>
          {t("home.install.goSettings")}
        </Button>
      );
    }
    if (action === "doctor") {
      return (
        <Button size="xs" variant="outline" onClick={() => onNavigate?.("doctor")}>
          {t("home.install.openDoctor")}
        </Button>
      );
    }
    if (action === "instances") {
      return (
        <Button size="xs" variant="outline" onClick={() => onNavigate?.("home")}>
          {t("home.install.openInstances")}
        </Button>
      );
    }
    return null;
  };

  return (
    <>
      <h3 className="text-lg font-semibold mt-8 mb-4">{t("home.install.title")}</h3>
      <Card>
        <CardContent className="space-y-4">
          <p className="text-sm text-muted-foreground">{t("home.install.description")}</p>
          <div className="flex flex-wrap items-center gap-2">
            <Select
              value={selectedMethod}
              onValueChange={(value) => setSelectedMethod(value as InstallMethod)}
              disabled={loadingMethods || creating || runningStep !== null || autoRunning}
            >
              <SelectTrigger size="sm" className="w-[240px]">
                <SelectValue placeholder={t("home.install.selectMethod")} />
              </SelectTrigger>
              <SelectContent>
                {methods.map((method) => (
                  <SelectItem key={method.method} value={method.method}>
                    {methodLabel(method.method)}
                  </SelectItem>
                ))}
              </SelectContent>
            </Select>
            {selectedMeta && (
              <Badge variant={selectedMeta.available ? "secondary" : "outline"}>
                {selectedMeta.available
                  ? t("home.install.available")
                  : t("home.install.needsSetup")}
              </Badge>
            )}
            <Button size="sm" disabled={creating || loadingMethods || runningStep !== null || autoRunning} onClick={handleCreateSession}>
              {creating ? t("home.install.creating") : t("home.install.start")}
            </Button>
            {autoRunning && (
              <Badge variant="outline">{t("home.install.autoRunning")}</Badge>
            )}
          </div>
          {selectedMeta?.hint && (
            <p className="text-xs text-muted-foreground">{selectedMeta.hint}</p>
          )}
          {selectedMethod === "remote_ssh" && (
            <div className="flex items-center gap-2">
              <Select
                value={selectedSshHostId}
                onValueChange={setSelectedSshHostId}
                disabled={creating || runningStep !== null || autoRunning || sshHosts.length === 0}
              >
                <SelectTrigger size="sm" className="w-[260px]">
                  <SelectValue placeholder={t("home.install.selectRemoteHost")} />
                </SelectTrigger>
                <SelectContent>
                  {sshHosts.map((host) => (
                    <SelectItem key={host.id} value={host.id}>
                      {host.label}
                    </SelectItem>
                  ))}
                </SelectContent>
              </Select>
              {sshHosts.length === 0 && (
                <span className="text-xs text-muted-foreground">{t("home.install.noRemoteHosts")}</span>
              )}
            </div>
          )}

          {session && (
            <div className="space-y-3 rounded-md border p-3 text-sm">
              <div>
                <div className="font-medium">{t("home.install.currentSession")}</div>
                <div className="text-muted-foreground">ID: {session.id}</div>
                <div className="text-muted-foreground">
                  {t("home.install.sessionState", { state: session.state })}
                </div>
                {lastOrchestratorSource && (
                  <div className="text-muted-foreground">
                    {t("home.install.orchestrator", {
                      source: lastOrchestratorSource,
                      reason: lastOrchestratorReason || "-",
                    })}
                  </div>
                )}
              </div>

              <div className="rounded border bg-muted/30 p-2 text-xs space-y-1">
                <div className="font-medium">{t("home.install.access.title")}</div>
                <div className="text-muted-foreground">
                  {ensuringAccess
                    ? t("home.install.access.probing")
                    : lastAccessResult
                      ? t("home.install.access.probed")
                      : lastAccessError
                        ? t("home.install.access.failedInline")
                        : t("home.install.access.notStarted")}
                </div>
                {lastAccessResult && (
                  <>
                    <div className="text-muted-foreground">
                      {t("home.install.access.chain", { chain: lastAccessResult.workingChain.join(" -> ") })}
                    </div>
                    <div className="text-muted-foreground">
                      {lastAccessResult.profileReused
                        ? t("home.install.access.reused")
                        : t("home.install.access.created")}
                      {lastAccessResult.usedLegacyFallback ? ` · ${t("home.install.access.fallback")}` : ""}
                    </div>
                  </>
                )}
                {lastAccessError && (
                  <div className="text-red-600 dark:text-red-400">{lastAccessError}</div>
                )}
              </div>

              <div className="space-y-2">
                {STEP_ORDER.map((step) => {
                  const status = getStepStatus(session.state, step);
                  const actionable = canRunStep(session.state, step);
                  return (
                    <div key={step} className="flex items-center justify-between rounded border px-2 py-1.5">
                      <div className="flex items-center gap-2">
                        <span className="text-xs font-medium">{t(`home.install.step.${step}`)}</span>
                        <Badge variant="outline" className="text-[10px]">
                          {t(`home.install.status.${status}`)}
                        </Badge>
                      </div>
                      <Button
                        size="xs"
                        variant={status === "failed" ? "outline" : "default"}
                        disabled={runningStep !== null || autoRunning || !actionable}
                        onClick={() => runStep(step)}
                      >
                        {runningStep === step
                          ? t("home.install.running")
                          : status === "failed"
                            ? t("home.install.retry")
                            : t("home.install.runStep")}
                      </Button>
                    </div>
                  );
                })}
              </div>

              {lastResult && (
                <div className="rounded border bg-muted/30 p-2 text-xs space-y-1">
                  <div className="font-medium">{lastResult.summary}</div>
                  <div className="max-h-24 overflow-auto rounded border bg-background/70 p-2 text-muted-foreground whitespace-pre-wrap break-all">
                    {lastResult.details}
                  </div>
                  {lastResult.commands.length > 0 && (
                    <div className="max-h-40 overflow-auto rounded border bg-background/70 p-2 font-mono text-[11px] whitespace-pre-wrap break-all">
                      {lastResult.commands.join("\n")}
                    </div>
                  )}
                  {lastResult.next_step && (
                    <Button size="xs" variant="outline" onClick={() => runStep(lastResult.next_step as InstallStep)}>
                      {t("home.install.nextStep", { step: t(`home.install.step.${lastResult.next_step}`) })}
                    </Button>
                  )}
                </div>
              )}

              {autoBlocker && session.state !== "ready" && !autoRunning && (
                <div className="rounded border border-amber-500/40 bg-amber-500/5 p-2 text-xs space-y-2">
                  <div className="font-medium">{autoBlocker.message}</div>
                  <div className="text-muted-foreground">
                    {t("home.install.blocked.code", { code: autoBlocker.code })}
                  </div>
                  {autoBlocker.details && (
                    <div className="max-h-24 overflow-auto rounded border bg-background/70 p-2 text-muted-foreground whitespace-pre-wrap break-all">
                      {autoBlocker.details}
                    </div>
                  )}
                  <div className="flex flex-wrap items-center gap-2">
                    {autoBlocker.actions.map((action) => (
                      <span key={action}>{renderBlockerAction(action)}</span>
                    ))}
                  </div>
                </div>
              )}

              {session.state === "ready" && (
                <div className="rounded border border-emerald-500/30 bg-emerald-500/5 p-2 text-xs space-y-2">
                  <div className="font-medium">{t("home.install.ready")}</div>
                  <div className="flex items-center gap-2">
                    <Button size="xs" variant="outline" onClick={() => onNavigate?.("settings")}>
                      {t("home.install.goSettings")}
                    </Button>
                    <Button size="xs" onClick={() => onNavigate?.("channels")}>
                      {t("home.install.goChannels")}
                    </Button>
                  </div>
                </div>
              )}
            </div>
          )}
        </CardContent>
      </Card>
    </>
  );
}
