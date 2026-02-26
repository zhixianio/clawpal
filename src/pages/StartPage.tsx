import { useCallback, useEffect, useState } from "react";
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
import type { DockerInstance, SshHost, InstallSession, RegisteredInstance } from "@/lib/types";

interface StartPageProps {
  dockerInstances: DockerInstance[];
  sshHosts: SshHost[];
  openTabIds: Set<string>;
  onOpenInstance: (id: string) => void;
  onRenameDocker: (id: string, label: string) => void;
  onDeleteDocker: (instance: DockerInstance, deleteData: boolean) => Promise<void>;
  onDeleteSsh: (hostId: string) => void;
  onEditSsh: (host: SshHost) => void;
  onInstallReady: (session: InstallSession) => void;
  showToast: (message: string, type?: "success" | "error") => void;
  onNavigate: (route: string) => void;
}

export function StartPage({
  dockerInstances,
  sshHosts,
  openTabIds,
  onOpenInstance,
  onRenameDocker,
  onDeleteDocker,
  onDeleteSsh,
  onEditSsh,
  onInstallReady,
  showToast,
  onNavigate,
}: StartPageProps) {
  const { t } = useTranslation();

  // Health state
  const [healthMap, setHealthMap] = useState<
    Record<string, { healthy: boolean | null; agentCount: number }>
  >({});

  // SSH manual check state: tracks which hosts have been checked / are checking
  const [sshChecked, setSshChecked] = useState<Record<string, boolean>>({});
  const [sshChecking, setSshChecking] = useState<Record<string, boolean>>({});
  const [registeredInstances, setRegisteredInstances] = useState<RegisteredInstance[]>([]);

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
    const refresh = async () => {
      try {
        const data = await api.listRegisteredInstances();
        if (!cancelled) setRegisteredInstances(data);
      } catch {
        if (!cancelled) setRegisteredInstances([]);
      }
    };
    refresh();
    const timer = setInterval(refresh, 30_000);
    return () => {
      cancelled = true;
      clearInterval(timer);
    };
  }, []);

  useEffect(() => {
    let cancelled = false;
    const poll = async () => {
      const updates: Record<string, { healthy: boolean | null; agentCount: number }> = {};

      // Poll local instance
      try {
        const status = await api.getInstanceStatus();
        updates.local = { healthy: status.healthy, agentCount: status.activeAgents };
      } catch {
        updates.local = { healthy: null, agentCount: 0 };
      }

      const dockerTargets =
        registeredInstances.filter((r) => r.instanceType === "docker").length > 0
          ? registeredInstances
              .filter((r) => r.instanceType === "docker")
              .map((r) => ({
                id: r.id,
                openclawHome: r.openclawHome || undefined,
                clawpalDataDir: r.clawpalDataDir || undefined,
              }))
          : dockerInstances;

      // Poll each Docker instance using its own openclawHome
      for (const d of dockerTargets) {
        if (cancelled) break;
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
    };
    poll();
    const timer = setInterval(poll, 30_000);
    return () => {
      cancelled = true;
      clearInterval(timer);
    };
  }, [dockerInstances, registeredInstances]);

  // Manual SSH health check
  const handleSshCheck = useCallback(async (hostId: string) => {
    setSshChecking((prev) => ({ ...prev, [hostId]: true }));
    try {
      await api.sshConnect(hostId);
      const status = await api.remoteGetInstanceStatus(hostId);
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
  }, []);

  // Build unified instances list
  const instances = registeredInstances.length > 0
    ? registeredInstances.map((r) => ({
        id: r.id,
        label: r.label || r.id,
        type: r.instanceType === "remote_ssh" ? "ssh" as const : r.instanceType as "local" | "docker",
      }))
    : [
        { id: "local", label: t("instance.local"), type: "local" as const },
        ...dockerInstances.map((d) => ({
          id: d.id,
          label: d.label || d.id,
          type: "docker" as const,
        })),
        ...sshHosts.map((h) => ({
          id: h.id,
          label: h.label || h.host,
          type: "ssh" as const,
        })),
      ];

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
