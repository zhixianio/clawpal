import { useCallback, useEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { useApi } from "@/lib/use-api";
import { ScrollArea } from "@/components/ui/scroll-area";
import {
  Select,
  SelectTrigger,
  SelectValue,
  SelectContent,
  SelectItem,
} from "@/components/ui/select";
import { Input } from "@/components/ui/input";
import { Button } from "@/components/ui/button";
import { cn } from "@/lib/utils";

interface Message {
  role: "user" | "assistant";
  content: string;
}

const AGENT_ID = "main";
const SESSION_KEY_PREFIX = "clawpal_chat_session_";

function loadSessionId(instanceId: string, agent: string): string | undefined {
  return localStorage.getItem(SESSION_KEY_PREFIX + instanceId + "_" + agent) || undefined;
}
function saveSessionId(instanceId: string, agent: string, sid: string) {
  localStorage.setItem(SESSION_KEY_PREFIX + instanceId + "_" + agent, sid);
}
function clearSessionId(instanceId: string, agent: string) {
  localStorage.removeItem(SESSION_KEY_PREFIX + instanceId + "_" + agent);
}

const CLAWPAL_CONTEXT = `[ClawPal Context] You are responding inside ClawPal, a desktop GUI for OpenClaw configuration.
Rules:
- You are in READ-ONLY advisory mode. Do NOT execute commands, send messages, or modify config directly.
- When the user asks to change something, explain what should be changed and show the config diff, but do NOT apply it.
- Only discuss OpenClaw configuration topics (agents, models, channels, recipes, memory, sessions).
- Keep responses concise (2-3 sentences unless the user asks for detail).
User message: `;

export function Chat() {
  const { t } = useTranslation();
  const ua = useApi();
  const [messages, setMessages] = useState<Message[]>([]);
  const [input, setInput] = useState("");
  const [loading, setLoading] = useState(false);
  const [agents, setAgents] = useState<string[]>([]);
  const [agentId, setAgentId] = useState(AGENT_ID);
  const [sessionId, setSessionId] = useState<string | undefined>(undefined);
  const bottomRef = useRef<HTMLDivElement>(null);

  useEffect(() => {
    setMessages([]);
    setAgentId(AGENT_ID);
    setSessionId(loadSessionId(ua.instanceId, AGENT_ID));
  }, [ua.instanceId]);

  useEffect(() => {
    ua.listAgents()
      .then((agents) => setAgents(agents.map((a) => a.id)))
      .catch((e) => console.error("Failed to load agent IDs:", e));
  }, [ua]);

  useEffect(() => {
    bottomRef.current?.scrollIntoView({ behavior: "smooth" });
  }, [messages, loading]);

  const send = useCallback(async () => {
    if (!input.trim() || loading) return;
    if (ua.isRemote && !ua.isConnected) return;

    const userMsg: Message = { role: "user", content: input.trim() };
    setMessages((prev) => [...prev, userMsg]);
    setInput("");
    setLoading(true);

    try {
      // Inject ClawPal context on first message of a session
      const payload = sessionId ? userMsg.content : CLAWPAL_CONTEXT + userMsg.content;
      const result = await ua.chatViaOpenclaw(agentId, payload, sessionId);
      // Extract session ID for conversation continuity
      const meta = result.meta as Record<string, unknown> | undefined;
      const agentMeta = meta?.agentMeta as Record<string, unknown> | undefined;
      if (agentMeta?.sessionId) {
        const sid = agentMeta.sessionId as string;
        setSessionId(sid);
        saveSessionId(ua.instanceId, agentId, sid);
      }
      // Extract reply text
      const payloads = result.payloads as Array<{ text?: string }> | undefined;
      const text = payloads?.map((p) => p.text).filter(Boolean).join("\n") || t('chat.noResponse');
      setMessages((prev) => [...prev, { role: "assistant", content: text }]);
    } catch (err) {
      setMessages((prev) => [...prev, { role: "assistant", content: `Error: ${err}` }]);
    } finally {
      setLoading(false);
    }
  }, [input, loading, agentId, sessionId, ua, t]);

  return (
    <div className="flex flex-col h-full">
      <div className="flex items-center gap-2 mb-2">
        <Select value={agentId} onValueChange={(a) => { setAgentId(a); setSessionId(loadSessionId(ua.instanceId, a)); setMessages([]); }}>
          <SelectTrigger size="sm" className="w-auto text-xs">
            <SelectValue />
          </SelectTrigger>
          <SelectContent>
            {agents.map((a) => (
              <SelectItem key={a} value={a}>{a}</SelectItem>
            ))}
          </SelectContent>
        </Select>
        <Button
          variant="ghost"
          size="sm"
          className="text-xs opacity-70"
          onClick={() => { clearSessionId(ua.instanceId, agentId); setSessionId(undefined); setMessages([]); }}
        >
          {t('chat.new')}
        </Button>
      </div>
      <ScrollArea className="flex-1 mb-2 overflow-hidden">
        {messages.map((msg, i) => (
          <div key={i} className={cn("mb-2", msg.role === "user" ? "text-right" : "text-left")}>
            <div className={cn(
              "inline-block px-3 py-2 rounded-lg max-w-[90%] text-left border border-border",
              msg.role === "user" ? "bg-muted" : "bg-card"
            )}>
              <div className="whitespace-pre-wrap text-sm">{msg.content}</div>
            </div>
          </div>
        ))}
        {loading && <div className="opacity-50 text-sm">{t('chat.thinking')}</div>}
        <div ref={bottomRef} />
      </ScrollArea>
      <div className="flex gap-2">
        <Input
          value={input}
          onChange={(e) => setInput(e.target.value)}
          onKeyDown={(e) => { if (e.key === "Enter" && !e.shiftKey) { e.preventDefault(); send(); } }}
          placeholder={t('chat.placeholder')}
          className="flex-1"
        />
        <Button
          onClick={send}
          disabled={loading}
        >
          {t('chat.send')}
        </Button>
      </div>
    </div>
  );
}
