import { useState } from "react";
import { useTranslation } from "react-i18next";
import { Button } from "@/components/ui/button";
import {
  Dialog,
  DialogContent,
  DialogHeader,
  DialogTitle,
  DialogFooter,
} from "@/components/ui/dialog";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
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
import { cn } from "@/lib/utils";
import { api } from "@/lib/api";
import type { DockerInstance, SshHost } from "@/lib/types";

interface InstanceTabBarProps {
  dockerInstances: DockerInstance[];
  hosts: SshHost[];
  activeId: string; // "local" or host.id
  connectionStatus: Record<string, "connected" | "disconnected" | "error">;
  onSelect: (id: string) => void;
  onHostsChange: () => void;
}

const emptyHost: Omit<SshHost, "id"> = {
  label: "",
  host: "",
  port: 22,
  username: "root",
  authMethod: "ssh_config",
  keyPath: undefined,
  password: undefined,
};

export function InstanceTabBar({
  dockerInstances,
  hosts,
  activeId,
  connectionStatus,
  onSelect,
  onHostsChange,
}: InstanceTabBarProps) {
  const { t } = useTranslation();
  const [dialogOpen, setDialogOpen] = useState(false);
  const [editingHost, setEditingHost] = useState<SshHost | null>(null);
  const [form, setForm] = useState<Omit<SshHost, "id">>(emptyHost);
  const [saving, setSaving] = useState(false);
  const [keyGuideOpen, setKeyGuideOpen] = useState(false);

  const openAddDialog = () => {
    setEditingHost(null);
    setForm({ ...emptyHost });
    setDialogOpen(true);
  };

  const openEditDialog = (host: SshHost) => {
    setEditingHost(host);
    setForm({
      label: host.label,
      host: host.host,
      port: host.port,
      username: host.username,
      authMethod: host.authMethod,
      keyPath: host.keyPath,
      password: host.password,
    });
    setDialogOpen(true);
  };

  const handleSave = () => {
    const host: SshHost = {
      id: editingHost?.id ?? crypto.randomUUID(),
      ...form,
    };
    setSaving(true);
    api
      .upsertSshHost(host)
      .then(() => {
        onHostsChange();
        setDialogOpen(false);
      })
      .catch((e) => console.error("Failed to save SSH host:", e))
      .finally(() => setSaving(false));
  };

  const handleDelete = (hostId: string) => {
    api
      .deleteSshHost(hostId)
      .then(() => {
        onHostsChange();
        if (activeId === hostId) onSelect("local");
      })
      .catch((e) => console.error("Failed to delete SSH host:", e));
  };

  const statusDot = (status: "connected" | "disconnected" | "error" | undefined) => {
    const color =
      status === "connected"
        ? "bg-emerald-500"
        : status === "error"
          ? "bg-red-400"
          : "bg-muted-foreground/40";
    return <span className={cn("inline-block w-2 h-2 rounded-full shrink-0 transition-colors duration-300", color)} />;
  };

  return (
    <>
      <div className="flex items-center gap-1 px-3 py-2 bg-sidebar border-b border-sidebar-border overflow-x-auto shrink-0">
        {/* Local tab */}
        <button
          className={cn(
            "flex items-center gap-1.5 px-3 py-1.5 rounded-lg text-sm whitespace-nowrap transition-all duration-200 cursor-pointer",
            activeId === "local"
              ? "bg-card shadow-sm font-semibold text-primary border-b-2 border-b-primary"
              : "text-muted-foreground hover:text-foreground"
          )}
          onClick={() => onSelect("local")}
        >
          {statusDot("connected")}
          {t('instance.local')}
        </button>

        {/* Docker tabs */}
        {dockerInstances.map((instance) => (
          <button
            key={instance.id}
            className={cn(
              "flex items-center gap-1.5 px-3 py-1.5 rounded-lg text-sm whitespace-nowrap transition-all duration-200 cursor-pointer",
              activeId === instance.id
                ? "bg-card shadow-sm font-semibold text-primary border-b-2 border-b-primary"
                : "text-muted-foreground hover:text-foreground"
            )}
            onClick={() => onSelect(instance.id)}
          >
            {statusDot("connected")}
            {instance.label}
          </button>
        ))}

        {/* Remote tabs */}
        {hosts.map((host) => (
          <div
            key={host.id}
            className="relative group flex items-center"
          >
            <button
              className={cn(
                "flex items-center gap-1.5 px-3 py-1.5 rounded-lg text-sm whitespace-nowrap transition-all duration-200 cursor-pointer",
                activeId === host.id
                  ? "bg-card shadow-sm font-semibold text-primary border-b-2 border-b-primary"
                  : "text-muted-foreground hover:text-foreground"
              )}
              onClick={() => onSelect(host.id)}
              onContextMenu={(e) => {
                e.preventDefault();
                openEditDialog(host);
              }}
            >
              {statusDot(connectionStatus[host.id])}
              {host.label || host.host}
            </button>
            <AlertDialog>
              <AlertDialogTrigger asChild>
                <button
                  className="absolute -top-0.5 -right-0.5 hidden group-hover:flex items-center justify-center w-4 h-4 rounded-full bg-muted-foreground/20 hover:bg-destructive hover:text-white text-[10px] leading-none"
                  onClick={(e) => {
                    e.stopPropagation();
                  }}
                >
                  &times;
                </button>
              </AlertDialogTrigger>
              <AlertDialogContent>
                <AlertDialogHeader>
                  <AlertDialogTitle>{t('instance.deleteTitle')}</AlertDialogTitle>
                  <AlertDialogDescription>
                    {t('instance.deleteDescription', { label: host.label || host.host })}
                  </AlertDialogDescription>
                </AlertDialogHeader>
                <AlertDialogFooter>
                  <AlertDialogCancel>{t('instance.cancel')}</AlertDialogCancel>
                  <AlertDialogAction
                    className="bg-destructive text-destructive-foreground hover:bg-destructive/90"
                    onClick={() => handleDelete(host.id)}
                  >
                    {t('instance.deleteConfirm')}
                  </AlertDialogAction>
                </AlertDialogFooter>
              </AlertDialogContent>
            </AlertDialog>
          </div>
        ))}

        {/* Add button */}
        <Button
          variant="ghost"
          size="sm"
          className="h-7 px-2 shrink-0 text-xs"
          onClick={openAddDialog}
        >
          {t('instance.addSsh')}
        </Button>
      </div>

      {/* Add/Edit Dialog */}
      <Dialog open={dialogOpen} onOpenChange={setDialogOpen}>
        <DialogContent>
          <DialogHeader>
            <DialogTitle>
              {editingHost ? t('instance.editRemote') : t('instance.addRemote')}
            </DialogTitle>
          </DialogHeader>
          <div className="space-y-4">
            <div className="space-y-1.5">
              <Label htmlFor="ssh-label">{t('instance.label')}</Label>
              <Input
                id="ssh-label"
                value={form.label}
                onChange={(e) => setForm((f) => ({ ...f, label: e.target.value }))}
                placeholder={t('instance.labelPlaceholder')}
              />
            </div>
            <div className="space-y-1.5">
              <Label htmlFor="ssh-host">{t('instance.host')}</Label>
              <Input
                id="ssh-host"
                value={form.host}
                onChange={(e) => setForm((f) => ({ ...f, host: e.target.value }))}
                placeholder="192.168.1.100"
                autoCapitalize="off"
                autoCorrect="off"
                spellCheck={false}
              />
            </div>
            <div className="space-y-1.5">
              <Label htmlFor="ssh-port">{t('instance.port')}</Label>
              <Input
                id="ssh-port"
                type="number"
                value={form.port}
                onChange={(e) =>
                  setForm((f) => ({ ...f, port: parseInt(e.target.value, 10) || 22 }))
                }
              />
            </div>
            <div className="space-y-1.5">
              <Label htmlFor="ssh-username">{t('instance.username')}</Label>
              <Input
                id="ssh-username"
                value={form.username}
                onChange={(e) => setForm((f) => ({ ...f, username: e.target.value }))}
                autoCapitalize="off"
                autoCorrect="off"
                spellCheck={false}
              />
            </div>
            <div className="space-y-1.5">
              <Label>{t('instance.authMethod')}</Label>
              <Select
                value={form.authMethod}
                onValueChange={(val) =>
                  setForm((f) => ({
                    ...f,
                    authMethod: val as SshHost["authMethod"],
                    keyPath: val === "key" ? (f.authMethod === "key" ? f.keyPath : "") : undefined,
                  }))
                }
              >
                <SelectTrigger>
                  <SelectValue />
                </SelectTrigger>
                <SelectContent>
                  <SelectItem value="ssh_config">{t('instance.authSshConfig')}</SelectItem>
                  <SelectItem value="key">{t('instance.authKey')}</SelectItem>
                </SelectContent>
              </Select>
              <button
                type="button"
                className="text-xs text-muted-foreground hover:text-foreground underline underline-offset-2 mt-1"
                onClick={() => setKeyGuideOpen(true)}
              >
                {t('instance.keyGuideLink')}
              </button>
            </div>
            {form.authMethod === "key" && (
              <div className="space-y-1.5">
                <Label htmlFor="ssh-keypath">{t('instance.keyPath')}</Label>
                <Input
                  id="ssh-keypath"
                  value={form.keyPath || ""}
                  onChange={(e) => setForm((f) => ({ ...f, keyPath: e.target.value }))}
                  placeholder="~/.ssh/id_rsa"
                  autoCapitalize="off"
                  autoCorrect="off"
                  spellCheck={false}
                />
              </div>
            )}
          </div>
          <DialogFooter>
            <Button variant="outline" onClick={() => setDialogOpen(false)} disabled={saving}>
              {t('instance.cancel')}
            </Button>
            <Button onClick={handleSave} disabled={saving || !form.host}>
              {saving ? t('instance.saving') : editingHost ? t('instance.update') : t('instance.add')}
            </Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>

      {/* SSH Key Setup Guide Dialog */}
      <Dialog open={keyGuideOpen} onOpenChange={setKeyGuideOpen}>
        <DialogContent className="max-w-lg">
          <DialogHeader>
            <DialogTitle>{t('instance.keyGuideTitle')}</DialogTitle>
          </DialogHeader>
          <div className="space-y-5 text-sm">
            {/* Step 1 */}
            <div className="space-y-1.5">
              <p className="font-medium">{t('instance.keyGuideStep1Title')}</p>
              <CopyBlock text="ssh-keygen -t ed25519" />
              <p className="text-xs text-muted-foreground">{t('instance.keyGuideStep1Hint')}</p>
            </div>
            {/* Step 2 */}
            <div className="space-y-1.5">
              <p className="font-medium">{t('instance.keyGuideStep2Title')}</p>
              <CopyBlock text={`ssh-copy-id ${form.username || "root"}@${form.host || "your-host"} -p ${form.port || 22}`} />
              <p className="text-xs text-muted-foreground">{t('instance.keyGuideStep2Hint')}</p>
            </div>
            {/* Step 3 */}
            <div className="space-y-1.5">
              <p className="font-medium">{t('instance.keyGuideStep3Title')}</p>
              <ul className="list-disc list-inside text-muted-foreground text-xs space-y-0.5">
                <li>{t('instance.keyGuideStep3Auth')}</li>
                <li>{t('instance.keyGuideStep3Path')}</li>
              </ul>
            </div>
          </div>
          <DialogFooter>
            <Button variant="outline" onClick={() => setKeyGuideOpen(false)}>
              {t('instance.keyGuideClose')}
            </Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>
    </>
  );
}

function CopyBlock({ text }: { text: string }) {
  const { t } = useTranslation();
  const [copied, setCopied] = useState(false);
  const handleCopy = () => {
    navigator.clipboard.writeText(text).then(() => {
      setCopied(true);
      setTimeout(() => setCopied(false), 1500);
    });
  };
  return (
    <div className="flex items-center gap-2 bg-muted rounded px-3 py-1.5 font-mono text-xs">
      <code className="flex-1 break-all">{text}</code>
      <button
        type="button"
        className="shrink-0 text-muted-foreground hover:text-foreground text-xs"
        onClick={handleCopy}
      >
        {copied ? t('instance.keyGuideCopied') : t('instance.keyGuideCopy')}
      </button>
    </div>
  );
}
