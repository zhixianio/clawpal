import { useCallback, useEffect, useRef, useState } from "react";
import { listen } from "@tauri-apps/api/event";
import i18n from "../i18n";
import { api } from "./api";
import type { DoctorChatMessage, DoctorInvoke } from "./types";

let msgCounter = 0;
function nextMsgId(): string {
  return `dm-${++msgCounter}`;
}

function extractApprovalPattern(invoke: DoctorInvoke): string {
  const path = (invoke.args?.path as string) ?? "";
  const prefix = path.includes("/") ? path.substring(0, path.lastIndexOf("/") + 1) : path;
  return `${invoke.command}:${prefix}`;
}

export function useDoctorAgent() {
  const [connected, setConnected] = useState(false);
  const [bridgeConnected, setBridgeConnected] = useState(false);
  const [messages, setMessages] = useState<DoctorChatMessage[]>([]);
  const [pendingInvokes, setPendingInvokes] = useState<Map<string, DoctorInvoke>>(new Map());
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [target, setTarget] = useState("local");
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
      bind<DoctorInvoke>("doctor:invoke", (e) => {
        // Ignore invokes arriving before diagnosis starts. Stale invokes
        // (replayed by gateway during reconnect) are rejected on the Rust side
        // after handshake completes — see bridge_client.rs connect().
        if (!sessionActiveRef.current) return;

        const invoke = e.payload;

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
            { id: nextMsgId(), role: "tool-call", content: invoke.command, invoke, status: isFullAuto ? "auto" : "pending" },
          ];
        });

        // Full-auto mode: approve everything immediately
        if (isFullAuto) {
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
      bind<{ message: string }>("doctor:error", (e) => {
        setError(e.payload.message);
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
      await api.doctorApproveInvoke(invokeId, targetRef.current, sessionKeyRef.current, agentIdRef.current, domainRef.current);
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
    ) => {
    agentIdRef.current = agentId;
    targetRef.current = target;
    instanceScopeRef.current = instanceScope ?? target;
    domainRef.current = domain;
    setLoading(true);
    setMessages([]);
    setPendingInvokes(new Map());
    streamingRef.current = "";
    streamEndedRef.current = false;
    // Fresh session key per diagnosis — no inherited stale state
    sessionKeyRef.current = `agent:${agentId}:clawpal-doctor:${instanceScopeRef.current}:${crypto.randomUUID()}`;
    sessionActiveRef.current = true;
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
        prompt = [
          `You are ClawPal's diagnostic assistant powered by Doctor Claw. Respond in ${lang}.`,
          "Identity rule: you are Doctor Claw (the diagnosing engine), not the target machine itself.",
          "When asked who/where you are, always state both: engine=Doctor Claw, target=<current target>.",
          transportLine,
          `\nSystem context:\n${context}\n`,
          "Analyze issues directly and give concrete next actions. Keep response concise.",
        ].join("\n");
      }
      if (domain === "install") {
        await api.installStartSession(prompt, sessionKeyRef.current, agentId, scope);
      } else {
        await api.doctorStartDiagnosis(prompt, sessionKeyRef.current, agentId, scope);
      }
    } catch (err) {
      setError(`Start diagnosis failed: ${err}`);
      setLoading(false);
    }
  }, [target]);

  const sendMessage = useCallback(async (message: string) => {
    setLoading(true);
    streamingRef.current = "";
    setMessages((prev) => [...prev, { id: nextMsgId(), role: "user", content: message }]);
    try {
      if (domainRef.current === "install") {
        await api.installSendMessage(message, sessionKeyRef.current, agentIdRef.current, instanceScopeRef.current);
      } else {
        await api.doctorSendMessage(message, sessionKeyRef.current, agentIdRef.current, instanceScopeRef.current);
      }
    } catch (err) {
      setError(`Send message failed: ${err}`);
      setLoading(false);
    }
  }, []);

  const approveInvoke = useCallback(async (invokeId: string) => {
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
      await api.doctorApproveInvoke(invokeId, targetRef.current, sessionKeyRef.current, agentIdRef.current, domainRef.current);
    } catch (err) {
      setError(`Approve failed: ${err}`);
    }
  }, []);

  const rejectInvoke = useCallback(async (invokeId: string, reason = "User rejected") => {
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
