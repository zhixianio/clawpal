import { useCallback, useEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { AgentMessageBubble } from "@/components/AgentMessageBubble";
import { SshFormWidget } from "@/components/SshFormWidget";
import { useDoctorAgent } from "@/lib/use-doctor-agent";
import { api } from "@/lib/api";
import { installHubFallbackPromptTemplate, renderPromptTemplate } from "@/lib/prompt-templates";
import type { DoctorChatMessage, InstallMethod, InstallSession, RegisteredInstance, SshHost } from "@/lib/types";
import {
  AlertDialog,
  AlertDialogAction,
  AlertDialogCancel,
  AlertDialogContent,
  AlertDialogDescription,
  AlertDialogFooter,
  AlertDialogHeader,
  AlertDialogTitle,
} from "@/components/ui/alert-dialog";
import i18n from "../i18n";

/**
 * Detect assistant messages that describe tool-call actions rather than user-facing content.
 * These are the agent "narrating" what it's doing (e.g. "建议执行诊断命令：docker ...").
 */
function isToolNarration(text: string): boolean {
  const t = text.trim();
  return /^建议执行.*命令[：:]/.test(t)
    || /^原因[：:]/.test(t)
    || /^(Running|Executing|Checking)[：: ]/i.test(t)
    || /^正在(执行|检查|运行)/.test(t);
}

/**
 * Parse numbered/bulleted choice lists from assistant text.
 * Matches many patterns the agent uses:
 *   1. Option text        |  1) Option text
 *   - Option text         |  • Option text
 *   选项 1: Option text   |  Option 1: text
 *   **Option text**       (bold list items)
 */
function extractChoices(text: string): { prose: string; options: Array<{ label: string; value: string }> } | null {
  const lines = text.split("\n");
  const optionLines: Array<{ idx: number; label: string }> = [];
  // Broad pattern: numbered (1. / 1) / 选项1: / Option 1:) or bulleted (- / •)
  const listPattern = /^\s*(?:(?:选项|option)\s*\d+\s*[:：]\s*|(?:\*{1,2})?\d+[.)：:]\s*(?:\*{1,2})?\s*|[-•]\s+)\*{0,2}(.+?)\*{0,2}\s*$/i;

  for (let i = 0; i < lines.length; i++) {
    const match = lines[i].match(listPattern);
    if (match) {
      optionLines.push({ idx: i, label: match[1].trim() });
    }
  }

  if (optionLines.length < 2) return null;

  const firstIdx = optionLines[0].idx;
  const lastIdx = optionLines[optionLines.length - 1].idx;
  const blockSize = lastIdx - firstIdx + 1;
  if (blockSize > optionLines.length + 2) return null;

  // Prose = lines before the list, excluding "请选择" / "你想要" type headers and trailing "请告诉我" lines
  const isHeaderLine = (l: string) => {
    const t = l.trim();
    return t.length === 0
      || /[：:]$/.test(t)
      || /^请/.test(t)
      || /choose|select/i.test(t);
  };
  const proseLines = lines.slice(0, firstIdx).filter((l) => !isHeaderLine(l));
  // Also skip trailing "请告诉我你的选择" / "Please tell me" after the list
  const afterLines = lines.slice(lastIdx + 1).filter((l) => {
    const t = l.trim();
    return t.length > 0 && !/^请/.test(t) && !/please/i.test(t);
  });
  const prose = [...proseLines, ...afterLines].join("\n").trim();

  const options = optionLines.map((o) => {
    // Split "label — description" for cleaner buttons
    const dashMatch = o.label.match(/^(.+?)\s*[-—–]+\s+(.+)$/);
    return {
      label: dashMatch ? dashMatch[1].trim() : o.label,
      value: o.label,
    };
  });

  return { prose, options };
}

function ToolResultCollapsible({ content }: { content: string }) {
  const [expanded, setExpanded] = useState(false);
  const preview = content.length > 120 ? content.slice(0, 120) + "…" : content;
  return (
    <div className="rounded-md text-xs border border-border/50 bg-muted/20 text-muted-foreground font-mono">
      <button
        type="button"
        className="w-full text-left px-3 py-1.5 hover:text-foreground cursor-pointer"
        onClick={() => setExpanded(!expanded)}
      >
        {expanded ? "▾ result" : `▸ ${preview}`}
      </button>
      {expanded && (
        <pre className="px-3 pb-2 overflow-auto max-h-48 whitespace-pre-wrap break-all">
          {content}
        </pre>
      )}
    </div>
  );
}

