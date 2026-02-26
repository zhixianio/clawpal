import { useState } from "react";
import { useTranslation } from "react-i18next";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import type { SshHost } from "@/lib/types";

interface SshFormWidgetProps {
  invokeId: string;
  defaults?: Partial<SshHost>;
  onSubmit: (invokeId: string, host: SshHost) => void;
  onCancel: (invokeId: string) => void;
}

export function SshFormWidget({ invokeId, defaults, onSubmit, onCancel }: SshFormWidgetProps) {
  const { t } = useTranslation();
  const [host, setHost] = useState(defaults?.host ?? "");
  const [port, setPort] = useState(String(defaults?.port ?? 22));
  const [username, setUsername] = useState(defaults?.username ?? "");
  const [authMethod, setAuthMethod] = useState<"ssh_config" | "key">(
    (defaults?.authMethod as "ssh_config" | "key") ?? "ssh_config",
  );
  const [keyPath, setKeyPath] = useState(defaults?.keyPath ?? "");
  const [label, setLabel] = useState(defaults?.label ?? "");

  const isValid = host.trim().length > 0;

  const handleSubmit = () => {
    if (!isValid) return;
    onSubmit(invokeId, {
      id: "",
      label: label.trim() || host.trim(),
      host: host.trim(),
      port: parseInt(port, 10) || 22,
      username: username.trim(),
      authMethod,
      keyPath: authMethod === "key" ? keyPath.trim() : undefined,
    });
  };

  return (
    <div className="rounded-lg border p-3 space-y-3 bg-[oklch(0.96_0_0)] dark:bg-muted/50">
      <div className="text-xs font-medium">{t("installChat.sshFormTitle")}</div>
      <div className="grid grid-cols-3 gap-2">
        <div className="col-span-2 space-y-1">
          <label className="text-xs font-medium">{t("installChat.sshHost")}</label>
          <Input
            value={host}
            onChange={(e) => setHost(e.target.value)}
            placeholder="192.168.1.100"
            className="h-8 text-sm"
          />
        </div>
        <div className="space-y-1">
          <label className="text-xs font-medium">{t("installChat.sshPort")}</label>
          <Input
            value={port}
            onChange={(e) => setPort(e.target.value)}
            placeholder="22"
            className="h-8 text-sm"
          />
        </div>
      </div>
      <div className="space-y-1">
        <label className="text-xs font-medium">{t("installChat.sshUsername")}</label>
        <Input
          value={username}
          onChange={(e) => setUsername(e.target.value)}
          placeholder="root"
          className="h-8 text-sm"
        />
      </div>
      <div className="space-y-1">
        <label className="text-xs font-medium">{t("installChat.sshAuthMethod")}</label>
        <Select
          value={authMethod}
          onValueChange={(v) => setAuthMethod(v as "ssh_config" | "key")}
        >
          <SelectTrigger className="h-8 text-sm">
            <SelectValue />
          </SelectTrigger>
          <SelectContent>
            <SelectItem value="ssh_config">{t("installChat.sshAuthSshConfig")}</SelectItem>
            <SelectItem value="key">{t("installChat.sshAuthKey")}</SelectItem>
          </SelectContent>
        </Select>
      </div>
      {authMethod === "key" && (
        <div className="space-y-1">
          <label className="text-xs font-medium">{t("installChat.sshKeyPath")}</label>
          <Input
            value={keyPath}
            onChange={(e) => setKeyPath(e.target.value)}
            placeholder="~/.ssh/id_ed25519"
            className="h-8 text-sm"
          />
        </div>
      )}
      <div className="space-y-1">
        <label className="text-xs font-medium">{t("installChat.sshLabel")}</label>
        <Input
          value={label}
          onChange={(e) => setLabel(e.target.value)}
          placeholder={host || "My Server"}
          className="h-8 text-sm"
        />
      </div>
      <div className="flex items-center gap-2">
        <Button size="sm" onClick={handleSubmit} disabled={!isValid}>
          {t("installChat.submit")}
        </Button>
        <Button size="sm" variant="outline" onClick={() => onCancel(invokeId)}>
          {t("installChat.cancel")}
        </Button>
      </div>
    </div>
  );
}
