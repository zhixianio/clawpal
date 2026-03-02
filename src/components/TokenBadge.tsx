import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { Badge } from "@/components/ui/badge";

interface SessionUsageStats {
  totalCalls: number;
  usageCalls: number;
  promptTokens: number;
  completionTokens: number;
  totalTokens: number;
  lastUpdatedMs: number;
}

interface CostEstimate {
  model: string;
  promptTokens: number;
  completionTokens: number;
  estimatedCostUsd: number | null;
}

interface TokenBadgeProps {
  sessionId: string;
  model?: string;
}

export function TokenBadge({ sessionId, model }: TokenBadgeProps) {
  const [stats, setStats] = useState<SessionUsageStats | null>(null);
  const [cost, setCost] = useState<number | null>(null);

  useEffect(() => {
    if (!sessionId) return;

    const fetchStats = async () => {
      try {
        const usage = await invoke<SessionUsageStats>("get_session_usage_stats", {
          sessionId,
        });
        setStats(usage);

        if (model && (usage.promptTokens > 0 || usage.completionTokens > 0)) {
          const estimate = await invoke<CostEstimate>("estimate_query_cost", {
            model,
            promptTokens: usage.promptTokens,
            completionTokens: usage.completionTokens,
          });
          setCost(estimate.estimatedCostUsd);
        }
      } catch {
        // silently ignore
      }
    };

    fetchStats();
    const interval = setInterval(fetchStats, 5000);
    return () => clearInterval(interval);
  }, [sessionId, model]);

  if (!stats || stats.totalTokens === 0) return null;

  const formatTokens = (n: number) => {
    if (n >= 1000000) return `${(n / 1000000).toFixed(1)}M`;
    if (n >= 1000) return `${(n / 1000).toFixed(1)}k`;
    return String(n);
  };

  const formatCost = (c: number) => {
    if (c < 0.01) return `$${c.toFixed(4)}`;
    return `$${c.toFixed(2)}`;
  };

  return (
    <Badge variant="secondary" className="text-xs font-mono gap-1">
      <span>🪙 {formatTokens(stats.totalTokens)}</span>
      {cost !== null && <span className="text-muted-foreground">({formatCost(cost)})</span>}
    </Badge>
  );
}
