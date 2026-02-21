import { invoke } from "@tauri-apps/api/core";
import type { AgentOverview, AgentSessionAnalysis, ApplyQueueResult, ApplyResult, BackupInfo, Binding, ChannelNode, CronJob, CronRun, DiscordGuildChannel, HistoryItem, InstanceStatus, ModelCatalogProvider, ModelProfile, PendingCommand, PreviewQueueResult, PreviewResult, ProviderAuthSuggestion, Recipe, ResolvedApiKey, SystemStatus, DoctorReport, SessionFile, SshHost, WatchdogStatus } from "./types";

export const api = {
  getSystemStatus: (): Promise<SystemStatus> =>
    invoke("get_system_status", {}),
  getInstanceStatus: (): Promise<InstanceStatus> =>
    invoke("get_status_light", {}),
  getCachedModelCatalog: (): Promise<ModelCatalogProvider[]> =>
    invoke("get_cached_model_catalog", {}),
  refreshModelCatalog: (): Promise<ModelCatalogProvider[]> =>
    invoke("refresh_model_catalog", {}),
  listRecipes: (source?: string): Promise<Recipe[]> =>
    invoke("list_recipes", source ? { source } : {}),
  applyConfigPatch: (patchTemplate: string, params: Record<string, string>): Promise<ApplyResult> =>
    invoke("apply_config_patch", { patchTemplate, params }),
  listHistory: (limit = 20, offset = 0): Promise<{ items: HistoryItem[] }> =>
    invoke("list_history", { limit, offset }),
  previewRollback: (snapshotId: string): Promise<PreviewResult> =>
    invoke("preview_rollback", { snapshotId }),
  rollback: (snapshotId: string): Promise<ApplyResult> =>
    invoke("rollback", { snapshotId }),
  listModelProfiles: (): Promise<ModelProfile[]> =>
    invoke("list_model_profiles", {}),
  extractModelProfilesFromConfig: (): Promise<{ created: number; reused: number; skippedInvalid: number }> =>
    invoke("extract_model_profiles_from_config", {}),
  upsertModelProfile: (profile: ModelProfile): Promise<ModelProfile> =>
    invoke("upsert_model_profile", { profile }),
  deleteModelProfile: (profileId: string): Promise<boolean> =>
    invoke("delete_model_profile", { profile_id: profileId }),
  resolveProviderAuth: (provider: string): Promise<ProviderAuthSuggestion> =>
    invoke("resolve_provider_auth", { provider }),
  resolveApiKeys: (): Promise<ResolvedApiKey[]> =>
    invoke("resolve_api_keys", {}),
  listAgentsOverview: (): Promise<AgentOverview[]> =>
    invoke("list_agents_overview", {}),
  createAgent: (agentId: string, modelValue?: string, independent?: boolean): Promise<AgentOverview> =>
    invoke("create_agent", { agentId, modelValue, independent }),
  deleteAgent: (agentId: string): Promise<boolean> =>
    invoke("delete_agent", { agentId }),
  setupAgentIdentity: (agentId: string, name: string, emoji?: string): Promise<boolean> =>
    invoke("setup_agent_identity", { agentId, name, emoji }),
  listSessionFiles: (): Promise<SessionFile[]> =>
    invoke("list_session_files", {}),
  clearAllSessions: (): Promise<number> =>
    invoke("clear_all_sessions", {}),
  analyzeSessions: (): Promise<AgentSessionAnalysis[]> =>
    invoke("analyze_sessions", {}),
  deleteSessionsByIds: (agentId: string, sessionIds: string[]): Promise<number> =>
    invoke("delete_sessions_by_ids", { agentId, sessionIds }),
  previewSession: (agentId: string, sessionId: string): Promise<{ role: string; content: string }[]> =>
    invoke("preview_session", { agentId, sessionId }),
  runDoctor: (): Promise<DoctorReport> =>
    invoke("run_doctor_command", {}),
  fixIssues: (ids: string[]): Promise<{ ok: boolean; applied: string[]; remainingIssues: string[] }> =>
    invoke("fix_issues", { ids }),
  readRawConfig: (): Promise<string> =>
    invoke("read_raw_config", {}),
  openUrl: (url: string): Promise<void> =>
    invoke("open_url", { url }),
  chatViaOpenclaw: (agentId: string, message: string, sessionId?: string): Promise<Record<string, unknown>> =>
    invoke("chat_via_openclaw", { agentId, message, sessionId }),
  backupBeforeUpgrade: (): Promise<BackupInfo> =>
    invoke("backup_before_upgrade", {}),
  listBackups: (): Promise<BackupInfo[]> =>
    invoke("list_backups", {}),
  restoreFromBackup: (backupName: string): Promise<string> =>
    invoke("restore_from_backup", { backupName }),
  deleteBackup: (backupName: string): Promise<boolean> =>
    invoke("delete_backup", { backupName }),
  listChannelsMinimal: (): Promise<ChannelNode[]> =>
    invoke("list_channels_minimal", {}),
  listDiscordGuildChannels: (): Promise<DiscordGuildChannel[]> =>
    invoke("list_discord_guild_channels", {}),
  refreshDiscordGuildChannels: (): Promise<DiscordGuildChannel[]> =>
    invoke("refresh_discord_guild_channels", {}),
  restartGateway: (): Promise<boolean> =>
    invoke("restart_gateway", {}),
  setGlobalModel: (modelValue: string | null): Promise<boolean> =>
    invoke("set_global_model", { modelValue }),
  setAgentModel: (agentId: string, modelValue: string | null): Promise<boolean> =>
    invoke("set_agent_model", { agentId, modelValue }),
  listBindings: (): Promise<Binding[]> =>
    invoke("list_bindings", {}),
  assignChannelAgent: (channelType: string, peerId: string, agentId: string | null): Promise<boolean> =>
    invoke("assign_channel_agent", { channelType, peerId, agentId }),
  // SSH host management
  listSshHosts: (): Promise<SshHost[]> =>
    invoke("list_ssh_hosts", {}),
  upsertSshHost: (host: SshHost): Promise<SshHost> =>
    invoke("upsert_ssh_host", { host }),
  deleteSshHost: (hostId: string): Promise<boolean> =>
    invoke("delete_ssh_host", { hostId }),

  // SSH connection
  sshConnect: (hostId: string): Promise<boolean> =>
    invoke("ssh_connect", { hostId }),
  sshDisconnect: (hostId: string): Promise<boolean> =>
    invoke("ssh_disconnect", { hostId }),
  sshStatus: (hostId: string): Promise<string> =>
    invoke("ssh_status", { hostId }),

  // Remote business commands
  remoteReadRawConfig: (hostId: string): Promise<string> =>
    invoke("remote_read_raw_config", { hostId }),
  remoteGetInstanceStatus: (hostId: string): Promise<InstanceStatus> =>
    invoke("remote_get_system_status", { hostId }),
  remoteListAgentsOverview: (hostId: string): Promise<AgentOverview[]> =>
    invoke("remote_list_agents_overview", { hostId }),
  remoteListChannelsMinimal: (hostId: string): Promise<ChannelNode[]> =>
    invoke("remote_list_channels_minimal", { hostId }),
  remoteListBindings: (hostId: string): Promise<Binding[]> =>
    invoke("remote_list_bindings", { hostId }),
  remoteRestartGateway: (hostId: string): Promise<boolean> =>
    invoke("remote_restart_gateway", { hostId }),
  remoteApplyConfigPatch: (hostId: string, patchTemplate: string, params: Record<string, string>): Promise<ApplyResult> =>
    invoke("remote_apply_config_patch", { hostId, patchTemplate, params }),
  remoteCreateAgent: (hostId: string, agentId: string, modelValue?: string): Promise<AgentOverview> =>
    invoke("remote_create_agent", { hostId, agentId, modelValue }),
  remoteDeleteAgent: (hostId: string, agentId: string): Promise<boolean> =>
    invoke("remote_delete_agent", { hostId, agentId }),
  remoteAssignChannelAgent: (hostId: string, channelType: string, peerId: string, agentId: string | null): Promise<boolean> =>
    invoke("remote_assign_channel_agent", { hostId, channelType, peerId, agentId }),
  remoteSetGlobalModel: (hostId: string, modelValue: string | null): Promise<boolean> =>
    invoke("remote_set_global_model", { hostId, modelValue }),
  remoteSetAgentModel: (hostId: string, agentId: string, modelValue: string | null): Promise<boolean> =>
    invoke("remote_set_agent_model", { hostId, agentId, modelValue }),
  remoteListDiscordGuildChannels: (hostId: string): Promise<DiscordGuildChannel[]> =>
    invoke("remote_list_discord_guild_channels", { hostId }),
  remoteRunDoctor: (hostId: string): Promise<DoctorReport> =>
    invoke("remote_run_doctor", { hostId }),
  remoteFixIssues: (hostId: string, ids: string[]): Promise<{ ok: boolean; applied: string[]; remainingIssues: string[] }> =>
    invoke("remote_fix_issues", { hostId, ids }),
  remoteSetupAgentIdentity: (hostId: string, agentId: string, name: string, emoji?: string): Promise<boolean> =>
    invoke("remote_setup_agent_identity", { hostId, agentId, name, emoji }),
  remoteListHistory: (hostId: string): Promise<{ items: HistoryItem[] }> =>
    invoke("remote_list_history", { hostId }),
  remotePreviewRollback: (hostId: string, snapshotId: string): Promise<PreviewResult> =>
    invoke("remote_preview_rollback", { hostId, snapshotId }),
  remoteRollback: (hostId: string, snapshotId: string): Promise<ApplyResult> =>
    invoke("remote_rollback", { hostId, snapshotId }),
  remoteWriteRawConfig: (hostId: string, content: string): Promise<boolean> =>
    invoke("remote_write_raw_config", { hostId, content }),
  remoteAnalyzeSessions: (hostId: string): Promise<AgentSessionAnalysis[]> =>
    invoke("remote_analyze_sessions", { hostId }),
  remoteDeleteSessionsByIds: (hostId: string, agentId: string, sessionIds: string[]): Promise<number> =>
    invoke("remote_delete_sessions_by_ids", { hostId, agentId, sessionIds }),
  remoteListSessionFiles: (hostId: string): Promise<SessionFile[]> =>
    invoke("remote_list_session_files", { hostId }),
  remoteClearAllSessions: (hostId: string): Promise<number> =>
    invoke("remote_clear_all_sessions", { hostId }),
  remotePreviewSession: (hostId: string, agentId: string, sessionId: string): Promise<{ role: string; content: string }[]> =>
    invoke("remote_preview_session", { hostId, agentId, sessionId }),
  remoteListModelProfiles: (hostId: string): Promise<ModelProfile[]> =>
    invoke("remote_list_model_profiles", { hostId }),
  remoteUpsertModelProfile: (hostId: string, profile: ModelProfile): Promise<ModelProfile> =>
    invoke("remote_upsert_model_profile", { hostId, profile }),
  remoteDeleteModelProfile: (hostId: string, profileId: string): Promise<boolean> =>
    invoke("remote_delete_model_profile", { hostId, profileId }),
  remoteResolveApiKeys: (hostId: string): Promise<ResolvedApiKey[]> =>
    invoke("remote_resolve_api_keys", { hostId }),
  remoteExtractModelProfilesFromConfig: (hostId: string): Promise<{ created: number; reused: number; skippedInvalid: number }> =>
    invoke("remote_extract_model_profiles_from_config", { hostId }),
  remoteRefreshModelCatalog: (hostId: string): Promise<ModelCatalogProvider[]> =>
    invoke("remote_refresh_model_catalog", { hostId }),
  remoteChatViaOpenclaw: (hostId: string, agentId: string, message: string, sessionId?: string): Promise<Record<string, unknown>> =>
    invoke("remote_chat_via_openclaw", { hostId, agentId, message, sessionId }),
  remoteCheckOpenclawUpdate: (hostId: string): Promise<{ upgradeAvailable: boolean; latestVersion: string | null; installedVersion: string }> =>
    invoke("remote_check_openclaw_update", { hostId }),
  // Remote backup
  remoteBackupBeforeUpgrade: (hostId: string): Promise<BackupInfo> =>
    invoke("remote_backup_before_upgrade", { hostId }),
  remoteListBackups: (hostId: string): Promise<BackupInfo[]> =>
    invoke("remote_list_backups", { hostId }),
  remoteRestoreFromBackup: (hostId: string, backupName: string): Promise<string> =>
    invoke("remote_restore_from_backup", { hostId, backupName }),
  remoteDeleteBackup: (hostId: string, backupName: string): Promise<boolean> =>
    invoke("remote_delete_backup", { hostId, backupName }),

  // Upgrade
  checkOpenclawUpdate: (): Promise<{ upgradeAvailable: boolean; latestVersion: string | null; installedVersion: string }> =>
    invoke<SystemStatus>("get_system_status").then((s) => ({
      upgradeAvailable: s.openclawUpdate?.upgradeAvailable ?? false,
      latestVersion: s.openclawUpdate?.latestVersion ?? null,
      installedVersion: s.openclawVersion ?? "",
    })),
  runOpenclawUpgrade: (): Promise<string> =>
    invoke("run_openclaw_upgrade", {}),
  remoteRunOpenclawUpgrade: (hostId: string): Promise<string> =>
    invoke("remote_run_openclaw_upgrade", { hostId }),

  // Cron
  listCronJobs: (): Promise<CronJob[]> =>
    invoke("list_cron_jobs", {}),
  getCronRuns: (jobId: string, limit?: number): Promise<CronRun[]> =>
    invoke("get_cron_runs", { jobId, limit }),
  triggerCronJob: (jobId: string): Promise<string> =>
    invoke("trigger_cron_job", { jobId }),
  deleteCronJob: (jobId: string): Promise<string> =>
    invoke("delete_cron_job", { jobId }),

  // Watchdog
  getWatchdogStatus: (): Promise<WatchdogStatus & { alive: boolean; deployed: boolean }> =>
    invoke("get_watchdog_status", {}),
  deployWatchdog: (): Promise<boolean> =>
    invoke("deploy_watchdog", {}),
  startWatchdog: (): Promise<boolean> =>
    invoke("start_watchdog", {}),
  stopWatchdog: (): Promise<boolean> =>
    invoke("stop_watchdog", {}),
  uninstallWatchdog: (): Promise<boolean> =>
    invoke("uninstall_watchdog", {}),

  // Remote cron
  remoteListCronJobs: (hostId: string): Promise<CronJob[]> =>
    invoke("remote_list_cron_jobs", { hostId }),
  remoteGetCronRuns: (hostId: string, jobId: string, limit?: number): Promise<CronRun[]> =>
    invoke("remote_get_cron_runs", { hostId, jobId, limit }),
  remoteTriggerCronJob: (hostId: string, jobId: string): Promise<string> =>
    invoke("remote_trigger_cron_job", { hostId, jobId }),
  remoteDeleteCronJob: (hostId: string, jobId: string): Promise<string> =>
    invoke("remote_delete_cron_job", { hostId, jobId }),

  // Remote watchdog
  remoteGetWatchdogStatus: (hostId: string): Promise<WatchdogStatus & { alive: boolean; deployed: boolean }> =>
    invoke("remote_get_watchdog_status", { hostId }),
  remoteDeployWatchdog: (hostId: string): Promise<boolean> =>
    invoke("remote_deploy_watchdog", { hostId }),
  remoteStartWatchdog: (hostId: string): Promise<boolean> =>
    invoke("remote_start_watchdog", { hostId }),
  remoteStopWatchdog: (hostId: string): Promise<boolean> =>
    invoke("remote_stop_watchdog", { hostId }),
  remoteUninstallWatchdog: (hostId: string): Promise<boolean> =>
    invoke("remote_uninstall_watchdog", { hostId }),

  // Queue management
  queueCommand: (label: string, command: string[]): Promise<PendingCommand> =>
    invoke("queue_command", { label, command }),
  removeQueuedCommand: (id: string): Promise<boolean> =>
    invoke("remove_queued_command", { id }),
  listQueuedCommands: (): Promise<PendingCommand[]> =>
    invoke("list_queued_commands", {}),
  discardQueuedCommands: (): Promise<boolean> =>
    invoke("discard_queued_commands", {}),
  previewQueuedCommands: (): Promise<PreviewQueueResult> =>
    invoke("preview_queued_commands", {}),
  applyQueuedCommands: (): Promise<ApplyQueueResult> =>
    invoke("apply_queued_commands", {}),
  queuedCommandsCount: (): Promise<number> =>
    invoke("queued_commands_count", {}),

  // Remote queue management
  remoteQueueCommand: (hostId: string, label: string, command: string[]): Promise<PendingCommand> =>
    invoke("remote_queue_command", { hostId, label, command }),
  remoteRemoveQueuedCommand: (hostId: string, id: string): Promise<boolean> =>
    invoke("remote_remove_queued_command", { hostId, id }),
  remoteListQueuedCommands: (hostId: string): Promise<PendingCommand[]> =>
    invoke("remote_list_queued_commands", { hostId }),
  remoteDiscardQueuedCommands: (hostId: string): Promise<boolean> =>
    invoke("remote_discard_queued_commands", { hostId }),
  remotePreviewQueuedCommands: (hostId: string): Promise<PreviewQueueResult> =>
    invoke("remote_preview_queued_commands", { hostId }),
  remoteApplyQueuedCommands: (hostId: string): Promise<ApplyQueueResult> =>
    invoke("remote_apply_queued_commands", { hostId }),
  remoteQueuedCommandsCount: (hostId: string): Promise<number> =>
    invoke("remote_queued_commands_count", { hostId }),

  // Logs
  readAppLog: (lines?: number): Promise<string> =>
    invoke("read_app_log", { lines }),
  readErrorLog: (lines?: number): Promise<string> =>
    invoke("read_error_log", { lines }),
  remoteReadAppLog: (hostId: string, lines?: number): Promise<string> =>
    invoke("remote_read_app_log", { hostId, lines }),
  remoteReadErrorLog: (hostId: string, lines?: number): Promise<string> =>
    invoke("remote_read_error_log", { hostId, lines }),
};
