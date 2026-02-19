import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { check } from "@tauri-apps/plugin-updater";
import { relaunch } from "@tauri-apps/plugin-process";
import { api } from "../lib/api";
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
import { RecipeCard } from "@/components/RecipeCard";
import { Skeleton } from "@/components/ui/skeleton";
import type { StatusLight, AgentOverview, Recipe, BackupInfo, ModelProfile, RemoteSystemStatus } from "../lib/types";
import { formatTime, formatBytes } from "@/lib/utils";
import { useInstance } from "@/lib/instance-context";

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
  onCook,
  showToast,
}: {
  onCook?: (recipeId: string, source?: string) => void;
  showToast?: (message: string, type?: "success" | "error") => void;
}) {
  const { instanceId, isRemote, isConnected } = useInstance();
  const [status, setStatus] = useState<StatusLight | null>(null);
  const [version, setVersion] = useState<string | null>(null);
  const [updateInfo, setUpdateInfo] = useState<{ available: boolean; latest?: string } | null>(null);
  const [checkingUpdate, setCheckingUpdate] = useState(false);
  const [agents, setAgents] = useState<AgentOverview[] | null>(null);
  const [recipes, setRecipes] = useState<Recipe[]>([]);
  const [backups, setBackups] = useState<BackupInfo[] | null>(null);
  const [modelProfiles, setModelProfiles] = useState<ModelProfile[]>([]);
  const [savingModel, setSavingModel] = useState(false);
  const [backingUp, setBackingUp] = useState(false);
  const [backupMessage, setBackupMessage] = useState("");

  // ClawPal app self-update state
  const [appUpdate, setAppUpdate] = useState<{ version: string; body?: string } | null>(null);
  const [appUpdateChecking, setAppUpdateChecking] = useState(false);
  const [appUpdating, setAppUpdating] = useState(false);
  const [appUpdateProgress, setAppUpdateProgress] = useState<number | null>(null);

  // Create agent dialog
  const [showCreateAgent, setShowCreateAgent] = useState(false);
  const [showUpgradeDialog, setShowUpgradeDialog] = useState(false);

  // Health status with grace period: retry quickly when unhealthy, then slow-poll
  const [statusSettled, setStatusSettled] = useState(false);
  const retriesRef = useRef(0);

  const fetchStatus = useCallback(() => {
    if (isRemote) {
      if (!isConnected) return; // Wait for SSH connection
      api.remoteGetSystemStatus(instanceId).then((s: RemoteSystemStatus) => {
        setStatus({ healthy: s.healthy, activeAgents: s.activeAgents, globalDefaultModel: s.globalDefaultModel });
        setStatusSettled(true);
        if (s.openclawVersion) setVersion(s.openclawVersion);
      }).catch((e) => console.error("Failed to fetch remote status:", e));
    } else {
      api.getStatusLight().then((s) => {
        setStatus(s);
        if (s.healthy) {
          setStatusSettled(true);
          retriesRef.current = 0;
        } else if (retriesRef.current < 5) {
          retriesRef.current++;
        } else {
          setStatusSettled(true);
        }
      }).catch((e) => console.error("Failed to fetch status:", e));
    }
  }, [isRemote, isConnected, instanceId]);

  useEffect(() => {
    fetchStatus();
    // Poll fast (2s) while not settled, slow (10s) once settled
    const interval = setInterval(fetchStatus, statusSettled ? 10000 : 2000);
    return () => clearInterval(interval);
  }, [fetchStatus, statusSettled]);

  const refreshAgents = useCallback(() => {
    if (isRemote) {
      if (!isConnected) return; // Wait for SSH connection
      api.remoteListAgentsOverview(instanceId).then(setAgents).catch((e) => console.error("Failed to load remote agents:", e));
      return;
    }
    api.listAgentsOverview().then(setAgents).catch((e) => console.error("Failed to load agents:", e));
  }, [isRemote, isConnected, instanceId]);

  useEffect(() => {
    refreshAgents();
    // Auto-refresh agents every 15s
    const interval = setInterval(refreshAgents, 15000);
    return () => clearInterval(interval);
  }, [refreshAgents]);

  useEffect(() => {
    api.listRecipes().then((r) => setRecipes(r.slice(0, 4))).catch((e) => console.error("Failed to load recipes:", e));
  }, []);

  const refreshBackups = () => {
    if (isRemote) { setBackups([]); return; }
    api.listBackups().then(setBackups).catch((e) => console.error("Failed to load backups:", e));
  };
  // eslint-disable-next-line react-hooks/exhaustive-deps
  useEffect(refreshBackups, [isRemote]);

  useEffect(() => {
    if (isRemote) {
      if (!isConnected) return;
      api.remoteListModelProfiles(instanceId).then((p) => setModelProfiles(p.filter((m) => m.enabled))).catch((e) => console.error("Failed to load remote model profiles:", e));
    } else {
      api.listModelProfiles().then((p) => setModelProfiles(p.filter((m) => m.enabled))).catch((e) => console.error("Failed to load model profiles:", e));
    }
  }, [isRemote, isConnected, instanceId]);

  // Match current global model value to a profile ID
  const currentModelProfileId = useMemo(() => {
    const modelVal = status?.globalDefaultModel;
    if (!modelVal) return null;
    const normalized = modelVal.toLowerCase();
    for (const p of modelProfiles) {
      const profileVal = p.model.includes("/") ? p.model : `${p.provider}/${p.model}`;
      if (profileVal.toLowerCase() === normalized || p.model.toLowerCase() === normalized) {
        return p.id;
      }
    }
    return null;
  }, [status?.globalDefaultModel, modelProfiles]);

  const agentGroups = useMemo(() => groupAgents(agents || []), [agents]);

  // Update check — deferred, runs once (not in poll loop)
  useEffect(() => {
    setCheckingUpdate(true);
    setUpdateInfo(null);
    const timer = setTimeout(() => {
      if (isRemote) {
        if (!isConnected) { setCheckingUpdate(false); return; }
        api.remoteCheckOpenclawUpdate(instanceId).then((u) => {
          setUpdateInfo({ available: u.upgradeAvailable, latest: u.latestVersion ?? undefined });
        }).catch((e) => console.error("Failed to check remote update:", e))
          .finally(() => setCheckingUpdate(false));
      } else {
        api.getSystemStatus().then((s) => {
          setVersion(s.openclawVersion);
          if (s.openclawUpdate) {
            setUpdateInfo({
              available: s.openclawUpdate.upgradeAvailable,
              latest: s.openclawUpdate.latestVersion,
            });
          }
        }).catch((e) => console.error("Failed to fetch system status:", e))
          .finally(() => setCheckingUpdate(false));
      }
    }, 500);
    return () => clearTimeout(timer);
  }, [isRemote, isConnected, instanceId]);

  // ClawPal app self-update check (local only)
  useEffect(() => {
    if (isRemote) return;
    setAppUpdateChecking(true);
    check()
      .then((update) => {
        if (update) {
          setAppUpdate({ version: update.version, body: update.body });
        }
      })
      .catch((e) => console.error("Failed to check app update:", e))
      .finally(() => setAppUpdateChecking(false));
  }, [isRemote]);

  const handleAppUpdate = useCallback(async () => {
    if (isRemote) return;
    setAppUpdating(true);
    setAppUpdateProgress(0);
    try {
      const update = await check();
      if (!update) return;
      let totalBytes = 0;
      let downloadedBytes = 0;
      await update.downloadAndInstall((event) => {
        if (event.event === "Started" && event.data.contentLength) {
          totalBytes = event.data.contentLength;
        } else if (event.event === "Progress") {
          downloadedBytes += event.data.chunkLength;
          if (totalBytes > 0) {
            setAppUpdateProgress(Math.round((downloadedBytes / totalBytes) * 100));
          }
        } else if (event.event === "Finished") {
          setAppUpdateProgress(100);
        }
      });
      await relaunch();
    } catch (e) {
      console.error("App update failed:", e);
      setAppUpdating(false);
      setAppUpdateProgress(null);
    }
  }, [isRemote]);

  const handleDeleteAgent = (agentId: string) => {
    if (isRemote && !isConnected) return;
    const deletePromise = isRemote
      ? api.remoteDeleteAgent(instanceId, agentId)
      : api.deleteAgent(agentId);
    deletePromise
      .then(() => refreshAgents())
      .catch((e) => showToast?.(String(e), "error"));
  };

  return (
    <div>
      <h2 className="text-2xl font-bold mb-4">Home</h2>

        {/* Status Summary */}
        <h3 className="text-lg font-semibold mt-6 mb-3">Status</h3>
        <Card>
          <CardContent className="grid grid-cols-[auto_1fr] gap-x-6 gap-y-3 items-center">
            <span className="text-sm text-muted-foreground">Health</span>
            <span className="text-sm font-medium">
              {!status ? "..." : status.healthy ? (
                <Badge className="bg-green-100 text-green-700 border-0">Healthy</Badge>
              ) : !statusSettled ? (
                <Badge className="bg-amber-100 text-amber-700 border-0">Checking...</Badge>
              ) : (
                <Badge className="bg-red-100 text-red-700 border-0">Unhealthy</Badge>
              )}
            </span>

            <span className="text-sm text-muted-foreground">Version</span>
            <div className="flex items-center gap-2 flex-wrap">
              <span className="text-sm font-medium">{version || "..."}</span>
              {checkingUpdate && (
                <Badge variant="outline" className="text-muted-foreground">Checking for updates...</Badge>
              )}
              {!checkingUpdate && updateInfo?.available && updateInfo.latest && updateInfo.latest !== version && (
                <>
                  <Badge variant="outline" className="text-primary border-primary">
                    {updateInfo.latest} available
                  </Badge>
                  <Button
                    size="sm"
                    className="text-xs h-6"
                    variant="outline"
                    onClick={() => api.openUrl("https://github.com/openclaw/openclaw/releases")}
                  >
                    View
                  </Button>
                  <Button
                    size="sm"
                    className="text-xs h-6"
                    onClick={() => setShowUpgradeDialog(true)}
                  >
                    Upgrade
                  </Button>
                </>
              )}
            </div>

            {/* ClawPal app self-update (local only) */}
            {!isRemote && (appUpdateChecking || appUpdate) && (
              <>
                <span className="text-sm text-muted-foreground">App Update</span>
                <div className="flex items-center gap-2 flex-wrap">
                  {appUpdateChecking && (
                    <Badge variant="outline" className="text-muted-foreground">Checking for app updates...</Badge>
                  )}
                  {!appUpdateChecking && appUpdate && !appUpdating && (
                    <>
                      <Badge variant="outline" className="text-primary border-primary">
                        ClawPal v{appUpdate.version} available
                      </Badge>
                      <Button size="sm" className="text-xs h-6" onClick={handleAppUpdate}>
                        Update &amp; Restart
                      </Button>
                    </>
                  )}
                  {appUpdating && (
                    <>
                      <Badge variant="outline" className="text-muted-foreground">
                        {appUpdateProgress !== null && appUpdateProgress < 100
                          ? `Downloading... ${appUpdateProgress}%`
                          : appUpdateProgress === 100
                            ? "Installing..."
                            : "Preparing..."}
                      </Badge>
                      {appUpdateProgress !== null && appUpdateProgress < 100 && (
                        <div className="w-32 h-1.5 bg-muted rounded-full overflow-hidden">
                          <div
                            className="h-full bg-primary rounded-full transition-all"
                            style={{ width: `${appUpdateProgress}%` }}
                          />
                        </div>
                      )}
                    </>
                  )}
                </div>
              </>
            )}

            <span className="text-sm text-muted-foreground">Default Model</span>
            <div className="max-w-xs">
              {status ? (
                <Select
                  value={currentModelProfileId || "__none__"}
                  onValueChange={(val) => {
                    setSavingModel(true);
                    const setModelPromise = isRemote
                      ? (() => {
                          const profile = modelProfiles.find((p) => p.id === val);
                          const modelValue = profile ? `${profile.provider}/${profile.model}` : null;
                          return api.remoteSetGlobalModel(instanceId, modelValue);
                        })()
                      : api.setGlobalModel(val === "__none__" ? null : val);
                    setModelPromise
                      .then(() => fetchStatus())
                      .catch((e) => showToast?.(String(e), "error"))
                      .finally(() => setSavingModel(false));
                  }}
                  disabled={savingModel}
                >
                  <SelectTrigger size="sm" className="text-sm">
                    <SelectValue />
                  </SelectTrigger>
                  <SelectContent>
                    <SelectItem value="__none__">
                      <span className="text-muted-foreground">not set</span>
                    </SelectItem>
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
          </CardContent>
        </Card>

        {/* Agents Overview -- grouped by identity */}
        <div className="flex items-center justify-between mt-6 mb-3">
          <h3 className="text-lg font-semibold">Agents</h3>
          <Button size="sm" variant="outline" onClick={() => setShowCreateAgent(true)}>
            + New Agent
          </Button>
        </div>
        {agents === null ? (
          <div className="space-y-3">
            <Skeleton className="h-24 w-full" />
            <Skeleton className="h-24 w-full" />
          </div>
        ) : agentGroups.length === 0 ? (
          <p className="text-muted-foreground">No agents found.</p>
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
                          <span className="text-sm text-muted-foreground">
                            {agent.model || "default model"}
                          </span>
                        </div>
                        <div className="flex items-center gap-2">
                          {agent.online ? (
                            <Badge className="bg-green-100 text-green-700 border-0 text-xs">online</Badge>
                          ) : (
                            <Badge className="bg-red-100 text-red-700 border-0 text-xs">offline</Badge>
                          )}
                          {agent.id !== "main" && (
                            <AlertDialog>
                              <AlertDialogTrigger asChild>
                                <Button size="sm" variant="ghost" className="h-6 px-1.5 text-xs text-muted-foreground hover:text-destructive">
                                  Delete
                                </Button>
                              </AlertDialogTrigger>
                              <AlertDialogContent>
                                <AlertDialogHeader>
                                  <AlertDialogTitle>Delete agent "{agent.id}"?</AlertDialogTitle>
                                  <AlertDialogDescription>
                                    This will remove the agent from the config and any channel bindings associated with it.
                                  </AlertDialogDescription>
                                </AlertDialogHeader>
                                <AlertDialogFooter>
                                  <AlertDialogCancel>Cancel</AlertDialogCancel>
                                  <AlertDialogAction
                                    className="bg-destructive text-destructive-foreground hover:bg-destructive/90"
                                    onClick={() => handleDeleteAgent(agent.id)}
                                  >
                                    Delete
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

        {/* Recommended Recipes */}
        <h3 className="text-lg font-semibold mt-6 mb-3">Recommended Recipes</h3>
        {recipes.length === 0 ? (
          <p className="text-muted-foreground">No recipes available.</p>
        ) : (
          <div className="grid grid-cols-[repeat(auto-fit,minmax(180px,1fr))] gap-3">
            {recipes.map((recipe) => (
              <RecipeCard
                key={recipe.id}
                recipe={recipe}
                onCook={() => onCook?.(recipe.id)}
                compact
              />
            ))}
          </div>
        )}

        {/* Backups */}
        {!isRemote && (
          <>
            <div className="flex items-center justify-between mt-6 mb-3">
              <h3 className="text-lg font-semibold">Backups</h3>
              <Button
                size="sm"
                variant="outline"
                disabled={backingUp}
                onClick={() => {
                  setBackingUp(true);
                  setBackupMessage("");
                  api.backupBeforeUpgrade()
                    .then((info) => {
                      setBackupMessage(`Created backup: ${info.name}`);
                      refreshBackups();
                    })
                    .catch((e) => setBackupMessage(`Backup failed: ${e}`))
                    .finally(() => setBackingUp(false));
                }}
              >
                {backingUp ? "Creating..." : "Create Backup"}
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
              <p className="text-muted-foreground text-sm">No backups available.</p>
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
                      </div>
                      <div className="flex gap-1.5">
                        <Button
                          size="sm"
                          variant="outline"
                          onClick={() => api.openUrl(backup.path)}
                        >
                          Show
                        </Button>
                        <AlertDialog>
                          <AlertDialogTrigger asChild>
                            <Button size="sm" variant="outline">
                              Restore
                            </Button>
                          </AlertDialogTrigger>
                          <AlertDialogContent>
                            <AlertDialogHeader>
                              <AlertDialogTitle>Restore from backup?</AlertDialogTitle>
                              <AlertDialogDescription>
                                This will restore config and workspace files from backup "{backup.name}". Current files will be overwritten.
                              </AlertDialogDescription>
                            </AlertDialogHeader>
                            <AlertDialogFooter>
                              <AlertDialogCancel>Cancel</AlertDialogCancel>
                              <AlertDialogAction
                                onClick={() => {
                                  api.restoreFromBackup(backup.name)
                                    .then((msg) => setBackupMessage(msg))
                                    .catch((e) => setBackupMessage(`Restore failed: ${e}`));
                                }}
                              >
                                Restore
                              </AlertDialogAction>
                            </AlertDialogFooter>
                          </AlertDialogContent>
                        </AlertDialog>
                        <AlertDialog>
                          <AlertDialogTrigger asChild>
                            <Button size="sm" variant="destructive">
                              Delete
                            </Button>
                          </AlertDialogTrigger>
                          <AlertDialogContent>
                            <AlertDialogHeader>
                              <AlertDialogTitle>Delete backup?</AlertDialogTitle>
                              <AlertDialogDescription>
                                This will permanently delete backup "{backup.name}". This action cannot be undone.
                              </AlertDialogDescription>
                            </AlertDialogHeader>
                            <AlertDialogFooter>
                              <AlertDialogCancel>Cancel</AlertDialogCancel>
                              <AlertDialogAction
                                className="bg-destructive text-destructive-foreground hover:bg-destructive/90"
                                onClick={() => {
                                  api.deleteBackup(backup.name)
                                    .then(() => {
                                      setBackupMessage(`Deleted backup "${backup.name}"`);
                                      refreshBackups();
                                    })
                                    .catch((e) => setBackupMessage(`Delete failed: ${e}`));
                                }}
                              >
                                Delete
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
          </>
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
        onOpenChange={setShowUpgradeDialog}
        isRemote={isRemote}
        instanceId={instanceId}
        currentVersion={version || ""}
        latestVersion={updateInfo?.latest || ""}
      />
    </div>
  );
}
