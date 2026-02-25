import { useCallback, useEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { useApi } from "@/lib/use-api";
import { useInstance } from "@/lib/instance-context";
import { useDoctorAgent } from "@/lib/use-doctor-agent";
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
import { SessionAnalysisPanel } from "@/components/SessionAnalysisPanel";
import type { BackupInfo } from "@/lib/types";
import { formatTime, formatBytes } from "@/lib/utils";

export function Doctor() {
  const { t } = useTranslation();
  const ua = useApi();
  const { instanceId, isDocker, isRemote } = useInstance();
  const doctor = useDoctorAgent();

  const [diagnosing, setDiagnosing] = useState(false);
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

  // Keep execution target synced with current instance tab:
  // - local/docker: execute on local machine
  // - remote ssh: execute on selected remote host
  useEffect(() => {
    doctor.setTarget(isRemote ? instanceId : "local");
  }, [doctor.setTarget, instanceId, isRemote]);

  const handleStartDiagnosis = async () => {
    setStartError(null);
    setDiagnosing(true);
    try {
      const diagnosisScope = isRemote
        ? instanceId
        : isDocker
          ? instanceId
          : "local";
      const executionTarget = isRemote ? instanceId : "local";
      doctor.setTarget(executionTarget);

      if (isRemote) {
        const status = await ua.sshStatus(instanceId);
        if (status !== "connected") {
          await ua.sshConnect(instanceId);
        }
      }

      await doctor.connect();
      const context = isRemote
        ? await ua.collectDoctorContextRemote(instanceId)
        : await ua.collectDoctorContext();
      const diagnosisTransport: "local" | "docker_local" | "remote_ssh" = isRemote
        ? "remote_ssh"
        : isDocker
          ? "docker_local"
          : "local";
      await doctor.startDiagnosis(context, "main", diagnosisScope, diagnosisTransport);
    } catch (err) {
      const msg = String(err);
      setStartError(msg);
    } finally {
      setDiagnosing(false);
    }
  };

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
    fn(500)
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

  useEffect(() => {
    if (logsOpen) fetchLog(logsSource, logsTab);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [logsOpen, logsSource, logsTab]);

  // Backups
  const refreshBackups = useCallback(() => {
    ua.listBackups().then(setBackups).catch((e) => console.error("Failed to load backups:", e));
  }, [ua]);
  useEffect(refreshBackups, [refreshBackups]);

  return (
    <section>
      <h2 className="text-2xl font-bold mb-4">{t("doctor.title")}</h2>

      <Card className="gap-2 py-4">
        <CardHeader className="pb-0">
          <div className="flex items-center justify-between">
            <CardTitle className="text-base">{t("doctor.agentSource")}</CardTitle>
            <div className="flex items-center gap-1">
              <Button variant="ghost" size="sm" onClick={() => openLogs("clawpal")}>
                {t("doctor.clawpalLogs")}
              </Button>
              <Button variant="ghost" size="sm" onClick={() => openLogs("gateway")}>
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
              <Button onClick={handleStartDiagnosis} disabled={diagnosing}>
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
              <div className="flex items-center justify-between mb-3">
                <div className="flex items-center gap-2">
                  <Badge variant="outline" className="text-xs">
                    {t("doctor.engineZeroclaw")}
                  </Badge>
                  <Badge variant="outline" className="text-xs flex items-center gap-1.5">
                    <span className={`inline-block w-1.5 h-1.5 rounded-full ${doctor.bridgeConnected ? "bg-emerald-500" : "bg-muted-foreground/40"}`} />
                    {doctor.bridgeConnected ? t("doctor.bridgeConnected") : t("doctor.bridgeDisconnected")}
                  </Badge>
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
                .catch((e) => setBackupMessage(t("home.backupFailed", { error: String(e) })))
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
                                .catch((e) => setBackupMessage(t("home.restoreFailed", { error: String(e) })));
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
                                .catch((e) => setBackupMessage(t("home.deleteBackupFailed", { error: String(e) })));
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
