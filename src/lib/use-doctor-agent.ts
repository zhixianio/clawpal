import { useCallback, useEffect, useRef, useState } from "react";
import { listen } from "@tauri-apps/api/event";
import i18n from "../i18n";
import { api } from "./api";
import { doctorStartPromptTemplate, renderPromptTemplate } from "./prompt-templates";
import type { DiagnosisReportItem, DoctorChatMessage, DoctorInvoke } from "./types";

let msgCounter = 0;
function nextMsgId(): string {
  return `dm-${++msgCounter}`;
}

function extractApprovalPattern(invoke: DoctorInvoke): string {
  const path = (invoke.args?.path as string) ?? "";
  const prefix = path.includes("/") ? path.substring(0, path.lastIndexOf("/") + 1) : path;
  return `${invoke.command}:${prefix}`;
}

function normalizeInvokeArgs(invoke: DoctorInvoke): string {
  const raw = (invoke.args?.args as string) ?? "";
  return raw.trim().replace(/\s+/g, " ").toLowerCase();
}

function hasAnyPrefix(value: string, prefixes: string[]): boolean {
  return prefixes.some((prefix) => value === prefix || value.startsWith(`${prefix} `));
}

type DoctorSessionContext = {
  instanceScope: string;
  agentId: string;
  domain: "doctor" | "install";
  engine: DoctorEngineMode;
};

type DoctorEngineMode = "openclaw" | "zeroclaw";
type UseDoctorAgentOptions = {
  enableBridgeEvents?: boolean;
};
type DoctorSessionCache = {
  version: number;
  context: DoctorSessionContext;
  messages: DoctorChatMessage[];
  openclawSessionId?: string | null;
  sessionKey?: string;
  updatedAt: number;
};

const DOCTOR_CHAT_CACHE_PREFIX = "clawpal-doctor-chat-v1";
const DOCTOR_CHAT_CACHE_MAX_MESSAGES = 220;
const DOCTOR_CHAT_CACHE_TTL_MS = 14 * 24 * 60 * 60 * 1000;
const DOCTOR_CHAT_CACHE_VERSION = 1;

function buildDoctorCacheKey(context: DoctorSessionContext): string {
  const scope = encodeURIComponent(context.instanceScope);
  const agent = encodeURIComponent(context.agentId);
  return `${DOCTOR_CHAT_CACHE_PREFIX}-${context.domain}-${context.engine}-${scope}-${agent}`;
}

function sanitizeDoctorCacheMessages(rawMessages: unknown): DoctorChatMessage[] {
  if (!Array.isArray(rawMessages)) return [];
  return rawMessages.flatMap((raw) => {
    if (!raw || typeof raw !== "object") return [];
    const item = raw as Partial<DoctorChatMessage> & Record<string, unknown>;
    const role = item.role;
    if (role !== "assistant" && role !== "user" && role !== "tool-call" && role !== "tool-result") return [];
    const id = typeof item.id === "string" ? item.id : "";
    if (!id) return [];
    const content = typeof item.content === "string" ? item.content : "";
    const next: DoctorChatMessage = {
      id,
      role,
      content,
    };
    if (item.invoke && typeof item.invoke === "object") {
      next.invoke = item.invoke as DoctorInvoke;
    }
    if (typeof item.invokeId === "string") {
      next.invokeId = item.invokeId;
    }
    if (item.invokeResult !== undefined) {
      next.invokeResult = item.invokeResult;
    }
    const status = item.status;
    if (status === "pending" || status === "approved" || status === "rejected" || status === "auto") {
      next.status = status;
    }
    const diagnosisReport = item.diagnosisReport;
    if (diagnosisReport && typeof diagnosisReport === "object" && Array.isArray((diagnosisReport as { items?: unknown }).items)) {
      next.diagnosisReport = { items: ((diagnosisReport as { items: unknown }).items as DiagnosisReportItem[]) };
    }
    return [next];
  });
}

function normalizeDoctorMessages(messages: DoctorChatMessage[]): DoctorChatMessage[] {
  return messages.slice(-DOCTOR_CHAT_CACHE_MAX_MESSAGES);
}

