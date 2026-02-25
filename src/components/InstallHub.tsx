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
  InstallMethod,
  InstallMethodCapability,
  InstallSession,
  InstallStep,
  InstallStepResult,
  SshHost,
} from "@/lib/types";
import { useApi } from "@/lib/use-api";

const METHOD_ORDER: InstallMethod[] = ["local", "wsl2", "docker", "remote_ssh"];
const STEP_ORDER: InstallStep[] = ["precheck", "install", "init", "verify"];

type StepStatus = "pending" | "running" | "success" | "failed";

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
    if (state === "ready") return "success";
    return "pending";
  }
  if (step === "verify") {
    if (state === "ready") return "success";
    return "pending";
  }
  return "pending";
}

export function InstallHub({
  showToast,
  onNavigate,
  onReady,
}: {
  showToast?: (message: string, type?: "success" | "error") => void;
  onNavigate?: (route: string) => void;
  onReady?: () => void;
}) {
  const { t } = useTranslation();
  const ua = useApi();
  const [methods, setMethods] = useState<InstallMethodCapability[]>([]);
  const [loadingMethods, setLoadingMethods] = useState(true);
  const [selectedMethod, setSelectedMethod] = useState<InstallMethod>("local");
  const [creating, setCreating] = useState(false);
  const [runningStep, setRunningStep] = useState<InstallStep | null>(null);
  const [session, setSession] = useState<InstallSession | null>(null);
  const [lastResult, setLastResult] = useState<InstallStepResult | null>(null);
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

  const handleCreateSession = () => {
    if (selectedMethod === "remote_ssh" && !selectedSshHostId) {
      showToast?.(t("home.install.remoteHostRequired"), "error");
      return;
    }
    setCreating(true);
    setLastResult(null);
    const options = selectedMethod === "remote_ssh"
      ? { ssh_host_id: selectedSshHostId }
      : undefined;
    ua.installCreateSession(selectedMethod, options)
      .then((next) => {
        setSession(next);
        showToast?.(t("home.install.sessionCreated"), "success");
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
    setRunningStep(step);
    ua.installRunStep(session.id, step)
      .then((result) => {
        setLastResult(result);
        if (!result.ok) {
          showToast?.(result.summary, "error");
          return;
        }
        showToast?.(result.summary, "success");
        return refreshSession(session.id).then((next) => {
          if (next.state === "ready") {
            onReady?.();
          }
        });
      })
      .catch((e) => showToast?.(String(e), "error"))
      .finally(() => setRunningStep(null));
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
              disabled={loadingMethods || creating || runningStep !== null}
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
            <Button size="sm" disabled={creating || loadingMethods || runningStep !== null} onClick={handleCreateSession}>
              {creating ? t("home.install.creating") : t("home.install.start")}
            </Button>
          </div>
          {selectedMeta?.hint && (
            <p className="text-xs text-muted-foreground">{selectedMeta.hint}</p>
          )}
          {selectedMethod === "remote_ssh" && (
            <div className="flex items-center gap-2">
              <Select
                value={selectedSshHostId}
                onValueChange={setSelectedSshHostId}
                disabled={creating || runningStep !== null || sshHosts.length === 0}
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
              </div>

              <div className="space-y-2">
                {STEP_ORDER.map((step) => {
                  const status = getStepStatus(session.state, step);
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
                        disabled={runningStep !== null}
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
                  <div className="text-muted-foreground">{lastResult.details}</div>
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

              {session.state === "ready" && (
                <div className="rounded border border-emerald-500/30 bg-emerald-500/5 p-2 text-xs space-y-2">
                  <div className="font-medium">{t("home.install.ready")}</div>
                  <div className="flex items-center gap-2">
                    <Button size="xs" variant="outline" onClick={() => onNavigate?.("doctor")}>
                      {t("home.install.goDoctor")}
                    </Button>
                    <Button size="xs" onClick={() => onNavigate?.("recipes")}>
                      {t("home.install.goRecipes")}
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