/** Translate install step text if it looks like an i18n key (e.g. "install.docker.precheck.summary"). */
function translateInstallText(t: (key: string) => string, text: string): string {
  if (/^install\.\w+\.\w+\.\w+/.test(text)) {
    const translated = t(text);
    // t() returns the key itself when not found
    return translated !== text ? translated : text;
  }
  return text;
}

const PRESET_TAGS = [
  { key: "local", labelKey: "installChat.tag.local" },
  { key: "docker", labelKey: "installChat.tag.docker" },
  { key: "ssh", labelKey: "installChat.tag.ssh" },
  { key: "digitalocean", labelKey: "installChat.tag.digitalocean" },
];

function sanitizeSshIdSegment(raw: string): string {
  const lowered = raw.toLowerCase().trim();
  const replaced = lowered.replace(/[^a-z0-9_-]+/g, "-").replace(/^-+|-+$/g, "");
  return replaced || "remote";
}

function buildDefaultSshHostId(host: SshHost): string {
  const base = host.host || host.label || "remote";
  return `ssh:${sanitizeSshIdSegment(base)}`;
}

function buildInstallPrompt(userIntent: string): string {
  const language = i18n.language?.startsWith("zh") ? "Chinese (简体中文)" : "English";
  return renderPromptTemplate(installHubFallbackPromptTemplate(), {
    "{{LANGUAGE}}": language,
    "{{USER_INTENT}}": userIntent,
  });
}

