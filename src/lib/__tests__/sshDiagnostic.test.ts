import { describe, expect, test } from "bun:test";

import {
  buildFriendlySshError,
  extractErrorText,
  parseSshCommandError,
} from "../sshDiagnostic";

const t = (key: string): string => {
  const dict: Record<string, string> = {
    "ssh.errorConnectionRefused": "CONNECTION_REFUSED",
    "ssh.errorSessionStale": "SESSION_STALE",
    "ssh.errorRemoteCommandFailed": "REMOTE_COMMAND_FAILED",
    "ssh.errorUnknown": "UNKNOWN",
    "ssh.repairTitle": "REPAIR",
    "ssh.repairPromptPassphrase": "PROMPT_PASSPHRASE",
    "ssh.repairReconnectSession": "RECONNECT_SESSION",
    "ssh.repairRetryWithBackoff": "RETRY_BACKOFF",
    "config.sshFailed": "GENERIC_FAILURE",
  };
  return dict[key] || key;
};

describe("sshDiagnostic", () => {
  test("parses typed SSH command error object payload", () => {
    const raw = {
      message: "Permission denied (publickey)",
      diagnostic: {
        stage: "authNegotiation",
        intent: "connect",
        status: "failed",
        errorCode: "SSH_AUTH_FAILED",
        summary: "Auth failed",
        evidence: [{ kind: "raw_error", value: "permission denied" }],
        repairPlan: ["suggestAuthorizedKeysCheck"],
        confidence: 0.95,
      },
    };

    const parsed = parseSshCommandError(raw);
    expect(parsed).not.toBeNull();
    expect(parsed?.message).toBe("Permission denied (publickey)");
    expect(parsed?.diagnostic.errorCode).toBe("SSH_AUTH_FAILED");
  });

  test("parses typed SSH command error JSON-string payload", () => {
    const raw = JSON.stringify({
      message: "stale session",
      diagnostic: {
        stage: "sessionOpen",
        intent: "exec",
        status: "failed",
        errorCode: "SSH_SESSION_STALE",
        summary: "stale",
        evidence: [],
        repairPlan: ["reconnectSession"],
        confidence: 0.8,
      },
    });

    const parsed = parseSshCommandError(raw);
    expect(parsed).not.toBeNull();
    expect(parsed?.diagnostic.errorCode).toBe("SSH_SESSION_STALE");
  });

  test("extracts message from typed payload", () => {
    const raw = {
      message: "connect failed",
      diagnostic: {
        stage: "sessionOpen",
        intent: "connect",
        status: "failed",
        summary: "failed",
        evidence: [],
        repairPlan: [],
        confidence: 0.6,
      },
    };

    expect(extractErrorText(raw)).toBe("connect failed");
  });

  test("builds friendly message from errorCode and repair plan", () => {
    const raw = {
      message: "stale ssh channel",
      diagnostic: {
        stage: "sessionOpen",
        intent: "exec",
        status: "failed",
        errorCode: "SSH_SESSION_STALE",
        summary: "session stale",
        evidence: [],
        repairPlan: ["reconnectSession", "retryWithBackoff"],
        confidence: 0.9,
      },
    };

    const message = buildFriendlySshError(raw, t);
    expect(message).toContain("SESSION_STALE");
    expect(message).toContain("(stale ssh channel)");
    expect(message).toContain("REPAIR: RECONNECT_SESSION; RETRY_BACKOFF");
  });

  test("falls back to regex mapping when payload is plain string", () => {
    const message = buildFriendlySshError("Connection refused by host", t);
    expect(message).toContain("CONNECTION_REFUSED");
  });

  test("falls back to generic key when no mapping is available", () => {
    const message = buildFriendlySshError("totally unexpected failure", t);
    expect(message).toBe("GENERIC_FAILURE");
  });
});

import { repairActionToLabel } from "../sshDiagnostic";

describe("repairActionToLabel", () => {
  const t = (key: string): string => key.replace("ssh.repair", "").toUpperCase();

  test("promptPassphrase", () => {
    expect(repairActionToLabel("promptPassphrase", t)).toBe("PROMPTPASSPHRASE");
  });
  test("retryWithBackoff", () => {
    expect(repairActionToLabel("retryWithBackoff", t)).toBe("RETRYWITHBACKOFF");
  });
  test("switchAuthMethodToSshConfig", () => {
    expect(repairActionToLabel("switchAuthMethodToSshConfig", t)).toBe(
      "SWITCHAUTHMETHODTOSSHCONFIG",
    );
  });
  test("suggestKnownHostsBootstrap", () => {
    expect(repairActionToLabel("suggestKnownHostsBootstrap", t)).toBe(
      "SUGGESTKNOWNHOSTSBOOTSTRAP",
    );
  });
  test("suggestAuthorizedKeysCheck", () => {
    expect(repairActionToLabel("suggestAuthorizedKeysCheck", t)).toBe(
      "SUGGESTAUTHORIZEDKEYSCHECK",
    );
  });
  test("suggestPortHostValidation", () => {
    expect(repairActionToLabel("suggestPortHostValidation", t)).toBe(
      "SUGGESTPORTHOSTVALIDATION",
    );
  });
  test("reconnectSession", () => {
    expect(repairActionToLabel("reconnectSession", t)).toBe("RECONNECTSESSION");
  });
  test("unknown action falls back to raw value", () => {
    // @ts-expect-error intentional unknown action
    expect(repairActionToLabel("unknownAction", t)).toBe("unknownAction");
  });
});
