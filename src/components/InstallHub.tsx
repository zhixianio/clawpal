import { useCallback, useEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
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
import { hasGuidanceEmitted } from "@/lib/use-api";
import { isAlreadyExplainedGuidanceError } from "@/lib/guidance";
import { api } from "@/lib/api";
import { withGuidance } from "@/lib/guidance";
import { installHubFallbackPromptTemplate, renderPromptTemplate } from "@/lib/prompt-templates";
import type {
  DoctorChatMessage,
  InstallMethod,
  InstallSession,
  SshConfigHostSuggestion,
  SshHost,
} from "@/lib/types";
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

const PRESET_TAGS = [
  { key: "connect_ssh", labelKey: "installChat.tag.connectRemote" },
  { key: "connect_docker", labelKey: "installChat.tag.connectDocker" },
  { key: "connect_wsl2", labelKey: "installChat.tag.connectWsl2" },
];

const DIAGNOSTIC_LOG_LINES = 300;

type InstallHubDiagnosticLogs = {
  appLog: string;
  errorLog: string;
  gatewayLog: string;
  gatewayErrorLog: string;
};

const EMPTY_DIAGNOSTIC_LOGS: InstallHubDiagnosticLogs = {
  appLog: "",
  errorLog: "",
  gatewayLog: "",
  gatewayErrorLog: "",
};

function sanitizeSshIdSegment(raw: string): string {
  const lowered = raw.toLowerCase().trim();
  const replaced = lowered.replace(/[^a-z0-9_-]+/g, "-").replace(/^-+|-+$/g, "");
  return replaced || "remote";
}

function buildDefaultSshHostId(host: SshHost): string {
  const base = host.host || host.label || "remote";
  return `ssh:${sanitizeSshIdSegment(base)}`;
}

function sanitizeLocalIdSegment(raw: string): string {
  const lowered = raw.toLowerCase().trim();
  const replaced = lowered.replace(/[^a-z0-9_-]+/g, "-").replace(/^-+|-+$/g, "");
  return replaced || "default";
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
  showToast: _showToast,
  onNavigate,
  onReady,
  onOpenDoctor,
  connectRemoteHost,
}: {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  showToast?: (message: string, type?: "success" | "error") => void;
  onNavigate?: (route: string) => void;
  onReady?: (session: InstallSession) => void;
  onOpenDoctor?: () => void;
  connectRemoteHost?: (hostId: string) => Promise<void>;
}) {
  const { t } = useTranslation();
  const agent = useDoctorAgent();
  const [input, setInput] = useState("");
  const [sessionStarted, setSessionStarted] = useState(false);
  const [mode, setMode] = useState<"idle" | "running" | "failed" | "chat" | "connect_ssh" | "connect_docker" | "connect_wsl2">("idle");
  const [installMethod, setInstallMethod] = useState<InstallMethod>("local");
  const [runLogs, setRunLogs] = useState<string[]>([]);
  const [runError, setRunError] = useState<string | null>(null);
  const [runErrorHasGuidance, setRunErrorHasGuidance] = useState(false);
  const [activeSessionId, setActiveSessionId] = useState<string | null>(null);
  const [connectSubmitting, setConnectSubmitting] = useState(false);
  const [dockerConnectHome, setDockerConnectHome] = useState("~/.clawpal/docker-local");
  const [dockerConnectLabel, setDockerConnectLabel] = useState("docker-local");
  const [wsl2ConnectHome, setWsl2ConnectHome] = useState("");
  const [wsl2ConnectLabel, setWsl2ConnectLabel] = useState("wsl2-default");
  const [sshConfigSuggestions, setSshConfigSuggestions] = useState<
    SshConfigHostSuggestion[]
  >([]);
  const [sshConfigSuggestionsLoading, setSshConfigSuggestionsLoading] = useState(false);
  const [sshConfigSuggestionsError, setSshConfigSuggestionsError] = useState<string | null>(null);
  const [sshConfigSuggestionsLoaded, setSshConfigSuggestionsLoaded] = useState(false);
  const [diagnosticHostId, setDiagnosticHostId] = useState<string | null>(null);
  const [diagnosticLogs, setDiagnosticLogs] = useState<InstallHubDiagnosticLogs>(EMPTY_DIAGNOSTIC_LOGS);
  const [diagnosticsLoading, setDiagnosticsLoading] = useState(false);
  const [diagnosticsVisible, setDiagnosticsVisible] = useState(false);
  const [diagnosticsError, setDiagnosticsError] = useState<string | null>(null);
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
      setRunErrorHasGuidance(false);
      setActiveSessionId(null);
      setConnectSubmitting(false);
      setDockerConnectHome("~/.clawpal/docker-local");
      setDockerConnectLabel("docker-local");
      setWsl2ConnectHome("");
      setWsl2ConnectLabel("wsl2-default");
      setSshConfigSuggestions([]);
      setSshConfigSuggestionsLoading(false);
      setSshConfigSuggestionsError(null);
      setSshConfigSuggestionsLoaded(false);
      setDiagnosticHostId(null);
      setDiagnosticLogs(EMPTY_DIAGNOSTIC_LOGS);
      setDiagnosticsLoading(false);
      setDiagnosticsVisible(false);
      setDiagnosticsError(null);
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [open]);

  const loadSshConfigSuggestions = useCallback(async () => {
    if (sshConfigSuggestionsLoading) {
      return;
    }
    if (sshConfigSuggestionsLoaded) {
      return;
    }
    setSshConfigSuggestionsLoading(true);
    setSshConfigSuggestionsError(null);
    try {
      const list = await withGuidance(
        () => api.listSshConfigHosts(),
        "listSshConfigHosts",
        "local",
        "local",
      );
      setSshConfigSuggestions(list);
    } catch (e) {
      if (import.meta.env.DEV) {
        console.error("[dev exception] loadSshConfigSuggestions", e);
      }
      setSshConfigSuggestionsError(
        e instanceof Error ? e.message : String(e),
      );
      setSshConfigSuggestions([]);
    } finally {
      setSshConfigSuggestionsLoaded(true);
      setSshConfigSuggestionsLoading(false);
    }
  }, [sshConfigSuggestionsLoaded, sshConfigSuggestionsLoading]);

  const clearDiagnostics = useCallback(() => {
    setDiagnosticHostId(null);
    setDiagnosticLogs(EMPTY_DIAGNOSTIC_LOGS);
    setDiagnosticsLoading(false);
    setDiagnosticsVisible(false);
    setDiagnosticsError(null);
  }, []);

  const formatLogReadError = (label: string, error: unknown) => {
    const message = error instanceof Error ? error.message : String(error);
    return `[${label}] ${message}`;
  };

  const readRemoteDiagnostics = useCallback(async (hostId: string) => {
    const [appLog, errorLog, gatewayLog, gatewayErrorLog] = await Promise.all([
      api
        .remoteReadAppLog(hostId, DIAGNOSTIC_LOG_LINES)
        .catch((error) => {
          if (import.meta.env.DEV) {
            console.error("[dev exception] readRemoteDiagnostics app.log", {
              hostId,
              error,
            });
          }
          return formatLogReadError("app.log", error);
        }),
      api
        .remoteReadErrorLog(hostId, DIAGNOSTIC_LOG_LINES)
        .catch((error) => {
          if (import.meta.env.DEV) {
            console.error("[dev exception] readRemoteDiagnostics error.log", {
              hostId,
              error,
            });
          }
          return formatLogReadError("error.log", error);
        }),
      api
        .remoteReadGatewayLog(hostId, DIAGNOSTIC_LOG_LINES)
        .catch((error) => {
          if (import.meta.env.DEV) {
            console.error("[dev exception] readRemoteDiagnostics gateway.log", {
              hostId,
              error,
            });
          }
          return formatLogReadError("gateway.log", error);
        }),
      api
        .remoteReadGatewayErrorLog(hostId, DIAGNOSTIC_LOG_LINES)
        .catch((error) => {
          if (import.meta.env.DEV) {
            console.error("[dev exception] readRemoteDiagnostics gateway.err.log", {
              hostId,
              error,
            });
          }
          return formatLogReadError("gateway.err.log", error);
        }),
    ]);
    return { appLog, errorLog, gatewayLog, gatewayErrorLog };
  }, [formatLogReadError]);

  const readLocalDiagnostics = useCallback(async () => {
      const [appLog, errorLog, gatewayLog, gatewayErrorLog] = await Promise.all([
      api.readAppLog(DIAGNOSTIC_LOG_LINES).catch((error) => {
        if (import.meta.env.DEV) {
          console.error("[dev exception] readLocalDiagnostics app.log", error);
        }
        return formatLogReadError("app.log", error);
      }),
      api.readErrorLog(DIAGNOSTIC_LOG_LINES).catch((error) => {
        if (import.meta.env.DEV) {
          console.error("[dev exception] readLocalDiagnostics error.log", error);
        }
        return formatLogReadError("error.log", error);
      }),
      api.readGatewayLog(DIAGNOSTIC_LOG_LINES).catch((error) => {
        if (import.meta.env.DEV) {
          console.error("[dev exception] readLocalDiagnostics gateway.log", error);
        }
        return formatLogReadError("gateway.log", error);
      }),
      api.readGatewayErrorLog(DIAGNOSTIC_LOG_LINES).catch((error) => {
        if (import.meta.env.DEV) {
          console.error("[dev exception] readLocalDiagnostics gateway.err.log", error);
        }
        return formatLogReadError("gateway.err.log", error);
      }),
    ]);
    return { appLog, errorLog, gatewayLog, gatewayErrorLog };
  }, [formatLogReadError]);

  const refreshDiagnostics = useCallback(async (hostId: string | null) => {
    if (diagnosticsLoading) {
      return;
    }
    setDiagnosticsLoading(true);
    setDiagnosticsError(null);
    setDiagnosticsVisible(true);
    try {
      const logs = hostId
        ? await readRemoteDiagnostics(hostId)
        : await readLocalDiagnostics();
      setDiagnosticLogs(logs);
      setDiagnosticHostId(hostId);
    } catch (error) {
      if (import.meta.env.DEV) {
        console.error("[dev exception] refreshDiagnostics", {
          hostId,
          error,
        });
      }
      const message = error instanceof Error ? error.message : String(error);
      setDiagnosticsError(message);
    } finally {
      setDiagnosticsLoading(false);
    }
  }, [diagnosticsLoading, readRemoteDiagnostics, readLocalDiagnostics]);

  useEffect(() => {
    if (open && mode === "connect_ssh") {
      void loadSshConfigSuggestions();
    }
  }, [loadSshConfigSuggestions, mode, open]);

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
    if (tagKey === "connect_ssh") {
      setMode("connect_ssh");
      return;
    }
    if (tagKey === "connect_docker") {
      setMode("connect_docker");
      return;
    }
    if (tagKey === "connect_wsl2") {
      setMode("connect_wsl2");
      return;
    }
    setInstallMethod("remote_ssh");
    setMode("chat");
    if (!sessionStarted) startSession(tagLabel);
    else agent.sendMessage(tagLabel);
  }, [agent, sessionStarted, startSession]);

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
  const clearRunErrorState = useCallback(() => {
    setRunError(null);
    setRunErrorHasGuidance(false);
    clearDiagnostics();
  }, [clearDiagnostics]);

  const diagnosticTargetLabel = diagnosticHostId
    ? `remote (${diagnosticHostId})`
    : t("instance.local");
  const renderDiagnosticSection = (title: string, content: string) => (
    <details className="border border-border rounded-md" open>
      <summary className="px-3 py-2 cursor-pointer text-xs font-medium">
        {title}
      </summary>
      <pre className="px-3 pb-2 whitespace-pre-wrap break-words text-xs font-mono max-h-48 overflow-auto">
        {content || t("doctor.noLogs")}
      </pre>
    </details>
  );

  const runErrorPanel = runError ? (
    <div className="rounded-md border border-destructive/30 bg-destructive/5 text-destructive px-3 py-2 space-y-2">
      <p className="text-sm font-medium">{t("doctor.failed")}</p>
      {!runErrorHasGuidance && (
        <p className="text-sm whitespace-pre-wrap break-words">{runError}</p>
      )}
      {runErrorHasGuidance && (
        <p className="text-sm text-muted-foreground">
          {t("home.fixInDoctor")}
        </p>
      )}
      <div className="flex flex-wrap gap-2">
        {onOpenDoctor && (
          <Button
            type="button"
            size="sm"
            variant="outline"
            onClick={() => onOpenDoctor()}
          >
            {t("home.fixInDoctor")}
          </Button>
        )}
        <Button
          type="button"
          size="sm"
          variant={diagnosticsVisible ? "secondary" : "outline"}
          onClick={() => setDiagnosticsVisible((v) => !v)}
        >
          {diagnosticsVisible ? t("doctor.collapse") : t("doctor.details")}
        </Button>
        <Button
          type="button"
          size="sm"
          variant="outline"
          onClick={() => {
            void refreshDiagnostics(diagnosticHostId);
          }}
          disabled={diagnosticsLoading}
        >
          {t("doctor.refreshLogs")}
        </Button>
      </div>
      <p className="text-xs text-muted-foreground">
        {t("doctor.appLog")} / {t("doctor.errorLog")} / {t("doctor.gatewayLogs")} 来源：{diagnosticTargetLabel}
      </p>
      {diagnosticsLoading && (
        <p className="text-xs text-muted-foreground animate-pulse">loading…</p>
      )}
      {diagnosticsError && (
        <p className="text-xs text-destructive">diagnostics: {diagnosticsError}</p>
      )}
      {diagnosticsVisible && (
        <div className="space-y-2">
          {renderDiagnosticSection(t("doctor.appLog"), diagnosticLogs.appLog)}
          {renderDiagnosticSection(t("doctor.errorLog"), diagnosticLogs.errorLog)}
          {renderDiagnosticSection(`${t("doctor.gatewayLogs")} (app)`, diagnosticLogs.gatewayLog)}
          {renderDiagnosticSection(`${t("doctor.gatewayLogs")} (error)`, diagnosticLogs.gatewayErrorLog)}
        </div>
      )}
    </div>
  ) : null;

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
    clearDiagnostics();
    let targetHostId: string | null = null;
    try {
      const existingHosts = await withGuidance(
        () => api.listSshHosts(),
        "listSshHosts",
        "local",
        "local",
      ).catch((error) => {
        if (import.meta.env.DEV) {
          console.error("[dev exception] listSshHosts in handleSshConnectSubmit", error);
        }
        return [] as SshHost[];
      });
      const requestedId = host.id?.trim();
      const idBase = requestedId || buildDefaultSshHostId(host);
      const existingIds = new Set(existingHosts.map((item) => item.id));
      let resolvedId = idBase;
      let suffix = 2;
      while (existingIds.has(resolvedId) && resolvedId !== requestedId) {
        resolvedId = `${idBase}-${suffix}`;
        suffix += 1;
      }
      targetHostId = resolvedId;
      const saved = await withGuidance(
        () => api.upsertSshHost({
          ...host,
          id: resolvedId,
        }),
        "upsertSshHost",
        resolvedId,
        "remote_ssh",
      );
      targetHostId = saved.id;
      if (connectRemoteHost) {
        await connectRemoteHost(saved.id);
      } else {
        await withGuidance(
          () => api.sshConnect(saved.id),
          "sshConnect",
          saved.id,
          "remote_ssh",
        );
      }
      try {
        await withGuidance(
          () => api.remoteGetInstanceStatus(saved.id),
          "remoteGetInstanceStatus",
          saved.id,
          "remote_ssh",
        );
      } catch {
        // Remote openclaw might not be installed yet — that's OK for connect
        if (import.meta.env.DEV) {
          console.warn("[dev exception] remoteGetInstanceStatus skipped (not installed)", saved.id);
        }
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
      const errorText = e instanceof Error ? e.message : String(e);
      const guidanceError = hasGuidanceEmitted(e) || isAlreadyExplainedGuidanceError(errorText);
      if (import.meta.env.DEV) {
        console.error("[dev exception] handleSshConnectSubmit", {
          targetHostId,
          error: e,
          guidanceError,
        });
      }
      setDiagnosticHostId(targetHostId);
      setRunError(guidanceError ? t("doctor.failed") : errorText);
      setRunErrorHasGuidance(guidanceError);
      void refreshDiagnostics(targetHostId);
    } finally {
      setConnectSubmitting(false);
    }
  }, [onReady, refreshDiagnostics, clearDiagnostics, connectRemoteHost]);

  const handleDockerConnectSubmit = useCallback(async () => {
    setConnectSubmitting(true);
    setRunError(null);
    clearDiagnostics();
    try {
      const home = dockerConnectHome.trim();
      if (!home) throw new Error("Docker OpenClaw home is required");
      const label = dockerConnectLabel.trim() || undefined;
      const connected = await withGuidance(
        () => api.connectDockerInstance(home, label, undefined),
        "connectDockerInstance",
        "docker:manual",
        "docker_local",
      );
      const now = new Date().toISOString();
      onReady?.({
        id: `install-${Date.now()}`,
        method: "docker",
        state: "ready",
        current_step: null,
        logs: [],
        artifacts: {
          docker_instance_id: connected.id,
          docker_instance_label: connected.label,
          docker_openclaw_home: connected.openclawHome || home,
          docker_clawpal_data_dir: connected.clawpalDataDir || "",
        },
        created_at: now,
        updated_at: now,
      });
    } catch (e) {
      const errorText = e instanceof Error ? e.message : String(e);
      const guidanceError = hasGuidanceEmitted(e) || isAlreadyExplainedGuidanceError(errorText);
      setRunError(guidanceError ? t("doctor.failed") : errorText);
      setRunErrorHasGuidance(guidanceError);
      void refreshDiagnostics(null);
    } finally {
      setConnectSubmitting(false);
    }
  }, [clearDiagnostics, dockerConnectHome, dockerConnectLabel, onReady, refreshDiagnostics]);

  const handleWsl2ConnectSubmit = useCallback(async () => {
    setConnectSubmitting(true);
    setRunError(null);
    clearDiagnostics();
    try {
      const home = wsl2ConnectHome.trim();
      if (!home) throw new Error("WSL2 OpenClaw home path is required");
      const baseId = `wsl2:${sanitizeLocalIdSegment(
        wsl2ConnectLabel.trim() || home.split(/[\\/]/).pop() || "default",
      )}`;
      const existing = await withGuidance(
        () => api.listRegisteredInstances(),
        "listRegisteredInstances",
        "local",
        "local",
      ).catch(() => [] as Array<{ id: string }>);
      const existingIds = new Set(existing.map((inst) => inst.id));
      let id = baseId;
      let suffix = 2;
      while (existingIds.has(id)) {
        id = `${baseId}-${suffix}`;
        suffix += 1;
      }
      const label = wsl2ConnectLabel.trim() || undefined;
      const connected = await withGuidance(
        () => api.connectLocalInstance(home, label, id),
        "connectLocalInstance",
        id || "wsl2:manual",
        "local",
      );
      const now = new Date().toISOString();
      onReady?.({
        id: `install-${Date.now()}`,
        method: "wsl2",
        state: "ready",
        current_step: null,
        logs: [],
        artifacts: {
          local_instance_id: connected.id,
          local_instance_label: connected.label,
          local_openclaw_home: connected.openclawHome || home,
        },
        created_at: now,
        updated_at: now,
      });
    } catch (e) {
      const errorText = e instanceof Error ? e.message : String(e);
      const guidanceError = hasGuidanceEmitted(e) || isAlreadyExplainedGuidanceError(errorText);
      setRunError(guidanceError ? t("doctor.failed") : errorText);
      setRunErrorHasGuidance(guidanceError);
      void refreshDiagnostics(null);
    } finally {
      setConnectSubmitting(false);
    }
  }, [clearDiagnostics, wsl2ConnectHome, wsl2ConnectLabel, onReady, refreshDiagnostics]);

  return (
    <>
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent className="max-w-2xl max-h-[80vh] flex flex-col">
        <DialogHeader>
          <DialogTitle>
            {(mode === "connect_ssh" || mode === "connect_docker" || mode === "connect_wsl2")
              ? t("installChat.connectTitle")
              : t("installChat.title")}
          </DialogTitle>
          <DialogDescription className="sr-only">
            {(mode === "connect_ssh" || mode === "connect_docker" || mode === "connect_wsl2")
              ? t("installChat.connectTitle")
              : t("installChat.title")}
          </DialogDescription>
        </DialogHeader>

        {mode === "connect_ssh" ? (
          /* ── Connect Remote SSH form ── */
          <div className="space-y-4 py-2">
            <div className="text-sm text-muted-foreground">
              {t("installChat.connectRemoteDescription")}
            </div>
            <SshFormWidget
              invokeId="connect-ssh-form"
              sshConfigSuggestions={sshConfigSuggestions}
              onSubmit={(_invokeId, host) => handleSshConnectSubmit(host)}
              onCancel={() => {
                setMode("idle");
                clearRunErrorState();
              }}
            />
            {sshConfigSuggestionsLoading && (
              <div className="text-xs text-muted-foreground">
                {t("installChat.sshConfigPresetLoading")}
              </div>
            )}
            {sshConfigSuggestionsError && !sshConfigSuggestionsLoading && (
              <div className="text-sm text-destructive border border-destructive/30 rounded-md px-3 py-2 bg-destructive/5">
                {sshConfigSuggestionsError}
              </div>
            )}
          </div>
        ) : mode === "connect_docker" ? (
          <div className="space-y-4 py-2">
            <div className="text-sm text-muted-foreground">{t("installChat.connectDockerDescription")}</div>
            <div className="space-y-1.5">
              <Label>{t("installChat.dockerHomeLabel")}</Label>
              <Input
                value={dockerConnectHome}
                onChange={(e) => setDockerConnectHome(e.target.value)}
                placeholder={t("installChat.dockerHomePlaceholder")}
              />
            </div>
            <div className="space-y-1.5">
              <Label>{t("installChat.dockerLabelLabel")}</Label>
              <Input
                value={dockerConnectLabel}
                onChange={(e) => setDockerConnectLabel(e.target.value)}
                placeholder={t("installChat.dockerLabelPlaceholder")}
              />
            </div>
            <DialogFooter>
              <Button
                variant="outline"
                onClick={() => {
                  setMode("idle");
                  clearRunErrorState();
                }}
                disabled={connectSubmitting}
              >
                {t("installChat.cancel")}
              </Button>
              <Button onClick={handleDockerConnectSubmit} disabled={connectSubmitting}>
                {t("installChat.submit")}
              </Button>
            </DialogFooter>
          </div>
        ) : mode === "connect_wsl2" ? (
          <div className="space-y-4 py-2">
            <div className="text-sm text-muted-foreground">{t("installChat.connectWsl2Description")}</div>
            <div className="space-y-1.5">
              <Label>{t("installChat.wsl2HomeLabel")}</Label>
              <Input
                value={wsl2ConnectHome}
                onChange={(e) => setWsl2ConnectHome(e.target.value)}
                placeholder={t("installChat.wsl2HomePlaceholder")}
              />
            </div>
            <div className="space-y-1.5">
              <Label>{t("installChat.wsl2LabelLabel")}</Label>
              <Input
                value={wsl2ConnectLabel}
                onChange={(e) => setWsl2ConnectLabel(e.target.value)}
                placeholder={t("installChat.wsl2LabelPlaceholder")}
              />
            </div>
            <DialogFooter>
              <Button
                variant="outline"
                onClick={() => {
                  setMode("idle");
                  clearRunErrorState();
                }}
                disabled={connectSubmitting}
              >
                {t("installChat.cancel")}
              </Button>
              <Button onClick={handleWsl2ConnectSubmit} disabled={connectSubmitting}>
                {t("installChat.submit")}
              </Button>
            </DialogFooter>
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

        {runErrorPanel}

        {/* Footer with Done / Connect button */}
        {(mode === "connect_ssh" || mode === "connect_docker" || mode === "connect_wsl2") && connectSubmitting && (
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
            <Button
              variant="outline"
              onClick={() => {
                setMode("idle");
                clearRunErrorState();
              }}
            >
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
    </>
  );
}
