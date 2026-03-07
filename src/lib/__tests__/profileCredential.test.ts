import { describe, expect, test } from "bun:test";

import { getProfilePushEligibility, resolveProfileCredentialView } from "../profile-credential";
import type { ModelProfile, ResolvedApiKey } from "../types";

function makeProfile(partial: Partial<ModelProfile>): ModelProfile {
  return {
    id: "p1",
    name: "openai/gpt-4o",
    provider: "openai",
    model: "gpt-4o",
    authRef: "",
    enabled: true,
    ...partial,
  };
}

function makeResolved(partial: Partial<ResolvedApiKey>): ResolvedApiKey {
  return {
    profileId: "p1",
    maskedKey: "...",
    ...partial,
  };
}

describe("resolveProfileCredentialView", () => {
  test("uses backend oauth kind and auth ref", () => {
    const profile = makeProfile({ provider: "openai-codex", authRef: "openai-codex:default" });
    const resolved = makeResolved({
      credentialKind: "oauth",
      authRef: "openai-codex:default",
      maskedKey: "sk-a...z9x8",
      resolved: true,
    });

    const view = resolveProfileCredentialView(profile, resolved);
    expect(view.kind).toBe("oauth");
    expect(view.authRef).toBe("openai-codex:default");
    expect(view.status).toBe("sk-a...z9x8");
    expect(view.resolved).toBe(true);
  });

  test("uses backend env_ref unresolved state", () => {
    const profile = makeProfile({ authRef: "OPENAI_API_KEY" });
    const resolved = makeResolved({
      credentialKind: "env_ref",
      authRef: "OPENAI_API_KEY",
      maskedKey: "not set",
      resolved: false,
    });

    const view = resolveProfileCredentialView(profile, resolved);
    expect(view.kind).toBe("env_ref");
    expect(view.resolved).toBe(false);
    expect(view.status).toBe("not set");
  });

  test("falls back to authRef-based inference when backend kind is missing", () => {
    const profile = makeProfile({ provider: "openai-codex", authRef: "openai-codex:work" });
    const resolved = makeResolved({ maskedKey: "not set", authRef: "openai-codex:work" });

    const view = resolveProfileCredentialView(profile, resolved);
    expect(view.kind).toBe("oauth");
  });

  test("falls back to manual when masked secret is present and authRef is empty", () => {
    const profile = makeProfile({ authRef: "" });
    const resolved = makeResolved({ maskedKey: "sk-a...x1y2" });

    const view = resolveProfileCredentialView(profile, resolved);
    expect(view.kind).toBe("manual");
    expect(view.resolved).toBe(true);
  });

  test("falls back to unset when no signal is available", () => {
    const profile = makeProfile({ authRef: "" });
    const resolved = makeResolved({ maskedKey: "not set" });

    const view = resolveProfileCredentialView(profile, resolved);
    expect(view.kind).toBe("unset");
    expect(view.resolved).toBe(false);
  });
});

describe("getProfilePushEligibility", () => {
  test("blocks oauth-backed profiles from automatic push", () => {
    const profile = makeProfile({
      provider: "openai-codex",
      model: "gpt-5.3-codex",
      authRef: "openai-codex:default",
    });
    const resolved = makeResolved({
      credentialKind: "oauth",
      authRef: "openai-codex:default",
      maskedKey: "oauth-ready",
      resolved: true,
    });

    expect(getProfilePushEligibility(profile, resolved)).toEqual({
      allowed: false,
      reason: "oauth",
    });
  });

  test("allows static resolved credentials to be pushed", () => {
    const profile = makeProfile({ authRef: "OPENAI_API_KEY" });
    const resolved = makeResolved({
      credentialKind: "env_ref",
      authRef: "OPENAI_API_KEY",
      maskedKey: "sk-a...z9x8",
      resolved: true,
    });

    expect(getProfilePushEligibility(profile, resolved)).toEqual({
      allowed: true,
      reason: null,
    });
  });

  test("allows providers with optional api keys even when unresolved", () => {
    const profile = makeProfile({
      provider: "ollama",
      model: "qwen3:latest",
      authRef: "",
    });
    const resolved = makeResolved({
      credentialKind: "unset",
      maskedKey: "not set",
      resolved: false,
    });

    expect(getProfilePushEligibility(profile, resolved)).toEqual({
      allowed: true,
      reason: null,
    });
  });

  test("blocks unresolved static credentials", () => {
    const profile = makeProfile({ authRef: "OPENAI_API_KEY" });
    const resolved = makeResolved({
      credentialKind: "env_ref",
      authRef: "OPENAI_API_KEY",
      maskedKey: "not set",
      resolved: false,
    });

    expect(getProfilePushEligibility(profile, resolved)).toEqual({
      allowed: false,
      reason: "missing_static_credential",
    });
  });
});
