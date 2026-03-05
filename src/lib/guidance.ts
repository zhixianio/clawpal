import { api } from "./api";
import i18n from "../i18n";
import { extractErrorText } from "./sshDiagnostic";

// ── Throttle / filter logic (shared by withGuidance and use-api dispatch) ──

const AGENT_GUIDANCE_THROTTLE = new Map<string, number>();
const AGENT_GUIDANCE_THROTTLE_TTL_MS = 90_000;

function logDevGuidanceError(context: string, detail: unknown): void {
  if (!import.meta.env.DEV) return;
  console.error(`[dev guidance] ${context}`, detail);
}

function normalizeErrorSignature(raw: string): string {
  return raw
    .toLowerCase()
    .replace(/\s+/g, " ")
    .replace(/\d{2,}/g, "#")
    .trim()
    .slice(0, 220);
}

export function isSshCooldownProtectionError(errorText: string): boolean {
  const text = errorText.toLowerCase();
  return (
    text.includes("ssh_cooldown:")
    || text.includes("cooling down after repeated timeouts")
    || text.includes("are cooling down")
    || text.includes("retry in")
    || text.includes("冷却期")
    || text.includes("多次超时")
  );
}

export function isTransientSshChannelError(errorText: string): boolean {
  const text = errorText.toLowerCase();
  return (
    text.includes("ssh open channel failed")
    || text.includes("connection reset")
    || text.includes("broken pipe")
    || text.includes("connection closed")
    || text.includes("failed to open channel")
  );
}

export function isAlreadyExplainedGuidanceError(errorText: string): boolean {
  const text = errorText.toLowerCase();
  return (
    text.includes("下一步建议")
    || text.includes("建议先做诊断再继续")
    || text.includes("建议先执行诊断")
    || text.includes("建议先进行诊断")
    || text.includes("建议先打开")
    || text.includes("建议前往")
    || text.includes("建议打开")
    || text.includes("建议执行诊断命令")
    || text.includes("本机未安装 openclaw")
    || text.includes("recommend")
    || text.includes("next step")
    || text.includes("open doctor")
    || text.includes("run doctor")
    || text.includes("start doctor")
    || text.includes("ssh连接失败，无法打开通道")
  );
}

export function isRegistryCorruptError(errorText: string): boolean {
  const text = errorText.toLowerCase();
  return (
    (text.includes("registry") || text.includes("instances.json"))
    && (text.includes("parse") || text.includes("corrupt") || text.includes("invalid json"))
  );
}

export function isContainerOrphanedError(errorText: string): boolean {
  const text = errorText.toLowerCase();
  return (
    text.includes("no such container")
    || (text.includes("container") && text.includes("not found") && !text.includes("openclaw"))
  );
}

export function shouldEmitAgentGuidance(instanceId: string, operation: string, errorText: string): boolean {
  if (
    isSshCooldownProtectionError(errorText)
    || isTransientSshChannelError(errorText)
    || isAlreadyExplainedGuidanceError(errorText)
  ) {
    return false;
  }
  const signature = `${instanceId}::${operation}::${normalizeErrorSignature(errorText)}`;
  const now = Date.now();
  const lastAt = AGENT_GUIDANCE_THROTTLE.get(signature) || 0;
  if (now - lastAt < AGENT_GUIDANCE_THROTTLE_TTL_MS) {
    return false;
  }
  AGENT_GUIDANCE_THROTTLE.set(signature, now);
  if (AGENT_GUIDANCE_THROTTLE.size > 256) {
    for (const [key, ts] of AGENT_GUIDANCE_THROTTLE.entries()) {
      if (now - ts > AGENT_GUIDANCE_THROTTLE_TTL_MS * 3) {
        AGENT_GUIDANCE_THROTTLE.delete(key);
      }
    }
  }
  return true;
}

type ExplainGuidanceInput = {
  method: string;
  instanceId: string;
  transport: "local" | "docker_local" | "remote_ssh";
  rawError: unknown;
  emitEvent?: boolean;
};

export async function explainAndBuildGuidanceError({
  method,
  instanceId,
  transport,
  rawError,
  emitEvent = true,
}: ExplainGuidanceInput): Promise<Error> {
  const original = extractErrorText(rawError);
  if (
    isSshCooldownProtectionError(original)
    || isTransientSshChannelError(original)
    || isAlreadyExplainedGuidanceError(original)
  ) {
    return new Error(original);
  }

  try {
    const language =
      i18n.language ||
      (typeof navigator !== "undefined" ? navigator.language : "en");
    const explained = await api.explainOperationError(
      instanceId,
      method,
      transport,
      original,
      language,
    );
    const shouldEmit =
      emitEvent
      && typeof window !== "undefined"
      && shouldEmitAgentGuidance(instanceId, method, original);
    if (shouldEmit) {
      window.dispatchEvent(
        new CustomEvent("clawpal:agent-guidance", {
          detail: {
            ...explained,
            operation: method,
            instanceId,
            transport,
            rawError: original,
            createdAt: Date.now(),
          },
        }),
      );
    }
    const wrapped = new Error(explained.message || original);
    if (shouldEmit) {
      (wrapped as any)._guidanceEmitted = true;
    }
    return wrapped;
  } catch {
    logDevGuidanceError("explainAndBuildGuidanceError", {
      method,
      instanceId,
      transport,
      rawError: original,
    });
    return new Error(original);
  }
}

// ── withGuidance wrapper for App-level lifecycle calls ──

/**
 * Wraps an async operation with zeroclaw guidance emission on failure.
 * Use this for App-level lifecycle calls (SSH connect, instance listing, etc.)
 * that bypass the useApi() dispatch() wrapper.
 *
 * Applies the same throttle/filter logic as dispatch():
 * - Skips cooldown, transient, and already-explained errors
 * - 90s throttle per unique error signature
 * - Wraps error so _guidanceEmitted flag works even for string rejects
 */
export async function withGuidance<T>(
  fn: () => Promise<T>,
  method: string,
  instanceId: string,
  transport: "local" | "docker_local" | "remote_ssh",
): Promise<T> {
  try {
    return await fn();
  } catch (error) {
    logDevGuidanceError("withGuidance", {
      method,
      instanceId,
      transport,
      rawError: error,
    });
    throw await explainAndBuildGuidanceError({
      method,
      instanceId,
      transport,
      rawError: error,
      emitEvent: true,
    });
  }
}
