import type { DoctorChatMessage, DoctorInvoke, DiagnosisReportItem } from "./types";

export function extractApprovalPattern(invoke: DoctorInvoke): string {
  const path = (invoke.args?.path as string) ?? "";
  const prefix = path.includes("/") ? path.substring(0, path.lastIndexOf("/") + 1) : path;
  return `${invoke.command}:${prefix}`;
}

export function normalizeInvokeArgs(invoke: DoctorInvoke): string {
  const raw = (invoke.args?.args as string) ?? "";
  return raw.trim().replace(/\s+/g, " ").toLowerCase();
}

export function hasAnyPrefix(value: string, prefixes: string[]): boolean {
  return prefixes.some((prefix) => value === prefix || value.startsWith(`${prefix} `));
}

export function buildDoctorCacheKey(context: {
  instanceScope: string;
  agentId: string;
  domain: "doctor" | "install";
  engine: "openclaw" | "zeroclaw";
}): string {
  const DOCTOR_CHAT_CACHE_PREFIX = "clawpal-doctor-chat-v1";
  const scope = encodeURIComponent(context.instanceScope);
  const agent = encodeURIComponent(context.agentId);
  return `${DOCTOR_CHAT_CACHE_PREFIX}-${context.domain}-${context.engine}-${scope}-${agent}`;
}

export function sanitizeDoctorCacheMessages(rawMessages: unknown): DoctorChatMessage[] {
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

export function extractOpenclawText(result: Record<string, unknown>): string {
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

export function extractOpenclawSessionId(result: Record<string, unknown>): string | undefined {
  const meta = result.meta as Record<string, unknown> | undefined;
  if (!meta || typeof meta !== "object") return;
  const agentMeta = meta.agentMeta as Record<string, unknown> | undefined;
  if (!agentMeta || typeof agentMeta !== "object") return;
  const rawSessionId = agentMeta.sessionId;
  return typeof rawSessionId === "string" ? rawSessionId.trim() || undefined : undefined;
}

export function isDoctorAutoSafeInvoke(invoke: DoctorInvoke, domain: "doctor" | "install"): boolean {
  if (domain !== "doctor") return false;
  const args = normalizeInvokeArgs(invoke);
  if (invoke.command === "clawpal") {
    return hasAnyPrefix(args, [
      "doctor probe-openclaw",
      "doctor fix-openclaw-path",
      "doctor file read",
      "doctor file write",
      "doctor config-read",
      "doctor config-upsert",
      "doctor config-delete",
      "doctor sessions-read",
      "doctor sessions-upsert",
      "doctor sessions-delete",
    ]);
  }
  if (invoke.command === "openclaw") {
    return hasAnyPrefix(args, [
      "--version",
      "doctor",
      "gateway status",
      "health",
      "config get",
      "config set",
      "config delete",
      "config unset",
      "agents list",
      "memory status",
      "security audit",
    ]);
  }
  return false;
}
