import { useCallback, useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { check } from "@tauri-apps/plugin-updater";
import { getVersion } from "@tauri-apps/api/app";
import { Home } from "./pages/Home";
import { Recipes } from "./pages/Recipes";
import { Cook } from "./pages/Cook";
import { History } from "./pages/History";
import { Settings } from "./pages/Settings";
import { Doctor } from "./pages/Doctor";
import { Channels } from "./pages/Channels";
import { Cron } from "./pages/Cron";
import { Chat } from "./components/Chat";
import logoUrl from "./assets/logo.png";
import { PendingChangesBar } from "./components/PendingChangesBar";
import { InstanceTabBar } from "./components/InstanceTabBar";
import { InstanceContext } from "./lib/instance-context";
import { api } from "./lib/api";
import { Button } from "@/components/ui/button";
import { Separator } from "@/components/ui/separator";
import { cn } from "@/lib/utils";
import type { DiscordGuildChannel, SshHost } from "./lib/types";

const PING_URL = "https://api.clawpal.zhixian.io/ping";

type Route = "home" | "recipes" | "cook" | "history" | "channels" | "cron" | "doctor" | "settings";

interface ToastItem {
  id: number;
  message: string;
  type: "success" | "error";
}

let toastIdCounter = 0;

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

  const showToast = useCallback((message: string, type: "success" | "error" = "success") => {
    const id = ++toastIdCounter;
    setToasts((prev) => [...prev, { id, message, type }]);
    if (type !== "error") {
      setTimeout(() => {
        setToasts((prev) => prev.filter((t) => t.id !== id));
      }, 3000);
    }
  }, []);

  const dismissToast = useCallback((id: number) => {
    setToasts((prev) => prev.filter((t) => t.id !== id));
  }, []);


  const handleInstanceSelect = useCallback((id: string) => {
    setActiveInstance(id);
    if (id !== "local") {
      // Check if backend still has a live connection before reconnecting
      setConnectionStatus((prev) => ({ ...prev, [id]: "disconnected" }));
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
              showToast(t('config.sshFailed', { error: String(e2) }), "error");
            });
        });
    }
  }, [showToast, t]);

  const [configVersion, setConfigVersion] = useState(0);

  const isRemote = activeInstance !== "local";
  const isConnected = !isRemote || connectionStatus[activeInstance] === "connected";

  // Load Discord data + extract profiles on startup or instance change
  useEffect(() => {
    setDiscordGuildChannels([]);
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


  return (
    <>
    <div className="flex flex-col h-screen">
      <InstanceTabBar
        hosts={sshHosts}
        activeId={activeInstance}
        connectionStatus={connectionStatus}
        onSelect={handleInstanceSelect}
        onHostsChange={refreshHosts}
      />
      <InstanceContext.Provider value={{ instanceId: activeInstance, isRemote, isConnected, discordGuildChannels }}>
      <div className="flex flex-1 overflow-hidden">
      <aside className="w-[200px] min-w-[200px] bg-muted border-r border-border flex flex-col py-4">
        <h1 className="px-4 text-lg font-bold mb-4 flex items-center gap-2">
          <img src={logoUrl} alt="" className="w-9 h-9 rounded-lg" />
          ClawPal
        </h1>
        <nav className="flex flex-col gap-1 px-2 flex-1">
          <Button
            variant="ghost"
            className={cn(
              "justify-start hover:bg-accent",
              (route === "home") && "bg-accent text-accent-foreground border-l-[3px] border-primary"
            )}
            onClick={() => setRoute("home")}
          >
            {t('nav.home')}
          </Button>
          <Button
            variant="ghost"
            className={cn(
              "justify-start hover:bg-accent",
              (route === "recipes" || route === "cook") && "bg-accent text-accent-foreground border-l-[3px] border-primary"
            )}
            onClick={() => setRoute("recipes")}
          >
            {t('nav.recipes')}
          </Button>
          <Button
            variant="ghost"
            className={cn(
              "justify-start hover:bg-accent",
              (route === "channels") && "bg-accent text-accent-foreground border-l-[3px] border-primary"
            )}
            onClick={() => setRoute("channels")}
          >
            {t('nav.channels')}
          </Button>
          <Button
            variant="ghost"
            className={cn(
              "justify-start hover:bg-accent",
              (route === "cron") && "bg-accent text-accent-foreground border-l-[3px] border-primary"
            )}
            onClick={() => setRoute("cron")}
          >
            {t('nav.cron')}
            {hasEscalatedCron && (
              <span className="ml-auto w-2 h-2 rounded-full bg-red-500" />
            )}
          </Button>
          <Button
            variant="ghost"
            className={cn(
              "justify-start hover:bg-accent",
              (route === "history") && "bg-accent text-accent-foreground border-l-[3px] border-primary"
            )}
            onClick={() => setRoute("history")}
          >
            {t('nav.history')}
          </Button>
          <Button
            variant="ghost"
            className={cn(
              "justify-start hover:bg-accent",
              (route === "doctor") && "bg-accent text-accent-foreground border-l-[3px] border-primary"
            )}
            onClick={() => setRoute("doctor")}
          >
            {t('nav.doctor')}
          </Button>
          <Separator className="my-2" />
          <Button
            variant="ghost"
            className={cn(
              "justify-start hover:bg-accent",
              (route === "settings") && "bg-accent text-accent-foreground border-l-[3px] border-primary"
            )}
            onClick={() => setRoute("settings")}
          >
            {t('nav.settings')}
            {appUpdateAvailable && (
              <span className="ml-1.5 w-2 h-2 rounded-full bg-destructive inline-block" />
            )}
          </Button>
        </nav>

        <div className="px-4 pb-2 flex items-center gap-1.5 text-xs text-muted-foreground">
          <a
            href="#"
            className="hover:text-foreground transition-colors"
            onClick={(e) => { e.preventDefault(); api.openUrl("https://clawpal.zhixian.io"); }}
          >
            {t('nav.website')}
          </a>
          <span>·</span>
          <a
            href="#"
            className="hover:text-foreground transition-colors"
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
      <main className="flex-1 overflow-y-auto p-4 relative">
        {/* Chat toggle -- top-right corner */}
        {!chatOpen && (
          <Button
            variant="outline"
            size="sm"
            className="absolute top-4 right-4 z-10"
            onClick={() => setChatOpen(true)}
          >
            {t('nav.chat')}
          </Button>
        )}

        {route === "home" && (
          <Home
            key={`${activeInstance}-${configVersion}`}
            onCook={(id, source) => {
              setRecipeId(id);
              setRecipeSource(source);
              setRoute("cook");
            }}
            showToast={showToast}
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
        {route === "doctor" && <Doctor />}
        {route === "settings" && (
          <Settings
            key={`${activeInstance}-${configVersion}`}
            onDataChange={bumpConfigVersion}
            hasAppUpdate={appUpdateAvailable}
            onAppUpdateSeen={() => setAppUpdateAvailable(false)}
          />
        )}
      </main>

      {/* Chat Panel -- inline, pushes main content */}
      {chatOpen && (
        <aside className="w-[360px] min-w-[360px] border-l border-border flex flex-col bg-background">
          <div className="flex items-center justify-between px-4 pt-4 pb-2">
            <h2 className="text-lg font-semibold">{t('nav.chat')}</h2>
            <Button
              variant="ghost"
              size="sm"
              className="h-7 w-7 p-0"
              onClick={() => setChatOpen(false)}
            >
              &times;
            </Button>
          </div>
          <div className="flex-1 overflow-hidden px-4 pb-4">
            <Chat />
          </div>
        </aside>
      )}
      </div>
      </InstanceContext.Provider>
    </div>

    {/* Toast Stack */}
    {toasts.length > 0 && (
      <div className="fixed bottom-4 right-4 z-50 flex flex-col-reverse gap-2">
        {toasts.map((toast) => (
          <div
            key={toast.id}
            className={cn(
              "flex items-center gap-2 px-4 py-2.5 rounded-md shadow-lg text-sm font-medium animate-in fade-in slide-in-from-bottom-2",
              toast.type === "success" ? "bg-green-600 text-white" : "bg-destructive text-destructive-foreground"
            )}
          >
            <span className="flex-1">{toast.message}</span>
            <button
              className="opacity-70 hover:opacity-100 text-current ml-2"
              onClick={() => dismissToast(toast.id)}
            >
              &times;
            </button>
          </div>
        ))}
      </div>
    )}
    </>
  );
}
