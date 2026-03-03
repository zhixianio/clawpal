import { useCallback, useEffect, useMemo, useRef, useState } from "react";
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
import { CreateAgentDialog } from "@/components/CreateAgentDialog";
import { UpgradeDialog } from "@/components/UpgradeDialog";
import { Skeleton } from "@/components/ui/skeleton";
import type { InstanceStatus, StatusExtra, AgentOverview, ModelProfile } from "../lib/types";
import { useApi, hasGuidanceEmitted } from "@/lib/use-api";
import { profileToModelValue } from "@/lib/model-value";

type OpenclawUpdateLatch = {
  checkedAt: number;
  available: boolean;
  latest?: string;
  installedVersion?: string;
};

const OPENCLAW_UPDATE_LATCH = new Map<string, OpenclawUpdateLatch>();
const OPENCLAW_UPDATE_NO_UPDATE_TTL_MS = 30 * 60 * 1000;

interface AgentGroup {
  identity: string;
  emoji?: string;
  agents: AgentOverview[];
}

function groupAgents(agents: AgentOverview[]): AgentGroup[] {
  const map = new Map<string, AgentGroup>();
  for (const a of agents) {
    // Group by workspace path (shared identity), fallback to agent id
    const key = a.workspace || a.id;
    if (!map.has(key)) {
      map.set(key, {
        identity: a.name || a.id,
        emoji: a.emoji,
        agents: [],
      });
    }
    map.get(key)!.agents.push(a);
  }
  return Array.from(map.values());
}

