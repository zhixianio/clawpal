import { useMemo, useState } from "react";
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
import type { SshConfigHostSuggestion, SshHost } from "@/lib/types";

interface SshFormWidgetProps {
  invokeId: string;
  defaults?: Partial<SshHost>;
  sshConfigSuggestions?: SshConfigHostSuggestion[];
  onSubmit: (invokeId: string, host: SshHost) => void;
  onCancel: (invokeId: string) => void;
}

const SSH_CONFIG_MANUAL_ALIAS = "__manual__";

export function SshFormWidget({
  invokeId,
  defaults,
  sshConfigSuggestions = [],
  onSubmit,
  onCancel,
}: SshFormWidgetProps) {
  const { t } = useTranslation();
  const [host, setHost] = useState(defaults?.host ?? "");
  const [port, setPort] = useState(String(defaults?.port ?? 22));
  const [username, setUsername] = useState(defaults?.username ?? "");
  const [authMethod, setAuthMethod] = useState<"ssh_config" | "key" | "password">(
    (defaults?.authMethod as "ssh_config" | "key" | "password") ?? "key",
  );
  const [keyPath, setKeyPath] = useState(defaults?.keyPath ?? "");
  const [password, setPassword] = useState(defaults?.password ?? "");
  const [label, setLabel] = useState(defaults?.label ?? "");
  const [selectedSshConfigAlias, setSelectedSshConfigAlias] = useState(
    SSH_CONFIG_MANUAL_ALIAS,
  );

  const filteredSshConfigHosts = useMemo(() => {
    const seen = new Set<string>();
    const sorted = [...sshConfigSuggestions]
      .filter((item) => {
        const key = item.hostAlias.trim();
        if (!key || seen.has(key)) return false;
        seen.add(key);
        return true;
      })
      .sort((a, b) => a.hostAlias.localeCompare(b.hostAlias, undefined, { sensitivity: "base" }));
    return sorted;
  }, [sshConfigSuggestions]);

  const applySshConfigSuggestion = (alias: string) => {
    if (alias === SSH_CONFIG_MANUAL_ALIAS) {
      setSelectedSshConfigAlias(SSH_CONFIG_MANUAL_ALIAS);
      return;
    }
    const preset = filteredSshConfigHosts.find((item) => item.hostAlias === alias);
    if (!preset) {
      setSelectedSshConfigAlias(SSH_CONFIG_MANUAL_ALIAS);
      return;
    }
    setSelectedSshConfigAlias(alias);
    setHost(preset.hostAlias);
    setUsername(preset.user ?? "");
    setPort(String(preset.port ?? 22));
    setKeyPath(preset.identityFile ?? "");
    setPassword("");
    setAuthMethod("ssh_config");
    setLabel(preset.hostAlias);
  };

  const isValid = host.trim().length > 0 && (authMethod !== "password" || password.length > 0);

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
      password: authMethod === "password" ? password : undefined,
    });
  };

  return (
    <div className="rounded-lg border p-3 space-y-3 bg-[oklch(0.96_0_0)] dark:bg-muted/50">
      <div className="text-xs font-medium">{t("installChat.sshFormTitle")}</div>
      {filteredSshConfigHosts.length > 0 && (
        <div className="space-y-1">
          <label className="text-xs font-medium">{t("installChat.sshConfigPreset")}</label>
          <Select
            value={selectedSshConfigAlias}
            onValueChange={applySshConfigSuggestion}
          >
            <SelectTrigger className="h-8 text-sm">
              <SelectValue placeholder={t("installChat.sshConfigPresetPlaceholder")} />
            </SelectTrigger>
            <SelectContent>
              <SelectItem value={SSH_CONFIG_MANUAL_ALIAS}>
                {t("installChat.sshConfigPresetManual")}
              </SelectItem>
              {filteredSshConfigHosts.map((item) => (
                <SelectItem key={item.hostAlias} value={item.hostAlias}>
                  {item.hostAlias}
                  {item.hostName ? ` (${item.hostName})` : ""}
                  {item.user ? ` • ${item.user}` : ""}
                  {item.port && item.port !== 22 ? `:${item.port}` : ""}
                </SelectItem>
              ))}
            </SelectContent>
          </Select>
          <p className="text-xs text-muted-foreground">{t("installChat.sshConfigPresetHint")}</p>
        </div>
      )}
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
          onValueChange={(v) => setAuthMethod(v as "ssh_config" | "key" | "password")}
        >
          <SelectTrigger className="h-8 text-sm">
            <SelectValue />
          </SelectTrigger>
          <SelectContent>
            <SelectItem value="ssh_config">{t("installChat.sshAuthSshConfig")}</SelectItem>
            <SelectItem value="key">{t("installChat.sshAuthKey")}</SelectItem>
            <SelectItem value="password">{t("installChat.sshAuthPassword")}</SelectItem>
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
      {authMethod === "password" && (
        <div className="space-y-1">
          <label className="text-xs font-medium">{t("installChat.sshPassword")}</label>
          <Input
            type="password"
            value={password}
            onChange={(e) => setPassword(e.target.value)}
            placeholder="••••••••"
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
