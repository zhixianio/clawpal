import { useTranslation } from "react-i18next";
import { MonitorIcon, ContainerIcon, ServerIcon, LaptopIcon, EllipsisIcon, PencilIcon, Trash2Icon, RefreshCwIcon, LinkIcon, StethoscopeIcon } from "lucide-react";
import { Card, CardContent } from "@/components/ui/card";
import { Badge } from "@/components/ui/badge";
import { Popover, PopoverTrigger, PopoverContent } from "@/components/ui/popover";
import { Button } from "@/components/ui/button";
import { cn } from "@/lib/utils";
import type { SshConnectionProfile, SshConnectionBottleneckStage } from "@/lib/types";

type InstanceType = "local" | "docker" | "ssh" | "wsl2";

interface InstanceCardProps {
  id: string;
  label: string;
  type: InstanceType;
  healthy: boolean | null; // null = unknown/loading
  agentCount: number;
  opened: boolean; // whether this instance is currently open in tab bar
  checked?: boolean; // whether health has been checked (SSH only)
  checking?: boolean; // whether a check is in progress (SSH only)
  onCheck?: () => void; // trigger manual health check (SSH only)
  onClick: () => void;
  onRename?: () => void;
  onEdit?: () => void; // SSH edit only
  onDelete?: () => void;
  discovered?: boolean;
  discoveredSource?: string;
  onConnect?: () => void;
  onQuickDiagnose?: (() => void) | null;
}

const typeIcons: Record<InstanceType, typeof MonitorIcon> = {
  local: MonitorIcon,
  docker: ContainerIcon,
  ssh: ServerIcon,
  wsl2: LaptopIcon,
};

function HealthDot({ healthy, offline }: { healthy: boolean | null; offline: boolean }) {
  return (
    <span
      className={cn(
        "inline-block size-2 rounded-full shrink-0",
        offline && "bg-muted-foreground/40",
        !offline && healthy === true && "bg-green-500",
        !offline && healthy === false && "bg-red-500",
        !offline && healthy === null && "bg-muted-foreground/40 animate-pulse",
      )}
    />
  );
}

function formatLatency(ms: number): string {
  if (!Number.isFinite(ms) || ms < 0) return "-";
  if (ms >= 1000) return `${(ms / 1000).toFixed(2)} s`;
  return `${Math.round(ms)} ms`;
}

function getConnectionQualityLabel(quality: string, t: TFunction): string {
  switch (quality) {
    case "excellent":
      return t("start.sshQualityExcellent");
    case "good":
      return t("start.sshQualityGood");
    case "fair":
      return t("start.sshQualityFair");
    case "poor":
      return t("start.sshQualityPoor");
    default:
      return t("start.sshQualityUnknown");
  }
}

function getConnectionStageLabel(stage: SshConnectionBottleneckStage, t: TFunction): string {
  switch (stage) {
    case "connect":
      return t("start.sshStage.connect");
    case "gateway":
      return t("start.sshStage.gateway");
    case "config":
      return t("start.sshStage.config");
    case "version":
      return t("start.sshStage.version");
    default:
      return t("start.sshStage.other");
  }
}

function getSshDotClass(quality: string): string {
  switch (quality) {
    case "excellent":
      return "bg-emerald-500 shadow-[0_0_12px_rgba(16,185,129,0.45)]";
    case "good":
      return "bg-lime-500 shadow-[0_0_12px_rgba(132,204,22,0.45)]";
    case "fair":
      return "bg-amber-500 shadow-[0_0_12px_rgba(217,119,6,0.45)]";
    case "poor":
      return "bg-red-500 shadow-[0_0_12px_rgba(220,38,38,0.45)]";
    default:
      return "bg-muted-foreground/40";
  }
}

function SshConnectionDot({ profile, t }: { profile: SshConnectionProfile; t: TFunction }) {
  const qualityText = getConnectionQualityLabel(profile.quality, t);
  const qualityClass = getSshDotClass(profile.quality);
  const bottleneckStageLabel = getConnectionStageLabel(profile.bottleneck.stage, t);
  const bottleneckLatencyText = formatLatency(profile.bottleneck.latencyMs);
  const totalLatencyText = formatLatency(profile.totalLatencyMs);

  return (
    <span className="relative inline-flex items-center justify-center group/ssh-dot">
      <span
        tabIndex={0}
        className={cn(
          "size-2 rounded-full shrink-0 animate-ssh-dot-jitter",
          qualityClass,
        )}
      />
      <span className="pointer-events-none absolute left-1/2 top-full z-10 mt-2 w-max max-w-56 -translate-x-1/2 rounded-md border border-border bg-popover px-2.5 py-2 text-xs text-popover-foreground shadow-[0_8px_24px_rgba(0,0,0,0.18)] opacity-0 invisible transition-all duration-150 group-hover/ssh-dot:opacity-100 group-hover/ssh-dot:visible group-focus-within/ssh-dot:opacity-100 group-focus-within/ssh-dot:visible">
        <p className="font-semibold text-[12px] mb-1">{t("start.sshConnectionMetrics")}</p>
        <p>
          {t("start.sshSpeed")}: {totalLatencyText}
        </p>
        <p>
          {t("start.sshQualityLabel")}: {qualityText} ({profile.qualityScore})
        </p>
        <p>
          {t("start.sshBottleneck")}: {bottleneckStageLabel} ({bottleneckLatencyText})
        </p>
      </span>
    </span>
  );
}

