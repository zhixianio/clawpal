import { describe, expect, test } from "bun:test";

import {
  applyConfigSnapshotToHomeState,
  buildInstanceCardSummary,
  buildInitialChannelsState,
  buildInitialCronState,
  buildInitialHomeState,
  buildInitialRescueState,
  shouldShowAvailableUpdateBadge,
  shouldStartDeferredUpdateCheck,
  shouldShowLatestReleaseBadge,
} from "../overview-loading";

describe("overview-loading helpers", () => {
  test("uses config agent count before runtime snapshot arrives", () => {
    expect(
      buildInstanceCardSummary(
        {
          agents: [{ id: "main" }, { id: "worker-1" }],
        },
        null,
      ),
    ).toEqual({
      healthy: null,
      agentCount: 2,
    });
  });

  test("runtime snapshot overwrites health and agent count", () => {
    expect(
      buildInstanceCardSummary(
        {
          agents: [{ id: "main" }, { id: "worker-1" }],
        },
        {
          status: { healthy: true, activeAgents: 4 },
        },
      ),
    ).toEqual({
      healthy: true,
      agentCount: 4,
    });
  });

  test("starts background update checks as soon as the remote instance is connected", () => {
    expect(
      shouldStartDeferredUpdateCheck({
        isRemote: true,
        isConnected: true,
      }),
    ).toBe(true);

    expect(
      shouldStartDeferredUpdateCheck({
        isRemote: true,
        isConnected: false,
      }),
    ).toBe(false);
  });

  test("builds initial home state from persisted runtime without fallback loading state", () => {
    const initial = buildInitialHomeState(
      {
        globalDefaultModel: "config/model",
        fallbackModels: ["config/fallback"],
        agents: [{ id: "config-agent", model: null, channels: [], online: false }],
      },
      {
        status: { healthy: true, activeAgents: 3 },
        agents: [{ id: "runtime-agent", model: null, channels: [], online: true }],
        globalDefaultModel: "runtime/model",
        fallbackModels: ["runtime/fallback"],
      },
      { openclawVersion: "2026.3.2" },
    );

    expect(initial.status).toEqual({
      healthy: true,
      activeAgents: 3,
      globalDefaultModel: "runtime/model",
      fallbackModels: ["runtime/fallback"],
    });
    expect(initial.agents).toEqual([{ id: "runtime-agent", model: null, channels: [], online: true }]);
    expect(initial.statusSettled).toBe(true);
    expect(initial.version).toBe("2026.3.2");
  });

  test("keeps visible runtime state when a config snapshot arrives after runtime cache", () => {
    const next = applyConfigSnapshotToHomeState(
      {
        status: {
          healthy: true,
          activeAgents: 3,
          globalDefaultModel: "runtime/model",
          fallbackModels: ["runtime/fallback"],
        },
        agents: [{ id: "runtime-agent", model: "gpt-5.3-codex", channels: [], online: true }],
        statusSettled: true,
        version: "2026.3.2",
        statusExtra: { openclawVersion: "2026.3.2" },
      },
      {
        globalDefaultModel: "config/model",
        fallbackModels: ["config/fallback"],
        agents: [{ id: "config-agent", model: null, channels: [], online: false }],
      },
    );

    expect(next.status).toEqual({
      healthy: true,
      activeAgents: 3,
      globalDefaultModel: "runtime/model",
      fallbackModels: ["runtime/fallback"],
    });
    expect(next.agents).toEqual([
      { id: "runtime-agent", model: "gpt-5.3-codex", channels: [], online: true },
    ]);
    expect(next.statusSettled).toBe(true);
  });

  test("builds initial channels state from persisted config when runtime is not cached", () => {
    const initial = buildInitialChannelsState(
      {
        channels: [{ path: "channels.discord", channelType: "platform", mode: null, allowlist: [], model: null, hasModelField: false, displayName: null, nameStatus: null }],
        bindings: [{ agentId: "main", match: { channel: "discord", peer: { id: "123", kind: "channel" } } }],
      },
      null,
    );

    expect(initial.loaded).toBe(true);
    expect(initial.channels).toHaveLength(1);
    expect(initial.bindings).toHaveLength(1);
    expect(initial.agents).toEqual([]);
  });

  test("builds initial cron state from persisted runtime before live refresh", () => {
    const initial = buildInitialCronState(
      { jobs: [{ id: "daily", enabled: true }] as never[] },
      {
        jobs: [{ id: "daily", enabled: true }, { id: "hourly", enabled: true }] as never[],
        watchdog: { alive: true, deployed: true, pid: 1, startedAt: "", lastCheckAt: "", gatewayHealthy: true, jobs: {} },
      },
    );

    expect(initial.jobs).toHaveLength(2);
    expect(initial.watchdog?.alive).toBe(true);
  });

  test("builds initial rescue state from persisted helper status", () => {
    const initial = buildInitialRescueState({
      action: "status",
      profile: "rescue",
      mainPort: 18789,
      rescuePort: 19789,
      minRecommendedPort: 19789,
      configured: true,
      active: true,
      runtimeState: "active",
      wasAlreadyConfigured: true,
      commands: [],
    });

    expect(initial).toEqual({
      runtimeState: "active",
      configured: true,
      active: true,
      profile: "rescue",
      port: 19789,
    });
  });

  test("hides the separate latest release badge when an upgrade is already available", () => {
    expect(
      shouldShowAvailableUpdateBadge({
        checkingUpdate: false,
        updateInfo: { available: true, latest: "2026.3.7" },
        version: "2026.2.26",
      }),
    ).toBe(true);

    expect(
      shouldShowLatestReleaseBadge({
        checkingUpdate: false,
        updateInfo: { available: true, latest: "2026.3.7" },
        version: "2026.2.26",
      }),
    ).toBe(false);
  });
});