export function Home({
  instanceLabel,
  showToast,
  onNavigate,
}: {
  instanceLabel?: string;
  showToast?: (message: string, type?: "success" | "error") => void;
  onNavigate?: (route: string) => void;
}) {
  const { t } = useTranslation();
  const ua = useApi();
  const [status, setStatus] = useState<InstanceStatus | null>(null);
  const [statusExtra, setStatusExtra] = useState<StatusExtra | null>(null);
  const [version, setVersion] = useState<string | null>(null);
  const [updateInfo, setUpdateInfo] = useState<{ available: boolean; latest?: string } | null>(null);
  const [checkingUpdate, setCheckingUpdate] = useState(false);
  const [agents, setAgents] = useState<AgentOverview[] | null>(null);
  const [modelProfiles, setModelProfiles] = useState<ModelProfile[]>([]);
  const [savingModel, setSavingModel] = useState(false);
  const [fallbackSelectKey, setFallbackSelectKey] = useState(0);

  // Create agent dialog
  const [showCreateAgent, setShowCreateAgent] = useState(false);
  const [showUpgradeDialog, setShowUpgradeDialog] = useState(false);

  const resolveModelValue = (profileId: string | null): string | null => {
    if (!profileId) return null;
    const profile = modelProfiles.find((p) => p.id === profileId);
    if (!profile) return profileId;
    return profileToModelValue(profile);
  };

  // Skip polling refreshes while there are queued commands (to preserve optimistic UI)
  const hasPendingRef = useRef(false);
  // Timestamp until which polls should not overwrite optimistic component state.
  // This closes the race window between queueCommand() and the next queuedCommandsCount() poll.
  const optimisticLockedUntilRef = useRef(0);

  /** Mark state as optimistically locked for the given duration. */
  const lockOptimistic = useCallback((durationMs = 15_000) => {
    optimisticLockedUntilRef.current = Date.now() + durationMs;
    hasPendingRef.current = true;
  }, []);

  useEffect(() => {
    const check = () => { ua.queuedCommandsCount().then((n) => {
      // Don't clear the flag if we're within the optimistic lock window
      if (optimisticLockedUntilRef.current > Date.now()) return;
      hasPendingRef.current = n > 0;
    }).catch(() => {}); };
    check();
    const interval = setInterval(check, ua.isRemote ? 10000 : 3000);
    return () => clearInterval(interval);
  }, [ua]);

  // Health status with grace period: retry quickly when unhealthy, then slow-poll
  const [statusSettled, setStatusSettled] = useState(false);
  const retriesRef = useRef(0);
  const remoteErrorShownRef = useRef(false);
  const remoteUnhealthyStreakRef = useRef(0);
  const duplicateInstallGuidanceSigRef = useRef<string>("");
  const onboardingGuidanceSigRef = useRef<string>("");

  const statusInFlightRef = useRef(false);

  // On instance switch, immediately clear stale state so UI shows loading
  // placeholders instead of previous instance data.
  useEffect(() => {
    setStatus(null);
    setStatusExtra(null);
    setVersion(null);
    setUpdateInfo(null);
    setCheckingUpdate(false);
    setAgents(null);
    setModelProfiles([]);
    setStatusSettled(false);
    retriesRef.current = 0;
    remoteErrorShownRef.current = false;
    remoteUnhealthyStreakRef.current = 0;
    statusInFlightRef.current = false;
    duplicateInstallGuidanceSigRef.current = "";
    onboardingGuidanceSigRef.current = "";
  }, [ua.instanceToken]);

  useEffect(() => {
    const entries = statusExtra?.duplicateInstalls || [];
    if (entries.length === 0) return;
    const signature = `${ua.instanceId}:${entries.join("|")}`;
    if (duplicateInstallGuidanceSigRef.current === signature) return;
    duplicateInstallGuidanceSigRef.current = signature;
    const transport = ua.isRemote ? "remote_ssh" : (ua.isDocker ? "docker_local" : "local");
    const rawError = `Duplicate openclaw installs detected: ${entries.join(" ; ")}`;
    window.dispatchEvent(new CustomEvent("clawpal:agent-guidance", {
      detail: {
        message: t("home.duplicateInstalls"),
        summary: t("home.duplicateInstalls"),
        actions: [
          t("home.fixInDoctor"),
          "Run `which -a openclaw` and keep only one valid binary in PATH",
        ],
        source: "status-extra",
        operation: "status.extra.duplicate_installs",
        instanceId: ua.instanceId,
        transport,
        rawError,
        createdAt: Date.now(),
      },
    }));
  }, [statusExtra?.duplicateInstalls, t, ua.instanceId, ua.isDocker, ua.isRemote]);

  // Post-install onboarding guidance: when status settles and instance needs setup,
  // emit guidance so Doctor Claw can walk the user through remaining configuration.
  useEffect(() => {
    if (!statusSettled || !status) return;
    const remote = ua.isRemote;
    // Model profiles/default model are global host-level concerns, not remote-instance-local setup.
    const needsSetup = !status.healthy || (!remote && (modelProfiles.length === 0 || !status.globalDefaultModel));
    if (!needsSetup) return;
    const issues: string[] = [];
    if (!status.healthy) issues.push("unhealthy");
    if (!remote && modelProfiles.length === 0) issues.push("no_profiles");
    if (!remote && !status.globalDefaultModel) issues.push("no_default_model");
    const signature = `${ua.instanceId}:onboarding:${issues.join(",")}`;
    if (onboardingGuidanceSigRef.current === signature) return;
    onboardingGuidanceSigRef.current = signature;
    const transport = ua.isRemote ? "remote_ssh" : (ua.isDocker ? "docker_local" : "local");
    const actions: string[] = [];
    if (!status.healthy) actions.push(t("onboarding.actionCheckDoctor"));
    if (!remote && modelProfiles.length === 0) actions.push(t("onboarding.actionAddProfile"));
    if (!remote && !status.globalDefaultModel && modelProfiles.length > 0) actions.push(t("onboarding.actionSetDefault"));
    window.dispatchEvent(new CustomEvent("clawpal:agent-guidance", {
      detail: {
        message: t("onboarding.summary"),
        summary: t("onboarding.summary"),
        actions,
        source: "onboarding",
        operation: "post_install.onboarding",
        instanceId: ua.instanceId,
        transport,
        rawError: `Instance needs setup: ${issues.join(", ")}`,
        createdAt: Date.now(),
      },
    }));
  }, [statusSettled, status, modelProfiles, t, ua.instanceId, ua.isDocker, ua.isRemote]);

  const fetchStatus = useCallback(() => {
    if (ua.isRemote && !ua.isConnected) return; // Wait for SSH connection
    if (hasPendingRef.current || optimisticLockedUntilRef.current > Date.now()) return; // Don't overwrite optimistic UI
    if (statusInFlightRef.current) return; // Prevent overlapping polls
    statusInFlightRef.current = true;
    ua.getInstanceStatus().then((s) => {
      let resolvedHealthy = s.healthy;
      if (ua.isRemote) {
        if (s.healthy) {
          remoteUnhealthyStreakRef.current = 0;
        } else {
          remoteUnhealthyStreakRef.current += 1;
          if (remoteUnhealthyStreakRef.current < 2) {
            resolvedHealthy = true;
          }
        }
      }
      const next = { ...s, healthy: resolvedHealthy };
      // If remote config fetch failed (agents=0, no model), keep previous good data
      // rather than flashing "unset" — only update health which is independent.
      if (ua.isRemote && s.activeAgents === 0 && !s.globalDefaultModel) {
        setStatus((prev) => prev ? { ...prev, healthy: resolvedHealthy } : next);
      } else {
        setStatus(next);
      }
      if (ua.isRemote) {
        setStatusSettled(true);
        remoteErrorShownRef.current = false;
      } else {
        if (s.healthy) {
          setStatusSettled(true);
          retriesRef.current = 0;
        } else if (retriesRef.current < 5) {
          retriesRef.current++;
        } else {
          setStatusSettled(true);
        }
      }
    }).catch((e) => {
      if (ua.isRemote) {
        console.error("Failed to fetch remote status:", e);
        if (!remoteErrorShownRef.current) {
          remoteErrorShownRef.current = true;
          showToast?.(t('home.remoteReadFailed', { error: String(e) }), "error");
        }
      } else {
        console.error("Failed to fetch status:", e);
      }
    }).finally(() => {
      statusInFlightRef.current = false;
    });
  }, [ua, showToast, t]);

  useEffect(() => {
    remoteErrorShownRef.current = false;
    remoteUnhealthyStreakRef.current = 0;
    const initial = setTimeout(fetchStatus, 250);
    // Poll fast (2s) while not settled, slow (10s) once settled; remote always slow
    const interval = setInterval(fetchStatus, ua.isRemote ? 30000 : (statusSettled ? 10000 : 2000));
    return () => {
      clearTimeout(initial);
      clearInterval(interval);
    };
  }, [fetchStatus, statusSettled, ua.isRemote]);

  // Tier 2: version + duplicate detection — called once on mount (not polled)
  const fetchStatusExtra = useCallback(() => {
    if (ua.isRemote && !ua.isConnected) return;
    ua.getStatusExtra().then((extra) => {
      setStatusExtra(extra);
      if (extra.openclawVersion) setVersion(extra.openclawVersion);
    }).catch((e) => {
      console.error("Failed to fetch status extra:", e);
    });
  }, [ua]);

  useEffect(() => {
    // Delay for remote to avoid SSH burst (tier 1 + tier 2 = 4 concurrent SSH
    // processes on Windows which has no ControlMaster multiplexing).
    if (ua.isRemote) {
      const timer = setTimeout(fetchStatusExtra, 3000);
      return () => clearTimeout(timer);
    }
    const timer = setTimeout(fetchStatusExtra, 350);
    return () => clearTimeout(timer);
  }, [fetchStatusExtra, ua.isRemote]);

  const refreshAgents = useCallback(() => {
    if (ua.isRemote && !ua.isConnected) return; // Wait for SSH connection
    if (hasPendingRef.current || optimisticLockedUntilRef.current > Date.now()) return; // Don't overwrite optimistic UI
    ua.listAgents().then((a) => {
      setAgents(a);
      if (ua.isRemote) remoteErrorShownRef.current = false;
    }).catch((e) => {
      if (ua.isRemote) {
        // SSH sessions can be transiently unavailable during tab switch;
        // retry once after a short delay before surfacing the error.
        setTimeout(() => {
          ua.listAgents().then((a) => {
            setAgents(a);
            remoteErrorShownRef.current = false;
          }).catch((e2) => {
            console.error("Failed to load remote agents:", e2);
            if (!remoteErrorShownRef.current) {
              remoteErrorShownRef.current = true;
            }
          });
        }, 1500);
      } else {
        console.error("Failed to load agents:", e);
      }
    });
  }, [ua, showToast, t]);

  useEffect(() => {
    const initial = setTimeout(refreshAgents, 300);
    // Auto-refresh agents (remote less frequently to avoid ssh process spam)
    const interval = setInterval(refreshAgents, ua.isRemote ? 30000 : 15000);
    return () => {
      clearTimeout(initial);
      clearInterval(interval);
    };
  }, [refreshAgents, ua.isRemote]);

  useEffect(() => {
    if (ua.isRemote && !ua.isConnected) return;
    const timer = setTimeout(() => {
      ua.listModelProfiles().then((p) => setModelProfiles(p.filter((m) => m.enabled))).catch((e) => console.error("Failed to load model profiles:", e));
    }, 350);
    return () => clearTimeout(timer);
  }, [ua]);

  // Match current global model value to a profile ID
  const currentModelProfileId = useMemo(() => {
    const modelVal = status?.globalDefaultModel;
    if (!modelVal) return null;
    const normalized = modelVal.toLowerCase();
    for (const p of modelProfiles) {
      const profileVal = profileToModelValue(p);
      if (profileVal.toLowerCase() === normalized || p.model.toLowerCase() === normalized) {
        return p.id;
      }
    }
    return null;
  }, [status?.globalDefaultModel, modelProfiles]);

  const agentGroups = useMemo(() => groupAgents(agents || []), [agents]);

  // Update check — deferred, runs once (not in poll loop)
  useEffect(() => {
    const instanceKey = `${ua.instanceId}#${ua.instanceToken}`;
    const latched = OPENCLAW_UPDATE_LATCH.get(instanceKey);
    const now = Date.now();
    if (latched?.available) {
      setUpdateInfo({ available: true, latest: latched.latest });
      if (latched.installedVersion) setVersion((prev) => prev || latched.installedVersion || null);
      setCheckingUpdate(false);
      return;
    }
    if (latched && now - latched.checkedAt < OPENCLAW_UPDATE_NO_UPDATE_TTL_MS) {
      setUpdateInfo({ available: false, latest: latched.latest });
      if (latched.installedVersion) setVersion((prev) => prev || latched.installedVersion || null);
      setCheckingUpdate(false);
      return;
    }

    setCheckingUpdate(true);
    setUpdateInfo(null);
    const timer = setTimeout(() => {
      if (ua.isRemote && !ua.isConnected) { setCheckingUpdate(false); return; }
      ua.checkOpenclawUpdate()
        .then((u) => {
          const next = {
            checkedAt: Date.now(),
            available: u.upgradeAvailable,
            latest: u.latestVersion ?? undefined,
            installedVersion: u.installedVersion,
          };
          OPENCLAW_UPDATE_LATCH.set(instanceKey, next);
          setUpdateInfo({ available: next.available, latest: next.latest });
          // Fallback: set version from update check if tier 2 hasn't provided it yet
          if (u.installedVersion) setVersion((prev) => prev || u.installedVersion);
        })
        .catch((e) => console.error("Failed to check update:", e))
        .finally(() => setCheckingUpdate(false));
    }, 2000); // Defer to avoid blocking startup with heavy CLI calls
    return () => clearTimeout(timer);
  }, [ua]);

  const handleDeleteAgent = (agentId: string) => {
    if (ua.isRemote && !ua.isConnected) return;
    lockOptimistic();
    ua.queueCommand(
      `Delete agent: ${agentId}`,
      ["openclaw", "agents", "delete", agentId, "--force"],
    ).then(() => {
      // Optimistic UI update + pin in cache so polling doesn't overwrite
      const updated = agents?.filter((a) => a.id !== agentId) ?? null;
      setAgents(updated);
      if (updated) ua.pinOptimistic("listAgents", updated);
    }).catch((e) => { if (!hasGuidanceEmitted(e)) showToast?.(String(e), "error"); });
  };

  return (
    <div>
      <div className="flex items-center gap-2 mb-1">
        <h2 className="text-2xl font-bold">{instanceLabel || t('home.title')}</h2>
      </div>

      {/* Status Summary */}
      <h3 className="text-lg font-semibold mt-8 mb-4">{t('home.status')}</h3>
      <Card>
        <CardContent className="grid grid-cols-[auto_1fr] gap-x-8 gap-y-4 items-center">
          <span className="text-sm text-muted-foreground font-medium">{t('home.health')}</span>
          <span className="text-sm font-medium">
            {!status ? (
              <span className="inline-flex items-center gap-1.5 text-muted-foreground">
                <span className="w-2 h-2 rounded-full bg-muted-foreground/30 animate-pulse" />
                ...
              </span>
            ) : status.healthy ? (
              <Badge className="bg-emerald-500/10 text-emerald-600 dark:bg-emerald-500/15 dark:text-emerald-400">{t('home.healthy')}</Badge>
            ) : !statusSettled ? (
              <Badge className="bg-amber-500/10 text-amber-600 dark:bg-amber-500/15 dark:text-amber-400">{t('home.checking')}</Badge>
            ) : (
              <Badge className="bg-red-500/10 text-red-600 dark:bg-red-500/15 dark:text-red-400">{t('home.unhealthy')}</Badge>
            )}
          </span>

          <span className="text-sm text-muted-foreground font-medium">{t('home.version')}</span>
          <div className="flex items-center gap-2.5 flex-wrap">
            <span className="text-sm font-semibold font-mono">{version || "..."}</span>
            {checkingUpdate && (
              <Badge variant="outline" className="text-muted-foreground">{t('home.checkingUpdates')}</Badge>
            )}
            {!checkingUpdate && updateInfo?.available && updateInfo.latest && updateInfo.latest !== version && (
              <>
                <Badge className="bg-primary/10 text-primary border border-primary/20">
                  {t('home.available', { version: updateInfo.latest })}
                </Badge>
                <Button
                  size="xs"
                  variant="outline"
                  onClick={() => ua.openUrl("https://github.com/openclaw/openclaw/releases")}
                >
                  {t('home.view')}
                </Button>
                <Button
                  size="xs"
                  onClick={() => setShowUpgradeDialog(true)}
                >
                  {t('home.upgrade')}
                </Button>
              </>
            )}
          </div>
          <span className="text-sm text-muted-foreground font-medium">{t('home.defaultModel')}</span>
          <div className="max-w-xs">
            {status ? (
              <Select
                value={currentModelProfileId || (status?.globalDefaultModel ? "__raw__" : "__none__")}
                onValueChange={(val) => {
                  if (val === "__raw__") return;
                  setSavingModel(true);
                  const modelValue = resolveModelValue(val === "__none__" ? null : val);
                  // Lock optimistic state immediately to prevent polls from overwriting
                  lockOptimistic();
                  const p = modelValue
                    ? ua.queueCommand(
                        `Set global model: ${modelValue}`,
                        ["openclaw", "config", "set", "agents.defaults.model.primary", modelValue],
                      )
                    : ua.queueCommand(
                        "Clear global model override",
                        ["openclaw", "config", "unset", "agents.defaults.model.primary"],
                      );
                  // Optimistic UI update — applied immediately, protected by lockOptimistic
                  setStatus((prev) => prev ? { ...prev, globalDefaultModel: modelValue ?? "" } : prev);
                  p.catch((e) => { if (!hasGuidanceEmitted(e)) showToast?.(String(e), "error"); })
                    .finally(() => setSavingModel(false));
                }}
                disabled={savingModel}
              >
                <SelectTrigger size="sm" className="text-sm">
                  <SelectValue />
                </SelectTrigger>
                <SelectContent>
                  <SelectItem value="__none__">
                    <span className="text-muted-foreground">{t('home.notSet')}</span>
                  </SelectItem>
                  {status?.globalDefaultModel && !currentModelProfileId && (
                    <SelectItem value="__raw__">
                      {status.globalDefaultModel}
                    </SelectItem>
                  )}
                  {modelProfiles.map((p) => (
                    <SelectItem key={p.id} value={p.id}>
                      {p.provider}/{p.model}
                    </SelectItem>
                  ))}
                </SelectContent>
              </Select>
            ) : (
              <span className="text-sm">...</span>
            )}
          </div>

          <span className="text-sm text-muted-foreground font-medium">{t('home.fallbackModels')}</span>
          <div className="max-w-xs">
            {status ? (
              <div className="space-y-1.5">
                {(status.fallbackModels ?? []).length === 0 ? (
                  <span className="text-xs text-muted-foreground">{t('home.noFallbacks')}</span>
                ) : (
                  <div className="space-y-1">
                    {(status.fallbackModels ?? []).map((fb, idx) => (
                      <div key={`${fb}-${idx}`} className="flex items-center gap-1">
                        <Badge variant="secondary" className="text-xs font-normal">
                          {fb}
                        </Badge>
                        <Button
                          size="xs"
                          variant="ghost"
                          className="h-5 w-5 p-0 text-muted-foreground hover:text-foreground"
                          disabled={idx === 0}
                          onClick={() => {
                            lockOptimistic();
                            const arr = [...(status.fallbackModels ?? [])];
                            [arr[idx - 1], arr[idx]] = [arr[idx], arr[idx - 1]];
                            setStatus((prev) => prev ? { ...prev, fallbackModels: arr } : prev);
                            ua.queueCommand(
                              `Reorder fallback models`,
                              ["openclaw", "config", "set", "agents.defaults.model.fallbacks", JSON.stringify(arr), "--json"],
                            ).catch((e) => { if (!hasGuidanceEmitted(e)) showToast?.(String(e), "error"); });
                          }}
                        >
                          ↑
                        </Button>
                        <Button
                          size="xs"
                          variant="ghost"
                          className="h-5 w-5 p-0 text-muted-foreground hover:text-foreground"
                          disabled={idx === (status.fallbackModels ?? []).length - 1}
                          onClick={() => {
                            lockOptimistic();
                            const arr = [...(status.fallbackModels ?? [])];
                            [arr[idx], arr[idx + 1]] = [arr[idx + 1], arr[idx]];
                            setStatus((prev) => prev ? { ...prev, fallbackModels: arr } : prev);
                            ua.queueCommand(
                              `Reorder fallback models`,
                              ["openclaw", "config", "set", "agents.defaults.model.fallbacks", JSON.stringify(arr), "--json"],
                            ).catch((e) => { if (!hasGuidanceEmitted(e)) showToast?.(String(e), "error"); });
                          }}
                        >
                          ↓
                        </Button>
                        <Button
                          size="xs"
                          variant="ghost"
                          className="h-5 w-5 p-0 text-muted-foreground hover:text-destructive"
                          onClick={() => {
                            lockOptimistic();
                            const arr = (status.fallbackModels ?? []).filter((_, i) => i !== idx);
                            setStatus((prev) => prev ? { ...prev, fallbackModels: arr } : prev);
                            const cmd = arr.length > 0
                              ? ua.queueCommand(
                                  `Remove fallback model: ${fb}`,
                                  ["openclaw", "config", "set", "agents.defaults.model.fallbacks", JSON.stringify(arr), "--json"],
                                )
                              : ua.queueCommand(
                                  `Remove last fallback model`,
                                  ["openclaw", "config", "unset", "agents.defaults.model.fallbacks"],
                                );
                            cmd.catch((e) => { if (!hasGuidanceEmitted(e)) showToast?.(String(e), "error"); });
                          }}
                        >
                          ✕
                        </Button>
                      </div>
                    ))}
                  </div>
                )}
                <Select
                  key={fallbackSelectKey}
                  onValueChange={(val) => {
                    if (!val) return;
                    const modelValue = resolveModelValue(val);
                    if (!modelValue) return;
                    lockOptimistic();
                    const arr = [...(status.fallbackModels ?? []), modelValue];
                    setStatus((prev) => prev ? { ...prev, fallbackModels: arr } : prev);
                    ua.queueCommand(
                      `Add fallback model: ${modelValue}`,
                      ["openclaw", "config", "set", "agents.defaults.model.fallbacks", JSON.stringify(arr), "--json"],
                    ).catch((e) => { if (!hasGuidanceEmitted(e)) showToast?.(String(e), "error"); });
                    setFallbackSelectKey((k) => k + 1);
                  }}
                >
                  <SelectTrigger size="sm" className="text-xs h-7 w-auto">
                    <SelectValue placeholder={t('home.addFallback')} />
                  </SelectTrigger>
                  <SelectContent>
                    {modelProfiles.map((p) => (
                      <SelectItem key={p.id} value={p.id}>
                        {p.provider}/{p.model}
                      </SelectItem>
                    ))}
                  </SelectContent>
                </Select>
              </div>
            ) : (
              <span className="text-sm">...</span>
            )}
          </div>
        </CardContent>
      </Card>

      {/* Agents Overview -- grouped by identity */}
      <div className="flex items-center justify-between mt-8 mb-4">
        <h3 className="text-lg font-semibold">{t('home.agents')}</h3>
        <Button size="sm" variant="outline" onClick={() => setShowCreateAgent(true)}>
          {t('home.newAgent')}
        </Button>
      </div>
      {agents === null ? (
        <div className="space-y-3">
          <Skeleton className="h-24 w-full" />
          <Skeleton className="h-24 w-full" />
        </div>
      ) : agentGroups.length === 0 ? (
        <p className="text-muted-foreground">{t('home.noAgents')}</p>
      ) : (
        <div className="space-y-3">
          {agentGroups.map((group) => (
            <Card key={group.agents[0].workspace || group.agents[0].id}>
              <CardContent>
                <div className="flex items-center gap-1.5 mb-2">
                  {group.emoji && <span>{group.emoji}</span>}
                  <strong className="text-base">{group.identity}</strong>
                </div>
                <div className="space-y-1.5">
                  {group.agents.map((agent) => (
                    <div
                      key={agent.id}
                      className="flex items-center justify-between rounded-md border px-3 py-1.5"
                    >
                      <div className="flex items-center gap-2.5">
                        <code className="text-sm text-foreground font-medium">{agent.id}</code>
                        <Select
                          value={(() => {
                            if (!agent.model) return "__none__";
                            const normalized = agent.model.toLowerCase();
                            for (const p of modelProfiles) {
                              const profileVal = profileToModelValue(p);
                              if (profileVal.toLowerCase() === normalized || p.model.toLowerCase() === normalized) {
                                return p.id;
                              }
                            }
                            return "__none__";
                          })()}
                          onValueChange={async (val) => {
                            const modelValue = resolveModelValue(val === "__none__" ? null : val);
                            lockOptimistic();
                            try {
                              // Find agent index in config list
                              const raw = await ua.readRawConfig();
                              const cfg = JSON.parse(raw);
                              const list: { id: string }[] = cfg?.agents?.list ?? [];
                              const idx = list.findIndex((a) => a.id === agent.id);
                              const label = modelValue
                                ? `Set model for ${agent.id}: ${modelValue}`
                                : `Clear model override for ${agent.id}`;
                              if (idx >= 0) {
                                if (modelValue) {
                                  await ua.queueCommand(label, ["openclaw", "config", "set", `agents.list.${idx}.model.primary`, JSON.stringify(modelValue), "--json"]);
                                } else {
                                  await ua.queueCommand(label, ["openclaw", "config", "unset", `agents.list.${idx}.model.primary`]);
                                }
                              } else if (modelValue) {
                                // Agent not in list yet — append
                                await ua.queueCommand(label, ["openclaw", "config", "set", `agents.list.${list.length}`, JSON.stringify({ id: agent.id, model: modelValue }), "--json"]);
                              }
                              // Optimistic UI update + pin in cache
                              const updated = agents?.map((a) =>
                                a.id === agent.id ? { ...a, model: modelValue ?? null } : a
                              ) ?? null;
                              setAgents(updated);
                              if (updated) ua.pinOptimistic("listAgents", updated);
                            } catch (e) {
                              if (!hasGuidanceEmitted(e)) showToast?.(String(e), "error");
                            }
                          }}
                        >
                          <SelectTrigger size="sm" className="text-xs h-6 w-auto min-w-[120px] max-w-[200px]">
                            <SelectValue />
                          </SelectTrigger>
                          <SelectContent>
                            <SelectItem value="__none__">
                              <span className="text-muted-foreground">{t('home.defaultModelOption')}</span>
                            </SelectItem>
                            {modelProfiles.map((p) => (
                              <SelectItem key={p.id} value={p.id}>
                                {p.provider}/{p.model}
                              </SelectItem>
                            ))}
                          </SelectContent>
                        </Select>
                      </div>
                      <div className="flex items-center gap-2">
                        {agent.online ? (
                          <Badge className="bg-emerald-500/10 text-emerald-600 dark:bg-emerald-500/15 dark:text-emerald-400 text-xs">{t('home.active')}</Badge>
                        ) : (
                          <Badge className="bg-muted text-muted-foreground border border-border text-xs">{t('home.idle')}</Badge>
                        )}
                        {agent.id !== "main" && (
                          <AlertDialog>
                            <AlertDialogTrigger asChild>
                              <Button size="sm" variant="ghost" className="h-6 px-1.5 text-xs text-muted-foreground hover:text-destructive">
                                {t('home.delete')}
                              </Button>
                            </AlertDialogTrigger>
                            <AlertDialogContent>
                              <AlertDialogHeader>
                                <AlertDialogTitle>{t('home.deleteAgentTitle', { agentId: agent.id })}</AlertDialogTitle>
                                <AlertDialogDescription>
                                  {t('home.deleteAgentDescription')}
                                </AlertDialogDescription>
                              </AlertDialogHeader>
                              <AlertDialogFooter>
                                <AlertDialogCancel>{t('config.cancel')}</AlertDialogCancel>
                                <AlertDialogAction
                                  className="bg-destructive text-destructive-foreground hover:bg-destructive/90"
                                  onClick={() => handleDeleteAgent(agent.id)}
                                >
                                  {t('home.delete')}
                                </AlertDialogAction>
                              </AlertDialogFooter>
                            </AlertDialogContent>
                          </AlertDialog>
                        )}
                      </div>
                    </div>
                  ))}
                </div>
              </CardContent>
            </Card>
          ))}
        </div>
      )}

      {/* Create Agent Dialog */}
      <CreateAgentDialog
        open={showCreateAgent}
        onOpenChange={setShowCreateAgent}
        modelProfiles={modelProfiles}
        onCreated={() => refreshAgents()}
      />

      {/* Upgrade Dialog */}
      <UpgradeDialog
        open={showUpgradeDialog}
        onOpenChange={(open) => {
          setShowUpgradeDialog(open);
          if (!open) {
            // Refresh version + update status after closing upgrade dialog
            fetchStatus();
            fetchStatusExtra();
            ua.checkOpenclawUpdate()
              .then((u) => setUpdateInfo({ available: u.upgradeAvailable, latest: u.latestVersion ?? undefined }))
              .catch(() => {});
          }
        }}
        isRemote={ua.isRemote}
        instanceId={ua.instanceId}
        currentVersion={version || ""}
        latestVersion={updateInfo?.latest || ""}
      />
    </div>
  );
}
