import { useCallback, useEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { check } from "@tauri-apps/plugin-updater";
import { getVersion } from "@tauri-apps/api/app";
import {
  HomeIcon,
  BookOpenIcon,
  HashIcon,
  ClockIcon,
  HistoryIcon,
  StethoscopeIcon,
  LayersIcon,
  SettingsIcon,
  MessageCircleIcon,
  XIcon,
} from "lucide-react";
import { Home } from "./pages/Home";
import { Recipes } from "./pages/Recipes";
import { Cook } from "./pages/Cook";
import { History } from "./pages/History";
import { Settings } from "./pages/Settings";
import { Doctor } from "./pages/Doctor";
import { Sessions } from "./pages/Sessions";
import { Channels } from "./pages/Channels";
import { Cron } from "./pages/Cron";
import { Chat } from "./components/Chat";
import logoUrl from "./assets/logo.png";
import { PendingChangesBar } from "./components/PendingChangesBar";
import { InstanceTabBar } from "./components/InstanceTabBar";
import { InstanceContext } from "./lib/instance-context";
import { api } from "./lib/api";
import { Button } from "@/components/ui/button";
import { cn } from "@/lib/utils";
import type { DiscordGuildChannel, SshHost } from "./lib/types";

const PING_URL = "https://api.clawpal.zhixian.io/ping";

type Route = "home" | "recipes" | "cook" | "history" | "channels" | "cron" | "doctor" | "sessions" | "settings";

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

export function App() {
  const { t } = useTranslation();
  const [route, setRoute] = useState<Route>("home");
  const [recipeId, setRecipeId] = useState<string | null>(null);
  const [recipeSource, setRecipeSource] = useState<string | undefined>(undefined);
  const [discordGuildChannels, setDiscordGuildChannels] = useState<DiscordGuildChannel[]>([]);
  const [chatOpen, setChatOpen] = useState(false);

  // SSH remote instance state
  const [activeInstance, setActiveInstance] = useState("local");
  const [sshHosts, setSshHosts] = useState<SshHost[]>([]);
  const [connectionStatus, setConnectionStatus] = useState<Record<string, "connected" | "disconnected" | "error">>({});

  const refreshHosts = useCallback(() => {
    api.listSshHosts().then(setSshHosts).catch((e) => console.error("Failed to load SSH hosts:", e));
  }, []);

  useEffect(() => {
    refreshHosts();
  }, [refreshHosts]);

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

  const showToast = useCallback((message: string, type: "success" | "error" = "success") => {
    const id = ++toastIdCounter;
    setToasts((prev) => [...prev, { id, message, type }]);
    setTimeout(() => {
      setToasts((prev) => prev.filter((t) => t.id !== id));
    }, type === "error" ? 5000 : 3000);
  }, []);

  const dismissToast = useCallback((id: number) => {
    setToasts((prev) => prev.filter((t) => t.id !== id));
  }, []);


  const handleInstanceSelect = useCallback((id: string) => {
    setActiveInstance(id);
    if (id !== "local") {
      // Check if backend still has a live connection before reconnecting.
      // Do not pre-mark as disconnected — transient status failures would
      // otherwise gray out the whole remote UI.
      api.sshStatus(id)
        .then((status) => {
          if (status === "connected") {
            setConnectionStatus((prev) => ({ ...prev, [id]: "connected" }));
          } else {
            return api.sshConnect(id)
              .then(() => setConnectionStatus((prev) => ({ ...prev, [id]: "connected" })));
          }
        })
        .catch((e) => {
          // sshStatus failed or reconnect failed — try fresh connect
          api.sshConnect(id)
            .then(() => setConnectionStatus((prev) => ({ ...prev, [id]: "connected" })))
            .catch((e2) => {
              setConnectionStatus((prev) => ({ ...prev, [id]: "error" }));
              const raw = String(e2);
              const friendly = friendlySshError(raw, t);
              showToast(friendly, "error");
            });
        });
    }
  }, [showToast, t]);

  const [configVersion, setConfigVersion] = useState(0);

  const isRemote = activeInstance !== "local";
  const isConnected = !isRemote || connectionStatus[activeInstance] === "connected";

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

  // Load Discord data + extract profiles on startup or connection ready
  useEffect(() => {
    if (activeInstance === "local") {
      if (!localStorage.getItem("clawpal_profiles_extracted")) {
        api.extractModelProfilesFromConfig()
          .then(() => localStorage.setItem("clawpal_profiles_extracted", "1"))
          .catch((e) => console.error("Failed to extract model profiles:", e));
      }
      api.listDiscordGuildChannels().then(setDiscordGuildChannels).catch((e) => console.error("Failed to load Discord channels:", e));
    } else if (isConnected) {
      api.remoteExtractModelProfilesFromConfig(activeInstance)
        .catch((e) => console.error("Failed to extract remote model profiles:", e));
      api.remoteListDiscordGuildChannels(activeInstance).then(setDiscordGuildChannels).catch((e) => console.error("Failed to load remote Discord channels:", e));
    }
  }, [activeInstance, isConnected]);

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


  const navItems: { route: Route | Route[]; icon: React.ReactNode; label: string; badge?: React.ReactNode }[] = [
    { route: "home", icon: <HomeIcon className="size-4" />, label: t('nav.home') },
    { route: ["recipes", "cook"] as Route[], icon: <BookOpenIcon className="size-4" />, label: t('nav.recipes') },
    { route: "channels", icon: <HashIcon className="size-4" />, label: t('nav.channels') },
    {
      route: "cron",
      icon: <ClockIcon className="size-4" />,
      label: t('nav.cron'),
      badge: hasEscalatedCron ? <span className="ml-auto w-2 h-2 rounded-full bg-red-500 animate-pulse" /> : undefined,
    },
    { route: "history", icon: <HistoryIcon className="size-4" />, label: t('nav.history') },
    { route: "doctor", icon: <StethoscopeIcon className="size-4" />, label: t('nav.doctor') },
    { route: "sessions", icon: <LayersIcon className="size-4" />, label: t('nav.sessions') },
  ];

  const isRouteActive = (item: typeof navItems[0]) => {
    if (Array.isArray(item.route)) return item.route.includes(route);
    return route === item.route;
  };

  return (
    <>
    <div className="flex flex-col h-screen bg-background text-foreground">
      <InstanceTabBar
        hosts={sshHosts}
        activeId={activeInstance}
        connectionStatus={connectionStatus}
        onSelect={handleInstanceSelect}
        onHostsChange={refreshHosts}
      />
      <InstanceContext.Provider value={{ instanceId: activeInstance, isRemote, isConnected, discordGuildChannels }}>
      <div className="flex flex-1 overflow-hidden">

      {/* ── Sidebar ── */}
      <aside className="w-[220px] min-w-[220px] bg-sidebar border-r border-sidebar-border flex flex-col py-5">
        <div className="px-5 mb-6 flex items-center gap-2.5">
          <img src={logoUrl} alt="" className="w-9 h-9 rounded-xl shadow-sm" />
          <h1 className="text-xl font-bold tracking-tight" style={{ fontFamily: "'Fraunces', Georgia, serif" }}>
            ClawPal
          </h1>
        </div>

        <nav className="flex flex-col gap-0.5 px-3 flex-1">
          {navItems.map((item) => {
            const active = isRouteActive(item);
            const targetRoute = Array.isArray(item.route) ? item.route[0] : item.route;
            return (
              <button
                key={targetRoute}
                className={cn(
                  "flex items-center gap-2.5 px-3 py-2 rounded-lg text-sm font-medium transition-all duration-200 cursor-pointer",
                  active
                    ? "bg-primary/10 text-primary shadow-sm"
                    : "text-muted-foreground hover:bg-accent hover:text-accent-foreground"
                )}
                onClick={() => setRoute(targetRoute)}
              >
                {item.icon}
                <span>{item.label}</span>
                {item.badge}
              </button>
            );
          })}

          <div className="my-3 h-px bg-border/60" />

          <button
            className={cn(
              "flex items-center gap-2.5 px-3 py-2 rounded-lg text-sm font-medium transition-all duration-200 cursor-pointer",
              route === "settings"
                ? "bg-primary/10 text-primary shadow-sm"
                : "text-muted-foreground hover:bg-accent hover:text-accent-foreground"
            )}
            onClick={() => setRoute("settings")}
          >
            <SettingsIcon className="size-4" />
            <span>{t('nav.settings')}</span>
            {appUpdateAvailable && (
              <span className="ml-auto w-2 h-2 rounded-full bg-destructive animate-pulse" />
            )}
          </button>
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

        <PendingChangesBar
          showToast={showToast}
          onApplied={bumpConfigVersion}
        />
      </aside>

      {/* ── Main Content ── */}
      <main className="flex-1 overflow-y-auto p-6 relative">
        {/* Chat toggle — floating pill */}
        {!chatOpen && (
          <button
            className="absolute top-5 right-5 z-10 flex items-center gap-2 px-3.5 py-2 rounded-full bg-primary/10 text-primary text-sm font-medium hover:bg-primary/15 transition-all duration-200 shadow-sm cursor-pointer"
            onClick={() => setChatOpen(true)}
          >
            <MessageCircleIcon className="size-4" />
            {t('nav.chat')}
          </button>
        )}

        <div className="animate-warm-enter">
          {route === "home" && (
            <Home
              key={`${activeInstance}-${configVersion}`}
              onCook={(id, source) => {
                setRecipeId(id);
                setRecipeSource(source);
                setRoute("cook");
              }}
              showToast={showToast}
              onNavigate={(r) => setRoute(r as Route)}
            />
          )}
          {route === "recipes" && (
            <Recipes
              onCook={(id, source) => {
                setRecipeId(id);
                setRecipeSource(source);
                setRoute("cook");
              }}
            />
          )}
          {route === "cook" && recipeId && (
            <Cook
              recipeId={recipeId}
              recipeSource={recipeSource}
              onDone={() => {
                setRoute("recipes");
              }}
            />
          )}
          {route === "cook" && !recipeId && <p>{t('config.noRecipeSelected')}</p>}
          {route === "channels" && (
            <Channels
              key={`${activeInstance}-${configVersion}`}
              showToast={showToast}
            />
          )}
          {route === "cron" && <Cron key={`${activeInstance}`} />}
          {route === "history" && <History key={`${activeInstance}-${configVersion}`} />}
          <div className={route === "doctor" ? undefined : "hidden"}><Doctor sshHosts={sshHosts} /></div>
          {route === "sessions" && <Sessions />}
          {route === "settings" && (
            <Settings
              key={`${activeInstance}-${configVersion}`}
              onDataChange={bumpConfigVersion}
              hasAppUpdate={appUpdateAvailable}
              onAppUpdateSeen={() => setAppUpdateAvailable(false)}
            />
          )}
        </div>
      </main>

      {/* ── Chat Panel ── */}
      {chatOpen && (
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
