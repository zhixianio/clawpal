import { describe, expect, test } from "bun:test";

import {
  SSH_PASSPHRASE_RETRY_HINT,
  SSH_PASSPHRASE_REJECT_HINT,
  SSH_NO_KEY_HINT,
  SSH_PUBLIC_KEY_PERMISSION_HINT,
  buildSshPassphraseCancelMessage,
  buildSshPassphraseConnectErrorMessage,
} from "../sshConnectErrors";

const t = (key: string, opts: Record<string, string | number | boolean> = {}) => {
  const text = {
    "ssh.passphraseValidationFailed": "PASS_FAIL_{{host}}",
    "ssh.missingKeyFile": "MISSING_KEY_{{host}}",
    "ssh.publicKeyRejected": "PUBLIC_KEY_REJECTED_{{host}}",
    "ssh.passphraseCancelled": "CANCEL_{{host}}",
  }[key] || key;
  return text.replace("{{host}}", String(opts.host ?? ""));
};

describe("sshConnectErrors", () => {
  test("classifies retry hint", () => {
    expect(SSH_PASSPHRASE_RETRY_HINT.test("The key is encrypted.")).toBe(true);
  });

  test("classifies public-key auth failure from russh connect output", () => {
    expect(
      SSH_PASSPHRASE_RETRY_HINT.test(
        "public key authentication failed for root@5.78.141.96:22 after trying /Users/user/.ssh/hetzner: encrypted or passphrase mismatch (key exchange failed)",
      ),
    ).toBe(true);
  });

  test("classifies password auth failure hint", () => {
    expect(SSH_PASSPHRASE_RETRY_HINT.test("password is empty")).toBe(true);
  });

  test("classifies reject hint", () => {
    expect(SSH_PASSPHRASE_REJECT_HINT.test("bad decrypt")).toBe(true);
  });

  test("classifies missing key", () => {
    expect(SSH_NO_KEY_HINT.test("Could not open /Users/foo/.ssh/hetzner")).toBe(true);
  });

  test("classifies public key rejected", () => {
    expect(SSH_PUBLIC_KEY_PERMISSION_HINT.test("public key authentication failed")).toBe(true);
  });

  test("maps passphrase retry error to localized message", () => {
    const msg = buildSshPassphraseConnectErrorMessage("bad decrypt", "hetzner", t);
    expect(msg).toBe("PASS_FAIL_hetzner");
  });

  test("maps missing key error to localized message", () => {
    const msg = buildSshPassphraseConnectErrorMessage("Could not open key file", "hetzner", t);
    expect(msg).toBe("MISSING_KEY_hetzner");
  });

  test("maps permission-denied error to localized message", () => {
    const msg = buildSshPassphraseConnectErrorMessage("public key authentication failed", "hetzner", t);
    expect(msg).toBe("PUBLIC_KEY_REJECTED_hetzner");
  });

  test("maps unknown errors to null", () => {
    expect(buildSshPassphraseConnectErrorMessage("ssh connect timeout after 10s", "hetzner", t)).toBeNull();
  });

  test("maps cancel message to localized message", () => {
    expect(buildSshPassphraseCancelMessage("hetzner", t)).toBe("CANCEL_hetzner");
  });
});
