import type { ModelProfile, ResolvedApiKey } from "./types";

export type ProfileCredentialKind = "oauth" | "env_ref" | "manual" | "unset";

export type ProfileCredentialView = {
  kind: ProfileCredentialKind;
  authRef: string;
  status: string;
  resolved: boolean;
};

export type ProfilePushEligibility = {
  allowed: boolean;
  reason: "oauth" | "missing_static_credential" | null;
};

const STATUS_LOADING = "...";
const STATUS_NOT_SET = "not set";

const OAUTH_PROVIDER_ALIASES = new Set([
  "openai-codex",
  "openai_codex",
  "github-copilot",
  "copilot",
]);

function normalizeProvider(provider: string): string {
  return provider.trim().toLowerCase();
}

function providerUsesOauth(provider: string): boolean {
  return OAUTH_PROVIDER_ALIASES.has(normalizeProvider(provider));
}

function providerSupportsOptionalApiKey(provider: string): boolean {
  return [
    "ollama",
    "lmstudio",
    "lm-studio",
    "localai",
    "vllm",
    "llamacpp",
    "llama.cpp",
  ].includes(normalizeProvider(provider));
}

function isOauthAuthRef(provider: string, authRef: string): boolean {
  if (!providerUsesOauth(provider)) return false;
  const lower = authRef.trim().toLowerCase();
  return lower.startsWith("openai-codex:") || lower.startsWith("openai:");
}

function normalizeKind(kind: ResolvedApiKey["credentialKind"]): ProfileCredentialKind | null {
  if (kind === "oauth" || kind === "env_ref" || kind === "manual" || kind === "unset") {
    return kind;
  }
  return null;
}

function inferKind(profile: ModelProfile, authRef: string, status: string): ProfileCredentialKind {
  if (authRef) {
    return isOauthAuthRef(profile.provider, authRef) ? "oauth" : "env_ref";
  }
  if (status !== STATUS_LOADING && status.toLowerCase() !== STATUS_NOT_SET) {
    return "manual";
  }
  if (profile.apiKey?.trim()) {
    return "manual";
  }
  return "unset";
}

export function resolveProfileCredentialView(
  profile: ModelProfile,
  entry?: ResolvedApiKey,
): ProfileCredentialView {
  const status = (entry?.maskedKey || STATUS_LOADING).trim() || STATUS_LOADING;
  const authRef = (entry?.authRef ?? profile.authRef ?? "").trim();
  const kind = normalizeKind(entry?.credentialKind) || inferKind(profile, authRef, status);
  const resolved = typeof entry?.resolved === "boolean"
    ? entry.resolved
    : (status !== STATUS_LOADING && status.toLowerCase() !== STATUS_NOT_SET);

  return {
    kind,
    authRef,
    status,
    resolved,
  };
}

export function getProfilePushEligibility(
  profile: ModelProfile,
  entry?: ResolvedApiKey,
): ProfilePushEligibility {
  const credential = resolveProfileCredentialView(profile, entry);
  if (credential.kind === "oauth") {
    return {
      allowed: false,
      reason: "oauth",
    };
  }
  if (credential.resolved || providerSupportsOptionalApiKey(profile.provider)) {
    return {
      allowed: true,
      reason: null,
    };
  }
  return {
    allowed: false,
    reason: "missing_static_credential",
  };
}
