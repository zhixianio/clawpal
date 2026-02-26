import { useCallback, useEffect, useMemo, useRef, useState } from "react";
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
import { Home } from "./pages/Home";
import { StartPage } from "./pages/StartPage";
import { Recipes } from "./pages/Recipes";
import { Cook } from "./pages/Cook";
import { History } from "./pages/History";
import { Settings } from "./pages/Settings";
import { Doctor } from "./pages/Doctor";
import { Channels } from "./pages/Channels";
import { Cron } from "./pages/Cron";
import { Orchestrator } from "./pages/Orchestrator";
import { Chat } from "./components/Chat";
import logoUrl from "./assets/logo.png";
import { PendingChangesBar } from "./components/PendingChangesBar";
import { InstanceTabBar } from "./components/InstanceTabBar";
import { InstanceContext } from "./lib/instance-context";
import { api } from "./lib/api";
import { Button } from "@/components/ui/button";
import { cn } from "@/lib/utils";
import type { DiscordGuildChannel, DockerInstance, InstallSession, SshHost } from "./lib/types";

const PING_URL = "https://api.clawpal.zhixian.io/ping";
const DOCKER_INSTANCES_KEY = "clawpal_docker_instances";
const DEFAULT_DOCKER_OPENCLAW_HOME = "~/.clawpal/docker-local";
const DEFAULT_DOCKER_CLAWPAL_DATA_DIR = "~/.clawpal/docker-local/data";
const DEFAULT_DOCKER_INSTANCE_ID = "docker:local";

type Route = "home" | "recipes" | "cook" | "history" | "channels" | "cron" | "doctor" | "orchestrator";
const INSTANCE_ROUTES: Route[] = ["home", "channels", "recipes", "cron", "doctor", "history"];
const OPEN_TABS_STORAGE_KEY = "clawpal_open_tabs";

interface ToastItem {
  id: number;
  message: string;
  type: "success" | "error";
}

let toastIdCounter = 0;

const SSH_ERROR_MAP: Array<[RegExp, string]> = [
  [/connection refused/i, "ssh.errorConnectionRefused"],
  [/no such file/i, "ssh.errorNoSuchFile"],
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
  if (instanceId === DEFAULT_DOCKER_INSTANCE_ID) return "Docker Local";
  const suffix = instanceId.startsWith("docker:") ? instanceId.slice(7) : instanceId;
  const match = suffix.match(/^local-(\d+)$/);
  if (match) return `Docker Local ${match[1]}`;
  return `Docker ${suffix}`;
}

