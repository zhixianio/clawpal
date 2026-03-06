import { describe, expect, test } from "bun:test";

import {
  formatSshConnectionLatency,
  shouldAutoProbeSshConnectionProfile,
  shouldDeferInteractiveSshAutoProbe,
} from "../sshConnectionProfile";

describe("sshConnectionProfile helpers", () => {
  test("formats slow SSH latency in seconds", () => {
    expect(formatSshConnectionLatency(2420)).toBe("2.42 s");
  });

  test("auto-probes only untouched hosts", () => {
    expect(shouldAutoProbeSshConnectionProfile({
      checked: false,
      checking: false,
      hasProfile: false,
      deferredInteractive: false,
    })).toBe(true);

    expect(shouldAutoProbeSshConnectionProfile({
      checked: true,
      checking: false,
      hasProfile: false,
      deferredInteractive: false,
    })).toBe(false);

    expect(shouldAutoProbeSshConnectionProfile({
      checked: false,
      checking: true,
      hasProfile: false,
      deferredInteractive: false,
    })).toBe(false);

    expect(shouldAutoProbeSshConnectionProfile({
      checked: false,
      checking: false,
      hasProfile: false,
      deferredInteractive: true,
    })).toBe(false);
  });

  test("defers auto-probe when SSH needs interactive passphrase input", () => {
    expect(shouldDeferInteractiveSshAutoProbe("The key is encrypted")).toBe(true);
    expect(shouldDeferInteractiveSshAutoProbe("ssh connect timeout after 10s")).toBe(false);
  });
});