function loadDoctorSessionCache(context: DoctorSessionContext): DoctorSessionCache | null {
  const key = buildDoctorCacheKey(context);
  try {
    const raw = localStorage.getItem(key);
    if (!raw) return null;
    const parsed = JSON.parse(raw) as DoctorSessionCache | null;
    if (
      !parsed
      || typeof parsed !== "object"
      || parsed.version !== DOCTOR_CHAT_CACHE_VERSION
      || typeof parsed.updatedAt !== "number"
      || !parsed.context
      || !parsed.context.instanceScope
      || !parsed.context.agentId
    ) {
      return null;
    }
    if (Date.now() - parsed.updatedAt > DOCTOR_CHAT_CACHE_TTL_MS) {
      localStorage.removeItem(key);
      return null;
    }
    const messages = sanitizeDoctorCacheMessages(parsed.messages);
    if (messages.length === 0) return null;
    return {
      version: DOCTOR_CHAT_CACHE_VERSION,
      context: parsed.context,
      messages,
      openclawSessionId: typeof parsed.openclawSessionId === "string" ? parsed.openclawSessionId : null,
      sessionKey: typeof parsed.sessionKey === "string" ? parsed.sessionKey : undefined,
      updatedAt: parsed.updatedAt,
    };
  } catch (error) {
    console.warn("Failed to load doctor chat cache:", error);
    return null;
  }
}

function saveDoctorSessionCache(context: DoctorSessionContext, payload: {
  messages: DoctorChatMessage[];
  openclawSessionId?: string | null;
  sessionKey?: string;
}) {
  const key = buildDoctorCacheKey(context);
  try {
    const next: DoctorSessionCache = {
      version: DOCTOR_CHAT_CACHE_VERSION,
      context,
      messages: payload.messages,
      openclawSessionId: payload.openclawSessionId ?? null,
      sessionKey: payload.sessionKey,
      updatedAt: Date.now(),
    };
    localStorage.setItem(key, JSON.stringify(next));
  } catch (error) {
    console.warn("Failed to persist doctor chat cache:", error);
  }
}

function extractOpenclawText(result: Record<string, unknown>): string {
  const payloads = result.payloads;
  if (Array.isArray(payloads)) {
    const text = payloads
      .map((item) => (item as { text?: string }).text)
      .filter((v): v is string => typeof v === "string" && v.length > 0)
      .join("\n");
    if (text) return text;
  }
  const text = result.text;
  if (typeof text === "string") return text;
  const content = result.content;
  if (typeof content === "string") return content;
  return "";
}

function extractOpenclawSessionId(result: Record<string, unknown>): string | undefined {
  const meta = result.meta as Record<string, unknown> | undefined;
  if (!meta || typeof meta !== "object") return;
  const agentMeta = meta.agentMeta as Record<string, unknown> | undefined;
  if (!agentMeta || typeof agentMeta !== "object") return;
  const rawSessionId = agentMeta.sessionId;
  return typeof rawSessionId === "string" ? rawSessionId.trim() || undefined : undefined;
}

function isDoctorAutoSafeInvoke(invoke: DoctorInvoke, domain: "doctor" | "install"): boolean {
  if (domain !== "doctor") return false;
  const args = normalizeInvokeArgs(invoke);
  if (invoke.command === "clawpal") {
    // Only read-only diagnostic commands are safe to auto-approve.
    return hasAnyPrefix(args, [
      "doctor probe-openclaw",
      "doctor file read",
      "doctor config-read",
      "doctor sessions-read",
    ]);
  }
  if (invoke.command === "openclaw") {
    // Only read-only diagnostic commands are safe to auto-approve.
    return hasAnyPrefix(args, [
      "--version",
      "doctor",
      "gateway status",
      "health",
      "config get",
      "agents list",
      "memory status",
      "security audit",
    ]);
  }
  return false;
}

