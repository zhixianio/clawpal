import { useState, type ReactNode } from "react";
import { useTranslation } from "react-i18next";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { SimpleMarkdown } from "@/components/SimpleMarkdown";
import type { DoctorChatMessage } from "@/lib/types";

interface AgentMessageBubbleProps {
  message: DoctorChatMessage;
  onApprove: (id: string) => void;
  onReject: (id: string, reason?: string) => void;
  extraRenderer?: (message: DoctorChatMessage) => ReactNode | null;
}

export function AgentMessageBubble({
  message,
  onApprove,
  onReject,
  extraRenderer,
}: AgentMessageBubbleProps) {
  const { t } = useTranslation();
  const [expanded, setExpanded] = useState(false);

  // Let caller handle special messages (e.g. A2UI form widgets)
  if (extraRenderer) {
    const custom = extraRenderer(message);
    if (custom !== null) return <>{custom}</>;
  }

  if (message.role === "user") {
    return (
      <div className="flex justify-end">
        <div className="px-3 py-2 rounded-lg max-w-[85%] bg-[oklch(0.205_0_0)] dark:bg-[oklch(0.35_0.02_55)] text-white">
          <div className="whitespace-pre-wrap text-sm">{message.content}</div>
        </div>
      </div>
    );
  }

  if (message.role === "assistant") {
    return (
      <div className="flex justify-start">
        <div className="px-3 py-2 rounded-lg max-w-[85%] bg-[oklch(0.93_0_0)] dark:bg-muted dark:text-foreground">
          <div className="text-sm"><SimpleMarkdown content={message.content} /></div>
        </div>
      </div>
    );
  }

  if (message.role === "tool-call" && message.invoke) {
    const inv = message.invoke;
    const isPendingWrite = message.status === "pending" && inv.type === "write";
    const isPendingRead = message.status === "pending" && inv.type === "read";
    const statusBadge = message.status === "auto"
      ? <Badge variant="outline" className="text-xs">{t("doctor.autoExecuted")}</Badge>
      : message.status === "approved"
        ? <Badge variant="secondary" className="text-xs">{t("doctor.execute")}</Badge>
        : message.status === "rejected"
          ? <Badge variant="destructive" className="text-xs">{t("doctor.rejected")}</Badge>
          : isPendingRead
            ? <Badge variant="outline" className="text-xs">{t("doctor.firstTimeApproval")}</Badge>
            : <Badge variant="secondary" className="text-xs">{t("doctor.awaitingApproval")}</Badge>;

    return (
      <div className="rounded-md p-3 text-sm border-l-[3px] border-l-primary/40 border border-border bg-[oklch(0.96_0_0)] dark:bg-muted/50">
        <div className="flex items-center justify-between mb-1">
          <span className="font-mono font-medium text-xs">{inv.command}</span>
          <div className="flex items-center gap-2">
            {isPendingWrite && (
              <>
                <Button size="sm" variant="default" onClick={() => onApprove(inv.id)}>
                  {t("doctor.execute")}
                </Button>
                <Button size="sm" variant="outline" onClick={() => onReject(inv.id)}>
                  {t("doctor.skip")}
                </Button>
              </>
            )}
            {isPendingRead && (
              <Button size="sm" variant="outline" onClick={() => onApprove(inv.id)}>
                {t("doctor.allowRead")}
              </Button>
            )}
            {statusBadge}
          </div>
        </div>
        {inv.args && Object.keys(inv.args).length > 0 && (
          <pre className="text-xs text-muted-foreground bg-muted rounded p-2 mt-1 overflow-auto max-h-24">
            {JSON.stringify(inv.args, null, 2)}
          </pre>
        )}
      </div>
    );
  }

  if (message.role === "tool-result") {
    return (
      <div className="rounded-md text-sm border-l-[3px] border-l-border border border-border bg-[oklch(0.95_0_0)] dark:bg-muted/30">
        <button
          className="w-full text-left px-3 py-2 text-xs text-muted-foreground hover:text-foreground"
          onClick={() => setExpanded(!expanded)}
        >
          {expanded ? t("doctor.collapse") : t("doctor.details")}
        </button>
        {expanded && (
          <pre className="px-3 pb-2 text-xs font-mono overflow-auto max-h-48 whitespace-pre-wrap break-all">
            {message.content}
          </pre>
        )}
      </div>
    );
  }

  return null;
}
