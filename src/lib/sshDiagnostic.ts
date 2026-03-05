import type { SshCommandError, SshErrorCode, SshRepairAction } from "./types";

type TranslateFn = (
  key: string,
  options?: Record<string, string | number | boolean>,
) => string;

const SSH_FALLBACK_ERROR_MAP: Array<[RegExp, string]> = [
  [/connection refused/i, "ssh.errorConnectionRefused"],
  [/no such file/i, "ssh.errorNoSuchFile"],
  [/name or service not known|nodename nor servname provided|temporary failure in name resolution|no address associated with hostname|getaddrinfo|failed to lookup address information|unknown host|hostname was not found/i, "ssh.errorHostUnreachable"],
  [/passphrase|sign_and_send_pubkey|agent refused operation|can't open \/dev\/tty|authentication agent/i, "ssh.errorPassphrase"],
  [/permission denied/i, "ssh.errorPermissionDenied"],
  [/host key verification failed/i, "ssh.errorHostKey"],
  [/timed?\s*out/i, "ssh.errorTimeout"],
];

function isRecord(value: unknown): value is Record<string, unknown> {
  return !!value && typeof value === "object";
}

export function parseSshCommandError(raw: unknown): SshCommandError | null {
  if (isRecord(raw)) {
    const message = raw.message;
    const diagnostic = raw.diagnostic;
    if (typeof message === "string" && isRecord(diagnostic)) {
      return raw as unknown as SshCommandError;
    }
  }
  if (typeof raw === "string") {
    try {
      const parsed = JSON.parse(raw) as unknown;
      if (isRecord(parsed)) {
        const message = parsed.message;
        const diagnostic = parsed.diagnostic;
        if (typeof message === "string" && isRecord(diagnostic)) {
          return parsed as unknown as SshCommandError;
        }
      }
    } catch {
      return null;
    }
  }
  return null;
}

export function extractErrorText(raw: unknown): string {
  const sshError = parseSshCommandError(raw);
  if (sshError) return sshError.message;
  if (isRecord(raw) && typeof raw.message === "string") return raw.message;
  return String(raw);
}

function errorCodeToI18nKey(code?: SshErrorCode | null): string {
  switch (code) {
    case "SSH_CONNECTION_REFUSED":
      return "ssh.errorConnectionRefused";
    case "SSH_KEYFILE_MISSING":
      return "ssh.errorNoSuchFile";
    case "SSH_HOST_UNREACHABLE":
      return "ssh.errorHostUnreachable";
    case "SSH_PASSPHRASE_REQUIRED":
      return "ssh.errorPassphrase";
    case "SSH_AUTH_FAILED":
    case "SSH_SFTP_PERMISSION_DENIED":
      return "ssh.errorPermissionDenied";
    case "SSH_HOST_KEY_FAILED":
      return "ssh.errorHostKey";
    case "SSH_TIMEOUT":
      return "ssh.errorTimeout";
    case "SSH_SESSION_STALE":
      return "ssh.errorSessionStale";
    case "SSH_REMOTE_COMMAND_FAILED":
      return "ssh.errorRemoteCommandFailed";
    default:
      return "ssh.errorUnknown";
  }
}

function repairActionToLabel(action: SshRepairAction, t: TranslateFn): string {
  switch (action) {
    case "promptPassphrase":
      return t("ssh.repairPromptPassphrase");
    case "retryWithBackoff":
      return t("ssh.repairRetryWithBackoff");
    case "switchAuthMethodToSshConfig":
      return t("ssh.repairSwitchAuthMethodToSshConfig");
    case "suggestKnownHostsBootstrap":
      return t("ssh.repairSuggestKnownHostsBootstrap");
    case "suggestAuthorizedKeysCheck":
      return t("ssh.repairSuggestAuthorizedKeysCheck");
    case "suggestPortHostValidation":
      return t("ssh.repairSuggestPortHostValidation");
    case "reconnectSession":
      return t("ssh.repairReconnectSession");
    default:
      return action;
  }
}

export function buildFriendlySshError(raw: unknown, t: TranslateFn): string {
  const parsed = parseSshCommandError(raw);
  if (parsed) {
    const key = errorCodeToI18nKey(parsed.diagnostic?.errorCode);
    const base = t(key);
    const repairPlan = parsed.diagnostic?.repairPlan || [];
    const repairHints = repairPlan.map((action) => repairActionToLabel(action, t));
    const repairText =
      repairHints.length > 0
        ? `\n${t("ssh.repairTitle")}: ${repairHints.join("; ")}`
        : "";
    return `${base}\n(${parsed.message})${repairText}`;
  }

  const text = extractErrorText(raw);
  for (const [pattern, key] of SSH_FALLBACK_ERROR_MAP) {
    if (pattern.test(text)) {
      return `${t(key)}\n(${text})`;
    }
  }
  return t("config.sshFailed", { error: text });
}