export function useDoctorAgent(options: UseDoctorAgentOptions = {}) {
  const enableBridgeEvents = options.enableBridgeEvents ?? true;
  const [connected, setConnected] = useState(false);
  const [bridgeConnected, setBridgeConnected] = useState(false);
  const [messages, setMessages] = useState<DoctorChatMessage[]>([]);
  const [pendingInvokes, setPendingInvokes] = useState<Map<string, DoctorInvoke>>(new Map());
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [target, setTargetState] = useState("local");
  const [approvedPatterns, setApprovedPatterns] = useState<Set<string>>(new Set());
  const [fullAuto, setFullAuto] = useState(false);

  // Track streaming assistant message
  const streamingRef = useRef("");
  // True after chat-final received — next delta is a NEW turn, not a continuation.
  // Prevents race: operator WS sends next-turn delta before bridge WS delivers invoke.
  const streamEndedRef = useRef(false);

  // Unique session key per diagnosis — avoids inheriting stale state
  const sessionKeyRef = useRef("");
  const agentIdRef = useRef("main");
  // Locked at diagnosis start — immune to tab switching during diagnosis
  const targetRef = useRef("local");
  const instanceScopeRef = useRef("local");
  const domainRef = useRef<"doctor" | "install">("doctor");
  const instanceTransportRef = useRef<"local" | "docker_local" | "remote_ssh">("local");
  const engineRef = useRef<DoctorEngineMode>("zeroclaw");
  const openclawSessionIdRef = useRef<string | undefined>(undefined);
  // Last connection params for reconnect
  const wasConnectedRef = useRef(false);

  // Gate: only process invokes after startDiagnosis has been called.
  // Prevents stale invokes (replayed by the gateway on node reconnect)
  // from appearing and then being cleared by startDiagnosis.
  const sessionActiveRef = useRef(false);

  // Refs to avoid stale closures in useEffect listeners
  const approvedPatternsRef = useRef(approvedPatterns);
  useEffect(() => { approvedPatternsRef.current = approvedPatterns; }, [approvedPatterns]);
  const fullAutoRef = useRef(fullAuto);
  useEffect(() => { fullAutoRef.current = fullAuto; }, [fullAuto]);
  const autoApproveRef = useRef<(invokeId: string) => Promise<void>>(null!);

  const setTarget = useCallback((next: string) => {
    const resolved = (next || "local").trim() || "local";
    targetRef.current = resolved;
    setTargetState(resolved);
  }, []);

  const buildDoctorContext = useCallback((): DoctorSessionContext => ({
    instanceScope: (instanceScopeRef.current || "local").trim() || "local",
    agentId: (agentIdRef.current || "main").trim() || "main",
    domain: domainRef.current,
    engine: engineRef.current,
  }), []);

  const persistDoctorMessages = useCallback((nextMessages: DoctorChatMessage[]) => {
    const context = buildDoctorContext();
    if (!context.instanceScope || !context.agentId || !context.engine) return;
    saveDoctorSessionCache(context, {
      messages: normalizeDoctorMessages(nextMessages),
      openclawSessionId: openclawSessionIdRef.current,
      sessionKey: sessionKeyRef.current,
    });
  }, [buildDoctorContext]);

  const restoreDoctorMessagesFromCache = useCallback((context: DoctorSessionContext): DoctorSessionCache | null => {
    return loadDoctorSessionCache(context);
  }, []);


  useEffect(() => {
    if (!enableBridgeEvents) return;
    let disposed = false;
    const unlistenFns: Array<() => void> = [];
    const bind = async <T,>(event: string, handler: Parameters<typeof listen<T>>[1]) => {
      const off = await listen<T>(event, handler);
      if (disposed) {
        off();
        return;
      }
      unlistenFns.push(off);
    };

    void Promise.all([
      bind("doctor:connected", () => {
        setConnected(true);
        setError(null);
      }),
      bind<{ reason: string }>("doctor:disconnected", (e) => {
        setConnected(false);
        setLoading(false);
        if (e.payload.reason && e.payload.reason !== "server closed") {
          setError(e.payload.reason);
        }
      }),
      bind<{ text: string }>("doctor:chat-delta", (e) => {
        if (!sessionActiveRef.current) return;
        const text = e.payload.text;
        // Skip empty deltas — the gateway sends them between tool calls
        // and they create empty assistant bubbles that obscure tool-call UI
        if (!text) return;
        // If previous streaming ended (chat-final received), this delta is
        // a NEW agent turn. Don't replace the previous assistant message —
        // the invoke event from the bridge WS may not have arrived yet.
        const isNewTurn = streamEndedRef.current;
        streamEndedRef.current = false;
        streamingRef.current = text;
        setMessages((prev) => {
          const last = prev[prev.length - 1];
          const next = (!isNewTurn && last?.role === "assistant" && !last.invoke)
            ? [...prev.slice(0, -1), { ...last, content: text }]
            : [...prev, { id: nextMsgId(), role: "assistant" as const, content: text, timestamp: Date.now() }];
          persistDoctorMessages(next);
          return next;
        });
      }),
      bind<{ text: string }>("doctor:chat-final", (e) => {
        if (!sessionActiveRef.current) return;
        const text = e.payload.text || streamingRef.current;
        streamingRef.current = "";
        // If previous turn already ended, this final is for a NEW turn.
        // Same race-condition guard as the delta handler.
        const isNewTurn = streamEndedRef.current;
        streamEndedRef.current = true;
        // When the agent issues a tool call, the gateway sends a "final"
        // chat event with no message content. Don't create an empty bubble
        // and don't clear loading — we're waiting for tool approval/result.
        if (!text) return;
        setLoading(false);
        setMessages((prev) => {
          const last = prev[prev.length - 1];
          const next = (!isNewTurn && last?.role === "assistant" && !last.invoke)
            ? [...prev.slice(0, -1), { ...last, content: text }]
            : [...prev, { id: nextMsgId(), role: "assistant" as const, content: text, timestamp: Date.now() }];
          persistDoctorMessages(next);
          return next;
        });
      }),
      bind<{ items: DiagnosisReportItem[] }>("doctor:diagnosis-report", (e) => {
        if (!sessionActiveRef.current) return;
        const items = e.payload.items;
        if (!items || items.length === 0) return;
        // Attach the diagnosis report to the most recent assistant message,
        // or create a new one if none exists yet.
        setMessages((prev) => {
          let lastAssistantIdx = -1;
          for (let i = prev.length - 1; i >= 0; i--) {
            if (prev[i].role === "assistant" && !prev[i].invoke) {
              lastAssistantIdx = i;
              break;
            }
          }
          if (lastAssistantIdx !== -1) {
            const updated = [...prev];
            updated[lastAssistantIdx] = {
              ...updated[lastAssistantIdx],
              diagnosisReport: { items },
            };
            persistDoctorMessages(updated);
            return updated;
          }
          const next = [
            ...prev,
            {
              id: nextMsgId(),
              role: "assistant" as const,
              content: "",
              diagnosisReport: { items },
              timestamp: Date.now(),
            },
          ];
          persistDoctorMessages(next);
          return next;
        });
      }),
      bind<DoctorInvoke>("doctor:invoke", (e) => {
        // Ignore invokes arriving before diagnosis starts. Stale invokes
        // (replayed by gateway during reconnect) are rejected on the Rust side
        // after handshake completes — see bridge_client.rs connect().
        if (!sessionActiveRef.current) return;

        const invoke = e.payload;
        const isSafeAuto = isDoctorAutoSafeInvoke(invoke, domainRef.current);

        // Deduplicate: gateway may send the same invoke twice
        setPendingInvokes((prev) => {
          if (prev.has(invoke.id)) return prev; // already seen
          return new Map(prev).set(invoke.id, invoke);
        });

        const isFullAuto = fullAutoRef.current;
        setMessages((prev) => {
          if (prev.some((m) => m.invoke?.id === invoke.id)) return prev; // already shown
          const next = [
            ...prev,
            {
              id: nextMsgId(),
              role: "tool-call" as const,
              content: invoke.command,
              invoke,
              status: (isFullAuto || isSafeAuto) ? ("auto" as const) : ("pending" as const),
              timestamp: Date.now(),
            },
          ];
          persistDoctorMessages(next);
          return next;
        });

        // Full-auto mode: approve everything immediately
        if (isFullAuto) {
          autoApproveRef.current(invoke.id);
          return;
        }

        // Explicit safe-list for doctor self-heal commands.
        if (isSafeAuto) {
          autoApproveRef.current(invoke.id);
          return;
        }

        // Auto-approve read commands if pattern already approved
        if (invoke.type === "read") {
          const pattern = extractApprovalPattern(invoke);
          if (approvedPatternsRef.current.has(pattern)) {
            autoApproveRef.current(invoke.id);
          }
          // else: show in chat, wait for user to click Allow
        }
      }),
      bind<{ id: string; result: unknown }>("doctor:invoke-result", (e) => {
        const { id, result } = e.payload;
        setPendingInvokes((prev) => {
          if (!prev.has(id)) return prev; // already handled
          const next = new Map(prev);
          next.delete(id);
          return next;
        });
        // Deduplicate: only append result if we haven't already
        setMessages((prev) => {
          if (prev.some((m) => m.role === "tool-result" && m.invokeId === id)) return prev;
          const resultMsg = { id: nextMsgId(), role: "tool-result" as const, content: JSON.stringify(result, null, 2), invokeResult: result, invokeId: id, timestamp: Date.now() };
          const callIdx = prev.findIndex((m) => m.role === "tool-call" && m.invoke?.id === id);
          if (callIdx === -1) {
            const next = [...prev, resultMsg];
            persistDoctorMessages(next);
            return next;
          }
          const next = [...prev];
          next.splice(callIdx + 1, 0, resultMsg);
          persistDoctorMessages(next);
          return next;
        });
        // Reset streaming for next assistant message
        streamingRef.current = "";
      }),
      bind("doctor:bridge-connected", () => {
        setBridgeConnected(true);
      }),
      bind<{ reason: string }>("doctor:bridge-disconnected", () => {
        setBridgeConnected(false);
      }),
      bind<{ message: string; code?: string }>("doctor:error", (e) => {
        const code = e.payload.code?.trim();
        const message = e.payload.message || "Unknown runtime error";
        const display = code ? `[${code}] ${message}` : message;
        setError(display);
        setLoading(false);
        setMessages((prev) => {
          const next = [
            ...prev,
            {
              id: nextMsgId(),
              role: "assistant" as const,
              content: `[runtime error] ${display}`,
              timestamp: Date.now(),
            },
          ];
          persistDoctorMessages(next);
          return next;
        });
      }),
    ]).catch((err) => {
      if (!disposed) {
        setError(`Doctor listener setup failed: ${String(err)}`);
      }
    });

    return () => {
      disposed = true;
      unlistenFns.forEach((off) => {
        try {
          off();
        } catch {
          // ignore listener teardown failures
        }
      });
    };
  }, []);

  const autoApprove = useCallback(async (invokeId: string) => {
    try {
      await api.doctorApproveInvoke(
        invokeId,
        targetRef.current,
        instanceScopeRef.current,
        sessionKeyRef.current,
        agentIdRef.current,
        domainRef.current,
      );
      setMessages((prev) =>
        prev.map((m) => {
          if (m.invoke?.id === invokeId && m.role === "tool-call") {
            const pattern = extractApprovalPattern(m.invoke);
            setApprovedPatterns((p) => new Set(p).add(pattern));
            return { ...m, status: "auto" as const };
          }
          return m;
        })
      );
    } catch (err) {
      setError(`Auto-approve failed: ${err}`);
    }
  }, []);
  autoApproveRef.current = autoApprove;

  const connect = useCallback(async () => {
    setError(null);
    try {
      await api.doctorConnect();
      setBridgeConnected(true);
      wasConnectedRef.current = true;
    } catch (err) {
      const msg = `Connection failed: ${err}`;
      setError(msg);
      throw new Error(msg);
    }
  }, []);

  const reconnect = useCallback(async () => {
    if (!wasConnectedRef.current) {
      setError("No previous connection to reconnect to");
      return;
    }
    setError(null);
    try {
      await api.doctorConnect();
      setBridgeConnected(true);
    } catch (err) {
      setError(`Reconnect failed: ${err}`);
    }
  }, []);

  const disconnect = useCallback(async () => {
    try {
      await api.doctorDisconnect();
    } catch (err) {
      setError(`Disconnect failed: ${err}`);
    }
    setConnected(false);
    setBridgeConnected(false);
    setLoading(false);
  }, []);

  const startDiagnosis = useCallback(
    async (
      launchContext: string,
      agentId = "main",
      instanceScope?: string,
      instanceTransport: "local" | "docker_local" | "remote_ssh" = "local",
      systemPrompt?: string,
      domain: "doctor" | "install" = "doctor",
      engine: DoctorEngineMode = "zeroclaw",
    ) => {
      agentIdRef.current = agentId;
      const scope = (instanceScope ?? targetRef.current ?? "local").trim() || "local";
      const executionTarget = instanceTransport === "remote_ssh" ? scope : "local";
      targetRef.current = executionTarget;
      instanceScopeRef.current = scope;
      setTargetState(executionTarget);
      domainRef.current = domain;
      instanceTransportRef.current = instanceTransport;
      engineRef.current = domain === "install" ? "zeroclaw" : engine;
      setLoading(true);
      setError(null);
      const context = buildDoctorContext();
      const restored = restoreDoctorMessagesFromCache(context);
      const restoredMessages = restored?.messages ?? [];
      const restoredSessionId = restored?.openclawSessionId;
      // Always replace message state with the target-engine cache snapshot.
      // This prevents stale messages from another engine from lingering when
      // there is no cache for the current engine/scope.
      setMessages(restoredMessages);
      persistDoctorMessages(restoredMessages);
      openclawSessionIdRef.current = restoredSessionId ?? undefined;
      if (restored?.sessionKey) {
        sessionKeyRef.current = restored.sessionKey;
      } else {
        sessionKeyRef.current = `agent:${agentId}:clawpal-doctor:${instanceScopeRef.current}:${crypto.randomUUID()}`;
      }
      if (!restored || restored.openclawSessionId === null) {
        openclawSessionIdRef.current = undefined;
      }
      setPendingInvokes(new Map());
      streamingRef.current = "";
      streamEndedRef.current = false;
      sessionActiveRef.current = engineRef.current === "zeroclaw";
      try {
        const scope = instanceScopeRef.current;
        let prompt: string;
        if (systemPrompt) {
          prompt = systemPrompt;
        } else {
          const lang = i18n.language?.startsWith("zh") ? "Chinese (简体中文)" : "English";
          const transportLine =
            instanceTransport === "docker_local"
              ? `Current target transport is docker_local (instance: ${scope}).`
              : instanceTransport === "remote_ssh"
                ? `Current target transport is remote_ssh (instance: ${scope}).`
                : "Current target transport is local.";
          prompt = renderPromptTemplate(doctorStartPromptTemplate(), {
            "{{language}}": lang,
            "{{transport_line}}": transportLine,
            "{{context}}": launchContext,
          });
        }
        if (domain === "install") {
          await api.installStartSession(prompt, sessionKeyRef.current, agentId, scope);
        } else if (engineRef.current === "zeroclaw") {
          await api.doctorStartDiagnosis(prompt, sessionKeyRef.current, agentId, scope);
        } else {
          const chatMessageId = nextMsgId();
          setConnected(true);
          setBridgeConnected(false);
          const sessionKeyForRequest = sessionKeyRef.current;
          try {
            const currentSessionId = openclawSessionIdRef.current ?? sessionKeyRef.current;
            const response = instanceTransportRef.current === "remote_ssh"
              ? await api.remoteChatViaOpenclaw(scope, agentId, prompt, currentSessionId)
              : await api.chatViaOpenclaw(agentId, prompt, currentSessionId);
            if (
              sessionKeyRef.current !== sessionKeyForRequest
              || engineRef.current !== "openclaw"
            ) {
              return;
            }
            const assistantText = extractOpenclawText(response as Record<string, unknown>);
            const restoredSessionId = extractOpenclawSessionId(response as Record<string, unknown>);
            if (restoredSessionId) {
              openclawSessionIdRef.current = restoredSessionId;
            }
            if (!assistantText) {
              throw new Error("No text returned from openclaw diagnosis");
            }
            setMessages((prev) => {
              const next = [...prev, {
                id: chatMessageId,
                role: "assistant" as const,
                content: assistantText,
                timestamp: Date.now(),
              }];
              persistDoctorMessages(next);
              return next;
            });
            setLoading(false);
          } catch (err) {
            if (
              sessionKeyRef.current !== sessionKeyForRequest
              || engineRef.current !== "openclaw"
            ) {
              return;
            }
            setConnected(false);
            throw err;
          }
        }
      } catch (err) {
        setError(`Start diagnosis failed: ${err}`);
        setLoading(false);
      }
    }, []);

  const sendMessage = useCallback(async (message: string) => {
    setLoading(true);
    streamingRef.current = "";
    // Cached sessions can be restored without calling startDiagnosis again.
    // Re-activate event intake before sending so chat-final isn't dropped.
    if (engineRef.current === "zeroclaw") {
      sessionActiveRef.current = true;
    }
    const userMessage = { id: nextMsgId(), role: "user" as const, content: message, timestamp: Date.now() };
    setMessages((prev) => {
      const next = [...prev, userMessage];
      persistDoctorMessages(next);
      return next;
    });
    const sessionKeyForRequest = sessionKeyRef.current;
    const engineForRequest = engineRef.current;
    try {
      if (domainRef.current === "install") {
        await api.installSendMessage(message, sessionKeyRef.current, agentIdRef.current, instanceScopeRef.current);
      } else if (engineRef.current === "openclaw") {
        const currentSessionId = openclawSessionIdRef.current ?? sessionKeyRef.current;
        const response = instanceTransportRef.current === "remote_ssh"
          ? await api.remoteChatViaOpenclaw(
            instanceScopeRef.current,
            agentIdRef.current,
            message,
            currentSessionId,
          )
          : await api.chatViaOpenclaw(agentIdRef.current, message, currentSessionId);
        if (
          sessionKeyRef.current !== sessionKeyForRequest
          || engineRef.current !== "openclaw"
        ) {
          return;
        }
        const assistantText = extractOpenclawText(response as Record<string, unknown>);
        const restoredSessionId = extractOpenclawSessionId(response as Record<string, unknown>);
        if (restoredSessionId) {
          openclawSessionIdRef.current = restoredSessionId;
        }
        if (!assistantText) {
          throw new Error("No text returned from openclaw diagnosis");
        }
        setMessages((prev) => {
          const next = [...prev, { id: nextMsgId(), role: "assistant" as const, content: assistantText, timestamp: Date.now() }];
          persistDoctorMessages(next);
          return next;
        });
        setLoading(false);
      } else {
        await api.doctorSendMessage(message, sessionKeyRef.current, agentIdRef.current, instanceScopeRef.current);
        // Zeroclaw send is request/response over Tauri command. If runtime
        // events are dropped on the frontend side, avoid indefinite "thinking".
        if (
          sessionKeyRef.current === sessionKeyForRequest
          && engineRef.current === engineForRequest
        ) {
          setLoading(false);
        }
      }
    } catch (err) {
      if (
        sessionKeyRef.current !== sessionKeyForRequest
        || engineRef.current !== engineForRequest
      ) {
        return;
      }
      setError(`Send message failed: ${err}`);
      setLoading(false);
    }
  }, []);

  const approveInvoke = useCallback(async (invokeId: string) => {
    if (domainRef.current === "doctor" && engineRef.current === "openclaw") {
      return;
    }
    setMessages((prev) => {
      const next = prev.map((m) => {
        if (m.invoke?.id === invokeId && m.role === "tool-call") {
          if (m.invoke) {
            const pattern = extractApprovalPattern(m.invoke);
            setApprovedPatterns((p) => new Set(p).add(pattern));
          }
          return { ...m, status: "approved" as const };
        }
        return m;
      });
      persistDoctorMessages(next);
      return next;
    });
    try {
      await api.doctorApproveInvoke(
        invokeId,
        targetRef.current,
        instanceScopeRef.current,
        sessionKeyRef.current,
        agentIdRef.current,
        domainRef.current,
      );
    } catch (err) {
      const text = String(err);
      if (text.includes("No pending invoke with id")) {
        // Already auto-approved/consumed; treat as idempotent success.
        return;
      }
      setError(`Approve failed: ${err}`);
    }
  }, []);

  const rejectInvoke = useCallback(async (invokeId: string, reason = "User rejected") => {
    if (domainRef.current === "doctor" && engineRef.current === "openclaw") {
      return;
    }
    setPendingInvokes((prev) => {
      const next = new Map(prev);
      next.delete(invokeId);
      return next;
    });
    setMessages((prev) => {
      const next = prev.map((m) =>
        m.invoke?.id === invokeId && m.role === "tool-call"
          ? { ...m, status: "rejected" as const }
          : m
      );
      persistDoctorMessages(next);
      return next;
    });
    try {
      await api.doctorRejectInvoke(invokeId, reason);
    } catch (err) {
      setError(`Reject failed: ${err}`);
    }
  }, []);

  const restoreFromCache = useCallback((params?: {
    agentId?: string;
    instanceScope?: string;
    domain?: "doctor" | "install";
    engine?: DoctorEngineMode;
  }): boolean => {
    const nextAgentId = (params?.agentId ?? agentIdRef.current ?? "main").trim() || "main";
    const nextScope = (params?.instanceScope ?? instanceScopeRef.current ?? "local").trim() || "local";
    const nextDomain = params?.domain ?? domainRef.current;
    const nextEngine = params?.engine ?? engineRef.current;

    agentIdRef.current = nextAgentId;
    instanceScopeRef.current = nextScope;
    domainRef.current = nextDomain;
    engineRef.current = nextEngine;

    const context: DoctorSessionContext = {
      instanceScope: nextScope,
      agentId: nextAgentId,
      domain: nextDomain,
      engine: nextEngine,
    };
    const restored = restoreDoctorMessagesFromCache(context);
    const restoredMessages = restored?.messages ?? [];
    if (restoredMessages.length === 0) {
      return false;
    }

    setMessages(restoredMessages);
    setPendingInvokes(new Map());
    setLoading(false);
    setError(null);
    // Restoring cached diagnosis means there was a live session in this scope.
    // Keep the UI in connected mode after tab switches/remounts.
    setConnected(true);
    setBridgeConnected(nextEngine === "zeroclaw");
    wasConnectedRef.current = true;
    streamingRef.current = "";
    streamEndedRef.current = false;
    sessionActiveRef.current = false;
    openclawSessionIdRef.current = restored?.openclawSessionId ?? undefined;
    if (restored?.sessionKey) {
      sessionKeyRef.current = restored.sessionKey;
    }
    persistDoctorMessages(restoredMessages);
    return true;
  }, [persistDoctorMessages, restoreDoctorMessagesFromCache]);

  const reset = useCallback(() => {
    sessionActiveRef.current = false;
    wasConnectedRef.current = false;
    setMessages([]);
    setPendingInvokes(new Map());
    setLoading(false);
    setError(null);
    setBridgeConnected(false);
    setApprovedPatterns(new Set());
    streamingRef.current = "";
    streamEndedRef.current = false;
    openclawSessionIdRef.current = undefined;
    sessionKeyRef.current = `agent:${agentIdRef.current}:clawpal-doctor:${instanceScopeRef.current}:${crypto.randomUUID()}`;
  }, []);

  return {
    connected,
    bridgeConnected,
    messages,
    pendingInvokes,
    loading,
    error,
    target,
    setTarget,
    approvedPatterns,
    fullAuto,
    setFullAuto,
    connect,
    reconnect,
    disconnect,
    startDiagnosis,
    sendMessage,
    approveInvoke,
    rejectInvoke,
    restoreFromCache,
    reset,
  };
}
