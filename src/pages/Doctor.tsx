import { useEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { api } from "@/lib/api";
import { useApi } from "@/lib/use-api";
import { useInstance } from "@/lib/instance-context";
import { useDoctorAgent } from "@/lib/use-doctor-agent";
import type { SshHost } from "@/lib/types";
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
import { DoctorChat } from "@/components/DoctorChat";

interface DoctorProps {
  sshHosts: SshHost[];
}

export function Doctor({ sshHosts }: DoctorProps) {
  const { t } = useTranslation();
  const ua = useApi();
  const { instanceId, isRemote } = useInstance();
  const doctor = useDoctorAgent();

  // Agent source: an instance id ("local" / host uuid) or "remote" (hosted doctor)
  const [agentSource, setAgentSource] = useState("remote");
  const [diagnosing, setDiagnosing] = useState(false);

  // Logs state
  const [logsOpen, setLogsOpen] = useState(false);
  const [logsSource, setLogsSource] = useState<"clawpal" | "gateway">("clawpal");
  const [logsTab, setLogsTab] = useState<"app" | "error">("app");
  const [logsContent, setLogsContent] = useState("");
  const [logsLoading, setLogsLoading] = useState(false);
  const logsContentRef = useRef<HTMLPreElement>(null);

  // Reset doctor agent when switching instances
  useEffect(() => {
    doctor.reset();
    doctor.disconnect();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [instanceId]);

  // Auto-infer target from active instance tab
  useEffect(() => {
    if (isRemote) {
      doctor.setTarget(instanceId);
    } else {
      doctor.setTarget("local");
    }
  }, [instanceId, isRemote, doctor.setTarget]);

  const handleStartDiagnosis = async () => {
    setDiagnosing(true);
    try {
      let url: string;
      let credentials;
      let agentId = "main";
      if (agentSource === "remote") {
        url = "wss://doctor.openclaw.ai";
      } else if (agentSource === "local") {
        url = "ws://localhost:18789";
      } else {
        // Remote gateway: ensure SSH connected, read credentials, tunnel
        const status = await api.sshStatus(agentSource);
        if (status !== "connected") {
          await api.sshConnect(agentSource);
        }
        credentials = await api.doctorReadRemoteCredentials(agentSource);
        // Get the first agent ID from the remote gateway
        const agents = await api.remoteListAgentsOverview(agentSource);
        if (agents.length > 0) {
          agentId = agents[0].id;
        }
        const localPort = await api.doctorPortForward(agentSource);
        url = `ws://localhost:${localPort}`;
      }

      const isRemoteGateway = agentSource !== "local" && agentSource !== "remote";
      try {
        await doctor.connect(url, credentials, isRemoteGateway ? agentSource : undefined);
      } catch (connectErr) {
        // Auto-fix NOT_PAIRED: approve pending device requests via SSH and retry
        if (String(connectErr).includes("NOT_PAIRED") && isRemoteGateway) {
          const approved = await api.doctorAutoPair(agentSource);
          if (approved > 0) {
            await doctor.connect(url, credentials, agentSource);
          } else {
            throw connectErr;
          }
        } else {
          throw connectErr;
        }
      }

      const context = doctor.target === "local"
        ? await ua.collectDoctorContext()
        : await ua.collectDoctorContextRemote(doctor.target);

      await doctor.startDiagnosis(context, agentId);
    } catch {
      // Error is surfaced via doctor.error state from the hook
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

  return (
    <section>
      <h2 className="text-2xl font-bold mb-4">{t("doctor.title")}</h2>

      <Card>
        <CardHeader>
          <div className="flex items-center justify-between">
            <CardTitle>{t("doctor.agentSource")}</CardTitle>
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
              {/* Source radio — instance gateways (excluding current target) + remote doctor */}
              <div className="text-sm text-muted-foreground mb-2">{t("doctor.agentSourceHint")}</div>
              <div className="flex items-center gap-4 mb-4 flex-wrap">
                {doctor.target !== "local" && (
                  <label className="flex items-center gap-1.5 text-sm cursor-pointer">
                    <input
                      type="radio"
                      name="agentSource"
                      value="local"
                      checked={agentSource === "local"}
                      onChange={() => setAgentSource("local")}
                      className="accent-primary"
                    />
                    {t("instance.local")}
                  </label>
                )}
                {sshHosts
                  .filter((h) => h.id !== doctor.target)
                  .map((h) => (
                    <label key={h.id} className="flex items-center gap-1.5 text-sm cursor-pointer">
                      <input
                        type="radio"
                        name="agentSource"
                        value={h.id}
                        checked={agentSource === h.id}
                        onChange={() => setAgentSource(h.id)}
                        className="accent-primary"
                      />
                      {h.label || h.host}
                    </label>
                  ))}
                <label className="flex items-center gap-1.5 text-sm cursor-not-allowed text-muted-foreground">
                  <input
                    type="radio"
                    name="agentSource"
                    value="remote"
                    disabled
                    className="accent-primary"
                  />
                  {t("doctor.remoteDoctor")}
                  <span className="text-xs">(coming soon)</span>
                </label>
              </div>
              {doctor.error && (
                <div className="mb-3 text-sm text-destructive">
                  {doctor.error}
                  {doctor.error.includes("NOT_PAIRED") && (
                    <p className="mt-1 text-muted-foreground">
                      {t("doctor.notPairedHint", {
                        host: agentSource === "local"
                          ? "localhost"
                          : sshHosts.find((h) => h.id === agentSource)?.label || agentSource,
                      })}
                    </p>
                  )}
                </div>
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
                    {agentSource === "remote"
                      ? t("doctor.remoteDoctor")
                      : agentSource === "local"
                        ? t("instance.local")
                        : sshHosts.find((h) => h.id === agentSource)?.label || agentSource}
                  </Badge>
                  <Badge variant="outline" className="text-xs">
                    {doctor.bridgeConnected ? t("doctor.bridgeConnected") : t("doctor.bridgeDisconnected")}
                  </Badge>
                </div>
                <Button variant="outline" size="sm" onClick={handleStopDiagnosis}>
                  {t("doctor.stopDiagnosis")}
                </Button>
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
    </section>
  );
}
