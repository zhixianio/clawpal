import { XIcon } from "lucide-react";
import { Button } from "@/components/ui/button";
import type { GuidanceAction } from "../lib/types";

export interface AgentGuidanceItem {
  message: string;
  summary: string;
  actions: string[];
  structuredActions?: GuidanceAction[];
  source: string;
  operation: string;
  instanceId: string;
  transport: string;
  rawError: string;
  createdAt: number;
}

interface GuidanceCardProps {
  guidance: AgentGuidanceItem;
  instanceLabel: string;
  onClose: () => void;
  onDismiss: () => void;
  onDoctorHandoff: (context?: string) => void;
  onInlineFix: (action: GuidanceAction) => Promise<void>;
}

export function GuidanceCard({
  guidance,
  instanceLabel,
  onClose,
  onDismiss,
  onDoctorHandoff,
  onInlineFix,
}: GuidanceCardProps) {
  return (
    <div className="w-[420px] max-w-[calc(100vw-2rem)] rounded-xl border border-border bg-card shadow-xl p-4 space-y-3">
      <div className="flex items-start justify-between gap-3">
        <div>
          <div className="text-sm font-semibold">小龙虾建议</div>
          <div className="text-xs text-muted-foreground">
            {instanceLabel} · {guidance.operation}
          </div>
        </div>
        <Button
          variant="ghost"
          size="icon-xs"
          onClick={onClose}
        >
          <XIcon className="size-4" />
        </Button>
      </div>
      <p className="text-sm leading-relaxed">{guidance.summary || guidance.message}</p>
      {guidance.actions.length > 0 && (
        <ol className="text-sm space-y-1.5 list-decimal pl-5">
          {guidance.actions.map((action, idx) => (
            <li key={`${idx}-${action}`}>{action}</li>
          ))}
        </ol>
      )}
      <div className="flex flex-wrap items-center gap-2 pt-1">
        {(guidance.structuredActions ?? []).map((sa, idx) => (
          sa.actionType === "inline_fix" ? (
            <Button
              key={`sa-${idx}`}
              size="sm"
              variant="outline"
              onClick={() => onInlineFix(sa)}
            >
              {sa.label}
            </Button>
          ) : (
            <Button
              key={`sa-${idx}`}
              size="sm"
              onClick={() => onDoctorHandoff(sa.context)}
            >
              {sa.label}
            </Button>
          )
        ))}
        {(!guidance.structuredActions || guidance.structuredActions.length === 0) && (
          <Button
            size="sm"
            onClick={() => onDoctorHandoff()}
          >
            打开 Doctor
          </Button>
        )}
        <Button
          size="sm"
          variant="outline"
          onClick={onDismiss}
        >
          稍后处理
        </Button>
      </div>
    </div>
  );
}
