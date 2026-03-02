import { useCallback, useEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { PlusIcon } from "lucide-react";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { Checkbox } from "@/components/ui/checkbox";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogHeader,
  DialogTitle,
  DialogFooter,
} from "@/components/ui/dialog";
import { InstanceCard } from "@/components/InstanceCard";
import { InstallHub } from "@/components/InstallHub";
import { api } from "@/lib/api";
import { withGuidance } from "@/lib/guidance";
import type { DockerInstance, SshHost, InstallSession, RegisteredInstance, DiscoveredInstance } from "@/lib/types";

const DEFAULT_DOCKER_OPENCLAW_HOME = "~/.clawpal/docker-local";
const DEFAULT_DOCKER_CLAWPAL_DATA_DIR = "~/.clawpal/docker-local/data";

function deriveDockerPaths(instanceId: string): { openclawHome: string; clawpalDataDir: string } {
  if (instanceId === "docker:local") {
    return {
      openclawHome: DEFAULT_DOCKER_OPENCLAW_HOME,
      clawpalDataDir: DEFAULT_DOCKER_CLAWPAL_DATA_DIR,
    };
  }
  const suffixRaw = instanceId.startsWith("docker:") ? instanceId.slice(7) : instanceId;
  const suffix = suffixRaw === "local"
    ? "docker-local"
    : suffixRaw.startsWith("docker-")
      ? suffixRaw
      : `docker-${suffixRaw || "local"}`;
  const openclawHome = `~/.clawpal/${suffix}`;
  return {
    openclawHome,
    clawpalDataDir: `${openclawHome}/data`,
  };
}

function normalizePathForCompare(raw: string): string {
  const trimmed = raw.trim().replace(/\\/g, "/");
  if (!trimmed) return "";
  return trimmed.replace(/\/+$/, "");
}

function dockerPathKey(raw: string): string {
  const normalized = normalizePathForCompare(raw);
  if (!normalized) return "";
  const segments = normalized.split("/").filter(Boolean);
  const clawpalIdx = segments.lastIndexOf(".clawpal");
  if (clawpalIdx >= 0 && clawpalIdx + 1 < segments.length) {
    const dir = segments[clawpalIdx + 1];
    if (dir.startsWith("docker-")) return `docker-dir:${dir.toLowerCase()}`;
  }
  const last = segments[segments.length - 1] || "";
  if (last.startsWith("docker-")) return `docker-dir:${last.toLowerCase()}`;
  return `path:${normalized.toLowerCase()}`;
}

function dockerIdKey(rawId: string): string {
  if (!rawId.startsWith("docker:")) return "";
  let slug = rawId.slice("docker:".length).trim().toLowerCase();
  if (!slug) slug = "local";
  if (slug.startsWith("docker-")) slug = slug.slice("docker-".length);
  return `docker-id:${slug}`;
}

interface StartPageProps {
  dockerInstances: DockerInstance[];
  sshHosts: SshHost[];
  registeredInstances: RegisteredInstance[];
  openTabIds: Set<string>;
  onOpenInstance: (id: string) => void;
  onRenameDocker: (id: string, label: string) => void;
  onDeleteDocker: (instance: DockerInstance, deleteData: boolean) => Promise<void>;
  onDeleteSsh: (hostId: string) => void;
  onEditSsh: (host: SshHost) => void;
  onInstallReady: (session: InstallSession) => void;
  showToast: (message: string, type?: "success" | "error") => void;
  onNavigate: (route: string) => void;
  onOpenDoctor?: () => void;
  connectRemoteHost?: (hostId: string) => Promise<void>;
  discoveredInstances: DiscoveredInstance[];
  discoveringInstances: boolean;
  onConnectDiscovered: (instance: DiscoveredInstance) => void;
}

