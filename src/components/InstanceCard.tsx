import { useTranslation } from "react-i18next";
import { MonitorIcon, ContainerIcon, ServerIcon, LaptopIcon, EllipsisIcon, PencilIcon, Trash2Icon, RefreshCwIcon, LinkIcon, StethoscopeIcon } from "lucide-react";
import type { TFunction } from "i18next";
import { Card, CardContent } from "@/components/ui/card";
import { Badge } from "@/components/ui/badge";
import { Popover, PopoverTrigger, PopoverContent } from "@/components/ui/popover";
import { Button } from "@/components/ui/button";
import { cn } from "@/lib/utils";
import type { SshConnectionProfile, SshConnectionBottleneckStage } from "@/lib/types";
import {
  getConnectionQualityLabel,
  getConnectionStageLabel,
  getSshDotClass,
} from "./instance-card-helpers";
import {
  formatSshConnectionLatency,
} from "@/lib/sshConnectionProfile";

type InstanceType = "local" | "docker" | "ssh" | "wsl2";

interface InstanceCardProps {
  id: string;
  label: string;
  type: InstanceType;
  healthy: boolean | null; // null = unknown/loading
  agentCount: number;
  opened: boolean; // whether this instance is currently open in tab bar
  notInstalled?: boolean;
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
  sshConnectionProfile?: SshConnectionProfile;
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

function SshConnectionDot({ profile, t }: { profile: SshConnectionProfile; t: TFunction }) {
  const qualityText = getConnectionQualityLabel(profile.quality, t);
  const qualityClass = getSshDotClass(profile.quality);
  const bottleneckStageLabel = getConnectionStageLabel(profile.bottleneck.stage, t);
  const bottleneckLatencyText = formatSshConnectionLatency(profile.bottleneck.latencyMs);
  const totalLatencyText = formatSshConnectionLatency(profile.totalLatencyMs);

  return (
    <Popover>
      <PopoverTrigger asChild>
        <button
          type="button"
          className="-m-2 inline-flex items-center gap-1.5 rounded-md p-2 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring/50"
          aria-label={qualityText}
          onClick={(e) => e.stopPropagation()}
        >
          <span
            className={cn(
              "size-2 rounded-full shrink-0 animate-ssh-dot-jitter",
              qualityClass,
            )}
          />
          <span className="leading-none text-muted-foreground">{qualityText}</span>
        </button>
      </PopoverTrigger>
      <PopoverContent
        align="start"
        side="bottom"
        sideOffset={8}
        className="w-60 p-3 text-xs"
        onClick={(e) => e.stopPropagation()}
        onOpenAutoFocus={(e) => e.preventDefault()}
      >
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
      </PopoverContent>
    </Popover>
  );
}

export function InstanceCard({
  label,
  type,
  healthy,
  agentCount,
  opened,
  notInstalled = false,
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
  sshConnectionProfile,
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
                  className="size-7 opacity-0 group-hover:opacity-100 transition-opacity duration-200"
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
            {notInstalled ? (
              <span>{t("start.notInstalled")}</span>
            ) : needsCheck ? (
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
                {type === "ssh" && showSshConnectionProfile && sshConnectionProfile ? (
                  <SshConnectionDot profile={sshConnectionProfile} t={t} />
                ) : (
                  <span className="flex items-center gap-1.5">
                    <HealthDot healthy={healthy} offline={checked === true && healthy === null} />
                    {checked === true && healthy === null
                      ? t("start.unreachable")
                      : healthy === true
                        ? t("start.healthy")
                        : healthy === false
                          ? t("start.unhealthy")
                          : t("start.checking")}
                  </span>
                )}
                <Badge variant="secondary" className="text-xs">
                  {t("start.agents", { count: agentCount })}
                </Badge>
              </>
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
