import { useTranslation } from "react-i18next";
import { MonitorIcon, ContainerIcon, ServerIcon, EllipsisIcon, PencilIcon, Trash2Icon, RefreshCwIcon } from "lucide-react";
import { Card, CardContent } from "@/components/ui/card";
import { Badge } from "@/components/ui/badge";
import { Popover, PopoverTrigger, PopoverContent } from "@/components/ui/popover";
import { Button } from "@/components/ui/button";
import { cn } from "@/lib/utils";

type InstanceType = "local" | "docker" | "ssh";

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
}

const typeIcons: Record<InstanceType, typeof MonitorIcon> = {
  local: MonitorIcon,
  docker: ContainerIcon,
  ssh: ServerIcon,
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
}: InstanceCardProps) {
  const { t } = useTranslation();
  const TypeIcon = typeIcons[type];

  const hasMenu = !!(onRename || onEdit || onDelete);

  // SSH instances that haven't been checked yet: no status to show
  const needsCheck = type === "ssh" && !checked && !checking;

  return (
    <Card
      className={cn(
        "cursor-pointer transition-all duration-300 group relative",
        "hover:shadow-[var(--shadow-warm-hover)]",
        opened && "border-primary/30",
      )}
      onClick={onClick}
    >
      <CardContent className="flex flex-col gap-3">
        {/* Top row: type icon + menu */}
        <div className="flex items-center justify-between">
          <TypeIcon className="size-5 text-muted-foreground" />
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

        {/* Label */}
        <div className="font-bold truncate">{label}</div>

        {/* Bottom row: health + agent count */}
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
                <HealthDot healthy={healthy} offline={checked === true && healthy === null} />
                {checked === true && healthy === null
                  ? t("start.unreachable")
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
      </CardContent>
    </Card>
  );
}
