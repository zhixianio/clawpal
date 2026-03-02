import { Suspense, lazy, startTransition, useCallback, useEffect, useMemo, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { check } from "@tauri-apps/plugin-updater";
import { getVersion } from "@tauri-apps/api/app";
import {
  HomeIcon,
  HashIcon,
  ClockIcon,
  HistoryIcon,
  StethoscopeIcon,
  BookOpenIcon,
  KeyRoundIcon,
  SettingsIcon,
  MessageCircleIcon,
  XIcon,
} from "lucide-react";
import { StartPage } from "./pages/StartPage";
import logoUrl from "./assets/logo.png";
import { InstanceTabBar } from "./components/InstanceTabBar";
import { InstanceContext } from "./lib/instance-context";
import { api } from "./lib/api";
import { invalidateGlobalReadCache } from "./lib/use-api";
import { explainAndBuildGuidanceError, withGuidance } from "./lib/guidance";
import { useFont } from "./lib/use-font";
import { Button } from "@/components/ui/button";
import { Dialog, DialogContent, DialogFooter, DialogHeader, DialogTitle } from "@/components/ui/dialog";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { cn } from "@/lib/utils";
import { Toaster } from "sonner";
import type { ChannelNode, DiscordGuildChannel, DiscoveredInstance, DockerInstance, GuidanceAction, InstallSession, PrecheckIssue, RegisteredInstance, SshHost } from "./lib/types";
import { GuidanceCard } from "./components/GuidanceCard";
import { SshFormWidget } from "./components/SshFormWidget";
import type { AgentGuidanceItem } from "./components/GuidanceCard";
import {
  SSH_PASSPHRASE_RETRY_HINT,
  buildSshPassphraseCancelMessage,
  buildSshPassphraseConnectErrorMessage,
} from "@/lib/sshConnectErrors";

const Home = lazy(() => import("./pages/Home").then((m) => ({ default: m.Home })));
const Recipes = lazy(() => import("./pages/Recipes").then((m) => ({ default: m.Recipes })));
const Cook = lazy(() => import("./pages/Cook").then((m) => ({ default: m.Cook })));
const History = lazy(() => import("./pages/History").then((m) => ({ default: m.History })));
const Settings = lazy(() => import("./pages/Settings").then((m) => ({ default: m.Settings })));
const Doctor = lazy(() => import("./pages/Doctor").then((m) => ({ default: m.Doctor })));
const Channels = lazy(() => import("./pages/Channels").then((m) => ({ default: m.Channels })));
const Cron = lazy(() => import("./pages/Cron").then((m) => ({ default: m.Cron })));
const Orchestrator = lazy(() => import("./pages/Orchestrator").then((m) => ({ default: m.Orchestrator })));
const Chat = lazy(() => import("./components/Chat").then((m) => ({ default: m.Chat })));
const PendingChangesBar = lazy(() => import("./components/PendingChangesBar").then((m) => ({ default: m.PendingChangesBar })));
const preloadRouteModules = () =>
  Promise.allSettled([
    import("./pages/Home"),
    import("./pages/Channels"),
    import("./pages/Recipes"),
    import("./pages/Cron"),
    import("./pages/Doctor"),
    import("./pages/History"),
    import("./components/Chat"),
    import("./components/PendingChangesBar"),
  ]);

const PING_URL = "https://api.clawpal.zhixian.io/ping";
const LEGACY_DOCKER_INSTANCES_KEY = "clawpal_docker_instances";
const DEFAULT_DOCKER_OPENCLAW_HOME = "~/.clawpal/docker-local";
const DEFAULT_DOCKER_CLAWPAL_DATA_DIR = "~/.clawpal/docker-local/data";
const DEFAULT_DOCKER_INSTANCE_ID = "docker:local";

type Route = "home" | "recipes" | "cook" | "history" | "channels" | "cron" | "doctor" | "orchestrator";
const INSTANCE_ROUTES: Route[] = ["home", "channels", "recipes", "cron", "doctor", "history"];
const OPEN_TABS_STORAGE_KEY = "clawpal_open_tabs";
const WATCHDOG_LATE_GRACE_MS = 5 * 60 * 1000;

interface ToastItem {
  id: number;
  message: string;
  type: "success" | "error";
}

interface ProfileSyncStatus {
  phase: "idle" | "syncing" | "success" | "error";
  message: string;
  instanceId: string | null;
}

function logDevException(label: string, detail: unknown): void {
  if (!import.meta.env.DEV) return;
  console.error(`[dev exception] ${label}`, detail);
}

function logDevIgnoredError(context: string, detail: unknown): void {
  if (!import.meta.env.DEV) return;
  console.warn(`[dev ignored error] ${context}`, detail);
}

// AgentGuidanceItem is imported from ./components/GuidanceCard

let toastIdCounter = 0;

const SSH_ERROR_MAP: Array<[RegExp, string]> = [
  [/connection refused/i, "ssh.errorConnectionRefused"],
  [/no such file/i, "ssh.errorNoSuchFile"],
  [/name or service not known|nodename nor servname provided|temporary failure in name resolution|no address associated with hostname|getaddrinfo|failed to lookup address information|unknown host|hostname was not found/i, "ssh.errorHostUnreachable"],
  [/passphrase|sign_and_send_pubkey|agent refused operation|can't open \/dev\/tty|authentication agent/i, "ssh.errorPassphrase"],
  [/permission denied/i, "ssh.errorPermissionDenied"],
  [/host key verification failed/i, "ssh.errorHostKey"],
  [/timed?\s*out/i, "ssh.errorTimeout"],
];

function friendlySshError(raw: string, t: (key: string, opts?: Record<string, string>) => string): string {
  for (const [pattern, key] of SSH_ERROR_MAP) {
    if (pattern.test(raw)) {
      return `${t(key)}\n(${raw})`;
    }
  }
  return t('config.sshFailed', { error: raw });
}

function sanitizeDockerPathSuffix(raw: string): string {
  const lowered = raw.toLowerCase().replace(/[^a-z0-9_-]/g, "");
  const trimmed = lowered.replace(/^[-_]+|[-_]+$/g, "");
  return trimmed || "docker-local";
}

function deriveDockerPaths(instanceId: string): { openclawHome: string; clawpalDataDir: string } {
  if (instanceId === DEFAULT_DOCKER_INSTANCE_ID) {
    return {
      openclawHome: DEFAULT_DOCKER_OPENCLAW_HOME,
      clawpalDataDir: DEFAULT_DOCKER_CLAWPAL_DATA_DIR,
    };
  }
  const suffixRaw = instanceId.startsWith("docker:") ? instanceId.slice(7) : instanceId;
  const suffix = suffixRaw === "local"
    ? "docker-local"
    : suffixRaw.startsWith("docker-")
      ? sanitizeDockerPathSuffix(suffixRaw)
      : `docker-${sanitizeDockerPathSuffix(suffixRaw)}`;
  const openclawHome = `~/.clawpal/${suffix}`;
  return {
    openclawHome,
    clawpalDataDir: `${openclawHome}/data`,
  };
}

function deriveDockerLabel(instanceId: string): string {
  if (instanceId === DEFAULT_DOCKER_INSTANCE_ID) return "docker-local";
  const suffix = instanceId.startsWith("docker:") ? instanceId.slice(7) : instanceId;
  const match = suffix.match(/^local-(\d+)$/);
  if (match) return `docker-local-${match[1]}`;
  return suffix.startsWith("docker-") ? suffix : `docker-${suffix}`;
}

function fallbackInstanceLabel(instanceId: string, t: (key: string) => string): string {
  if (instanceId === "local") return t("instance.local");
  if (instanceId.startsWith("docker:")) return deriveDockerLabel(instanceId);
  if (instanceId.startsWith("ssh:")) {
    const suffix = instanceId.slice("ssh:".length);
    return suffix || instanceId;
  }
  return instanceId;
}

function hashInstanceToken(raw: string): number {
  let hash = 2166136261;
  for (let i = 0; i < raw.length; i += 1) {
    hash ^= raw.charCodeAt(i);
    hash = Math.imul(hash, 16777619);
  }
  return hash >>> 0;
}

function watchdogJobLikelyLate(job: { lastScheduledAt?: string; lastRunAt?: string | null } | undefined): boolean {
  if (!job?.lastScheduledAt) return false;
  const scheduledAt = Date.parse(job.lastScheduledAt);
  if (!Number.isFinite(scheduledAt)) return false;
  const runAt = job.lastRunAt ? Date.parse(job.lastRunAt) : Number.NaN;
  const missedThisSchedule = !Number.isFinite(runAt) || runAt + 1000 < scheduledAt;
  const overdue = Date.now() - scheduledAt > WATCHDOG_LATE_GRACE_MS;
  return missedThisSchedule && overdue;
}

function normalizeDockerInstance(instance: DockerInstance): DockerInstance {
  const fallback = deriveDockerPaths(instance.id);
  return {
    ...instance,
    label: instance.label?.trim() || deriveDockerLabel(instance.id),
    openclawHome: instance.openclawHome || fallback.openclawHome,
    clawpalDataDir: instance.clawpalDataDir || fallback.clawpalDataDir,
  };
}

export function App() {
  const { t } = useTranslation();
  useFont();
  const [route, setRoute] = useState<Route>("home");
  const [recipeId, setRecipeId] = useState<string | null>(null);
  const [recipeSource, setRecipeSource] = useState<string | undefined>(undefined);
  const [channelNodes, setChannelNodes] = useState<ChannelNode[] | null>(null);
  const [discordGuildChannels, setDiscordGuildChannels] = useState<DiscordGuildChannel[] | null>(null);
  const [channelsLoading, setChannelsLoading] = useState(false);
  const [discordChannelsLoading, setDiscordChannelsLoading] = useState(false);
  const [chatOpen, setChatOpen] = useState(false);
  const [lastInstanceRoute, setLastInstanceRoute] = useState<Route>("home");
  const [startSection, setStartSection] = useState<"overview" | "profiles" | "settings">("overview");
  const [inStart, setInStart] = useState(true);

  // Workspace tabs — persisted to localStorage
  const [openTabIds, setOpenTabIds] = useState<string[]>(() => {
    try {
      const stored = localStorage.getItem(OPEN_TABS_STORAGE_KEY);
      if (stored) {
        const parsed = JSON.parse(stored);
        if (Array.isArray(parsed) && parsed.length > 0) return parsed;
      }
    } catch {}
    return ["local"];
  });

  // SSH remote instance state
  const [activeInstance, setActiveInstance] = useState("local");
  const [sshHosts, setSshHosts] = useState<SshHost[]>([]);
  const [registeredInstances, setRegisteredInstances] = useState<RegisteredInstance[]>([]);
  const [discoveredInstances, setDiscoveredInstances] = useState<DiscoveredInstance[]>([]);
  const [discoveringInstances, setDiscoveringInstances] = useState(false);
  const [connectionStatus, setConnectionStatus] = useState<Record<string, "connected" | "disconnected" | "error">>({});
  const [sshEditOpen, setSshEditOpen] = useState(false);
  const [editingSshHost, setEditingSshHost] = useState<SshHost | null>(null);
  const navigateRoute = useCallback((next: Route) => {
    startTransition(() => setRoute(next));
  }, []);

  const handleEditSsh = useCallback((host: SshHost) => {
    setEditingSshHost(host);
    setSshEditOpen(true);
  }, []);

  const refreshHosts = useCallback(() => {
    withGuidance(() => api.listSshHosts(), "listSshHosts", "local", "local")
      .then(setSshHosts)
      .catch((error) => {
        logDevIgnoredError("refreshHosts", error);
      });
  }, []);

  const refreshRegisteredInstances = useCallback(() => {
    withGuidance(() => api.listRegisteredInstances(), "listRegisteredInstances", "local", "local")
      .then(setRegisteredInstances)
      .catch((error) => {
        logDevIgnoredError("listRegisteredInstances", error);
        setRegisteredInstances([]);
      });
  }, []);

  const discoverInstances = useCallback(() => {
    setDiscoveringInstances(true);
    withGuidance(
      () => api.discoverLocalInstances(),
      "discoverLocalInstances",
      "local",
      "local",
    )
      .then(setDiscoveredInstances)
      .catch((error) => {
        logDevIgnoredError("discoverLocalInstances", error);
        setDiscoveredInstances([]);
      })
      .finally(() => setDiscoveringInstances(false));
  }, []);

  const dockerInstances = useMemo<DockerInstance[]>(() => {
    const seen = new Set<string>();
    const out: DockerInstance[] = [];
    for (const item of registeredInstances) {
      if (item.instanceType !== "docker") continue;
      if (!item.id || seen.has(item.id)) continue;
      seen.add(item.id);
      out.push(normalizeDockerInstance({
        id: item.id,
        label: item.label || deriveDockerLabel(item.id),
        openclawHome: item.openclawHome || undefined,
        clawpalDataDir: item.clawpalDataDir || undefined,
      }));
    }
    return out;
  }, [registeredInstances]);

  const upsertDockerInstance = useCallback(async (instance: DockerInstance): Promise<RegisteredInstance> => {
    const normalized = normalizeDockerInstance(instance);
    const registered = await withGuidance(
      () => api.connectDockerInstance(
        normalized.openclawHome || deriveDockerPaths(normalized.id).openclawHome,
        normalized.label,
        normalized.id,
      ),
      "connectDockerInstance",
      normalized.id,
      "docker_local",
    );
    // Await the refresh so callers can rely on registeredInstances being up-to-date
    const updated = await withGuidance(
      () => api.listRegisteredInstances(),
      "listRegisteredInstances",
      "local",
      "local",
    ).catch((error) => {
      logDevIgnoredError("listRegisteredInstances after connect", error);
      return null;
    });
    if (updated) setRegisteredInstances(updated);
    return registered;
  }, []);

  const renameDockerInstance = useCallback((id: string, label: string) => {
    const nextLabel = label.trim();
    if (!nextLabel) return;
    const instance = dockerInstances.find((item) => item.id === id);
    if (!instance) return;
    void withGuidance(
      () => api.connectDockerInstance(
        instance.openclawHome || deriveDockerPaths(instance.id).openclawHome,
        nextLabel,
        instance.id,
      ),
      "connectDockerInstance",
      instance.id,
      "docker_local",
    ).then(() => {
      refreshRegisteredInstances();
    });
  }, [dockerInstances, refreshRegisteredInstances]);

  const deleteDockerInstance = useCallback(async (instance: DockerInstance, deleteLocalData: boolean) => {
    const fallback = deriveDockerPaths(instance.id);
    const openclawHome = instance.openclawHome || fallback.openclawHome;
    if (deleteLocalData) {
      await withGuidance(
        () => api.deleteLocalInstanceHome(openclawHome),
        "deleteLocalInstanceHome",
        instance.id,
        "docker_local",
      );
    }
    await withGuidance(
      () => api.deleteRegisteredInstance(instance.id),
      "deleteRegisteredInstance",
      instance.id,
      "docker_local",
    );
    setOpenTabIds((prev) => prev.filter((t) => t !== instance.id));
    setActiveInstance((prev) => (prev === instance.id ? "local" : prev));
    refreshRegisteredInstances();
  }, [refreshRegisteredInstances]);

  useEffect(() => {
    refreshHosts();
    refreshRegisteredInstances();
    discoverInstances();
    const timer = setInterval(refreshRegisteredInstances, 30_000);
    return () => clearInterval(timer);
  }, [refreshHosts, refreshRegisteredInstances, discoverInstances]);

  useEffect(() => {
    const timer = window.setTimeout(() => {
      void preloadRouteModules();
    }, 1200);
    return () => window.clearTimeout(timer);
  }, []);

  const [appUpdateAvailable, setAppUpdateAvailable] = useState(false);
  const [hasEscalatedCron, setHasEscalatedCron] = useState(false);
  const [appVersion, setAppVersion] = useState("");

  // Startup: check for updates + analytics ping
  useEffect(() => {
    let installId = localStorage.getItem("clawpal_install_id");
    if (!installId) {
      installId = crypto.randomUUID();
      localStorage.setItem("clawpal_install_id", installId);
    }

    // Silent update check
    check()
      .then((update) => { if (update) setAppUpdateAvailable(true); })
      .catch((error) => logDevIgnoredError("check", error));

    // Analytics ping (fire-and-forget)
    getVersion().then((version) => {
      setAppVersion(version);
      const url = PING_URL;
      if (!url) return;
      fetch(url, {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify({ v: version, id: installId, platform: navigator.platform }),
      }).catch((error) => logDevIgnoredError("analytics ping request", error));
    }).catch((error) => logDevIgnoredError("getVersion", error));

  }, []);

  const [toasts, setToasts] = useState<ToastItem[]>([]);
  const [profileSyncStatus, setProfileSyncStatus] = useState<ProfileSyncStatus>({
    phase: "idle",
    message: "",
    instanceId: null,
  });
  const [agentGuidanceByInstance, setAgentGuidanceByInstance] = useState<Record<string, AgentGuidanceItem>>({});
  const [doctorLaunchByInstance, setDoctorLaunchByInstance] = useState<Record<string, AgentGuidanceItem | null>>({});
  const [agentGuidanceOpen, setAgentGuidanceOpen] = useState(false);
  const [unreadGuidance, setUnreadGuidance] = useState(false);
  const [doctorNavPulse, setDoctorNavPulse] = useState(false);
  const sshHealthFailStreakRef = useRef<Record<string, number>>({});
  const legacyMigrationDoneRef = useRef(false);
  const passphraseResolveRef = useRef<((value: string | null) => void) | null>(null);
  const [passphraseHostLabel, setPassphraseHostLabel] = useState<string>("");
  const [passphraseOpen, setPassphraseOpen] = useState(false);
  const [passphraseInput, setPassphraseInput] = useState("");
  const remoteAuthSyncAtRef = useRef<Record<string, number>>({});
  const accessProbeTimerRef = useRef<ReturnType<typeof setTimeout> | null>(null);
  const lastAccessProbeAtRef = useRef<Record<string, number>>({});
  const profileBootstrapGuidanceSigRef = useRef("");

  // Persist open tabs
  useEffect(() => {
    localStorage.setItem(OPEN_TABS_STORAGE_KEY, JSON.stringify(openTabIds));
  }, [openTabIds]);

  const showToast = useCallback((message: string, type: "success" | "error" = "success") => {
    const id = ++toastIdCounter;
    setToasts((prev) => [...prev, { id, message, type }]);
    setTimeout(() => {
      setToasts((prev) => prev.filter((t) => t.id !== id));
    }, type === "error" ? 5000 : 3000);
  }, []);

  const handleSshEditSave = useCallback(async (host: SshHost) => {
    try {
      await withGuidance(
        () => api.upsertSshHost(host),
        "upsertSshHost",
        host.id,
        "remote_ssh",
      );
      refreshHosts();
      refreshRegisteredInstances();
      setSshEditOpen(false);
      showToast(t("instance.sshUpdated"), "success");
    } catch (e) {
      showToast(e instanceof Error ? e.message : String(e), "error");
    }
  }, [refreshHosts, refreshRegisteredInstances, showToast, t]);

  const handleConnectDiscovered = useCallback(async (discovered: DiscoveredInstance) => {
    try {
      await withGuidance(
        () => api.connectDockerInstance(discovered.homePath, discovered.label, discovered.id),
        "connectDockerInstance",
        discovered.id,
        "docker_local",
      );
      refreshRegisteredInstances();
      discoverInstances();
      showToast(t("start.connected", { label: discovered.label }), "success");
    } catch (e) {
      showToast(e instanceof Error ? e.message : String(e), "error");
    }
  }, [refreshRegisteredInstances, discoverInstances, showToast, t]);

  // Startup precheck: validate registry
  useEffect(() => {
    withGuidance(
      () => api.precheckRegistry(),
      "precheckRegistry",
      "local",
      "local",
    ).then((issues) => {
      const errors = issues.filter((i: PrecheckIssue) => i.severity === "error");
      if (errors.length === 1) {
        showToast(errors[0].message, "error");
      } else if (errors.length > 1) {
        showToast(`${errors[0].message}（还有 ${errors.length - 1} 个问题）`, "error");
      }
    }).catch((error) => {
      logDevIgnoredError("precheckRegistry", error);
    });
  }, [showToast]);

  useEffect(() => {
    const onGuidance = (event: Event) => {
      const custom = event as CustomEvent<AgentGuidanceItem>;
      if (!custom.detail) return;
      setAgentGuidanceByInstance((prev) => ({
        ...prev,
        [custom.detail.instanceId]: custom.detail,
      }));
      setAgentGuidanceOpen(true);
      setUnreadGuidance(true);
    };
    window.addEventListener("clawpal:agent-guidance", onGuidance as EventListener);
    return () => {
      window.removeEventListener("clawpal:agent-guidance", onGuidance as EventListener);
    };
  }, []);

  useEffect(() => {
    let cancelled = false;
    const timer = window.setTimeout(() => {
      void (async () => {
        try {
          const before = await api.listModelProfiles();
          if (cancelled || before.some((p) => p.enabled)) return;

          await api.extractModelProfilesFromConfig().catch((error) => {
            logDevIgnoredError("bootstrap extractModelProfilesFromConfig", error);
            return null;
          });
          const after = await api.listModelProfiles();
          if (cancelled || after.some((p) => p.enabled)) return;

          const hasConnectableInstance =
            registeredInstances.some((inst) => inst.id !== "local")
            || discoveredInstances.length > 0;
          const signature = hasConnectableInstance ? "with-instance" : "no-instance";
          if (profileBootstrapGuidanceSigRef.current === signature) return;
          profileBootstrapGuidanceSigRef.current = signature;

          const actions = hasConnectableInstance
            ? [
              t("onboarding.actionSyncProfiles"),
              t("onboarding.actionAddProfile"),
            ]
            : [
              t("onboarding.actionConnectInstanceFirst"),
              t("onboarding.actionOpenConnectEntry"),
            ];
          window.dispatchEvent(new CustomEvent("clawpal:agent-guidance", {
            detail: {
              message: t("onboarding.noProfilesSummary"),
              summary: t("onboarding.noProfilesSummary"),
              actions,
              source: "onboarding",
              operation: "profiles.bootstrap.missing",
              instanceId: "local",
              transport: "local",
              rawError: hasConnectableInstance
                ? "No model profiles detected after auto extraction"
                : "No model profiles detected and no connectable instances found",
              createdAt: Date.now(),
            },
          }));
        } catch {
          // ignore bootstrap guidance failures
        }
      })();
    }, 1200);
    return () => {
      cancelled = true;
      window.clearTimeout(timer);
    };
  }, [discoveredInstances.length, registeredInstances, t]);

  const agentGuidance = agentGuidanceByInstance[activeInstance] || null;
  const resolveGuidanceForInstance = useCallback((instanceId: string) => {
    setAgentGuidanceByInstance((prev) => {
      if (!prev[instanceId]) return prev;
      const next = { ...prev };
      delete next[instanceId];
      return next;
    });
    setDoctorLaunchByInstance((prev) => {
      if (!(instanceId in prev)) return prev;
      const next = { ...prev };
      delete next[instanceId];
      return next;
    });
    if (instanceId === activeInstance) {
      setAgentGuidanceOpen(false);
      setUnreadGuidance(false);
    }
  }, [activeInstance]);

  useEffect(() => {
    if (!agentGuidance) {
      setAgentGuidanceOpen(false);
    }
  }, [activeInstance, agentGuidance]);

  const resolveInstanceTransport = useCallback((instanceId: string) => {
    if (instanceId === "local") return "local";
    const registered = registeredInstances.find((item) => item.id === instanceId);
    if (registered?.instanceType === "docker") return "docker_local";
    if (registered?.instanceType === "remote_ssh") return "remote_ssh";
    if (instanceId.startsWith("docker:")) return "docker_local";
    if (instanceId.startsWith("ssh:")) return "remote_ssh";
    if (dockerInstances.some((item) => item.id === instanceId)) return "docker_local";
    if (sshHosts.some((host) => host.id === instanceId)) return "remote_ssh";
    // Unknown id should not be treated as remote by default.
    return "local";
  }, [dockerInstances, sshHosts, registeredInstances]);

  useEffect(() => {
    const handleUnhandled = (operation: string, reason: unknown) => {
      if (reason && typeof reason === "object" && (reason as any)._guidanceEmitted) {
        return;
      }
      const transport = resolveInstanceTransport(activeInstance);
      void explainAndBuildGuidanceError({
        method: operation,
        instanceId: activeInstance,
        transport,
        rawError: reason,
        emitEvent: true,
      });
    };

    const onUnhandledRejection = (event: PromiseRejectionEvent) => {
      logDevException("unhandledRejection", event.reason);
      handleUnhandled("unhandledRejection", event.reason);
    };
    const onGlobalError = (event: ErrorEvent) => {
      const detail = event.error ?? event.message ?? "unknown error";
      logDevException("unhandledError", detail);
      handleUnhandled("unhandledError", detail);
    };

    window.addEventListener("unhandledrejection", onUnhandledRejection);
    window.addEventListener("error", onGlobalError);
    return () => {
      window.removeEventListener("unhandledrejection", onUnhandledRejection);
      window.removeEventListener("error", onGlobalError);
    };
  }, [activeInstance, resolveInstanceTransport]);

  const ensureAccessForInstance = useCallback((instanceId: string) => {
    const transport = resolveInstanceTransport(instanceId);
    withGuidance(
      () => api.ensureAccessProfile(instanceId, transport),
      "ensureAccessProfile",
      instanceId,
      transport,
    ).catch((error) => {
      logDevIgnoredError("ensureAccessProfile", error);
    });
    // Auth precheck: warn if model profiles are misconfigured
    withGuidance(
      () => api.precheckAuth(instanceId),
      "precheckAuth",
      instanceId,
      transport,
    ).then((issues) => {
      const errors = issues.filter((i: PrecheckIssue) => i.severity === "error");
      if (errors.length === 1) {
        showToast(errors[0].message, "error");
      } else if (errors.length > 1) {
        showToast(`${errors[0].message}（还有 ${errors.length - 1} 个问题）`, "error");
      }
    }).catch((error) => {
      logDevIgnoredError("precheckAuth", error);
    });
  }, [resolveInstanceTransport, showToast]);

  const scheduleEnsureAccessForInstance = useCallback((instanceId: string, delayMs = 1200) => {
    const now = Date.now();
    const last = lastAccessProbeAtRef.current[instanceId] || 0;
    // Debounce per-instance background probes to keep tab switching responsive.
    if (now - last < 30_000) return;
    if (accessProbeTimerRef.current !== null) {
      clearTimeout(accessProbeTimerRef.current);
      accessProbeTimerRef.current = null;
    }
    accessProbeTimerRef.current = setTimeout(() => {
      lastAccessProbeAtRef.current[instanceId] = Date.now();
      ensureAccessForInstance(instanceId);
      accessProbeTimerRef.current = null;
    }, delayMs);
  }, [ensureAccessForInstance]);

  const readLegacyDockerInstances = useCallback((): DockerInstance[] => {
    try {
      const raw = localStorage.getItem(LEGACY_DOCKER_INSTANCES_KEY);
      if (!raw) return [];
      const parsed = JSON.parse(raw) as DockerInstance[];
      if (!Array.isArray(parsed)) return [];
      const out: DockerInstance[] = [];
      const seen = new Set<string>();
      for (const item of parsed) {
        if (!item?.id || typeof item.id !== "string") continue;
        const id = item.id.trim();
        if (!id || seen.has(id)) continue;
        seen.add(id);
        out.push(normalizeDockerInstance({ ...item, id }));
      }
      return out;
    } catch {
      return [];
    }
  }, []);

  const readLegacyOpenTabs = useCallback((): string[] => {
    try {
      const raw = localStorage.getItem(OPEN_TABS_STORAGE_KEY);
      if (!raw) return [];
      const parsed = JSON.parse(raw);
      if (!Array.isArray(parsed)) return [];
      return parsed.filter((id): id is string => typeof id === "string" && id.trim().length > 0);
    } catch {
      return [];
    }
  }, []);

  useEffect(() => {
    return () => {
      if (accessProbeTimerRef.current !== null) {
        clearTimeout(accessProbeTimerRef.current);
        accessProbeTimerRef.current = null;
      }
    };
  }, []);

  useEffect(() => {
    if (legacyMigrationDoneRef.current) return;
    legacyMigrationDoneRef.current = true;
    const legacyDockerInstances = readLegacyDockerInstances();
    const legacyOpenTabIds = readLegacyOpenTabs();
    withGuidance(
      () => api.migrateLegacyInstances(legacyDockerInstances, legacyOpenTabIds),
      "migrateLegacyInstances",
      "local",
      "local",
    )
      .then((result) => {
        if (
          result.importedSshHosts > 0
          || result.importedDockerInstances > 0
          || result.importedOpenTabInstances > 0
        ) {
          refreshRegisteredInstances();
          refreshHosts();
          localStorage.removeItem(LEGACY_DOCKER_INSTANCES_KEY);
        }
      })
      .catch((e) => {
        console.error("Legacy instance migration failed:", e);
      });
  }, [readLegacyDockerInstances, readLegacyOpenTabs, refreshRegisteredInstances, refreshHosts]);

  const dismissToast = useCallback((id: number) => {
    setToasts((prev) => prev.filter((t) => t.id !== id));
  }, []);

  const requestPassphrase = useCallback((hostLabel: string): Promise<string | null> => {
    setPassphraseHostLabel(hostLabel);
    setPassphraseInput("");
    setPassphraseOpen(true);
    return new Promise((resolve) => {
      passphraseResolveRef.current = resolve;
    });
  }, []);

  const closePassphraseDialog = useCallback((value: string | null) => {
    setPassphraseOpen(false);
    const resolve = passphraseResolveRef.current;
    passphraseResolveRef.current = null;
    if (resolve) resolve(value);
  }, []);

  const connectWithPassphraseFallback = useCallback(async (hostId: string) => {
    const host = sshHosts.find((h) => h.id === hostId);
    const hostLabel = host?.label || host?.host || hostId;
    try {
      await api.sshConnect(hostId);
      return;
    } catch (err) {
      const raw = String(err);
      if (host && host.authMethod !== "password" && SSH_PASSPHRASE_RETRY_HINT.test(raw)) {
        const passphrase = await requestPassphrase(hostLabel);
        if (passphrase !== null) {
          try {
            await withGuidance(
              () => api.sshConnectWithPassphrase(hostId, passphrase),
              "sshConnectWithPassphrase",
              hostId,
              "remote_ssh",
            );
            return;
          } catch (passphraseErr) {
            const passphraseRaw = String(passphraseErr);
            const fallbackMessage = buildSshPassphraseConnectErrorMessage(passphraseRaw, hostLabel, t);
            if (fallbackMessage) {
              throw new Error(fallbackMessage);
            }
            throw await explainAndBuildGuidanceError({
              method: "sshConnectWithPassphrase",
              instanceId: hostId,
              transport: "remote_ssh",
              rawError: passphraseErr,
            });
          }
        } else {
          throw new Error(buildSshPassphraseCancelMessage(hostLabel, t));
        }
      }
      const fallbackMessage = buildSshPassphraseConnectErrorMessage(raw, hostLabel, t);
      if (fallbackMessage) {
        throw new Error(fallbackMessage);
      }
      throw await explainAndBuildGuidanceError({
        method: "sshConnect",
        instanceId: hostId,
        transport: "remote_ssh",
        rawError: err,
      });
    }
  }, [requestPassphrase, sshHosts, t]);

  const syncRemoteAuthAfterConnect = useCallback(async (hostId: string) => {
    const now = Date.now();
    const last = remoteAuthSyncAtRef.current[hostId] || 0;
    if (now - last < 30_000) return;
    remoteAuthSyncAtRef.current[hostId] = now;
    setProfileSyncStatus({
      phase: "syncing",
      message: "正在同步远程模型认证…",
      instanceId: hostId,
    });
    try {
      const result = await api.remoteSyncProfilesToLocalAuth(hostId);
      invalidateGlobalReadCache(["listModelProfiles", "resolveApiKeys"]);
      const localProfiles = await api.listModelProfiles().catch((error) => {
        logDevIgnoredError("syncRemoteAuthAfterConnect listModelProfiles", error);
        return [];
      });
      if (result.resolvedKeys > 0 || result.syncedProfiles > 0) {
        if (localProfiles.length > 0) {
          const message = `已同步远程认证：profiles ${result.syncedProfiles}，keys ${result.resolvedKeys}`;
          showToast(message, "success");
          setProfileSyncStatus({
            phase: "success",
            message,
            instanceId: hostId,
          });
        } else {
          const message = "远程同步返回成功，但本地模型列表仍为空（请检查本地 profiles 路径和读取权限）";
          showToast(message, "error");
          setProfileSyncStatus({
            phase: "error",
            message,
            instanceId: hostId,
          });
        }
      } else if (result.totalRemoteProfiles > 0) {
        const message = "远程已有 profiles，但未解析到可用 key（请检查 auth_ref/环境变量）";
        showToast(message, "error");
        setProfileSyncStatus({
          phase: "error",
          message,
          instanceId: hostId,
        });
      } else {
        const message = "远程实例未发现可同步的模型配置";
        showToast(message, "error");
        setProfileSyncStatus({
          phase: "error",
          message,
          instanceId: hostId,
        });
      }
    } catch (e) {
      const message = `同步远程认证信息失败：${e}`;
      showToast(message, "error");
      setProfileSyncStatus({
        phase: "error",
        message,
        instanceId: hostId,
      });
    }
  }, [showToast]);


  const openTab = useCallback((id: string) => {
    startTransition(() => {
      setOpenTabIds((prev) => prev.includes(id) ? prev : [...prev, id]);
      setActiveInstance(id);
      setInStart(false);
      // Entering instance mode from Start should prefer a fast route.
      navigateRoute("home");
    });
  }, [navigateRoute]);

  const closeTab = useCallback((id: string) => {
    setOpenTabIds((prev) => {
      const next = prev.filter((t) => t !== id);
      if (activeInstance === id) {
        if (next.length === 0) {
          setInStart(true);
          setStartSection("overview");
        } else {
          setActiveInstance(next[next.length - 1]);
        }
      }
      return next;
    });
  }, [activeInstance]);

  const handleInstanceSelect = useCallback((id: string) => {
    if (id === activeInstance && !inStart) {
      return;
    }
    startTransition(() => {
      setActiveInstance(id);
      setOpenTabIds((prev) => prev.includes(id) ? prev : [...prev, id]);
      setInStart(false);
      // Always land on Home when switching instance to avoid route-specific
      // heavy reloads (e.g., Channels) on the critical interaction path.
      navigateRoute("home");
    });
    // Instance switch precheck
    withGuidance(
      () => api.precheckInstance(id),
      "precheckInstance",
      id,
      resolveInstanceTransport(id),
    ).then((issues) => {
      const blocking = issues.filter((i: PrecheckIssue) => i.severity === "error");
      if (blocking.length === 1) {
        showToast(blocking[0].message, "error");
      } else if (blocking.length > 1) {
        showToast(`${blocking[0].message}（还有 ${blocking.length - 1} 个问题）`, "error");
      }
    }).catch((error) => {
      logDevIgnoredError("precheckInstance", error);
    });
    const transport = resolveInstanceTransport(id);
    // Transport precheck for non-SSH targets.
    // SSH switching immediately triggers reconnect flow below, so running
    // precheckTransport here would cause noisy transient "not active" toasts.
    if (transport !== "remote_ssh") {
      withGuidance(
        () => api.precheckTransport(id),
        "precheckTransport",
        id,
        transport,
      ).then((issues) => {
        const blocking = issues.filter((i: PrecheckIssue) => i.severity === "error");
        if (blocking.length === 1) {
          showToast(blocking[0].message, "error");
        } else if (blocking.length > 1) {
          showToast(`${blocking[0].message}（还有 ${blocking.length - 1} 个问题）`, "error");
        } else {
          const warnings = issues.filter((i: PrecheckIssue) => i.severity === "warn");
          if (warnings.length > 0) {
            showToast(warnings[0].message, "error");
          }
        }
      }).catch((error) => {
        logDevIgnoredError("precheckTransport", error);
      });
    }
    if (transport !== "remote_ssh") return;
    // Check if backend still has a live connection before reconnecting.
    // Do not pre-mark as disconnected — transient status failures would
    // otherwise gray out the whole remote UI.
    withGuidance(
      () => api.sshStatus(id),
      "sshStatus",
      id,
      "remote_ssh",
    )
      .then((status) => {
        if (status === "connected") {
          setConnectionStatus((prev) => ({ ...prev, [id]: "connected" }));
          scheduleEnsureAccessForInstance(id, 1500);
          void syncRemoteAuthAfterConnect(id);
        } else {
          return connectWithPassphraseFallback(id)
            .then(() => {
              setConnectionStatus((prev) => ({ ...prev, [id]: "connected" }));
              scheduleEnsureAccessForInstance(id, 1500);
              void syncRemoteAuthAfterConnect(id);
            });
        }
      })
      .catch((error) => {
        logDevIgnoredError("sshStatus or reconnect", error);
        // sshStatus failed or reconnect failed — try fresh connect
        connectWithPassphraseFallback(id)
          .then(() => {
            setConnectionStatus((prev) => ({ ...prev, [id]: "connected" }));
            scheduleEnsureAccessForInstance(id, 1500);
            void syncRemoteAuthAfterConnect(id);
          })
          .catch((e2) => {
            setConnectionStatus((prev) => ({ ...prev, [id]: "error" }));
            const raw = String(e2);
            const friendly = friendlySshError(raw, t);
            showToast(friendly, "error");
          });
      });
  }, [activeInstance, inStart, resolveInstanceTransport, scheduleEnsureAccessForInstance, connectWithPassphraseFallback, syncRemoteAuthAfterConnect, showToast, t, navigateRoute]);

  const [configVersion, setConfigVersion] = useState(0);
  const [instanceToken, setInstanceToken] = useState(0);

  const isDocker = registeredInstances.some((item) => item.id === activeInstance && item.instanceType === "docker")
    || dockerInstances.some((item) => item.id === activeInstance);
  const isRemote = registeredInstances.some((item) => item.id === activeInstance && item.instanceType === "remote_ssh")
    || sshHosts.some((host) => host.id === activeInstance);
  const isConnected = !isRemote || connectionStatus[activeInstance] === "connected";

  useEffect(() => {
    let cancelled = false;
    let nextHome: string | null = null;
    let nextDataDir: string | null = null;
    const activeRegistered = registeredInstances.find((item) => item.id === activeInstance);
    if (activeInstance === "local" || isRemote) {
      nextHome = null;
      nextDataDir = null;
    } else if (isDocker) {
      const instance = dockerInstances.find((item) => item.id === activeInstance);
      const fallback = deriveDockerPaths(activeInstance);
      nextHome = instance?.openclawHome || fallback.openclawHome;
      nextDataDir = instance?.clawpalDataDir || fallback.clawpalDataDir;
    } else if (activeRegistered?.instanceType === "local" && activeRegistered.openclawHome) {
      nextHome = activeRegistered.openclawHome;
      nextDataDir = activeRegistered.clawpalDataDir || null;
    }
    const tokenSeed = `${activeInstance}|${nextHome || ""}|${nextDataDir || ""}`;

    const applyOverrides = async () => {
      if (nextHome === null && nextDataDir === null) {
        await Promise.all([
          api.setActiveOpenclawHome(null).catch((error) => logDevIgnoredError("setActiveOpenclawHome", error)),
          api.setActiveClawpalDataDir(null).catch((error) => logDevIgnoredError("setActiveClawpalDataDir", error)),
        ]);
      } else {
        await Promise.all([
          api.setActiveOpenclawHome(nextHome).catch((error) => logDevIgnoredError("setActiveOpenclawHome", error)),
          api.setActiveClawpalDataDir(nextDataDir).catch((error) => logDevIgnoredError("setActiveClawpalDataDir", error)),
        ]);
      }
      if (!cancelled) {
        // Token bumps only after overrides are applied, so data panels can
        // safely refetch with the correct per-instance OPENCLAW_HOME.
        setInstanceToken(hashInstanceToken(tokenSeed));
      }
    };
    void applyOverrides();
    return () => {
      cancelled = true;
    };
  }, [activeInstance, isDocker, isRemote, dockerInstances, registeredInstances]);

  // Keep active remote instance self-healed: detect dropped SSH and reconnect.
  useEffect(() => {
    if (!isRemote) return;
    let cancelled = false;
    let inFlight = false;
    const hostId = activeInstance;
    const reportAutoHealFailure = (rawError: unknown) => {
      const errorText = String(rawError);
      void explainAndBuildGuidanceError({
        method: "sshConnect",
        instanceId: hostId,
        transport: "remote_ssh",
        rawError: rawError,
        emitEvent: true,
      }).catch((error) => {
        logDevIgnoredError("autoheal explainAndBuildGuidanceError", error);
      });
      showToast(friendlySshError(errorText, t), "error");
    };
    const markFailure = (rawError: unknown) => {
      if (cancelled) return;
      const streak = (sshHealthFailStreakRef.current[hostId] || 0) + 1;
      sshHealthFailStreakRef.current[hostId] = streak;
      // Avoid flipping UI to disconnected/error on a single transient failure.
      if (streak >= 2) {
        setConnectionStatus((prev) => ({ ...prev, [hostId]: "error" }));
        // Escalate the first stable failure in this streak to guidance + toast.
        if (streak === 2) {
          reportAutoHealFailure(rawError);
        }
      }
    };

    const checkAndHeal = async () => {
      if (cancelled || inFlight) return;
      inFlight = true;
      try {
        const status = await api.sshStatus(hostId);
        if (cancelled) return;
        if (status === "connected") {
          sshHealthFailStreakRef.current[hostId] = 0;
          setConnectionStatus((prev) => ({ ...prev, [hostId]: "connected" }));
          return;
        }
        try {
          await connectWithPassphraseFallback(hostId);
          if (!cancelled) {
            sshHealthFailStreakRef.current[hostId] = 0;
            setConnectionStatus((prev) => ({ ...prev, [hostId]: "connected" }));
          }
        } catch (connectError) {
          markFailure(connectError);
        }
      } catch (statusError) {
        markFailure(statusError);
      } finally {
        inFlight = false;
      }
    };

    checkAndHeal();
    const timer = setInterval(checkAndHeal, 15_000);
    return () => {
      cancelled = true;
      clearInterval(timer);
    };
  }, [activeInstance, isRemote, showToast, t]);

  // Clear cached channel data only when switching instance.
  // Avoid clearing on transient connection-status changes, which causes
  // Channels page to flicker between "loading" and loaded data.
  useEffect(() => {
    setChannelNodes(null);
    setDiscordGuildChannels(null);
  }, [activeInstance]);

  const refreshChannelNodesCache = useCallback(async () => {
    setChannelsLoading(true);
    try {
      const nodes = isRemote
        ? await api.remoteListChannelsMinimal(activeInstance)
        : await api.listChannelsMinimal();
      setChannelNodes(nodes);
      return nodes;
    } finally {
      setChannelsLoading(false);
    }
  }, [activeInstance, isRemote]);

  const refreshDiscordChannelsCache = useCallback(async () => {
    setDiscordChannelsLoading(true);
    try {
      const channels = isRemote
        ? await api.remoteListDiscordGuildChannels(activeInstance)
        : await api.listDiscordGuildChannels();
      setDiscordGuildChannels(channels);
      return channels;
    } finally {
      setDiscordChannelsLoading(false);
    }
  }, [activeInstance, isRemote]);

  // Load unified channel cache lazily when Channels tab is active.
  useEffect(() => {
    if (route !== "channels") return;
    if (isRemote && !isConnected) return;
    void Promise.allSettled([
      refreshChannelNodesCache(),
      refreshDiscordChannelsCache(),
    ]);
  }, [
    route,
    isRemote,
    isConnected,
    refreshChannelNodesCache,
    refreshDiscordChannelsCache,
  ]);

  // Poll watchdog status for escalated cron jobs (red dot badge)
  useEffect(() => {
    const check = () => {
      const p = isRemote
        ? api.remoteGetWatchdogStatus(activeInstance)
        : api.getWatchdogStatus();
      p.then((status: any) => {
        if (status?.jobs) {
          const hasLikelyLateJob = Object.values(status.jobs).some((j: any) => watchdogJobLikelyLate(j));
          setHasEscalatedCron(hasLikelyLateJob);
        } else {
          setHasEscalatedCron(false);
        }
      }).catch((error) => {
        logDevIgnoredError("watchdog status fetch", error);
        setHasEscalatedCron(false);
      });
    };
    const initialDelayMs = isRemote ? 5000 : 500;
    const initial = setTimeout(check, initialDelayMs);
    const interval = setInterval(check, 30000);
    return () => {
      clearTimeout(initial);
      clearInterval(interval);
    };
  }, [activeInstance, isRemote]);

  const bumpConfigVersion = useCallback(() => {
    setConfigVersion((v) => v + 1);
  }, []);

  const openControlCenter = useCallback(() => {
    setInStart(true);
    setStartSection("overview");
  }, []);

  const openDoctor = useCallback(() => {
    setDoctorNavPulse(true);
    setInStart(false);
    navigateRoute("doctor");
    window.setTimeout(() => {
      setDoctorNavPulse(false);
    }, 1400);
  }, [navigateRoute]);

  useEffect(() => {
    if (INSTANCE_ROUTES.includes(route)) {
      setLastInstanceRoute(route);
    }
  }, [route]);

  const showSidebar = true;

  // Derive openTabs array for InstanceTabBar
  const openTabs = useMemo(() => {
    const registryById = new Map(registeredInstances.map((item) => [item.id, item]));
    return openTabIds.flatMap((id) => {
      if (id === "local") return { id, label: t("instance.local"), type: "local" as const };
      const registered = registryById.get(id);
      if (registered) {
        const fallbackLabel = registered.instanceType === "docker" ? deriveDockerLabel(id) : id;
        return {
          id,
          label: registered.label || fallbackLabel,
          type: registered.instanceType === "remote_ssh" ? "ssh" as const : registered.instanceType as "local" | "docker",
        };
      }
      return [];
    });
  }, [openTabIds, registeredInstances, t]);

  // Handle install completion — register docker instance and open tab
  const handleInstallReady = useCallback(async (session: InstallSession) => {
    const artifacts = session.artifacts || {};
    const readArtifactString = (keys: string[]): string => {
      for (const key of keys) {
        const value = artifacts[key];
        if (typeof value === "string" && value.trim()) {
          return value.trim();
        }
      }
      return "";
    };
    if (session.method === "docker") {
      const artifactId = readArtifactString(["docker_instance_id", "dockerInstanceId"]);
      const id = artifactId || DEFAULT_DOCKER_INSTANCE_ID;
      const fallback = deriveDockerPaths(id);
      const openclawHome = readArtifactString(["docker_openclaw_home", "dockerOpenclawHome"]) || fallback.openclawHome;
      const clawpalDataDir = readArtifactString(["docker_clawpal_data_dir", "dockerClawpalDataDir"]) || `${openclawHome}/data`;
      const label = readArtifactString(["docker_instance_label", "dockerInstanceLabel"]) || deriveDockerLabel(id);
      const registered = await upsertDockerInstance({ id, label, openclawHome, clawpalDataDir });
      openTab(registered.id);
    } else if (session.method === "remote_ssh") {
      let hostId = readArtifactString(["ssh_host_id", "sshHostId", "host_id", "hostId"]);
      const hostLabel = readArtifactString(["ssh_host_label", "sshHostLabel", "host_label", "hostLabel"]);
      const hostAddr = readArtifactString(["ssh_host", "sshHost", "host"]);
      if (!hostId) {
        const knownHosts = await api.listSshHosts().catch((error) => {
          logDevIgnoredError("handleInstallReady listSshHosts", error);
          return [] as SshHost[];
        });
        if (hostLabel) {
          const byLabel = knownHosts.find((item) => item.label === hostLabel);
          if (byLabel) hostId = byLabel.id;
        }
        if (!hostId && hostAddr) {
          const byHost = knownHosts.find((item) => item.host === hostAddr);
          if (byHost) hostId = byHost.id;
        }
      }
      if (hostId) {
        const activateRemoteInstance = (instanceId: string, status: "connected" | "error") => {
          setOpenTabIds((prev) => prev.includes(instanceId) ? prev : [...prev, instanceId]);
          setActiveInstance(instanceId);
          setConnectionStatus((prev) => ({ ...prev, [instanceId]: status }));
          setInStart(false);
          navigateRoute("home");
        };
        try {
          // Register the SSH host as an instance and update state
          // synchronously so the tab bar can render it immediately.
          const instance = await withGuidance(
            () => api.connectSshInstance(hostId),
            "connectSshInstance",
            hostId,
            "remote_ssh",
          );
          setRegisteredInstances((prev) => {
            const filtered = prev.filter((r) => r.id !== hostId && r.id !== instance.id);
            return [...filtered, instance];
          });
          refreshHosts();
          refreshRegisteredInstances();
          activateRemoteInstance(instance.id, "connected");
          scheduleEnsureAccessForInstance(instance.id, 600);
          void syncRemoteAuthAfterConnect(instance.id);
        } catch (err) {
          console.warn("connectSshInstance failed during install-ready:", err);
          refreshHosts();
          refreshRegisteredInstances();
          const alreadyRegistered = registeredInstances.some((item) => item.id === hostId);
          if (alreadyRegistered) {
            activateRemoteInstance(hostId, "error");
          } else {
            setInStart(true);
            setStartSection("overview");
          }
          const reason = friendlySshError(String(err), t);
          showToast(reason, "error");
        }
      } else {
        showToast("SSH host id missing after submit. Please reopen Connect and retry.", "error");
      }
    } else {
      // For local/SSH installs, just switch to the instance
      openTab("local");
    }
  }, [
    upsertDockerInstance,
    openTab,
    refreshHosts,
    refreshRegisteredInstances,
    navigateRoute,
    registeredInstances,
    scheduleEnsureAccessForInstance,
    syncRemoteAuthAfterConnect,
    showToast,
    t,
  ]);

  const navItems: { key: string; active: boolean; icon: React.ReactNode; label: string; badge?: React.ReactNode; onClick: () => void }[] = inStart
    ? [
      {
        key: "start-profiles",
        active: startSection === "profiles",
        icon: <KeyRoundIcon className="size-4" />,
        label: t("start.nav.profiles"),
        onClick: () => { navigateRoute("home"); setStartSection("profiles"); },
      },
      {
        key: "start-doctor",
        active: route === "doctor",
        icon: <StethoscopeIcon className="size-4" />,
        label: t("nav.doctor"),
        badge: doctorNavPulse ? <span className="ml-auto h-2 w-2 rounded-full bg-primary animate-pulse" /> : undefined,
        onClick: () => {
          openDoctor();
        },
      },
      {
        key: "start-settings",
        active: startSection === "settings",
        icon: <SettingsIcon className="size-4" />,
        label: t("start.nav.settings"),
        onClick: () => { navigateRoute("home"); setStartSection("settings"); },
      },
    ]
    : [
      {
        key: "instance-home",
        active: route === "home",
        icon: <HomeIcon className="size-4" />,
        label: t("nav.home"),
        onClick: () => navigateRoute("home"),
      },
      {
        key: "channels",
        active: route === "channels",
        icon: <HashIcon className="size-4" />,
        label: t("nav.channels"),
        onClick: () => navigateRoute("channels"),
      },
      {
        key: "recipes",
        active: route === "recipes",
        icon: <BookOpenIcon className="size-4" />,
        label: t("nav.recipes"),
        onClick: () => navigateRoute("recipes"),
      },
      {
        key: "cron",
        active: route === "cron",
        icon: <ClockIcon className="size-4" />,
        label: t("nav.cron"),
        badge: hasEscalatedCron ? <span className="ml-auto w-2 h-2 rounded-full bg-red-500 animate-pulse" /> : undefined,
        onClick: () => navigateRoute("cron"),
      },
      {
        key: "doctor",
        active: route === "doctor",
        icon: <StethoscopeIcon className="size-4" />,
        label: t("nav.doctor"),
        onClick: () => {
          openDoctor();
        },
        badge: doctorNavPulse
          ? <span className="ml-auto h-2 w-2 rounded-full bg-primary animate-pulse" />
          : undefined,
      },
      {
        key: "history",
        active: route === "history",
        icon: <HistoryIcon className="size-4" />,
        label: t("nav.history"),
        onClick: () => navigateRoute("history"),
      },
    ];

  return (
    <>
    <div className="flex flex-col h-screen bg-background text-foreground">
      <InstanceTabBar
        openTabs={openTabs}
        activeId={inStart ? null : activeInstance}
        startActive={inStart}
        connectionStatus={connectionStatus}
        appVersion={appVersion}
        onSelectStart={openControlCenter}
        onSelect={handleInstanceSelect}
        onClose={closeTab}
      />
      <InstanceContext.Provider value={{
        instanceId: activeInstance,
        instanceToken,
        isRemote,
        isDocker,
        isConnected,
        channelNodes,
        discordGuildChannels,
        channelsLoading,
        discordChannelsLoading,
        refreshChannelNodesCache,
        refreshDiscordChannelsCache,
      }}>
      <div className="flex flex-1 overflow-hidden">

      {/* ── Sidebar ── */}
      {showSidebar && (
      <aside className="w-[220px] min-w-[220px] bg-sidebar border-r border-sidebar-border flex flex-col py-5">
        <div className="px-5 mb-6 flex items-center gap-2.5">
          <img src={logoUrl} alt="" className="w-9 h-9 rounded-xl shadow-sm" />
          <h1 className="text-xl font-bold tracking-tight" style={{ fontFamily: "'Fraunces', Georgia, serif" }}>
            ClawPal
          </h1>
        </div>

        <nav className="flex flex-col gap-0.5 px-3 flex-1">
          {navItems.map((item) => (
              <button
                key={item.key}
                className={cn(
                  "flex items-center gap-2.5 px-3 py-2 rounded-lg text-sm font-medium transition-all duration-200 cursor-pointer",
                  item.active
                    ? "bg-primary/10 text-primary shadow-sm"
                    : "text-muted-foreground hover:bg-accent hover:text-accent-foreground"
                )}
                onClick={item.onClick}
              >
                {item.icon}
                <span>{item.label}</span>
                {item.badge}
              </button>
          ))}

          <div className="my-3 h-px bg-border/60" />

        </nav>

        <div className="px-5 pb-3 flex items-center gap-2 text-xs text-muted-foreground/70">
          <a
            href="#"
            className="hover:text-foreground transition-colors duration-200"
            onClick={(e) => { e.preventDefault(); api.openUrl("https://clawpal.xyz"); }}
          >
            {t('nav.website')}
          </a>
          <span className="text-border">·</span>
          <a
            href="#"
            className="hover:text-foreground transition-colors duration-200"
            onClick={(e) => { e.preventDefault(); api.openUrl("https://x.com/zhixianio"); }}
          >
            @zhixian
          </a>
        </div>
        <div className="px-5 pb-3 text-[11px] text-muted-foreground/80">
          <div className="flex items-center gap-1.5">
            <span
              className={cn(
                "inline-block h-1.5 w-1.5 rounded-full",
                profileSyncStatus.phase === "syncing" && "bg-amber-500 animate-pulse",
                profileSyncStatus.phase === "success" && "bg-green-500",
                profileSyncStatus.phase === "error" && "bg-red-500",
                profileSyncStatus.phase === "idle" && "bg-muted-foreground/40",
              )}
            />
            <span>
              {profileSyncStatus.phase === "idle"
                ? "模型同步：待命"
                : profileSyncStatus.phase === "syncing"
                  ? `模型同步中：${profileSyncStatus.instanceId || "当前实例"}`
                  : profileSyncStatus.phase === "success"
                    ? `模型同步成功：${profileSyncStatus.instanceId || "当前实例"}`
                    : `模型同步失败：${profileSyncStatus.instanceId || "当前实例"}`}
            </span>
          </div>
          {profileSyncStatus.message && (
            <div className="mt-1 break-words text-muted-foreground/70" title={profileSyncStatus.message}>
              {profileSyncStatus.message}
            </div>
          )}
        </div>

        {!inStart && (
          <Suspense fallback={null}>
            <PendingChangesBar
              showToast={showToast}
              onApplied={bumpConfigVersion}
            />
          </Suspense>
        )}
      </aside>
      )}

      {/* ── Main Content ── */}
      <main className="flex-1 overflow-y-auto p-6 relative">
        {/* Chat toggle — floating pill (instance mode only) */}
        {!inStart && !chatOpen && (
          <button
            className="absolute top-5 right-5 z-10 flex items-center gap-2 px-3.5 py-2 rounded-full bg-primary/10 text-primary text-sm font-medium hover:bg-primary/15 transition-all duration-200 shadow-sm cursor-pointer"
            onClick={() => setChatOpen(true)}
          >
            <MessageCircleIcon className="size-4" />
            {t('nav.chat')}
          </button>
        )}

        <div className="animate-warm-enter">
          <Suspense fallback={<p className="text-sm text-muted-foreground animate-pulse">Loading…</p>}>
          {/* ── Start mode content ── */}
          {inStart && startSection === "overview" && (
            <StartPage
              dockerInstances={dockerInstances}
              sshHosts={sshHosts}
              registeredInstances={registeredInstances}
              openTabIds={new Set(openTabIds)}
              connectRemoteHost={connectWithPassphraseFallback}
              onOpenInstance={openTab}
              onRenameDocker={renameDockerInstance}
              onDeleteDocker={deleteDockerInstance}
              onDeleteSsh={(hostId) => {
                withGuidance(
                  () => api.deleteSshHost(hostId),
                  "deleteSshHost",
                  hostId,
                  "remote_ssh",
                ).then(() => {
                  closeTab(hostId);
                  refreshHosts();
                  refreshRegisteredInstances();
                }).catch((e) => console.warn("deleteSshHost:", e));
              }}
              onEditSsh={handleEditSsh}
              onInstallReady={handleInstallReady}
              showToast={showToast}
              onNavigate={(r) => navigateRoute(r as Route)}
              onOpenDoctor={openDoctor}
              discoveredInstances={discoveredInstances}
              discoveringInstances={discoveringInstances}
              onConnectDiscovered={handleConnectDiscovered}
            />
          )}
          {inStart && startSection === "profiles" && (
            <Settings
              key="global-profiles"
              globalMode
              section="profiles"
              onOpenDoctor={openDoctor}
              onDataChange={bumpConfigVersion}
            />
          )}
          {inStart && startSection === "settings" && (
            <Settings
              key="global-settings"
              globalMode
              section="preferences"
              onOpenDoctor={openDoctor}
              onDataChange={bumpConfigVersion}
              hasAppUpdate={appUpdateAvailable}
              onAppUpdateSeen={() => setAppUpdateAvailable(false)}
              onNavigateToProfiles={() => setStartSection("profiles")}
            />
          )}

          {/* ── Instance mode content ── */}
          {!inStart && route === "home" && (
            <Home
              key={`home-${configVersion}`}
              instanceLabel={openTabs.find((t) => t.id === activeInstance)?.label || activeInstance}
              showToast={showToast}
              onNavigate={(r) => navigateRoute(r as Route)}
            />
          )}
          {!inStart && route === "recipes" && (
            <Recipes
              onCook={(id, source) => {
                setRecipeId(id);
                setRecipeSource(source);
                navigateRoute("cook");
              }}
            />
          )}
          {!inStart && route === "cook" && recipeId && (
            <Cook
              recipeId={recipeId}
              recipeSource={recipeSource}
              onDone={() => {
                navigateRoute("recipes");
              }}
            />
          )}
          {!inStart && route === "cook" && !recipeId && <p>{t('config.noRecipeSelected')}</p>}
          {!inStart && route === "channels" && (
            <Channels
              key={`channels-${configVersion}`}
              showToast={showToast}
            />
          )}
          {!inStart && route === "cron" && <Cron />}
          {!inStart && route === "history" && <History key={`history-${configVersion}`} />}
          {!inStart && route === "doctor" && (
            <Doctor
              key={activeInstance}
              active
              connectRemoteHost={connectWithPassphraseFallback}
              launchGuidance={doctorLaunchByInstance[activeInstance] || null}
              onLaunchGuidanceConsumed={(instanceId) => {
                setDoctorLaunchByInstance((prev) => ({
                  ...prev,
                  [instanceId]: null,
                }));
              }}
            />
          )}
          {!inStart && route === "orchestrator" && <Orchestrator />}
          </Suspense>
        </div>
      </main>

      {/* ── Chat Panel (instance mode only) ── */}
      {!inStart && chatOpen && (
        <aside className="w-[380px] min-w-[380px] border-l border-border flex flex-col bg-card">
          <div className="flex items-center justify-between px-5 pt-5 pb-3">
            <h2 className="text-lg font-semibold">{t('nav.chat')}</h2>
            <Button
              variant="ghost"
              size="icon-xs"
              onClick={() => setChatOpen(false)}
            >
              <XIcon className="size-4" />
            </Button>
          </div>
          <div className="flex-1 overflow-hidden px-5 pb-5">
            <Suspense fallback={<p className="text-sm text-muted-foreground animate-pulse">Loading…</p>}>
              <Chat />
            </Suspense>
          </div>
        </aside>
      )}
      </div>
      </InstanceContext.Provider>
    </div>

    {/* ── Toast Stack ── */}
    {toasts.length > 0 && (
      <div className="fixed bottom-5 right-5 z-50 flex flex-col-reverse gap-2.5">
        {toasts.map((toast) => (
          <div
            key={toast.id}
            className={cn(
              "flex items-center gap-3 px-4 py-3 rounded-xl text-sm font-medium animate-in fade-in slide-in-from-bottom-3 duration-300",
              toast.type === "success"
                ? "bg-green-500/10 text-green-700 border border-green-500/20 shadow-sm dark:bg-green-500/15 dark:text-green-400 dark:border-green-500/20"
                : "bg-red-500/10 text-red-700 border border-red-500/20 shadow-sm dark:bg-red-500/15 dark:text-red-400 dark:border-red-500/20"
            )}
          >
            <span className="flex-1">{toast.message}</span>
            <button
              className="opacity-50 hover:opacity-100 transition-opacity ml-1 cursor-pointer"
              onClick={() => dismissToast(toast.id)}
            >
              <XIcon className="size-3.5" />
            </button>
          </div>
        ))}
      </div>
    )}

    {agentGuidance && (
      <div className="fixed bottom-5 right-5 z-[60] flex flex-col items-end gap-2">
        {agentGuidanceOpen && (
          <GuidanceCard
            guidance={agentGuidance}
            instanceLabel={
              openTabs.find((tab) => tab.id === agentGuidance.instanceId)?.label
              || fallbackInstanceLabel(agentGuidance.instanceId, t)
            }
            onClose={() => setAgentGuidanceOpen(false)}
            onDismiss={() => { setAgentGuidanceOpen(false); setUnreadGuidance(false); }}
            onResolve={() => resolveGuidanceForInstance(agentGuidance.instanceId)}
            onDoctorHandoff={(context) => {
              setAgentGuidanceOpen(false);
              setDoctorLaunchByInstance((prev) => ({
                ...prev,
                [agentGuidance.instanceId]: {
                  ...agentGuidance,
                  rawError: context || agentGuidance.rawError,
                },
              }));
              // Ensure the correct instance tab is active so Doctor
              // runs commands against the right target.
              const gid = agentGuidance.instanceId;
              setOpenTabIds((prev) => prev.includes(gid) ? prev : [...prev, gid]);
              setActiveInstance(gid);
              setInStart(false);
              navigateRoute("doctor");
            }}
            onInlineFix={async (sa) => {
              try {
                if (sa.tool === "clawpal" && sa.args?.includes("ssh connect")) {
                  const hostId = agentGuidance.instanceId;
                  showToast(`正在重连 SSH...`, "success");
                  await connectWithPassphraseFallback(hostId);
                  showToast("SSH 重连成功", "success");
                  resolveGuidanceForInstance(hostId);
                } else {
                  setAgentGuidanceOpen(false);
                  setDoctorLaunchByInstance((prev) => ({
                    ...prev,
                    [agentGuidance.instanceId]: {
                      ...agentGuidance,
                      rawError: sa.context || agentGuidance.rawError,
                    },
                  }));
                  const gid = agentGuidance.instanceId;
                  setOpenTabIds((prev) => prev.includes(gid) ? prev : [...prev, gid]);
                  setActiveInstance(gid);
                  setInStart(false);
                  navigateRoute("doctor");
                }
              } catch (e) {
                showToast(`${sa.label} 失败: ${e}`, "error");
              }
            }}
          />
        )}
        <Button
          className="rounded-full shadow-md relative"
          size="sm"
          variant={agentGuidanceOpen ? "secondary" : "default"}
          onClick={() => {
            setAgentGuidanceOpen((v) => !v);
            setUnreadGuidance(false);
          }}
        >
          小龙虾
          {unreadGuidance && !agentGuidanceOpen && (
            <span className="absolute -top-1 -right-1 size-2.5 rounded-full bg-destructive" />
          )}
        </Button>
      </div>
    )}

    <Dialog
      open={passphraseOpen}
      onOpenChange={(open) => {
        if (!open) closePassphraseDialog(null);
      }}
    >
      <DialogContent>
        <DialogHeader>
          <DialogTitle>{t("ssh.passphraseTitle")}</DialogTitle>
        </DialogHeader>
        <div className="space-y-2">
          <p className="text-sm text-muted-foreground">
            {t("ssh.passphrasePrompt", { host: passphraseHostLabel })}
          </p>
          <Label htmlFor="ssh-passphrase">{t("ssh.passphraseLabel")}</Label>
          <Input
            id="ssh-passphrase"
            type="password"
            value={passphraseInput}
            onChange={(e) => setPassphraseInput(e.target.value)}
            placeholder={t("ssh.passphrasePlaceholder")}
            autoFocus
            onKeyDown={(e) => {
              if (e.key === "Enter") {
                closePassphraseDialog(passphraseInput);
              }
            }}
          />
        </div>
        <DialogFooter>
          <Button variant="outline" onClick={() => closePassphraseDialog(null)}>
            {t("instance.cancel")}
          </Button>
          <Button onClick={() => closePassphraseDialog(passphraseInput)}>
            {t("ssh.passphraseConfirm")}
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
    <Dialog open={sshEditOpen} onOpenChange={setSshEditOpen}>
      <DialogContent>
        <DialogHeader>
          <DialogTitle>{t("instance.editSsh")}</DialogTitle>
        </DialogHeader>
        {editingSshHost && (
          <SshFormWidget
            invokeId="ssh-edit-form"
            defaults={editingSshHost}
            onSubmit={(_invokeId, host) => {
              handleSshEditSave({ ...host, id: editingSshHost.id });
            }}
            onCancel={() => setSshEditOpen(false)}
          />
        )}
      </DialogContent>
    </Dialog>
    <Toaster position="top-right" richColors />
    </>
  );
}