export function StartPage({
  dockerInstances,
  sshHosts,
  registeredInstances,
  openTabIds,
  onOpenInstance,
  onRenameDocker,
  onDeleteDocker,
  onDeleteSsh,
  onEditSsh,
  onInstallReady,
  showToast,
  onNavigate,
  onOpenDoctor,
  connectRemoteHost,
  discoveredInstances,
  discoveringInstances,
  onConnectDiscovered,
}: StartPageProps) {
  const { t } = useTranslation();
  const fallbackLabelForId = useCallback((id: string): string => {
    if (id === "local") return t("instance.local");
    if (id.startsWith("docker:")) {
      const suffix = id.slice("docker:".length);
      if (!suffix) return "docker-local";
      if (suffix.startsWith("docker-")) return suffix;
      return `docker-${suffix}`;
    }
    if (id.startsWith("ssh:")) {
      const suffix = id.slice("ssh:".length);
      return suffix || id;
    }
    return id;
  }, [t]);

  // Health state
  const [healthMap, setHealthMap] = useState<
    Record<string, { healthy: boolean | null; agentCount: number }>
  >({});

  // SSH manual check state: tracks which hosts have been checked / are checking
  const [sshChecked, setSshChecked] = useState<Record<string, boolean>>({});
  const [sshChecking, setSshChecking] = useState<Record<string, boolean>>({});
  const healthPollInFlightRef = useRef(false);
  const localHealthCursorRef = useRef(0);
  const dockerHealthCursorRef = useRef(0);

  // Install dialog
  const [installDialogOpen, setInstallDialogOpen] = useState(false);

  // Docker rename dialog state
  const [dockerRenameOpen, setDockerRenameOpen] = useState(false);
  const [editingDocker, setEditingDocker] = useState<DockerInstance | null>(null);
  const [dockerLabel, setDockerLabel] = useState("");

  // Docker delete dialog state
  const [dockerDeleteOpen, setDockerDeleteOpen] = useState(false);
  const [deletingDocker, setDeletingDocker] = useState<DockerInstance | null>(null);
  const [deleteDockerData, setDeleteDockerData] = useState(true);
  const [dockerDeleting, setDockerDeleting] = useState(false);
  const [dockerDeleteError, setDockerDeleteError] = useState<string | null>(null);

  // SSH delete dialog state
  const [sshDeleteOpen, setSshDeleteOpen] = useState(false);
  const [deletingHost, setDeletingHost] = useState<SshHost | null>(null);

  // Health polling — local, Docker (own openclawHome), and connected SSH
  useEffect(() => {
    let cancelled = false;
    const poll = async () => {
      if (healthPollInFlightRef.current) return;
      healthPollInFlightRef.current = true;
      try {
        const updates: Record<string, { healthy: boolean | null; agentCount: number }> = {};

        // Poll local instance
        try {
          const status = await api.getInstanceStatus();
          updates.local = { healthy: status.healthy, agentCount: status.activeAgents };
        } catch {
          updates.local = { healthy: null, agentCount: 0 };
        }

        const localTargets = registeredInstances
          .filter((item) => item.instanceType === "local" && item.id !== "local" && !!item.openclawHome)
          .map((item) => ({
            id: item.id,
            openclawHome: item.openclawHome || "",
            clawpalDataDir: item.clawpalDataDir || "",
          }));
        if (localTargets.length > 0) {
          const idx = localHealthCursorRef.current % localTargets.length;
          localHealthCursorRef.current = (idx + 1) % localTargets.length;
          const target = localTargets[idx];
          try {
            await api.setActiveOpenclawHome(target.openclawHome);
            await api.setActiveClawpalDataDir(target.clawpalDataDir || null);
            const status = await api.getInstanceStatus();
            updates[target.id] = { healthy: status.healthy, agentCount: status.activeAgents };
          } catch {
            updates[target.id] = { healthy: null, agentCount: 0 };
          } finally {
            await api.setActiveOpenclawHome(null);
            await api.setActiveClawpalDataDir(null);
          }
        }

        const dockerTargetsById = new Map<string, {
          id: string;
          openclawHome?: string;
          clawpalDataDir?: string;
        }>();
        for (const r of registeredInstances.filter((item) => item.instanceType === "docker")) {
          dockerTargetsById.set(r.id, {
            id: r.id,
            openclawHome: r.openclawHome || undefined,
            clawpalDataDir: r.clawpalDataDir || undefined,
          });
        }
        for (const d of dockerInstances) {
          const existing = dockerTargetsById.get(d.id);
          const fallback = deriveDockerPaths(d.id);
          dockerTargetsById.set(d.id, {
            id: d.id,
            openclawHome: existing?.openclawHome || d.openclawHome || fallback.openclawHome,
            clawpalDataDir: existing?.clawpalDataDir || d.clawpalDataDir || fallback.clawpalDataDir,
          });
        }
        for (const [id, target] of dockerTargetsById.entries()) {
          if (!target.openclawHome) {
            const fallback = deriveDockerPaths(id);
            dockerTargetsById.set(id, {
              ...target,
              openclawHome: fallback.openclawHome,
              clawpalDataDir: target.clawpalDataDir || fallback.clawpalDataDir,
            });
          }
        }
        const dockerTargets = Array.from(dockerTargetsById.values());

        // Poll one Docker instance per cycle (round-robin) to avoid UI jank from
        // heavy serial checks when many instances are registered.
        if (dockerTargets.length > 0) {
          const idx = dockerHealthCursorRef.current % dockerTargets.length;
          dockerHealthCursorRef.current = (idx + 1) % dockerTargets.length;
          const d = dockerTargets[idx];
          if (d.openclawHome) {
            try {
              await api.setActiveOpenclawHome(d.openclawHome);
              if (d.clawpalDataDir) await api.setActiveClawpalDataDir(d.clawpalDataDir);
              const status = await api.getInstanceStatus();
              updates[d.id] = { healthy: status.healthy, agentCount: status.activeAgents };
            } catch {
              updates[d.id] = { healthy: null, agentCount: 0 };
            } finally {
              await api.setActiveOpenclawHome(null);
              await api.setActiveClawpalDataDir(null);
            }
          } else {
            updates[d.id] = { healthy: null, agentCount: 0 };
          }
        }

        if (!cancelled) {
          setHealthMap((prev) => ({ ...prev, ...updates }));
        }
      } finally {
        healthPollInFlightRef.current = false;
      }
    };
    const initial = setTimeout(() => {
      void poll();
    }, 800);
    const timer = setInterval(poll, 30_000);
    return () => {
      cancelled = true;
      healthPollInFlightRef.current = false;
      clearTimeout(initial);
      clearInterval(timer);
    };
  }, [dockerInstances, registeredInstances]);

  // Manual SSH health check
  const handleSshCheck = useCallback(async (hostId: string) => {
    setSshChecking((prev) => ({ ...prev, [hostId]: true }));
    try {
      if (connectRemoteHost) {
        await connectRemoteHost(hostId);
      } else {
        await withGuidance(
          () => api.sshConnect(hostId),
          "sshConnect",
          hostId,
          "remote_ssh",
        );
      }
      const status = await withGuidance(
        () => api.remoteGetInstanceStatus(hostId),
        "remoteGetInstanceStatus",
        hostId,
        "remote_ssh",
      );
      setHealthMap((prev) => ({
        ...prev,
        [hostId]: { healthy: status.healthy, agentCount: status.activeAgents },
      }));
    } catch {
      setHealthMap((prev) => ({
        ...prev,
        [hostId]: { healthy: null, agentCount: 0 },
      }));
    } finally {
      setSshChecking((prev) => ({ ...prev, [hostId]: false }));
      setSshChecked((prev) => ({ ...prev, [hostId]: true }));
    }
  }, [connectRemoteHost]);

  const toCardType = useCallback((instanceType: string, instanceId: string): "local" | "docker" | "ssh" | "wsl2" => {
    if (instanceType === "remote_ssh") return "ssh";
    if (instanceType === "docker") return "docker";
    if (instanceType === "wsl2" || instanceId.startsWith("wsl2:")) return "wsl2";
    return "local";
  }, []);

  // Build unified instances list
  const instancesMap = new Map<string, { id: string; label: string; type: "local" | "docker" | "ssh" | "wsl2" }>();
  instancesMap.set("local", { id: "local", label: t("instance.local"), type: "local" });
  for (const r of registeredInstances) {
    instancesMap.set(r.id, {
      id: r.id,
      label: r.id === "local" ? t("instance.local") : (r.label || fallbackLabelForId(r.id)),
      type: toCardType(r.instanceType, r.id),
    });
  }
  const instances = Array.from(instancesMap.values());
  const knownDockerKeys = new Set<string>();
  for (const item of registeredInstances) {
    if (item.instanceType !== "docker") continue;
    const idKey = dockerIdKey(item.id);
    if (idKey) knownDockerKeys.add(idKey);
    if (item.openclawHome) {
      const pathKey = dockerPathKey(item.openclawHome);
      if (pathKey) knownDockerKeys.add(pathKey);
    }
  }
  for (const item of dockerInstances) {
    const idKey = dockerIdKey(item.id);
    if (idKey) knownDockerKeys.add(idKey);
    if (item.openclawHome) {
      const pathKey = dockerPathKey(item.openclawHome);
      if (pathKey) knownDockerKeys.add(pathKey);
    }
  }

  // Docker rename handlers
  const openDockerRename = useCallback((instance: DockerInstance) => {
    setEditingDocker(instance);
    setDockerLabel(instance.label || "");
    setDockerRenameOpen(true);
  }, []);

  const handleDockerRenameSave = useCallback(() => {
    if (!editingDocker || !dockerLabel.trim()) return;
    onRenameDocker(editingDocker.id, dockerLabel.trim());
    setDockerRenameOpen(false);
  }, [editingDocker, dockerLabel, onRenameDocker]);

  // Docker delete handlers
  const openDockerDelete = useCallback((instance: DockerInstance) => {
    setDeletingDocker(instance);
    setDeleteDockerData(true);
    setDockerDeleteError(null);
    setDockerDeleteOpen(true);
  }, []);

  const handleDockerDeleteConfirm = useCallback(async () => {
    if (!deletingDocker) return;
    setDockerDeleting(true);
    setDockerDeleteError(null);
    try {
      await onDeleteDocker(deletingDocker, deleteDockerData);
      setDockerDeleteOpen(false);
    } catch (e) {
      setDockerDeleteError(e instanceof Error ? e.message : String(e));
    } finally {
      setDockerDeleting(false);
    }
  }, [deletingDocker, deleteDockerData, onDeleteDocker]);

  // SSH delete handler
  const openSshDelete = useCallback((host: SshHost) => {
    setDeletingHost(host);
    setSshDeleteOpen(true);
  }, []);

  return (
    <div className="max-w-4xl mx-auto">
      <div className="mb-8">
        <h2 className="text-2xl font-bold mb-1">{t("start.welcome")}</h2>
        <p className="text-muted-foreground">{t("start.welcomeHint")}</p>
      </div>

      {discoveringInstances && (
        <div className="text-sm text-muted-foreground animate-pulse mb-2">
          {t("start.scanning")}
        </div>
      )}

      <div className="grid grid-cols-1 sm:grid-cols-2 lg:grid-cols-3 gap-4">
        {instances.map((inst) => {
          const health = healthMap[inst.id];
          const dockerInst = inst.type === "docker"
            ? dockerInstances.find((d) => d.id === inst.id)
            : undefined;
          const sshHost = inst.type === "ssh"
            ? sshHosts.find((h) => h.id === inst.id)
            : undefined;

          return (
            <InstanceCard
              key={inst.id}
              id={inst.id}
              label={inst.label}
              type={inst.type}
              healthy={health?.healthy ?? null}
              agentCount={health?.agentCount ?? 0}
              opened={openTabIds.has(inst.id)}
              checked={inst.type === "ssh" ? sshChecked[inst.id] ?? false : undefined}
              checking={inst.type === "ssh" ? sshChecking[inst.id] ?? false : undefined}
              onCheck={inst.type === "ssh" ? () => handleSshCheck(inst.id) : undefined}
              onClick={() => onOpenInstance(inst.id)}
              onRename={
                inst.type === "docker" && dockerInst
                  ? () => openDockerRename(dockerInst)
                  : undefined
              }
              onEdit={
                inst.type === "ssh" && sshHost
                  ? () => onEditSsh(sshHost)
                  : undefined
              }
              onDelete={
                inst.type === "docker" && dockerInst
                  ? () => openDockerDelete(dockerInst)
                  : inst.type === "ssh" && sshHost
                    ? () => openSshDelete(sshHost)
                    : undefined
              }
            />
          );
        })}

        {discoveredInstances
          .filter((d) => {
            if (d.alreadyRegistered) return false;
            if (d.instanceType !== "docker") return true;
            const idKey = dockerIdKey(d.id);
            if (idKey && knownDockerKeys.has(idKey)) return false;
            const pathKey = dockerPathKey(d.homePath);
            if (pathKey && knownDockerKeys.has(pathKey)) return false;
            return true;
          })
          .map((d) => (
            <InstanceCard
              key={`discovered-${d.id}`}
              id={d.id}
              label={d.label}
              type={toCardType(d.instanceType, d.id)}
              healthy={null}
              agentCount={0}
              opened={false}
              onClick={() => {}}
              discovered
              discoveredSource={d.source}
              onConnect={() => onConnectDiscovered(d)}
            />
          ))}

        {/* + New/Connect card */}
        <button
          className="border-2 border-dashed border-muted-foreground/30 rounded-xl p-6 flex flex-col items-center justify-center gap-2 text-muted-foreground hover:border-primary/40 hover:text-primary transition-all duration-200 cursor-pointer min-h-[140px]"
          onClick={() => setInstallDialogOpen(true)}
        >
          <PlusIcon className="size-8" />
          <span className="font-medium text-sm">{t("start.addInstance")}</span>
          <span className="text-xs text-muted-foreground/70">
            {t("start.addInstanceHint")}
          </span>
        </button>
      </div>

      {/* InstallHub Dialog */}
      <InstallHub
        open={installDialogOpen}
        onOpenChange={setInstallDialogOpen}
        showToast={showToast}
        onNavigate={onNavigate}
        connectRemoteHost={connectRemoteHost}
        onOpenDoctor={onOpenDoctor}
        onReady={(session: InstallSession) => {
          setInstallDialogOpen(false);
          onInstallReady(session);
        }}
      />

      {/* Docker rename dialog */}
      <Dialog open={dockerRenameOpen} onOpenChange={setDockerRenameOpen}>
        <DialogContent>
          <DialogHeader>
            <DialogTitle>{t("instance.editName")}</DialogTitle>
          </DialogHeader>
          <div className="space-y-1.5">
            <Label htmlFor="docker-label">{t("instance.label")}</Label>
            <Input
              id="docker-label"
              value={dockerLabel}
              onChange={(e) => setDockerLabel(e.target.value)}
              placeholder={t("instance.labelPlaceholder")}
              autoFocus
            />
          </div>
          <DialogFooter>
            <Button
              variant="outline"
              onClick={() => setDockerRenameOpen(false)}
            >
              {t("instance.cancel")}
            </Button>
            <Button
              onClick={handleDockerRenameSave}
              disabled={!dockerLabel.trim()}
            >
              {t("instance.update")}
            </Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>

      {/* Docker delete dialog */}
      <Dialog
        open={dockerDeleteOpen}
        onOpenChange={(open) => {
          if (dockerDeleting) return;
          setDockerDeleteOpen(open);
        }}
      >
        <DialogContent>
          <DialogHeader>
            <DialogTitle>{t("instance.dockerDeleteTitle")}</DialogTitle>
            <DialogDescription>
              {t("instance.dockerDeleteDescription", {
                label: deletingDocker?.label || "",
              })}
            </DialogDescription>
          </DialogHeader>
          <div className="space-y-3 text-sm">
            <p className="text-muted-foreground">{t("instance.dockerDeleteBackupHint")}</p>
            <div className="rounded-md border bg-muted/40 px-3 py-2">
              <p className="text-xs text-muted-foreground mb-1">{t("instance.dockerDeletePath")}</p>
              <p className="font-mono break-all">{deletingDocker?.openclawHome || "-"}</p>
            </div>
            <div className="flex items-start gap-2">
              <Checkbox
                id="delete-docker-data"
                checked={deleteDockerData}
                onCheckedChange={(v) => setDeleteDockerData(Boolean(v))}
              />
              <div className="space-y-0.5">
                <Label htmlFor="delete-docker-data" className="text-sm font-medium cursor-pointer">
                  {t("instance.dockerDeleteRemoveData")}
                </Label>
                <p className="text-xs text-muted-foreground">
                  {t("instance.dockerDeleteRemoveDataHint")}
                </p>
              </div>
            </div>
            {dockerDeleteError && (
              <p className="text-xs text-destructive">{dockerDeleteError}</p>
            )}
          </div>
          <DialogFooter>
            <Button
              variant="outline"
              onClick={() => setDockerDeleteOpen(false)}
              disabled={dockerDeleting}
            >
              {t("instance.cancel")}
            </Button>
            <Button
              className="bg-destructive text-destructive-foreground hover:bg-destructive/90"
              onClick={handleDockerDeleteConfirm}
              disabled={dockerDeleting}
            >
              {dockerDeleting
                ? t("instance.deleting")
                : t("instance.dockerDeleteConfirm")}
            </Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>

      {/* SSH delete dialog */}
      <Dialog open={sshDeleteOpen} onOpenChange={setSshDeleteOpen}>
        <DialogContent>
          <DialogHeader>
            <DialogTitle>{t("instance.deleteTitle")}</DialogTitle>
            <DialogDescription>
              {t("instance.deleteDescription", {
                label: deletingHost?.label || deletingHost?.host || "",
              })}
            </DialogDescription>
          </DialogHeader>
          <DialogFooter>
            <Button
              variant="outline"
              onClick={() => setSshDeleteOpen(false)}
            >
              {t("instance.cancel")}
            </Button>
            <Button
              className="bg-destructive text-destructive-foreground hover:bg-destructive/90"
              onClick={() => {
                if (!deletingHost) return;
                onDeleteSsh(deletingHost.id);
                setSshDeleteOpen(false);
              }}
            >
              {t("instance.deleteConfirm")}
            </Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>
    </div>
  );
}
