import { useCallback, useMemo } from "react";
import { useInstance } from "./instance-context";
import { api } from "./api";

/**
 * Returns a unified API object that auto-dispatches to local or remote
 * based on the current instance context. Remote calls automatically
 * inject hostId and check connection state.
 */
export function useApi() {
  const { instanceId, isRemote, isConnected, discordGuildChannels } = useInstance();

  const dispatch = useCallback(
    <TArgs extends unknown[], TResult>(
      localFn: (...args: TArgs) => Promise<TResult>,
      remoteFn: (hostId: string, ...args: TArgs) => Promise<TResult>,
    ) => {
      return (...args: TArgs): Promise<TResult> => {
        if (isRemote) {
          if (!isConnected) {
            return Promise.reject(
              new Error("Not connected to remote instance"),
            );
          }
          return remoteFn(instanceId, ...args);
        }
        return localFn(...args);
      };
    },
    [instanceId, isRemote, isConnected],
  );

  return useMemo(
    () => ({
      // Instance state
      instanceId,
      isRemote,
      isConnected,
      discordGuildChannels,

      // Status
      getInstanceStatus: dispatch(
        api.getInstanceStatus,
        api.remoteGetInstanceStatus,
      ),

      // Agents
      listAgents: dispatch(
        api.listAgentsOverview,
        api.remoteListAgentsOverview,
      ),
      createAgent: dispatch(api.createAgent, api.remoteCreateAgent),
      deleteAgent: dispatch(api.deleteAgent, api.remoteDeleteAgent),
      setupAgentIdentity: dispatch(
        api.setupAgentIdentity,
        api.remoteSetupAgentIdentity,
      ),

      // Channels
      listChannels: dispatch(
        api.listChannelsMinimal,
        api.remoteListChannelsMinimal,
      ),
      listBindings: dispatch(api.listBindings, api.remoteListBindings),
      assignChannelAgent: dispatch(
        api.assignChannelAgent,
        api.remoteAssignChannelAgent,
      ),
      listDiscordGuildChannels: dispatch(
        api.listDiscordGuildChannels,
        api.remoteListDiscordGuildChannels,
      ),
      // Remote has no separate refresh command; reuse list which fetches fresh data
      refreshDiscordGuildChannels: dispatch(
        api.refreshDiscordGuildChannels,
        api.remoteListDiscordGuildChannels,
      ),

      // Models
      setGlobalModel: dispatch(api.setGlobalModel, api.remoteSetGlobalModel),
      setAgentModel: dispatch(api.setAgentModel, api.remoteSetAgentModel),
      listModelProfiles: dispatch(
        api.listModelProfiles,
        api.remoteListModelProfiles,
      ),
      upsertModelProfile: dispatch(
        api.upsertModelProfile,
        api.remoteUpsertModelProfile,
      ),
      deleteModelProfile: dispatch(
        api.deleteModelProfile,
        api.remoteDeleteModelProfile,
      ),
      resolveApiKeys: dispatch(api.resolveApiKeys, api.remoteResolveApiKeys),
      extractModelProfilesFromConfig: dispatch(
        api.extractModelProfilesFromConfig,
        api.remoteExtractModelProfilesFromConfig,
      ),
      refreshModelCatalog: dispatch(
        api.refreshModelCatalog,
        api.remoteRefreshModelCatalog,
      ),

      // Config
      readRawConfig: dispatch(api.readRawConfig, api.remoteReadRawConfig),
      applyConfigPatch: dispatch(
        api.applyConfigPatch,
        api.remoteApplyConfigPatch,
      ),
      restartGateway: dispatch(api.restartGateway, api.remoteRestartGateway),

      // Doctor
      runDoctor: dispatch(api.runDoctor, api.remoteRunDoctor),
      fixIssues: dispatch(api.fixIssues, api.remoteFixIssues),

      // History
      listHistory: dispatch(api.listHistory, api.remoteListHistory),
      previewRollback: dispatch(
        api.previewRollback,
        api.remotePreviewRollback,
      ),
      rollback: dispatch(api.rollback, api.remoteRollback),

      // Sessions
      analyzeSessions: dispatch(
        api.analyzeSessions,
        api.remoteAnalyzeSessions,
      ),
      deleteSessionsByIds: dispatch(
        api.deleteSessionsByIds,
        api.remoteDeleteSessionsByIds,
      ),
      listSessionFiles: dispatch(
        api.listSessionFiles,
        api.remoteListSessionFiles,
      ),
      clearAllSessions: dispatch(
        api.clearAllSessions,
        api.remoteClearAllSessions,
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
      listBackups: dispatch(api.listBackups, api.remoteListBackups),
      restoreFromBackup: dispatch(
        api.restoreFromBackup,
        api.remoteRestoreFromBackup,
      ),
      deleteBackup: dispatch(api.deleteBackup, api.remoteDeleteBackup),
      runOpenclawUpgrade: dispatch(
        api.runOpenclawUpgrade,
        api.remoteRunOpenclawUpgrade,
      ),
      checkOpenclawUpdate: dispatch(
        api.checkOpenclawUpdate,
        api.remoteCheckOpenclawUpdate,
      ),

      // Cron & Watchdog
      listCronJobs: dispatch(api.listCronJobs, api.remoteListCronJobs),
      getCronRuns: dispatch(api.getCronRuns, api.remoteGetCronRuns),
      triggerCronJob: dispatch(api.triggerCronJob, api.remoteTriggerCronJob),
      deleteCronJob: dispatch(api.deleteCronJob, api.remoteDeleteCronJob),
      getWatchdogStatus: dispatch(
        api.getWatchdogStatus,
        api.remoteGetWatchdogStatus,
      ),
      deployWatchdog: dispatch(api.deployWatchdog, api.remoteDeployWatchdog),
      startWatchdog: dispatch(api.startWatchdog, api.remoteStartWatchdog),
      stopWatchdog: dispatch(api.stopWatchdog, api.remoteStopWatchdog),
      uninstallWatchdog: dispatch(
        api.uninstallWatchdog,
        api.remoteUninstallWatchdog,
      ),

      // Queue
      queueCommand: dispatch(api.queueCommand, api.remoteQueueCommand),
      removeQueuedCommand: dispatch(api.removeQueuedCommand, api.remoteRemoveQueuedCommand),
      listQueuedCommands: dispatch(api.listQueuedCommands, api.remoteListQueuedCommands),
      discardQueuedCommands: dispatch(api.discardQueuedCommands, api.remoteDiscardQueuedCommands),
      previewQueuedCommands: dispatch(api.previewQueuedCommands, api.remotePreviewQueuedCommands),
      applyQueuedCommands: dispatch(api.applyQueuedCommands, api.remoteApplyQueuedCommands),
      queuedCommandsCount: dispatch(api.queuedCommandsCount, api.remoteQueuedCommandsCount),

      // Logs
      readAppLog: dispatch(api.readAppLog, api.remoteReadAppLog),
      readErrorLog: dispatch(api.readErrorLog, api.remoteReadErrorLog),

      // Local-only (no remote equivalent needed)
      openUrl: api.openUrl,
      resolveProviderAuth: api.resolveProviderAuth,
      getCachedModelCatalog: api.getCachedModelCatalog,
      getSystemStatus: api.getSystemStatus,
      listRecipes: api.listRecipes,

      // SSH management (infrastructure, not abstracted)
      listSshHosts: api.listSshHosts,
      upsertSshHost: api.upsertSshHost,
      deleteSshHost: api.deleteSshHost,
      sshConnect: api.sshConnect,
      sshDisconnect: api.sshDisconnect,
      sshStatus: api.sshStatus,

      // Remote-only
      remoteWriteRawConfig: api.remoteWriteRawConfig,
    }),
    [dispatch, instanceId, isRemote, isConnected, discordGuildChannels],
  );
}
