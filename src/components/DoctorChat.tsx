import { useEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { AgentMessageBubble } from "@/components/AgentMessageBubble";
import type { DoctorChatMessage } from "@/lib/types";

interface DoctorChatProps {
  messages: DoctorChatMessage[];
  loading: boolean;
  error: string | null;
  connected: boolean;
  onSendMessage: (message: string) => void;
  onApproveInvoke: (invokeId: string) => void;
  onRejectInvoke: (invokeId: string, reason?: string) => void;
}

export function DoctorChat({
  messages,
  loading,
  error,
  connected,
  onSendMessage,
  onApproveInvoke,
  onRejectInvoke,
}: DoctorChatProps) {
  const { t } = useTranslation();
  const [input, setInput] = useState("");
  const scrollRef = useRef<HTMLDivElement>(null);

  useEffect(() => {
    const el = scrollRef.current;
    if (el) el.scrollTop = el.scrollHeight;
  }, [messages, loading]);

  const handleSend = () => {
    if (!input.trim() || loading || !connected) return;
    onSendMessage(input.trim());
    setInput("");
  };

  return (
    <div className="flex flex-col">
      {/* Message list */}
      <div
        ref={scrollRef}
        className="h-[420px] mb-2 border rounded-md p-3 bg-muted/30 overflow-y-auto"
      >
        <div className="space-y-3">
          {error && (
            <div className="text-sm text-destructive border border-destructive/30 rounded-md px-3 py-2 bg-destructive/5">
              {error}
            </div>
          )}
          {messages.map((msg) => (
            <AgentMessageBubble
              key={msg.id}
              message={msg}
              onApprove={onApproveInvoke}
              onReject={onRejectInvoke}
            />
          ))}
          {loading && (
            <div className="text-sm text-muted-foreground animate-pulse">
              {t("doctor.agentThinking")}
            </div>
          )}
        </div>
      </div>

      {/* Input area */}
      <div className="flex gap-2">
        <Input
          value={input}
          onChange={(e) => setInput(e.target.value)}
          onKeyDown={(e) => {
            if (e.key === "Enter" && (e.metaKey || e.ctrlKey)) {
              e.preventDefault();
              handleSend();
            }
          }}
          placeholder={t("doctor.sendFollowUp")}
          disabled={!connected || loading}
          className="flex-1"
        />
        <Button
          onClick={handleSend}
          disabled={!connected || loading || !input.trim()}
          size="sm"
        >
          {t("chat.send")} <kbd className="ml-1 text-xs opacity-60">{navigator.platform.includes("Mac") ? "⌘↵" : "Ctrl↵"}</kbd>
        </Button>
      </div>
    </div>
  );
}

