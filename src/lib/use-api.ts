import { useCallback, useMemo } from "react";
import { invoke } from "@tauri-apps/api/core";
import { useInstance } from "./instance-context";
import { api } from "./api";
import {
  explainAndBuildGuidanceError,
} from "./guidance";

/** Returns true if the error already triggered a guidance panel, so toast can be skipped. */
export function hasGuidanceEmitted(error: unknown): boolean {
  return !!(error && typeof error === "object" && (error as any)._guidanceEmitted);
}

type ApiReadCacheEntry = {
  expiresAt: number;
  value: unknown;
  inFlight?: Promise<unknown>;
};

const API_READ_CACHE = new Map<string, ApiReadCacheEntry>();
const API_READ_CACHE_MAX_ENTRIES = 512;
function makeCacheKey(instanceCacheKey: string, method: string, args: unknown[]): string {
  let serializedArgs = "";
  try {
    serializedArgs = JSON.stringify(args);
  } catch {
    serializedArgs = String(args.length);
  }
  return `${instanceCacheKey}:${method}:${serializedArgs}`;
}

function trimReadCacheIfNeeded() {
  if (API_READ_CACHE.size <= API_READ_CACHE_MAX_ENTRIES) return;
  const deleteCount = API_READ_CACHE.size - API_READ_CACHE_MAX_ENTRIES;
  const keys = API_READ_CACHE.keys();
  for (let i = 0; i < deleteCount; i += 1) {
    const next = keys.next();
    if (next.done) break;
    API_READ_CACHE.delete(next.value);
  }
}

function invalidateReadCacheForInstance(instanceCacheKey: string, methods?: string[]) {
  const methodSet = methods ? new Set(methods) : null;
  for (const key of API_READ_CACHE.keys()) {
    if (!key.startsWith(`${instanceCacheKey}:`)) continue;
    if (!methodSet) {
      API_READ_CACHE.delete(key);
      continue;
    }
    const method = key.slice(instanceCacheKey.length + 1).split(":", 1)[0];
    if (methodSet.has(method)) {
      API_READ_CACHE.delete(key);
    }
  }
}

export function invalidateGlobalReadCache(methods?: string[]) {
  invalidateReadCacheForInstance("__global__", methods);
}

function callWithReadCache<TResult>(
  instanceCacheKey: string,
  method: string,
  args: unknown[],
  ttlMs: number,
  loader: () => Promise<TResult>,
): Promise<TResult> {
  if (ttlMs <= 0) return loader();
  const now = Date.now();
  const key = makeCacheKey(instanceCacheKey, method, args);
  const entry = API_READ_CACHE.get(key);
  if (entry) {
    if (entry.expiresAt > now) {
      return Promise.resolve(entry.value as TResult);
    }
    if (entry.inFlight) {
      return entry.inFlight as Promise<TResult>;
    }
  }
  const request = loader()
    .then((value) => {
      API_READ_CACHE.set(key, {
        value,
        expiresAt: Date.now() + ttlMs,
      });
      trimReadCacheIfNeeded();
      return value;
    })
    .catch((error) => {
      const current = API_READ_CACHE.get(key);
      if (current?.inFlight === request) {
        API_READ_CACHE.delete(key);
      }
      throw error;
    });
  API_READ_CACHE.set(key, {
    value: entry?.value,
    expiresAt: entry?.expiresAt ?? 0,
    inFlight: request as Promise<unknown>,
  });
  trimReadCacheIfNeeded();
  return request;
}

function emitRemoteInvokeMetric(payload: Record<string, unknown>) {
  const line = `[metrics][remote_invoke] ${JSON.stringify(payload)}`;
  // fire-and-forget: metrics collection must not affect user flow
  void invoke("log_app_event", { message: line }).catch((error) => {
    if (import.meta.env.DEV) {
      console.warn("[dev ignored error] emitRemoteInvokeMetric", error);
    }
  });
}

function logDevApiError(context: string, error: unknown, detail: Record<string, unknown> = {}): void {
  if (!import.meta.env.DEV) return;
  console.error(`[dev api error] ${context}`, {
    ...detail,
    error: String(error),
  });
}

function shouldLogRemoteInvokeMetric(ok: boolean, elapsedMs: number): boolean {
  // Always log failures and slow calls; sample a small percentage of fast-success calls.
  if (!ok) return true;
  if (elapsedMs >= 1500) return true;
  return Math.random() < 0.05;
}

/**
 * Returns a unified API object that auto-dispatches to local or remote
 * based on the current instance context. Remote calls automatically
 * inject hostId and check connection state.
 */
