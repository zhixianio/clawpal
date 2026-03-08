import type {
  AgentOverview,
  ChannelsConfigSnapshot,
  ChannelsRuntimeSnapshot,
  CronConfigSnapshot,
  CronRuntimeSnapshot,
  InstanceConfigSnapshot,
  InstanceRuntimeSnapshot,
  InstanceStatus,
  RescueBotManageResult,
  StatusExtra,
} from "@/lib/types";

export function buildInstanceCardSummary(
  configSnapshot: { agents?: { id: string }[] } | null,
  runtimeSnapshot: { status: { healthy: boolean; activeAgents: number } } | null,
): { healthy: boolean | null; agentCount: number } {
  if (runtimeSnapshot) {
    return {
      healthy: runtimeSnapshot.status.healthy,
      agentCount: runtimeSnapshot.status.activeAgents,
    };
  }

  return {
    healthy: null,
    agentCount: configSnapshot?.agents?.length ?? 0,
  };
}

export function shouldStartDeferredUpdateCheck({
  isRemote,
  isConnected,
}: {
  isRemote: boolean;
  isConnected: boolean;
}): boolean {
  if (isRemote && !isConnected) return false;
  return true;
}

export function buildInitialHomeState(
  configSnapshot: InstanceConfigSnapshot | null,
  runtimeSnapshot: InstanceRuntimeSnapshot | null,
  statusExtra: StatusExtra | null,
): {
  status: InstanceStatus | null;
  agents: AgentOverview[] | null;
  statusSettled: boolean;
  version: string | null;
  statusExtra: StatusExtra | null;
} {
  if (runtimeSnapshot) {
    return {
      status: {
        ...runtimeSnapshot.status,
        globalDefaultModel: runtimeSnapshot.globalDefaultModel,
        fallbackModels: runtimeSnapshot.fallbackModels,
      },
      agents: runtimeSnapshot.agents,
      statusSettled: true,
      version: statusExtra?.openclawVersion ?? null,
      statusExtra,
    };
  }

  if (configSnapshot) {
    return {
      status: {
        healthy: false,
        activeAgents: configSnapshot.agents.length,
        globalDefaultModel: configSnapshot.globalDefaultModel,
        fallbackModels: configSnapshot.fallbackModels,
        sshDiagnostic: null,
      },
      agents: configSnapshot.agents,
      statusSettled: false,
      version: statusExtra?.openclawVersion ?? null,
      statusExtra,
    };
  }

  return {
    status: null,
    agents: null,
    statusSettled: false,
    version: statusExtra?.openclawVersion ?? null,
    statusExtra,
  };
}

export function applyConfigSnapshotToHomeState(
  current: {
    status: InstanceStatus | null;
    agents: AgentOverview[] | null;
    statusSettled: boolean;
    version: string | null;
    statusExtra: StatusExtra | null;
  },
  snapshot: {
    globalDefaultModel?: string;
    fallbackModels: string[];
    agents: AgentOverview[];
  },
): {
  status: InstanceStatus | null;
  agents: AgentOverview[] | null;
  statusSettled: boolean;
  version: string | null;
  statusExtra: StatusExtra | null;
} {
  if (current.statusSettled && current.status && current.agents) {
    return current;
  }

  return {
    status: {
      healthy: false,
      activeAgents: snapshot.agents.length,
      globalDefaultModel: snapshot.globalDefaultModel,
      fallbackModels: snapshot.fallbackModels,
      sshDiagnostic: null,
    },
    agents: snapshot.agents,
    statusSettled: false,
    version: current.version,
    statusExtra: current.statusExtra,
  };
}

export function buildInitialChannelsState(
  configSnapshot: ChannelsConfigSnapshot | null,
  runtimeSnapshot: ChannelsRuntimeSnapshot | null,
): {
  channels: ChannelsRuntimeSnapshot["channels"];
  bindings: ChannelsRuntimeSnapshot["bindings"];
  agents: ChannelsRuntimeSnapshot["agents"];
  loaded: boolean;
} {
  if (runtimeSnapshot) {
    return {
      channels: runtimeSnapshot.channels,
      bindings: runtimeSnapshot.bindings,
      agents: runtimeSnapshot.agents,
      loaded: true,
    };
  }

  if (configSnapshot) {
    return {
      channels: configSnapshot.channels,
      bindings: configSnapshot.bindings,
      agents: [],
      loaded: true,
    };
  }

  return {
    channels: [],
    bindings: [],
    agents: [],
    loaded: false,
  };
}

export function buildInitialCronState(
  configSnapshot: CronConfigSnapshot | null,
  runtimeSnapshot: CronRuntimeSnapshot | null,
): {
  jobs: CronRuntimeSnapshot["jobs"];
  watchdog: CronRuntimeSnapshot["watchdog"] | null;
} {
  if (runtimeSnapshot) {
    return {
      jobs: runtimeSnapshot.jobs,
      watchdog: runtimeSnapshot.watchdog,
    };
  }

  return {
    jobs: configSnapshot?.jobs ?? [],
    watchdog: null,
  };
}

export function buildInitialRescueState(
  persistedStatus: RescueBotManageResult | null,
): {
  runtimeState: RescueBotManageResult["runtimeState"];
  configured: boolean;
  active: boolean;
  profile: string;
  port: number | null;
} | null {
  if (!persistedStatus) {
    return null;
  }
  return {
    runtimeState: persistedStatus.runtimeState,
    configured: persistedStatus.configured,
    active: persistedStatus.active,
    profile: persistedStatus.profile,
    port: persistedStatus.configured ? persistedStatus.rescuePort : null,
  };
}

export function shouldShowAvailableUpdateBadge({
  checkingUpdate,
  updateInfo,
  version,
}: {
  checkingUpdate: boolean;
  updateInfo: { available: boolean; latest?: string } | null;
  version: string | null;
}): boolean {
  return Boolean(
    !checkingUpdate
      && updateInfo?.available
      && updateInfo.latest
      && updateInfo.latest !== version,
  );
}

export function shouldShowLatestReleaseBadge({
  checkingUpdate,
  updateInfo,
  version,
}: {
  checkingUpdate: boolean;
  updateInfo: { available: boolean; latest?: string } | null;
  version: string | null;
}): boolean {
  if (checkingUpdate || !updateInfo?.latest) return false;
  return !shouldShowAvailableUpdateBadge({
    checkingUpdate,
    updateInfo,
    version,
  });
}