export function InstanceCard({
  label,
  type,
  healthy,
  agentCount,
  opened,
  checked,
  checking,
  onCheck,
  onClick,
  onRename,
  onEdit,
  onDelete,
  discovered,
  discoveredSource,
  onConnect,
  onQuickDiagnose,
}: InstanceCardProps) {
  const { t } = useTranslation();
  const TypeIcon = typeIcons[type];
  const typeLabel = (() => {
    if (type === "local") return t("instance.typeLocal");
    if (type === "docker") return t("instance.typeDocker");
    if (type === "ssh") return t("instance.typeSsh");
    return t("instance.typeWsl2");
  })();

  const hasMenu = !!(onRename || onEdit || onDelete);

  // SSH instances that haven't been checked yet: no status to show
  const needsCheck = type === "ssh" && !checked && !checking;
  const showSshConnectionProfile = type === "ssh" && checked === true && !!sshConnectionProfile;
  const sshConnectionQualityText = sshConnectionProfile
    ? getConnectionQualityLabel(sshConnectionProfile.quality, t)
    : undefined;

  return (
    <Card
      className={cn(
        "cursor-pointer transition-all duration-300 group relative",
        "hover:shadow-[var(--shadow-warm-hover)]",
        discovered && "border-dashed border-2 border-muted-foreground/30",
        opened && "border-primary/30",
      )}
      onClick={discovered ? undefined : onClick}
    >
      <CardContent className="flex flex-col gap-3">
        {/* Top row: type icon + type label + menu */}
        <div className="flex items-center justify-between">
          <div className="flex items-center gap-2">
            <TypeIcon className="size-5 text-muted-foreground" />
            <Badge variant="outline" className="text-[10px] uppercase tracking-wide">
              {typeLabel}
            </Badge>
          </div>
          {(onQuickDiagnose || hasMenu) && (
            <div className="flex items-center gap-1">
              {onQuickDiagnose && (
                <Button
                  variant="ghost"
                  size="icon"
                  className="size-7"
                  onClick={(e) => { e.stopPropagation(); onQuickDiagnose(); }}
                  aria-label={t("quickDiagnose.buttonLabel")}
                >
                  <StethoscopeIcon className="size-4" />
                </Button>
              )}
              {hasMenu && (
                <Popover>
                  <PopoverTrigger asChild>
                    <Button
                      variant="ghost"
                      size="icon"
                      className="size-7 opacity-0 group-hover:opacity-100 transition-opacity duration-200"
                      onClick={(e) => e.stopPropagation()}
                    >
                      <EllipsisIcon className="size-4" />
                    </Button>
                  </PopoverTrigger>
                  <PopoverContent
                    align="end"
                    className="w-40 p-1"
                    onClick={(e) => e.stopPropagation()}
                  >
                    <div className="flex flex-col">
                      {onRename && (
                        <button
                          className="flex items-center gap-2 rounded-sm px-2 py-1.5 text-sm hover:bg-accent transition-colors text-left"
                          onClick={onRename}
                        >
                          <PencilIcon className="size-3.5" />
                          {t("start.menuRename")}
                        </button>
                      )}
                      {onEdit && (
                        <button
                          className="flex items-center gap-2 rounded-sm px-2 py-1.5 text-sm hover:bg-accent transition-colors text-left"
                          onClick={onEdit}
                        >
                          <TypeIcon className="size-3.5" />
                          {t("start.menuEdit")}
                        </button>
                      )}
                      {onDelete && (
                        <button
                          className="flex items-center gap-2 rounded-sm px-2 py-1.5 text-sm text-destructive hover:bg-destructive/10 transition-colors text-left"
                          onClick={onDelete}
                        >
                          <Trash2Icon className="size-3.5" />
                          {t("start.menuDelete")}
                        </button>
                      )}
                    </div>
                  </PopoverContent>
                </Popover>
              )}
            </div>
          )}
        </div>

        {/* Label */}
        <div className="font-bold truncate">{label}</div>

        {/* Bottom row: health + agent count */}
        {!discovered && (
          <div className="flex items-center gap-3 text-sm text-muted-foreground">
            {needsCheck ? (
              <Button
                variant="outline"
                size="sm"
                className="h-6 text-xs gap-1.5"
                onClick={(e) => { e.stopPropagation(); onCheck?.(); }}
              >
                <RefreshCwIcon className="size-3" />
                {t("start.check")}
              </Button>
            ) : checking ? (
              <span className="flex items-center gap-1.5">
                <RefreshCwIcon className="size-3 animate-spin" />
                {t("start.checking")}
              </span>
            ) : (
              <>
                <span className="flex items-center gap-1.5">
                  {type === "ssh" && showSshConnectionProfile && sshConnectionProfile ? (
                    <SshConnectionDot profile={sshConnectionProfile} t={t} />
                  ) : (
                    <HealthDot healthy={healthy} offline={checked === true && healthy === null} />
                  )}
                  {checked === true && healthy === null
                    ? t("start.unreachable")
                    : showSshConnectionProfile && sshConnectionProfile
                      ? sshConnectionQualityText
                      : healthy === true
                        ? t("start.healthy")
                        : healthy === false
                          ? t("start.unhealthy")
                          : t("start.checking")}
                </span>
                <Badge variant="secondary" className="text-xs">
                  {t("start.agents", { count: agentCount })}
                </Badge>
              </>
            )}
            {opened && (
              <Badge variant="outline" className="text-xs">
                {t("start.opened")}
              </Badge>
            )}
          </div>
        )}

        {discovered && discoveredSource && (
          <div className="text-xs text-muted-foreground">
            {discoveredSource === "container" ? t("start.fromContainer") : t("start.fromDataDir")}
          </div>
        )}

        {discovered && onConnect && (
          <Button
            size="sm"
            className="w-full gap-1.5"
            onClick={(e) => { e.stopPropagation(); onConnect(); }}
          >
            <LinkIcon className="size-3.5" />
            {t("start.connect")}
          </Button>
        )}
      </CardContent>
    </Card>
  );
}