function hashInstanceToken(raw: string): number {
  let hash = 2166136261;
  for (let i = 0; i < raw.length; i += 1) {
    hash ^= raw.charCodeAt(i);
    hash = Math.imul(hash, 16777619);
  }
  return hash >>> 0;
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
  const [route, setRoute] = useState<Route>("home");
  const [recipeId, setRecipeId] = useState<string | null>(null);
  const [recipeSource, setRecipeSource] = useState<string | undefined>(undefined);
  const [discordGuildChannels, setDiscordGuildChannels] = useState<DiscordGuildChannel[]>([]);
  const [chatOpen, setChatOpen] = useState(false);
  const [lastInstanceRoute, setLastInstanceRoute] = useState<Route>("channels");
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
  const [dockerInstances, setDockerInstances] = useState<DockerInstance[]>([]);
  const [sshHosts, setSshHosts] = useState<SshHost[]>([]);
  const [connectionStatus, setConnectionStatus] = useState<Record<string, "connected" | "disconnected" | "error">>({});

  const refreshHosts = useCallback(() => {
    api.listSshHosts().then(setSshHosts).catch((e) => console.error("Failed to load SSH hosts:", e));
  }, []);

  const refreshDockerInstances = useCallback(async () => {
    try {
      const raw = localStorage.getItem(DOCKER_INSTANCES_KEY);
      if (!raw) {
        setDockerInstances([]);
        return;
      }
      const parsed = JSON.parse(raw) as DockerInstance[];
      const normalized: DockerInstance[] = [];
      const seen = new Set<string>();
      for (const item of Array.isArray(parsed) ? parsed : []) {
        if (!item?.id || typeof item.id !== "string") continue;
        const id = item.id.trim();
        if (!id || seen.has(id)) continue;
        seen.add(id);
        normalized.push(normalizeDockerInstance({ ...item, id }));
      }
      const checked = await Promise.all(
        normalized.map(async (item) => {
          try {
            const exists = await api.localOpenclawConfigExists(item.openclawHome || "");
            return exists ? item : null;
          } catch {
            // If probe fails unexpectedly, keep the tab instead of hiding it.
            return item;
          }
        }),
      );
      const next = checked.filter((item): item is DockerInstance => item !== null);
      localStorage.setItem(DOCKER_INSTANCES_KEY, JSON.stringify(next));
      setDockerInstances(next);
      setActiveInstance((prev) => {
        if (!prev.startsWith("docker:")) return prev;
        return next.some((item) => item.id === prev) ? prev : "local";
      });
    } catch {
      setDockerInstances([]);
      setActiveInstance((prev) => (prev.startsWith("docker:") ? "local" : prev));
    }
  }, []);

  const upsertDockerInstance = useCallback((instance: DockerInstance) => {
    const normalized = normalizeDockerInstance(instance);
    setDockerInstances((prev) => {
      const next = [...prev];
      const idx = next.findIndex((item) => item.id === normalized.id);
      if (idx >= 0) next[idx] = normalized;
      else next.push(normalized);
      localStorage.setItem(DOCKER_INSTANCES_KEY, JSON.stringify(next));
      return next;
    });
  }, []);

  const renameDockerInstance = useCallback((id: string, label: string) => {
    const nextLabel = label.trim();
    if (!nextLabel) return;
    setDockerInstances((prev) => {
      const next = prev.map((item) => (
        item.id === id
          ? { ...item, label: nextLabel }
          : item
      ));
      localStorage.setItem(DOCKER_INSTANCES_KEY, JSON.stringify(next));
      return next;
    });
  }, []);

  const deleteDockerInstance = useCallback(async (instance: DockerInstance, deleteLocalData: boolean) => {
    const fallback = deriveDockerPaths(instance.id);
    const openclawHome = instance.openclawHome || fallback.openclawHome;
    if (deleteLocalData) {
      await api.deleteLocalInstanceHome(openclawHome);
    }
    setDockerInstances((prev) => {
      const next = prev.filter((item) => item.id !== instance.id);
      localStorage.setItem(DOCKER_INSTANCES_KEY, JSON.stringify(next));
      return next;
    });
    setActiveInstance((prev) => (prev === instance.id ? "local" : prev));
  }, []);

  useEffect(() => {
    refreshHosts();
    void refreshDockerInstances();
  }, [refreshHosts, refreshDockerInstances]);

  const [appUpdateAvailable, setAppUpdateAvailable] = useState(false);
  const [hasEscalatedCron, setHasEscalatedCron] = useState(false);

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
      .catch(() => {});

    // Analytics ping (fire-and-forget)
    getVersion().then((version) => {
      const url = PING_URL;
      if (!url) return;
      fetch(url, {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify({ v: version, id: installId, platform: navigator.platform }),
      }).catch(() => {});
    }).catch(() => {});
  }, []);

  const [toasts, setToasts] = useState<ToastItem[]>([]);
  const sshHealthFailStreakRef = useRef<Record<string, number>>({});
  const accessProbeTimerRef = useRef<ReturnType<typeof setTimeout> | null>(null);
  const lastAccessProbeAtRef = useRef<Record<string, number>>({});

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

  const resolveInstanceTransport = useCallback((instanceId: string) => {
    if (instanceId === "local") return "local";
    if (dockerInstances.some((item) => item.id === instanceId)) return "docker_local";
    if (sshHosts.some((host) => host.id === instanceId)) return "remote_ssh";
    // Unknown id should not be treated as remote by default.
    return "local";
  }, [dockerInstances, sshHosts]);

  const ensureAccessForInstance = useCallback((instanceId: string) => {
    const transport = resolveInstanceTransport(instanceId);
    api.ensureAccessProfile(instanceId, transport).catch((e) => {
      console.warn("ensure_access_profile failed:", e);
    });
  }, [resolveInstanceTransport]);

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

  useEffect(() => {
    return () => {
      if (accessProbeTimerRef.current !== null) {
        clearTimeout(accessProbeTimerRef.current);
        accessProbeTimerRef.current = null;
      }
    };
  }, []);

  const dismissToast = useCallback((id: number) => {
    setToasts((prev) => prev.filter((t) => t.id !== id));
  }, []);


  const openTab = useCallback((id: string) => {
    setOpenTabIds((prev) => prev.includes(id) ? prev : [...prev, id]);
    setActiveInstance(id);
    setInStart(false);
    setRoute(lastInstanceRoute);
  }, [lastInstanceRoute]);

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
    setActiveInstance(id);
    setOpenTabIds((prev) => prev.includes(id) ? prev : [...prev, id]);
    setInStart(false);
    if (inStart) {
      setRoute(lastInstanceRoute);
    }
    const transport = resolveInstanceTransport(id);
    if (transport !== "remote_ssh") {
      scheduleEnsureAccessForInstance(id);
      return;
    }
    // Check if backend still has a live connection before reconnecting.
    // Do not pre-mark as disconnected — transient status failures would
    // otherwise gray out the whole remote UI.
    api.sshStatus(id)
      .then((status) => {
        if (status === "connected") {
          setConnectionStatus((prev) => ({ ...prev, [id]: "connected" }));
          scheduleEnsureAccessForInstance(id, 1500);
        } else {
          return api.sshConnect(id)
            .then(() => {
              setConnectionStatus((prev) => ({ ...prev, [id]: "connected" }));
              scheduleEnsureAccessForInstance(id, 1500);
            });
        }
      })
      .catch(() => {
        // sshStatus failed or reconnect failed — try fresh connect
        api.sshConnect(id)
          .then(() => {
            setConnectionStatus((prev) => ({ ...prev, [id]: "connected" }));
            scheduleEnsureAccessForInstance(id, 1500);
          })
          .catch((e2) => {
            setConnectionStatus((prev) => ({ ...prev, [id]: "error" }));
            const raw = String(e2);
            const friendly = friendlySshError(raw, t);
            showToast(friendly, "error");
          });
      });
  }, [activeInstance, inStart, lastInstanceRoute, resolveInstanceTransport, scheduleEnsureAccessForInstance, showToast, t]);

  const [configVersion, setConfigVersion] = useState(0);
  const [instanceToken, setInstanceToken] = useState(0);

  const isDocker = dockerInstances.some((item) => item.id === activeInstance);
  const isRemote = sshHosts.some((host) => host.id === activeInstance);
  const isConnected = !isRemote || connectionStatus[activeInstance] === "connected";

  useEffect(() => {
    let nextHome: string | null = null;
    let nextDataDir: string | null = null;
    if (activeInstance === "local" || isRemote) {
      nextHome = null;
      nextDataDir = null;
    } else if (isDocker) {
      const instance = dockerInstances.find((item) => item.id === activeInstance);
      const fallback = deriveDockerPaths(activeInstance);
      nextHome = instance?.openclawHome || fallback.openclawHome;
      nextDataDir = instance?.clawpalDataDir || fallback.clawpalDataDir;
    }
    const tokenSeed = `${activeInstance}|${nextHome || ""}|${nextDataDir || ""}`;
    setInstanceToken(hashInstanceToken(tokenSeed));

    const applyOverrides = async () => {
      if (nextHome === null && nextDataDir === null) {
        await Promise.all([
          api.setActiveOpenclawHome(null).catch(() => {}),
          api.setActiveClawpalDataDir(null).catch(() => {}),
        ]);
      } else {
        await Promise.all([
          api.setActiveOpenclawHome(nextHome).catch(() => {}),
          api.setActiveClawpalDataDir(nextDataDir).catch(() => {}),
        ]);
      }
    };
    void applyOverrides();
  }, [activeInstance, isDocker, isRemote, dockerInstances]);

  // Keep active remote instance self-healed: detect dropped SSH and reconnect.
  useEffect(() => {
    if (!isRemote) return;
    let cancelled = false;
    let inFlight = false;
    const hostId = activeInstance;

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
          await api.sshConnect(hostId);
          if (!cancelled) {
            sshHealthFailStreakRef.current[hostId] = 0;
            setConnectionStatus((prev) => ({ ...prev, [hostId]: "connected" }));
          }
        } catch {
          if (!cancelled) {
            const streak = (sshHealthFailStreakRef.current[hostId] || 0) + 1;
            sshHealthFailStreakRef.current[hostId] = streak;
            // Avoid flipping UI to disconnected/error on a single transient failure.
            if (streak >= 2) {
              setConnectionStatus((prev) => ({ ...prev, [hostId]: "error" }));
            }
          }
        }
      } catch {
        if (!cancelled) {
          const streak = (sshHealthFailStreakRef.current[hostId] || 0) + 1;
          sshHealthFailStreakRef.current[hostId] = streak;
          if (streak >= 2) {
            setConnectionStatus((prev) => ({ ...prev, [hostId]: "error" }));
          }
        }
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
  }, [activeInstance, isRemote]);

  // Clear cached Discord channels only when switching instance.
  // Avoid clearing on transient connection-status changes, which causes
  // Channels page to flicker between "no cache" and loaded data.
  useEffect(() => {
    setDiscordGuildChannels([]);
  }, [activeInstance]);

  // Load Discord channel cache lazily when Channels tab is active.
  useEffect(() => {
    if (route !== "channels") return;
    if (activeInstance === "local" || isDocker) {
      api.listDiscordGuildChannels()
        .then(setDiscordGuildChannels)
        .catch((e) => console.error("Failed to load Discord channels:", e));
      return;
    }
    if (!isConnected) return;
    api.remoteListDiscordGuildChannels(activeInstance)
      .then(setDiscordGuildChannels)
      .catch((e) => console.error("Failed to load remote Discord channels:", e));
  }, [route, activeInstance, isConnected, isDocker]);

  // Poll watchdog status for escalated cron jobs (red dot badge)
  useEffect(() => {
    const check = () => {
      const p = isRemote
        ? api.remoteGetWatchdogStatus(activeInstance)
        : api.getWatchdogStatus();
      p.then((status: any) => {
        if (status?.jobs) {
          const escalated = Object.values(status.jobs).some((j: any) => j.status === "escalated");
          setHasEscalatedCron(escalated);
        } else {
          setHasEscalatedCron(false);
        }
      }).catch(() => setHasEscalatedCron(false));
    };
    check();
    const interval = setInterval(check, 30000);
    return () => clearInterval(interval);
  }, [activeInstance, isRemote]);

  const bumpConfigVersion = useCallback(() => {
    setConfigVersion((v) => v + 1);
  }, []);

  const openControlCenter = useCallback(() => {
    setInStart(true);
    setStartSection("overview");
  }, []);

  useEffect(() => {
    if (INSTANCE_ROUTES.includes(route)) {
      setLastInstanceRoute(route);
    }
  }, [route]);

  const showSidebar = true;

  // Derive openTabs array for InstanceTabBar
  const openTabs = useMemo(() => {
    return openTabIds.map((id) => {
      if (id === "local") return { id, label: t("instance.local"), type: "local" as const };
      const docker = dockerInstances.find((d) => d.id === id);
      if (docker) return { id, label: docker.label || id, type: "docker" as const };
      const ssh = sshHosts.find((h) => h.id === id);
      if (ssh) return { id, label: ssh.label || ssh.host, type: "ssh" as const };
      return { id, label: id, type: "local" as const };
    });
  }, [openTabIds, dockerInstances, sshHosts, t]);

  // Handle install completion — register docker instance and open tab
  const handleInstallReady = useCallback((session: InstallSession) => {
    if (session.method === "docker") {
      const artifacts = session.artifacts || {};
      const artifactId = typeof artifacts.docker_instance_id === "string"
        ? artifacts.docker_instance_id.trim()
        : "";
      const id = artifactId || DEFAULT_DOCKER_INSTANCE_ID;
      const fallback = deriveDockerPaths(id);
      const openclawHome = typeof artifacts.docker_openclaw_home === "string"
        ? artifacts.docker_openclaw_home
        : fallback.openclawHome;
      const clawpalDataDir = typeof artifacts.docker_clawpal_data_dir === "string"
        ? artifacts.docker_clawpal_data_dir
        : `${openclawHome}/data`;
      const label = typeof artifacts.docker_instance_label === "string"
        ? artifacts.docker_instance_label
        : deriveDockerLabel(id);
      upsertDockerInstance({ id, label, openclawHome, clawpalDataDir });
      openTab(id);
    } else {
      // For local/SSH installs, just switch to the instance
      openTab("local");
    }
  }, [upsertDockerInstance, openTab]);

  const navItems: { key: string; active: boolean; icon: React.ReactNode; label: string; badge?: React.ReactNode; onClick: () => void }[] = inStart
    ? [
      {
        key: "start-profiles",
        active: startSection === "profiles",
        icon: <KeyRoundIcon className="size-4" />,
        label: t("start.nav.profiles"),
        onClick: () => { setRoute("home"); setStartSection("profiles"); },
      },
      {
        key: "start-settings",
        active: startSection === "settings",
        icon: <SettingsIcon className="size-4" />,
        label: t("start.nav.settings"),
        onClick: () => { setRoute("home"); setStartSection("settings"); },
      },
    ]
    : [
      {
        key: "instance-home",
        active: route === "home",
        icon: <HomeIcon className="size-4" />,
        label: t("nav.home"),
        onClick: () => setRoute("home"),
      },
      {
        key: "channels",
        active: route === "channels",
        icon: <HashIcon className="size-4" />,
        label: t("nav.channels"),
        onClick: () => setRoute("channels"),
      },
      {
        key: "recipes",
        active: route === "recipes",
        icon: <BookOpenIcon className="size-4" />,
        label: t("nav.recipes"),
        onClick: () => setRoute("recipes"),
      },
      {
        key: "cron",
        active: route === "cron",
        icon: <ClockIcon className="size-4" />,
        label: t("nav.cron"),
        badge: hasEscalatedCron ? <span className="ml-auto w-2 h-2 rounded-full bg-red-500 animate-pulse" /> : undefined,
        onClick: () => setRoute("cron"),
      },
      {
        key: "doctor",
        active: route === "doctor",
        icon: <StethoscopeIcon className="size-4" />,
        label: t("nav.doctor"),
        onClick: () => setRoute("doctor"),
      },
      {
        key: "history",
        active: route === "history",
        icon: <HistoryIcon className="size-4" />,
        label: t("nav.history"),
        onClick: () => setRoute("history"),
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
        onSelectStart={openControlCenter}
        onSelect={handleInstanceSelect}
        onClose={closeTab}
      />
      <InstanceContext.Provider value={{ instanceId: activeInstance, instanceToken, isRemote, isDocker, isConnected, discordGuildChannels }}>
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
            onClick={(e) => { e.preventDefault(); api.openUrl("https://clawpal.zhixian.io"); }}
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

        {!inStart && (
          <PendingChangesBar
            showToast={showToast}
            onApplied={bumpConfigVersion}
          />
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
          {/* ── Start mode content ── */}
          {inStart && startSection === "overview" && (
            <StartPage
              dockerInstances={dockerInstances}
              sshHosts={sshHosts}
              openTabIds={new Set(openTabIds)}
              onOpenInstance={openTab}
              onRenameDocker={renameDockerInstance}
              onDeleteDocker={deleteDockerInstance}
              onDeleteSsh={(hostId) => {
                api.deleteSshHost(hostId).then(refreshHosts);
              }}
              onEditSsh={() => {}}
              onInstallReady={handleInstallReady}
              showToast={showToast}
              onNavigate={(r) => setRoute(r as Route)}
            />
          )}
          {inStart && startSection === "profiles" && (
            <Settings
              key="global-profiles"
              globalMode
              section="profiles"
              onDataChange={bumpConfigVersion}
            />
          )}
          {inStart && startSection === "settings" && (
            <Settings
              key="global-settings"
              globalMode
              section="preferences"
              onDataChange={bumpConfigVersion}
              hasAppUpdate={appUpdateAvailable}
              onAppUpdateSeen={() => setAppUpdateAvailable(false)}
            />
          )}

          {/* ── Instance mode content ── */}
          {!inStart && route === "home" && (
            <Home
              key={`home-${configVersion}`}
              instanceLabel={openTabs.find((t) => t.id === activeInstance)?.label || activeInstance}
              showToast={showToast}
              onNavigate={(r) => setRoute(r as Route)}
            />
          )}
          {!inStart && route === "recipes" && (
            <Recipes
              onCook={(id, source) => {
                setRecipeId(id);
                setRecipeSource(source);
                setRoute("cook");
              }}
            />
          )}
          {!inStart && route === "cook" && recipeId && (
            <Cook
              recipeId={recipeId}
              recipeSource={recipeSource}
              onDone={() => {
                setRoute("recipes");
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
          {!inStart && (
            <div className={route === "doctor" ? undefined : "hidden"}>
              <Doctor key={activeInstance} />
            </div>
          )}
          {!inStart && route === "orchestrator" && <Orchestrator />}
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
            <Chat />
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
    </>
  );
}
