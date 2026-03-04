import { useCallback, useEffect, useRef, useState } from "react";
import { listen } from "@tauri-apps/api/event";
import i18n from "../i18n";
import { api } from "./api";
import { doctorStartPromptTemplate, renderPromptTemplate } from "./prompt-templates";
import type { DiagnosisReportItem, DoctorChatMessage, DoctorInvoke } from "./types";
import {
  extractApprovalPattern,
  sanitizeDoctorCacheMessages,
  buildDoctorCacheKey,
  extractOpenclawText,
  extractOpenclawSessionId,
  isDoctorAutoSafeInvoke,
} from "./doctor-agent-utils";

let msgCounter = 0;
function nextMsgId(): string {
  return `dm-${++msgCounter}`;
}

type DoctorSessionContext = {
  instanceScope: string;
  agentId: string;
  domain: "doctor" | "install";
  engine: DoctorEngineMode;
};

type DoctorEngineMode = "openclaw" | "zeroclaw";
type DoctorSessionCache = {
  version: number;
  context: DoctorSessionContext;
  messages: DoctorChatMessage[];
  openclawSessionId?: string | null;
  sessionKey?: string;
  updatedAt: number;
};

const DOCTOR_CHAT_CACHE_MAX_MESSAGES = 220;
const DOCTOR_CHAT_CACHE_TTL_MS = 14 * 24 * 60 * 60 * 1000;
const DOCTOR_CHAT_CACHE_VERSION = 1;

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

export function useDoctorAgent() {
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


  useEffect(() => {
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
          if (!isNewTurn && last?.role === "assistant" && !last.invoke) {
            return [...prev.slice(0, -1), { ...last, content: text }];
          }
          return [...prev, { id: nextMsgId(), role: "assistant", content: text }];
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
          if (!isNewTurn && last?.role === "assistant" && !last.invoke) {
            return [...prev.slice(0, -1), { ...last, content: text }];
          }
          return [...prev, { id: nextMsgId(), role: "assistant", content: text }];
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
            return updated;
          }
          return [
            ...prev,
            {
              id: nextMsgId(),
              role: "assistant",
              content: "",
              diagnosisReport: { items },
            },
          ];
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
          return [
            ...prev,
            {
              id: nextMsgId(),
              role: "tool-call",
              content: invoke.command,
              invoke,
              status: (isFullAuto || isSafeAuto) ? "auto" : "pending",
            },
          ];
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
          const resultMsg = { id: nextMsgId(), role: "tool-result" as const, content: JSON.stringify(result, null, 2), invokeResult: result, invokeId: id };
          const callIdx = prev.findIndex((m) => m.role === "tool-call" && m.invoke?.id === id);
          if (callIdx === -1) {
            return [...prev, resultMsg];
          }
          const next = [...prev];
          next.splice(callIdx + 1, 0, resultMsg);
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
        setError(code ? `[${code}] ${message}` : message);
        setLoading(false);
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
      context: string,
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
      setMessages([]);
      setPendingInvokes(new Map());
      streamingRef.current = "";
      streamEndedRef.current = false;
      // Fresh session key per diagnosis — no inherited stale state
      sessionKeyRef.current = `agent:${agentId}:clawpal-doctor:${instanceScopeRef.current}:${crypto.randomUUID()}`;
      openclawSessionIdRef.current = undefined;
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
            "{{context}}": context,
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
            setMessages((prev) => [...prev, {
              id: chatMessageId,
              role: "assistant",
              content: assistantText,
            }]);
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
    setMessages((prev) => [...prev, { id: nextMsgId(), role: "user", content: message }]);
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
        setMessages((prev) => [...prev, { id: nextMsgId(), role: "assistant", content: assistantText }]);
        setLoading(false);
      } else {
        await api.doctorSendMessage(message, sessionKeyRef.current, agentIdRef.current, instanceScopeRef.current);
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
    setMessages((prev) =>
      prev.map((m) => {
        if (m.invoke?.id === invokeId && m.role === "tool-call") {
          if (m.invoke) {
            const pattern = extractApprovalPattern(m.invoke);
            setApprovedPatterns((p) => new Set(p).add(pattern));
          }
          return { ...m, status: "approved" as const };
        }
        return m;
      })
    );
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
    setMessages((prev) =>
      prev.map((m) =>
        m.invoke?.id === invokeId && m.role === "tool-call"
          ? { ...m, status: "rejected" as const }
          : m
      )
    );
    try {
      await api.doctorRejectInvoke(invokeId, reason);
    } catch (err) {
      setError(`Reject failed: ${err}`);
    }
  }, []);

  const reset = useCallback(() => {
    sessionActiveRef.current = false;
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
    reset,
  };
}
