import { cn } from "@/lib/utils";

export function InlineProgressBar({
  title,
  detail,
  value,
  tone = "primary",
  animated = false,
}: {
  title: string;
  detail?: string;
  value: number;
  tone?: "primary" | "destructive";
  animated?: boolean;
}) {
  const clampedValue = Math.max(0, Math.min(100, Math.round(value)));

  return (
    <div className="rounded-xl border border-border/70 bg-muted/20 px-3 py-3">
      <div className="flex items-center justify-between gap-3 text-xs">
        <div className="min-w-0">
          <div className="font-medium text-foreground">{title}</div>
          {detail ? (
            <div className="mt-0.5 truncate text-muted-foreground">{detail}</div>
          ) : null}
        </div>
        <div className="shrink-0 font-mono text-muted-foreground">{clampedValue}%</div>
      </div>
      <div className="mt-2 h-2 overflow-hidden rounded-full bg-muted">
        <div
          className={cn(
            "h-full rounded-full transition-all duration-300",
            tone === "destructive" ? "bg-destructive" : "bg-primary",
            animated && "animate-pulse",
          )}
          style={{ width: `${clampedValue}%` }}
        />
      </div>
    </div>
  );
}
