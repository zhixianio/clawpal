import { useCallback, useEffect, useRef, useState } from "react";
import { Home } from "./pages/Home";
import { Recipes } from "./pages/Recipes";
import { Cook } from "./pages/Cook";
import { History } from "./pages/History";
import { Settings } from "./pages/Settings";
import { Doctor } from "./pages/Doctor";
import { Channels } from "./pages/Channels";
import { Chat } from "./components/Chat";
import logoUrl from "./assets/logo.png";
import { DiffViewer } from "./components/DiffViewer";
import { InstanceTabBar } from "./components/InstanceTabBar";
import { InstanceContext } from "./lib/instance-context";
import { api } from "./lib/api";
import { Button } from "@/components/ui/button";
import { Separator } from "@/components/ui/separator";
import {
  Dialog,
  DialogContent,
  DialogHeader,
  DialogTitle,
  DialogFooter,
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
} from "@/components/ui/alert-dialog";
import { cn } from "@/lib/utils";
import type { DiscordGuildChannel, SshHost } from "./lib/types";

type Route = "home" | "recipes" | "cook" | "history" | "channels" | "doctor" | "settings";

interface ToastItem {
  id: number;
  message: string;
  type: "success" | "error";
}

let toastIdCounter = 0;

export function App() {
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
      // Always set to disconnected first, then attempt (re)connect
      setConnectionStatus((prev) => ({ ...prev, [id]: "disconnected" }));
      api.sshConnect(id)
        .then(() => setConnectionStatus((prev) => ({ ...prev, [id]: "connected" })))
        .catch((e) => {
          setConnectionStatus((prev) => ({ ...prev, [id]: "error" }));
          showToast(`SSH connection failed: ${e}`, "error");
        });
    }
  }, [showToast]);

  // Config dirty state
  const [dirty, setDirty] = useState(false);
  const [showApplyDialog, setShowApplyDialog] = useState(false);
  const [showDiscardDialog, setShowDiscardDialog] = useState(false);
  const [applyDiffBaseline, setApplyDiffBaseline] = useState("");
  const [applyDiffCurrent, setApplyDiffCurrent] = useState("");
  const [applying, setApplying] = useState(false);
  const [applyError, setApplyError] = useState("");
  const [configVersion, setConfigVersion] = useState(0);
  const pollRef = useRef<ReturnType<typeof setInterval> | null>(null);

  const isRemote = activeInstance !== "local";
  const isConnected = !isRemote || connectionStatus[activeInstance] === "connected";

  // Establish baseline on startup or instance change
  useEffect(() => {
    if (isRemote) {
      if (!isConnected) return;
      api.remoteSaveConfigBaseline(activeInstance).catch((e) => console.error("Failed to save remote config baseline:", e));
    } else {
      api.saveConfigBaseline().catch((e) => console.error("Failed to save config baseline:", e));
    }
  }, [activeInstance, isRemote, isConnected]);

  // Poll for dirty state
  const checkDirty = useCallback(() => {
    if (isRemote) {
      if (!isConnected) return;
      api.remoteCheckConfigDirty(activeInstance)
        .then((state) => setDirty(state.dirty))
        .catch((e) => console.error("Failed to check remote config dirty state:", e));
    } else {
      api.checkConfigDirty()
        .then((state) => setDirty(state.dirty))
        .catch((e) => console.error("Failed to check config dirty state:", e));
    }
  }, [isRemote, isConnected, activeInstance]);

  useEffect(() => {
    setDirty(false); // Reset dirty on instance change
    checkDirty();
    pollRef.current = setInterval(checkDirty, isRemote ? 5000 : 2000);
    return () => {
      if (pollRef.current) clearInterval(pollRef.current);
    };
  }, [checkDirty]);

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
      api.remoteListDiscordGuildChannels(activeInstance).then(setDiscordGuildChannels).catch((e) => console.error("Failed to load remote Discord channels:", e));
    }
  }, [activeInstance, isConnected]);

  const bumpConfigVersion = useCallback(() => {
    setConfigVersion((v) => v + 1);
  }, []);

  const handleApplyClick = () => {
    if (isRemote && !isConnected) return;
    // Load diff data for the dialog
    const checkPromise = isRemote
      ? api.remoteCheckConfigDirty(activeInstance)
      : api.checkConfigDirty();
    checkPromise
      .then((state) => {
        setApplyDiffBaseline(state.baseline);
        setApplyDiffCurrent(state.current);
        setApplyError("");
        setShowApplyDialog(true);
      })
      .catch((e) => console.error("Failed to load config diff:", e));
  };

  const handleApplyConfirm = () => {
    setApplying(true);
    setApplyError("");
    const applyPromise = isRemote
      ? api.remoteApplyPendingChanges(activeInstance)
      : api.applyPendingChanges();
    applyPromise
      .then(() => {
        setShowApplyDialog(false);
        setDirty(false);
        bumpConfigVersion();
        showToast("Gateway restarted successfully");
      })
      .catch((e) => setApplyError(String(e)))
      .finally(() => setApplying(false));
  };

  const handleDiscardConfirm = () => {
    const discardPromise = isRemote
      ? api.remoteDiscardConfigChanges(activeInstance)
      : api.discardConfigChanges();
    discardPromise
      .then(() => {
        setShowDiscardDialog(false);
        setDirty(false);
        bumpConfigVersion();
        showToast("Changes discarded");
      })
      .catch((e) => showToast(`Discard failed: ${e}`, "error"));
  };

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
            Home
          </Button>
          <Button
            variant="ghost"
            className={cn(
              "justify-start hover:bg-accent",
              (route === "recipes" || route === "cook") && "bg-accent text-accent-foreground border-l-[3px] border-primary"
            )}
            onClick={() => setRoute("recipes")}
          >
            Recipes
          </Button>
          <Button
            variant="ghost"
            className={cn(
              "justify-start hover:bg-accent",
              (route === "channels") && "bg-accent text-accent-foreground border-l-[3px] border-primary"
            )}
            onClick={() => setRoute("channels")}
          >
            Channels
          </Button>
          <Button
            variant="ghost"
            className={cn(
              "justify-start hover:bg-accent",
              (route === "history") && "bg-accent text-accent-foreground border-l-[3px] border-primary"
            )}
            onClick={() => setRoute("history")}
          >
            History
          </Button>
          <Button
            variant="ghost"
            className={cn(
              "justify-start hover:bg-accent",
              (route === "doctor") && "bg-accent text-accent-foreground border-l-[3px] border-primary"
            )}
            onClick={() => setRoute("doctor")}
          >
            Doctor
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
            Settings
          </Button>
        </nav>

        {/* Dirty config action bar */}
        {dirty && (
          <div className="px-2 pb-2 space-y-1.5">
            <Separator className="mb-2" />
            <p className="text-xs text-center text-muted-foreground px-1">Pending changes</p>
            <Button
              className="w-full"
              size="sm"
              onClick={handleApplyClick}
            >
              Apply Changes
            </Button>
            <Button
              className="w-full"
              size="sm"
              variant="outline"
              onClick={() => setShowDiscardDialog(true)}
            >
              Discard
            </Button>
          </div>
        )}
      </aside>
      <InstanceContext.Provider value={{ instanceId: activeInstance, isRemote, isConnected, discordGuildChannels }}>
      <main className="flex-1 overflow-y-auto p-4 relative">
        {/* Chat toggle -- top-right corner */}
        {!chatOpen && (
          <Button
            variant="outline"
            size="sm"
            className="absolute top-4 right-4 z-10"
            onClick={() => setChatOpen(true)}
          >
            Chat
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
        {route === "cook" && !recipeId && <p>No recipe selected.</p>}
        {route === "channels" && (
          <Channels
            key={`${activeInstance}-${configVersion}`}
            showToast={showToast}
          />
        )}
        {route === "history" && <History key={`${activeInstance}-${configVersion}`} />}
        {route === "doctor" && <Doctor />}
        {route === "settings" && (
          <Settings key={`${activeInstance}-${configVersion}`} onDataChange={bumpConfigVersion} />
        )}
      </main>

      {/* Chat Panel -- inline, pushes main content */}
      {chatOpen && (
        <aside className="w-[360px] min-w-[360px] border-l border-border flex flex-col bg-background">
          <div className="flex items-center justify-between px-4 pt-4 pb-2">
            <h2 className="text-lg font-semibold">Chat</h2>
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
      </InstanceContext.Provider>
      </div>
    </div>

    {/* Apply Changes Dialog */}
    <Dialog open={showApplyDialog} onOpenChange={setShowApplyDialog}>
      <DialogContent className="max-w-3xl">
        <DialogHeader>
          <DialogTitle>Apply Changes</DialogTitle>
        </DialogHeader>
        <p className="text-sm text-muted-foreground">
          Review config changes. Applying will restart the gateway.
        </p>
        <DiffViewer oldValue={applyDiffBaseline} newValue={applyDiffCurrent} />
        {applyError && (
          <p className="text-sm text-destructive">{applyError}</p>
        )}
        <DialogFooter>
          <Button variant="outline" onClick={() => setShowApplyDialog(false)} disabled={applying}>
            Cancel
          </Button>
          <Button onClick={handleApplyConfirm} disabled={applying}>
            {applying ? "Applying..." : "Apply & Restart Gateway"}
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>

    {/* Discard Changes Dialog */}
    <AlertDialog open={showDiscardDialog} onOpenChange={setShowDiscardDialog}>
      <AlertDialogContent>
        <AlertDialogHeader>
          <AlertDialogTitle>Discard all pending changes?</AlertDialogTitle>
          <AlertDialogDescription>
            This will restore the config to its state before your recent changes. This cannot be undone.
          </AlertDialogDescription>
        </AlertDialogHeader>
        <AlertDialogFooter>
          <AlertDialogCancel>Cancel</AlertDialogCancel>
          <AlertDialogAction
            className="bg-destructive text-destructive-foreground hover:bg-destructive/90"
            onClick={handleDiscardConfirm}
          >
            Discard
          </AlertDialogAction>
        </AlertDialogFooter>
      </AlertDialogContent>
    </AlertDialog>

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