export function InstallHub({
  open,
  onOpenChange,
  showToast,
  onNavigate,
  onReady,
  existingInstances,
}: {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  showToast?: (message: string, type?: "success" | "error") => void;
  onNavigate?: (route: string) => void;
  onReady?: (session: InstallSession) => void;
  existingInstances?: RegisteredInstance[];
}) {
  const { t } = useTranslation();
  const agent = useDoctorAgent();
  const [input, setInput] = useState("");
  const [sessionStarted, setSessionStarted] = useState(false);
  const [mode, setMode] = useState<"idle" | "running" | "failed" | "chat" | "connect">("idle");
  const [installMethod, setInstallMethod] = useState<InstallMethod>("local");
  const [runLogs, setRunLogs] = useState<string[]>([]);
  const [runError, setRunError] = useState<string | null>(null);
  const [activeSessionId, setActiveSessionId] = useState<string | null>(null);
  const [connectSubmitting, setConnectSubmitting] = useState(false);
  // Connect-existing form state
  const [showExistingDialog, setShowExistingDialog] = useState(false);
  const [pendingInstallTag, setPendingInstallTag] = useState<{ method: InstallMethod; label: string } | null>(null);
  const scrollRef = useRef<HTMLDivElement>(null);

  // Auto-scroll on new messages
  useEffect(() => {
    const el = scrollRef.current;
    if (el) el.scrollTop = el.scrollHeight;
  }, [agent.messages, agent.loading]);

  // Connect on dialog open, disconnect on close
  useEffect(() => {
    if (open) {
      agent.connect().catch(() => {});
    } else {
      agent.disconnect().catch(() => {});
      agent.reset();
      agent.setFullAuto(false);
      setSessionStarted(false);
      setInput("");
      setMode("idle");
      setInstallMethod("local");
      setRunLogs([]);
      setRunError(null);
      setActiveSessionId(null);
      setConnectSubmitting(false);
      setShowExistingDialog(false);
      setPendingInstallTag(null);
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [open]);

  const startDeterministicInstall = useCallback(async (
    method: InstallMethod,
    goal: string,
    options?: Record<string, unknown>,
  ) => {
    if (!onReady) return;
    setInstallMethod(method);
    setMode("running");
    setRunLogs([]);
    setRunError(null);
    const STEPS: Array<"precheck" | "install" | "init" | "verify"> = ["precheck", "install", "init", "verify"];
    try {
      const session = await api.installCreateSession(method, options);
      setActiveSessionId(session.id);
      setRunLogs((prev) => [...prev, `[session] ${session.id}`]);

      for (const step of STEPS) {
        setRunLogs((prev) => [...prev, `[${step}] ...`]);
        const result = await api.installRunStep(session.id, step);
        const summaryText = translateInstallText(t, result.summary);
        setRunLogs((prev) => [...prev, summaryText]);
        if (!result.ok) {
          throw new Error(translateInstallText(t, result.details || result.summary) || t("installChat.stepFailed"));
        }
        const latest = await api.installGetSession(session.id);
        if (latest.state === "ready") {
          setRunLogs((prev) => [...prev, `[done] ${t("installChat.ready")}`]);
          onReady(latest);
          return;
        }
      }
      // All 4 steps passed — check final state
      const final_ = await api.installGetSession(session.id);
      if (final_.state === "ready") {
        setRunLogs((prev) => [...prev, `[done] ${t("installChat.ready")}`]);
        onReady(final_);
      } else {
        throw new Error(t("installChat.notReady"));
      }
    } catch (e) {
      const message = e instanceof Error ? e.message : String(e);
      setRunError(message);
      setMode("failed");
    }
  }, [onReady, t]);

  const startDeterministicFromTarget = useCallback(async (goal: string) => {
    try {
      const decision = await api.installDecideTarget(goal);
      const method = decision.method;
      if (method === "docker" || method === "local") {
        await startDeterministicInstall(method, goal);
        return;
      }
      if (method === "remote_ssh") {
        const hosts = await api.listSshHosts();
        if (hosts.length === 1) {
          await startDeterministicInstall("remote_ssh", goal, {
            ssh_host_id: hosts[0].id,
          });
          return;
        }
        if (hosts.length === 0) {
          throw new Error("No SSH host configured. Add one in Instances first.");
        }
        throw new Error("Multiple SSH hosts found. Choose one in Instances first.");
      }
      throw new Error(decision.reason || "No deterministic install target available");
    } catch (e) {
      const message = e instanceof Error ? e.message : String(e);
      setRunError(message);
      setMode("failed");
    }
  }, [startDeterministicInstall]);

  // Start agent session with the user's first message baked into the system prompt
  const startSession = useCallback((userIntent: string) => {
    if (sessionStarted || !agent.bridgeConnected) return;
    setSessionStarted(true);
    // Auto-approve all tool invocations — install runs silently
    agent.setFullAuto(true);
    const prompt = buildInstallPrompt(userIntent);
    agent.startDiagnosis("", "main", undefined, "local", prompt, "install").catch(() => {});
  }, [sessionStarted, agent]);

  const handleSend = useCallback(() => {
    const text = input.trim();
    if (!text || agent.loading) return;
    if (!sessionStarted) {
      // First message — intent is baked into the system prompt, don't also sendMessage
      setMode("chat");
      startSession(text);
    } else {
      agent.sendMessage(text);
    }
    setInput("");
  }, [input, agent, sessionStarted, startSession]);

  const handleTagClick = useCallback((tagKey: string, tagLabel: string) => {
    if (agent.loading) return;
    if (tagKey === "connect") {
      setMode("connect");
      return;
    }
    if (tagKey === "docker" || tagKey === "local") {
      const method = tagKey as InstallMethod;
      const existing = (existingInstances || []).filter(
        (inst) => inst.instanceType === (method === "docker" ? "docker" : "local"),
      );
      if (existing.length > 0) {
        setPendingInstallTag({ method, label: tagLabel });
        setShowExistingDialog(true);
        return;
      }
      void startDeterministicInstall(method, tagLabel);
      return;
    }
    if (tagKey === "ssh" || tagKey === "digitalocean") {
      void startDeterministicFromTarget(tagLabel);
      return;
    }
    setInstallMethod("remote_ssh");
    setMode("chat");
    if (!sessionStarted) startSession(tagLabel);
    else agent.sendMessage(tagLabel);
  }, [agent, sessionStarted, startSession, startDeterministicInstall, startDeterministicFromTarget]);

  // A2UI: intercept render_form tool-calls + parse text-based choices from assistant messages
  const extraRenderer = useCallback((msg: DoctorChatMessage) => {
    // Parse numbered/bulleted choices from assistant messages into clickable buttons
    if (msg.role === "assistant" && msg.content) {
      const parsed = extractChoices(msg.content);
      if (parsed) {
        return (
          <div className="flex flex-col gap-2">
            {parsed.prose && (
              <div className="flex justify-start">
                <div className="px-3 py-2 rounded-lg max-w-[85%] bg-[oklch(0.93_0_0)] dark:bg-muted dark:text-foreground">
                  <div className="text-sm whitespace-pre-wrap">{parsed.prose}</div>
                </div>
              </div>
            )}
            <div className="flex flex-wrap gap-2 pl-1">
              {parsed.options.map((opt) => (
                <button
                  key={opt.value}
                  type="button"
                  className="text-sm px-3 py-2 rounded-lg border cursor-pointer hover:bg-muted/60 hover:border-primary/40 transition-colors text-left"
                  onClick={() => agent.sendMessage(opt.value)}
                >
                  {opt.label}
                </button>
              ))}
            </div>
          </div>
        );
      }
    }

    // Render non-render_form tool-calls as minimal read-only activity indicators
    if (msg.role === "tool-call" && msg.invoke?.command !== "render_form") {
      const inv = msg.invoke!;
      return (
        <div className="rounded-md px-3 py-1.5 text-xs border border-border/50 bg-muted/20 text-muted-foreground font-mono">
          <span className="opacity-60">⚙</span> {inv.command}
          {inv.args?.path ? <span className="ml-1 opacity-70">{String(inv.args.path)}</span> : null}
        </div>
      );
    }

    // Render tool-results as collapsible detail
    if (msg.role === "tool-result") {
      return (
        <ToolResultCollapsible content={msg.content} />
      );
    }

    if (msg.role !== "tool-call" || msg.invoke?.command !== "render_form") return null;
    const formKind = msg.invoke.args?.formKind as string | undefined;

    if (formKind === "ssh_host") {
      return (
        <SshFormWidget
          invokeId={msg.invoke.id}
          defaults={msg.invoke.args?.defaults as Partial<SshHost> | undefined}
          onSubmit={(invokeId, host) => {
            agent.sendMessage(JSON.stringify(host));
            agent.approveInvoke(invokeId);
          }}
          onCancel={(invokeId) => {
            agent.rejectInvoke(invokeId, "User cancelled form");
          }}
        />
      );
    }

    if (formKind === "choice") {
      const options = (msg.invoke.args?.options as Array<{ label: string; value: string; description?: string }>) ?? [];
      const alreadyChosen = msg.status === "approved" || msg.status === "auto";
      return (
        <div className="flex flex-wrap gap-2">
          {options.map((opt) => (
            <button
              key={opt.value}
              type="button"
              disabled={alreadyChosen}
              className="text-sm px-3 py-2 rounded-lg border cursor-pointer hover:bg-muted/60 hover:border-primary/40 disabled:opacity-50 disabled:cursor-default transition-colors text-left"
              onClick={() => {
                agent.sendMessage(opt.value);
                agent.approveInvoke(msg.invoke!.id);
              }}
            >
              <div className="font-medium">{opt.label}</div>
              {opt.description && (
                <div className="text-xs text-muted-foreground mt-0.5">{opt.description}</div>
              )}
            </button>
          ))}
        </div>
      );
    }

    return null;
  }, [agent]);

  // Filter messages: hide tool-narration assistant messages, show tool activity
  const visibleMessages = agent.messages.filter((msg) => {
    // Hide assistant messages that just narrate tool actions
    if (msg.role === "assistant" && isToolNarration(msg.content)) return false;
    return true;
  });

  const hasMessages = visibleMessages.length > 0;

  // Done button handler — builds a synthetic InstallSession and calls onReady
  const handleDone = useCallback(() => {
    if (!onReady) return;
    const now = new Date().toISOString();
    onReady({
      id: `install-${Date.now()}`,
      method: installMethod,
      state: "ready",
      current_step: null,
      logs: [],
      artifacts: {},
      created_at: now,
      updated_at: now,
    });
  }, [onReady, installMethod]);

  // SSH connect submit handler
  const handleSshConnectSubmit = useCallback(async (host: SshHost) => {
    setConnectSubmitting(true);
    setRunError(null);
    try {
      const existingHosts = await api.listSshHosts().catch(() => [] as SshHost[]);
      const requestedId = host.id?.trim();
      const idBase = requestedId || buildDefaultSshHostId(host);
      const existingIds = new Set(existingHosts.map((item) => item.id));
      let resolvedId = idBase;
      let suffix = 2;
      while (existingIds.has(resolvedId) && resolvedId !== requestedId) {
        resolvedId = `${idBase}-${suffix}`;
        suffix += 1;
      }
      const saved = await api.upsertSshHost({
        ...host,
        id: resolvedId,
      });
      await api.sshConnect(saved.id);
      try {
        await api.remoteGetInstanceStatus(saved.id);
      } catch {
        // Remote openclaw might not be installed yet — that's OK for connect
      }
      const now = new Date().toISOString();
      onReady?.({
        id: `install-${Date.now()}`,
        method: "remote_ssh",
        state: "ready",
        current_step: null,
        logs: [],
        artifacts: {
          ssh_host_id: saved.id,
          ssh_host_label: saved.label,
        },
        created_at: now,
        updated_at: now,
      });
    } catch (e) {
      const message = e instanceof Error ? e.message : String(e);
      setRunError(message);
      showToast?.(message, "error");
    } finally {
      setConnectSubmitting(false);
    }
  }, [onReady, showToast]);

  return (
    <>
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent className="max-w-2xl max-h-[80vh] flex flex-col">
        <DialogHeader>
          <DialogTitle>
            {mode === "connect" ? t("installChat.connectRemoteTitle") : t("installChat.title")}
          </DialogTitle>
          <DialogDescription className="sr-only">
            {mode === "connect" ? t("installChat.connectRemoteTitle") : t("installChat.title")}
          </DialogDescription>
        </DialogHeader>

        {mode === "connect" ? (
          /* ── Connect Remote SSH form ── */
          <div className="space-y-4 py-2">
            <div className="text-sm text-muted-foreground">
              {t("installChat.connectRemoteDescription")}
            </div>
            <SshFormWidget
              invokeId="connect-ssh-form"
              onSubmit={(_invokeId, host) => handleSshConnectSubmit(host)}
              onCancel={() => setMode("idle")}
            />
            {runError && (
              <div className="text-sm text-destructive border border-destructive/30 rounded-md px-3 py-2 bg-destructive/5">
                {runError}
              </div>
            )}
          </div>
        ) : mode === "running" || mode === "failed" ? (
          <div className="space-y-3">
            <div className="text-sm text-muted-foreground">
              {mode === "running" ? t("installChat.connecting") : t("doctor.failed")}
            </div>
            <div className="border rounded-md p-3 bg-muted/30 max-h-[40vh] overflow-y-auto">
              <div className="space-y-1 text-xs font-mono">
                {activeSessionId && <div>session: {activeSessionId}</div>}
                {runLogs.length === 0 && (
                  <div className="animate-pulse text-muted-foreground">running…</div>
                )}
                {runLogs.map((line, idx) => (
                  <div key={`${idx}-${line.slice(0, 16)}`}>{line}</div>
                ))}
              </div>
            </div>
            {runError && (
              <div className="text-sm text-destructive border border-destructive/30 rounded-md px-3 py-2 bg-destructive/5">
                {runError}
              </div>
            )}
          </div>
        ) : (
          <>
            {/* Preset tags — visible until user starts the conversation */}
            {!hasMessages && mode === "idle" && (
              <div className="flex flex-wrap items-center gap-2">
                {PRESET_TAGS.map((tag) => (
                  <button
                    key={tag.key}
                    type="button"
                    className="text-sm px-3 py-1.5 rounded-full border cursor-pointer hover:bg-muted/60 hover:border-primary/40 transition-colors"
                    onClick={() => handleTagClick(tag.key, t(tag.labelKey))}
                    disabled={agent.loading || !agent.bridgeConnected}
                  >
                    {t(tag.labelKey)}
                  </button>
                ))}
                <button
                  type="button"
                  className="text-sm px-3 py-1.5 rounded-full border cursor-pointer hover:bg-muted/60 hover:border-primary/40 transition-colors"
                  onClick={() => handleTagClick("connect", t("installChat.tag.connectRemote"))}
                >
                  {t("installChat.tag.connectRemote")}
                </button>
              </div>
            )}

            {/* Chat message list */}
            <div
              ref={scrollRef}
              className="flex-1 min-h-[300px] max-h-[50vh] border rounded-md p-3 bg-muted/30 overflow-y-auto"
            >
              <div className="space-y-3">
                {agent.error && !agent.error.includes("Auto-approve") && (
                  <div className="text-sm text-destructive border border-destructive/30 rounded-md px-3 py-2 bg-destructive/5">
                    {agent.error}
                  </div>
                )}
                {!agent.bridgeConnected && !agent.error && (
                  <div className="text-sm text-muted-foreground animate-pulse">
                    {t("installChat.connecting")}
                  </div>
                )}
                {agent.bridgeConnected && !hasMessages && !agent.loading && (
                  <div className="text-sm text-muted-foreground">
                    {t("installChat.inputPlaceholder")}
                  </div>
                )}
                {visibleMessages.map((msg) => (
                  <AgentMessageBubble
                    key={msg.id}
                    message={msg}
                    onApprove={agent.approveInvoke}
                    onReject={agent.rejectInvoke}
                    extraRenderer={extraRenderer}
                  />
                ))}
                {agent.loading && (
                  <div className="text-sm text-muted-foreground animate-pulse">
                    {t("doctor.agentThinking")}
                  </div>
                )}
              </div>
            </div>

            {/* Input bar */}
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
                placeholder={t("installChat.inputPlaceholder")}
                disabled={!agent.bridgeConnected || agent.loading}
                className="flex-1"
              />
              <Button
                onClick={handleSend}
                disabled={!agent.bridgeConnected || agent.loading || !input.trim()}
                size="sm"
              >
                {t("chat.send")}{" "}
                <kbd className="ml-1 text-xs opacity-60">
                  {navigator.platform.includes("Mac") ? "⌘↵" : "Ctrl↵"}
                </kbd>
              </Button>
            </div>
          </>
        )}

        {/* Footer with Done / Connect button */}
        {mode === "connect" && connectSubmitting && (
          <DialogFooter>
            <div className="text-sm text-muted-foreground animate-pulse">
              {t("installChat.connecting")}
            </div>
          </DialogFooter>
        )}
        {mode === "chat" && sessionStarted && !agent.loading && (
          <DialogFooter>
            <Button onClick={handleDone}>
              {t("installChat.done")}
            </Button>
          </DialogFooter>
        )}
        {mode === "failed" && (
          <DialogFooter>
            <Button variant="outline" onClick={() => setMode("idle")}>
              {t("installChat.cancel")}
            </Button>
            <Button
              onClick={() => {
                setMode("chat");
                const fallbackIntent = runError
                  ? `Install failed: ${runError}`
                  : "Install failed";
                startSession(fallbackIntent);
              }}
              disabled={!agent.bridgeConnected || agent.loading}
            >
              {t("installChat.letAiHelp")}
            </Button>
          </DialogFooter>
        )}
      </DialogContent>
    </Dialog>

    {/* Existing instance confirmation */}
    <AlertDialog open={showExistingDialog} onOpenChange={setShowExistingDialog}>
      <AlertDialogContent>
        <AlertDialogHeader>
          <AlertDialogTitle>{t("installChat.existingTitle")}</AlertDialogTitle>
          <AlertDialogDescription>
            {t("installChat.existingDescription")}
          </AlertDialogDescription>
        </AlertDialogHeader>
        <AlertDialogFooter>
          <AlertDialogCancel>{t("installChat.cancel")}</AlertDialogCancel>
          <AlertDialogAction
            onClick={() => {
              if (pendingInstallTag) {
                const existing = (existingInstances || []).find(
                  (inst) => inst.instanceType === (pendingInstallTag.method === "docker" ? "docker" : "local"),
                );
                if (existing) {
                  onOpenChange(false);
                  // Directly open the existing instance
                  const now = new Date().toISOString();
                  onReady?.({
                    id: `install-${Date.now()}`,
                    method: pendingInstallTag.method,
                    state: "ready",
                    current_step: null,
                    logs: [],
                    artifacts: existing.instanceType === "docker" ? {
                      docker_instance_id: existing.id,
                      docker_instance_label: existing.label,
                      docker_openclaw_home: existing.openclawHome || "",
                      docker_clawpal_data_dir: existing.clawpalDataDir || "",
                    } : {},
                    created_at: now,
                    updated_at: now,
                  });
                }
              }
              setShowExistingDialog(false);
              setPendingInstallTag(null);
            }}
          >
            {t("installChat.useExisting")}
          </AlertDialogAction>
          <AlertDialogAction
            onClick={() => {
              setShowExistingDialog(false);
              if (pendingInstallTag) {
                // Compute next available instance number for isolation
                const typeFilter = pendingInstallTag.method === "docker" ? "docker" : "local";
                const sameType = (existingInstances || []).filter((inst) => inst.instanceType === typeFilter);
                const nextNum = sameType.length + 1;
                const nextId = `docker:local-${nextNum}`;
                const options = pendingInstallTag.method === "docker"
                  ? { docker_instance_id: nextId, docker_instance_label: `Docker Local ${nextNum}` }
                  : undefined;
                void startDeterministicInstall(pendingInstallTag.method, pendingInstallTag.label, options);
              }
              setPendingInstallTag(null);
            }}
          >
            {t("installChat.installNew")}
          </AlertDialogAction>
        </AlertDialogFooter>
      </AlertDialogContent>
    </AlertDialog>
    </>
  );
}