export function useApi() {
  const {
    instanceId,
    instanceToken,
    isRemote,
    isDocker,
    isConnected,
    channelNodes,
    discordGuildChannels,
    channelsLoading,
    discordChannelsLoading,
    refreshChannelNodesCache,
    refreshDiscordChannelsCache,
  } = useInstance();
  const instanceCacheKey = `${instanceId}#${instanceToken}`;
  const globalCacheKey = "__global__";
  const transport: "local" | "docker_local" | "remote_ssh" = isRemote
    ? "remote_ssh"
    : (isDocker ? "docker_local" : "local");

  const explainAndWrapError = useCallback(
    async (method: string | undefined, rawError: unknown) => {
      return explainAndBuildGuidanceError({
        method: method || "unknown",
        instanceId,
        transport,
        rawError,
        emitEvent: true,
      });
    },
    [instanceId, transport],
  );

  const dispatch = useCallback(
    <TArgs extends unknown[], TResult>(
      localFn: (...args: TArgs) => Promise<TResult>,
      remoteFn: (hostId: string, ...args: TArgs) => Promise<TResult>,
      method?: string,
    ) => {
      return (...args: TArgs): Promise<TResult> => {
        if (isRemote) {
          if (!isConnected) {
            return Promise.reject(
              new Error("Not connected to remote instance"),
            );
          }
          const startedAt = Date.now();
          return remoteFn(instanceId, ...args)
            .then((result) => {
              const elapsedMs = Date.now() - startedAt;
              if (shouldLogRemoteInvokeMetric(true, elapsedMs)) {
              emitRemoteInvokeMetric({
                method: method || "unknown",
                instanceId,
                argsCount: args.length,
                ok: true,
                elapsedMs,
              });
              }
              return result;
            })
            .catch(async (error) => {
              logDevApiError("useApi dispatch remote catch", error, {
                method: method || "unknown",
                transport,
                instanceId,
                argsCount: args.length,
              });
              const elapsedMs = Date.now() - startedAt;
              if (shouldLogRemoteInvokeMetric(false, elapsedMs)) {
              emitRemoteInvokeMetric({
                method: method || "unknown",
                instanceId,
                argsCount: args.length,
                ok: false,
                elapsedMs,
                error: String(error),
              });
              }
              throw await explainAndWrapError(method, error);
            });
        }
        if (isDocker) {
          return localFn(...args).catch(async (error) => {
            logDevApiError("useApi dispatch local catch (docker)", error, {
              method: method || "unknown",
              transport,
              argsCount: args.length,
            });
            throw await explainAndWrapError(method, error);
          });
        }
        return localFn(...args).catch(async (error) => {
          logDevApiError("useApi dispatch local catch", error, {
            method: method || "unknown",
            transport,
            argsCount: args.length,
          });
          throw await explainAndWrapError(method, error);
        });
      };
    },
    [instanceId, isRemote, isDocker, isConnected, explainAndWrapError],
  );

  const dispatchCached = useCallback(
    <TArgs extends unknown[], TResult>(
      method: string,
      ttlMs: number,
      localFn: (...args: TArgs) => Promise<TResult>,
      remoteFn: (hostId: string, ...args: TArgs) => Promise<TResult>,
    ) => {
      const call = dispatch(localFn, remoteFn, method);
      return (...args: TArgs): Promise<TResult> =>
        callWithReadCache(instanceCacheKey, method, args, ttlMs, () => call(...args));
    },
    [dispatch, instanceCacheKey],
  );

  const localCached = useCallback(
    <TArgs extends unknown[], TResult>(
      method: string,
      ttlMs: number,
      fn: (...args: TArgs) => Promise<TResult>,
    ) => {
      return (...args: TArgs): Promise<TResult> =>
        callWithReadCache(instanceCacheKey, method, args, ttlMs, () => fn(...args));
    },
    [instanceCacheKey],
  );

  const localGlobalCached = useCallback(
    <TArgs extends unknown[], TResult>(
      method: string,
      ttlMs: number,
      fn: (...args: TArgs) => Promise<TResult>,
    ) => {
      return (...args: TArgs): Promise<TResult> =>
        callWithReadCache(globalCacheKey, method, args, ttlMs, () => fn(...args));
    },
    [globalCacheKey],
  );

  const withInvalidation = useCallback(
    <TArgs extends unknown[], TResult>(
      fn: (...args: TArgs) => Promise<TResult>,
      methodsToInvalidate?: string[],
    ) => {
      return (...args: TArgs): Promise<TResult> =>
        fn(...args).then((result) => {
          invalidateReadCacheForInstance(instanceCacheKey, methodsToInvalidate);
          return result;
        });
    },
    [instanceCacheKey],
  );

  const withGlobalInvalidation = useCallback(
    <TArgs extends unknown[], TResult>(
      fn: (...args: TArgs) => Promise<TResult>,
      methodsToInvalidate?: string[],
    ) => {
      return (...args: TArgs): Promise<TResult> =>
        fn(...args).then((result) => {
          invalidateReadCacheForInstance(instanceCacheKey, methodsToInvalidate);
          invalidateReadCacheForInstance(globalCacheKey, methodsToInvalidate);
          return result;
        });
    },
    [instanceCacheKey, globalCacheKey],
  );

  return useMemo(
    () => ({
      // Instance state
      instanceId,
      instanceToken,
      isRemote,
      isDocker,
      isConnected,
      channelNodes,
      discordGuildChannels,
      channelsLoading,
      discordChannelsLoading,
      refreshChannelNodesCache,
      refreshDiscordChannelsCache,

      // Status
      getInstanceStatus: dispatch(
        api.getInstanceStatus,
        api.remoteGetInstanceStatus,
      ),
      getStatusExtra: dispatchCached(
        "getStatusExtra",
        isRemote ? 15_000 : 10_000,
        api.getStatusExtra,
        api.remoteGetStatusExtra,
      ),

      // Agents
      listAgents: dispatchCached(
        "listAgents",
        isRemote ? 12_000 : 6_000,
        api.listAgentsOverview,
        api.remoteListAgentsOverview,
      ),
      setupAgentIdentity: dispatch(
        api.setupAgentIdentity,
        api.remoteSetupAgentIdentity,
      ),

      // Channels
      listChannels: dispatchCached(
        "listChannels",
        isRemote ? 15_000 : 8_000,
        api.listChannelsMinimal,
        api.remoteListChannelsMinimal,
      ),
      listBindings: dispatchCached(
        "listBindings",
        isRemote ? 12_000 : 8_000,
        api.listBindings,
        api.remoteListBindings,
      ),
      listDiscordGuildChannels: dispatchCached(
        "listDiscordGuildChannels",
        isRemote ? 20_000 : 12_000,
        api.listDiscordGuildChannels,
        api.remoteListDiscordGuildChannels,
      ),
      // Remote has no separate refresh command; reuse list which fetches fresh data
      refreshDiscordGuildChannels: dispatch(
        api.refreshDiscordGuildChannels,
        api.remoteListDiscordGuildChannels,
      ),

      // Models
      listModelProfiles: localGlobalCached(
        "listModelProfiles",
        10_000,
        api.listModelProfiles,
      ),
      upsertModelProfile: withGlobalInvalidation(
        api.upsertModelProfile,
      ),
      deleteModelProfile: withGlobalInvalidation(
        api.deleteModelProfile,
      ),
      testModelProfile: ((profileId: string) =>
        dispatch(
          api.testModelProfile,
          api.remoteTestModelProfile,
          "testModelProfile",
        )(profileId)),
      resolveApiKeys: localGlobalCached(
        "resolveApiKeys",
        10_000,
        api.resolveApiKeys,
      ),
      extractModelProfilesFromConfig: withGlobalInvalidation(
        api.extractModelProfilesFromConfig,
        ["listModelProfiles", "resolveApiKeys"],
      ),
      refreshModelCatalog: dispatch(
        api.refreshModelCatalog,
        api.remoteRefreshModelCatalog,
      ),

      // Config
      readRawConfig: dispatch(api.readRawConfig, api.remoteReadRawConfig),
      applyConfigPatch: withInvalidation(
        dispatch(
          api.applyConfigPatch,
          api.remoteApplyConfigPatch,
        ),
      ),
      restartGateway: withInvalidation(
        dispatch(api.restartGateway, api.remoteRestartGateway),
        ["getInstanceStatus", "getStatusExtra"],
      ),
      manageRescueBot: withInvalidation(
        dispatch(api.manageRescueBot, api.remoteManageRescueBot),
        ["getInstanceStatus", "getStatusExtra"],
      ),
      diagnosePrimaryViaRescue: dispatch(
        api.diagnosePrimaryViaRescue,
        api.remoteDiagnosePrimaryViaRescue,
      ),
      repairPrimaryViaRescue: dispatch(
        api.repairPrimaryViaRescue,
        api.remoteRepairPrimaryViaRescue,
      ),

      // Doctor
      runDoctor: dispatch(api.runDoctor, api.remoteRunDoctor),
      fixIssues: withInvalidation(dispatch(api.fixIssues, api.remoteFixIssues)),

      // History
      listHistory: dispatchCached(
        "listHistory",
        isRemote ? 12_000 : 8_000,
        api.listHistory,
        api.remoteListHistory,
      ),
      previewRollback: dispatch(
        api.previewRollback,
        api.remotePreviewRollback,
      ),
      rollback: withInvalidation(dispatch(api.rollback, api.remoteRollback)),

      // Sessions
      analyzeSessions: dispatch(
        api.analyzeSessions,
        api.remoteAnalyzeSessions,
      ),
      deleteSessionsByIds: withInvalidation(
        dispatch(
          api.deleteSessionsByIds,
          api.remoteDeleteSessionsByIds,
        ),
        ["listSessionFiles"],
      ),
      listSessionFiles: dispatchCached(
        "listSessionFiles",
        isRemote ? 15_000 : 10_000,
        api.listSessionFiles,
        api.remoteListSessionFiles,
      ),
      clearAllSessions: withInvalidation(
        dispatch(
          api.clearAllSessions,
          api.remoteClearAllSessions,
        ),
        ["listSessionFiles"],
      ),
      previewSession: dispatch(api.previewSession, api.remotePreviewSession),

      // Chat
      chatViaOpenclaw: dispatch(
        api.chatViaOpenclaw,
        api.remoteChatViaOpenclaw,
      ),

      // Backup & Upgrade
      backupBeforeUpgrade: dispatch(
        api.backupBeforeUpgrade,
        api.remoteBackupBeforeUpgrade,
      ),
      listBackups: dispatchCached(
        "listBackups",
        isRemote ? 20_000 : 12_000,
        api.listBackups,
        api.remoteListBackups,
      ),
      restoreFromBackup: withInvalidation(
        dispatch(
          api.restoreFromBackup,
          api.remoteRestoreFromBackup,
        ),
      ),
      deleteBackup: withInvalidation(
        dispatch(api.deleteBackup, api.remoteDeleteBackup),
        ["listBackups"],
      ),
      runOpenclawUpgrade: withInvalidation(
        dispatch(
          api.runOpenclawUpgrade,
          api.remoteRunOpenclawUpgrade,
        ),
        ["getStatusExtra", "checkOpenclawUpdate", "getCachedModelCatalog"],
      ),
      checkOpenclawUpdate: dispatchCached(
        "checkOpenclawUpdate",
        isRemote ? 10 * 60_000 : 30 * 60_000,
        api.checkOpenclawUpdate,
        api.remoteCheckOpenclawUpdate,
      ),

      // Cron & Watchdog
      listCronJobs: dispatchCached(
        "listCronJobs",
        isRemote ? 12_000 : 8_000,
        api.listCronJobs,
        api.remoteListCronJobs,
      ),
      getCronRuns: dispatchCached(
        "getCronRuns",
        isRemote ? 8_000 : 5_000,
        api.getCronRuns,
        api.remoteGetCronRuns,
      ),
      triggerCronJob: withInvalidation(
        dispatch(api.triggerCronJob, api.remoteTriggerCronJob),
        ["listCronJobs", "getCronRuns", "getWatchdogStatus"],
      ),
      deleteCronJob: withInvalidation(
        dispatch(api.deleteCronJob, api.remoteDeleteCronJob),
        ["listCronJobs", "getCronRuns", "getWatchdogStatus"],
      ),
      getWatchdogStatus: dispatchCached(
        "getWatchdogStatus",
        isRemote ? 8_000 : 5_000,
        api.getWatchdogStatus,
        api.remoteGetWatchdogStatus,
      ),
      deployWatchdog: withInvalidation(
        dispatch(api.deployWatchdog, api.remoteDeployWatchdog),
        ["getWatchdogStatus", "listCronJobs"],
      ),
      startWatchdog: withInvalidation(
        dispatch(api.startWatchdog, api.remoteStartWatchdog),
        ["getWatchdogStatus", "listCronJobs"],
      ),
      stopWatchdog: withInvalidation(
        dispatch(api.stopWatchdog, api.remoteStopWatchdog),
        ["getWatchdogStatus", "listCronJobs"],
      ),
      uninstallWatchdog: withInvalidation(
        dispatch(
          api.uninstallWatchdog,
          api.remoteUninstallWatchdog,
        ),
        ["getWatchdogStatus", "listCronJobs"],
      ),

      // Queue
      queueCommand: withInvalidation(
        dispatch(api.queueCommand, api.remoteQueueCommand),
        ["listQueuedCommands", "queuedCommandsCount", "previewQueuedCommands"],
      ),
      removeQueuedCommand: withInvalidation(
        dispatch(api.removeQueuedCommand, api.remoteRemoveQueuedCommand),
        ["listQueuedCommands", "queuedCommandsCount", "previewQueuedCommands"],
      ),
      listQueuedCommands: dispatch(api.listQueuedCommands, api.remoteListQueuedCommands),
      discardQueuedCommands: withInvalidation(
        dispatch(api.discardQueuedCommands, api.remoteDiscardQueuedCommands),
        ["listQueuedCommands", "queuedCommandsCount", "previewQueuedCommands"],
      ),
      previewQueuedCommands: dispatch(api.previewQueuedCommands, api.remotePreviewQueuedCommands),
      applyQueuedCommands: withInvalidation(
        dispatch(api.applyQueuedCommands, api.remoteApplyQueuedCommands),
        ["listQueuedCommands", "queuedCommandsCount", "previewQueuedCommands"],
      ),
      queuedCommandsCount: dispatch(api.queuedCommandsCount, api.remoteQueuedCommandsCount),

      // Logs
      readAppLog: dispatch(api.readAppLog, api.remoteReadAppLog),
      readErrorLog: dispatch(api.readErrorLog, api.remoteReadErrorLog),
      readGatewayLog: dispatch(api.readGatewayLog, api.remoteReadGatewayLog),
      readGatewayErrorLog: dispatch(api.readGatewayErrorLog, api.remoteReadGatewayErrorLog),

      // Doctor Agent (local-only, no remote dispatch)
      doctorConnect: api.doctorConnect,
      doctorDisconnect: api.doctorDisconnect,
      doctorStartDiagnosis: api.doctorStartDiagnosis,
      doctorSendMessage: api.doctorSendMessage,
      doctorApproveInvoke: api.doctorApproveInvoke,
      doctorRejectInvoke: api.doctorRejectInvoke,
      collectDoctorContext: api.collectDoctorContext,
      collectDoctorContextRemote: api.collectDoctorContextRemote,

      // Local-only (no remote equivalent needed)
      getAppPreferences: localGlobalCached(
        "getAppPreferences",
        10_000,
        api.getAppPreferences,
      ),
      getZeroclawUsageStats: localGlobalCached(
        "getZeroclawUsageStats",
        2_000,
        api.getZeroclawUsageStats,
      ),
      getZeroclawRuntimeTarget: localGlobalCached(
        "getZeroclawRuntimeTarget",
        2_000,
        api.getZeroclawRuntimeTarget,
      ),
      setZeroclawModelPreference: withGlobalInvalidation(
        api.setZeroclawModelPreference,
        ["getAppPreferences", "getZeroclawRuntimeTarget"],
      ),
      ensureAccessProfile: api.ensureAccessProfile,
      recordInstallExperience: api.recordInstallExperience,
      openUrl: api.openUrl,
      resolveProviderAuth: api.resolveProviderAuth,
      getCachedModelCatalog: localCached(
        "getCachedModelCatalog",
        8_000,
        api.getCachedModelCatalog,
      ),
      getSystemStatus: api.getSystemStatus,
      listRecipes: localCached("listRecipes", 20_000, api.listRecipes),
      connectDockerInstance: api.connectDockerInstance,
      listInstallMethods: localCached(
        "installListMethods",
        20_000,
        api.installListMethods,
      ),
      installCreateSession: api.installCreateSession,
      installGetSession: api.installGetSession,
      installDecideTarget: api.installDecideTarget,
      installOrchestratorNext: api.installOrchestratorNext,
      installRunStep: api.installRunStep,

      // SSH management (infrastructure, not abstracted)
      listSshHosts: api.listSshHosts,
      upsertSshHost: api.upsertSshHost,
      deleteSshHost: api.deleteSshHost,
      sshConnect: api.sshConnect,
      sshDisconnect: api.sshDisconnect,
      sshStatus: api.sshStatus,

      // Remote-only
      remoteWriteRawConfig: withInvalidation(api.remoteWriteRawConfig),
    }),
    [
      dispatch,
      dispatchCached,
      localCached,
      localGlobalCached,
      withInvalidation,
      withGlobalInvalidation,
      instanceId,
      isRemote,
      isDocker,
      isConnected,
      channelNodes,
      discordGuildChannels,
      channelsLoading,
      discordChannelsLoading,
      refreshChannelNodesCache,
      refreshDiscordChannelsCache,
    ],
  );
}
