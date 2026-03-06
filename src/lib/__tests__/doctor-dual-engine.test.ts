import { describe, expect, test } from "bun:test";

import {
  hasZeroclawSession,
  resolveDoctorChatConnected,
  resolveEngineConnectionState,
  shouldDisableOpenclawStart,
  shouldDisableZeroclawStart,
  shouldShowDoctorDisconnectUi,
  shouldSurfaceDisconnectError,
} from "../doctor-dual-engine";

describe("resolveEngineConnectionState", () => {
  test("returns checking while the target engine is diagnosing", () => {
    expect(resolveEngineConnectionState({ diagnosing: true, connected: false })).toBe("checking");
    expect(resolveEngineConnectionState({ diagnosing: true, connected: true })).toBe("checking");
  });

  test("returns connected/disconnected when not diagnosing", () => {
    expect(resolveEngineConnectionState({ diagnosing: false, connected: true })).toBe("connected");
    expect(resolveEngineConnectionState({ diagnosing: false, connected: false })).toBe("disconnected");
  });
});

describe("dual engine start-button isolation", () => {
  test("openclaw loading does not disable zeroclaw start", () => {
    const openclawDisabled = shouldDisableOpenclawStart({ diagnosing: true });
    const zeroclawDisabled = shouldDisableZeroclawStart({
      diagnosing: false,
      doctorUiLoaded: true,
    });

    expect(openclawDisabled).toBe(true);
    expect(zeroclawDisabled).toBe(false);
  });

  test("zeroclaw loading does not disable openclaw start", () => {
    const zeroclawDisabled = shouldDisableZeroclawStart({
      diagnosing: true,
      doctorUiLoaded: true,
    });
    const openclawDisabled = shouldDisableOpenclawStart({ diagnosing: false });

    expect(zeroclawDisabled).toBe(true);
    expect(openclawDisabled).toBe(false);
  });
});

describe("hasZeroclawSession", () => {
  test("depends only on zeroclaw runtime snapshot", () => {
    expect(hasZeroclawSession({ connected: false, messageCount: 0 })).toBe(false);
    expect(hasZeroclawSession({ connected: true, messageCount: 0 })).toBe(true);
    expect(hasZeroclawSession({ connected: false, messageCount: 1 })).toBe(true);
  });
});

describe("embedded zeroclaw UI", () => {
  test("never shows disconnect ui for zeroclaw sessions", () => {
    expect(
      shouldShowDoctorDisconnectUi({
        engine: "zeroclaw",
        connected: false,
        messageCount: 3,
      }),
    ).toBe(false);
  });

  test("keeps zeroclaw chat interactive even when transport drops", () => {
    expect(
      resolveDoctorChatConnected({
        engine: "zeroclaw",
        connected: false,
      }),
    ).toBe(true);
  });

  test("does not surface transport disconnect errors for zeroclaw", () => {
    expect(shouldSurfaceDisconnectError({ engine: "zeroclaw" })).toBe(false);
    expect(shouldSurfaceDisconnectError({ engine: "openclaw" })).toBe(true);
  });
});
