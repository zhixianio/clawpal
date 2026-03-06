import { invoke } from "@tauri-apps/api/core";
import type { AgentOverview, AgentSessionAnalysis, AppPreferences, ApplyQueueResult, ApplyResult, BackupInfo, Binding, BugReportSettings, BugReportStats, ChannelNode, CronJob, CronRun, DiscordGuildChannel, DiscoveredInstance, DockerInstance, EnsureAccessResult, GuidanceAction, HistoryItem, InstallMethodCapability, InstallOrchestratorDecision, InstallSession, InstallStepResult, InstallTargetDecision, InstanceStatus, StatusExtra, ModelCatalogProvider, ModelProfile, PendingCommand, PrecheckIssue, PreviewQueueResult, PreviewResult, ProviderAuthSuggestion, Recipe, RecordInstallExperienceResult, RegisteredInstance, RelatedSecretPushResult, RemoteAuthSyncResult, RescueBotAction, RescueBotManageResult, RescuePrimaryDiagnosisResult, RescuePrimaryRepairResult, ResolvedApiKey, SshConfigHostSuggestion, SshDiagnosticReport, SshHost, SshIntent, SshTransferStats, SystemStatus, DoctorReport, SessionFile, WatchdogStatus, ZeroclawOauthCompleteResult, ZeroclawOauthLoginStartResult, ZeroclawRuntimeTarget, ZeroclawUsageStats } from "./types";

export const api = {
  setActiveOpenclawHome: (path: string | null): Promise<boolean> =>
    invoke("set_active_openclaw_home", { path }),
  setActiveClawpalDataDir: (path: string | null): Promise<boolean> =>
    invoke("set_active_clawpal_data_dir", { path }),
  getAppPreferences: (): Promise<AppPreferences> =>
    invoke("get_app_preferences", {}),
  getBugReportSettings: (): Promise<BugReportSettings> =>
    invoke("get_bug_report_settings", {}),
  setBugReportSettings: (settings: BugReportSettings): Promise<BugReportSettings> =>
    invoke("set_bug_report_settings", { settings }),
  getBugReportStats: (): Promise<BugReportStats> =>
    invoke("get_bug_report_stats", {}),
  testBugReportConnection: (): Promise<boolean> =>
    invoke("test_bug_report_connection", {}),
  captureFrontendError: (message: string, stack?: string, level?: string) =>
    invoke("capture_frontend_error", { message, stack, level }),
  getZeroclawUsageStats: (): Promise<ZeroclawUsageStats> =>
    invoke("get_zeroclaw_usage_stats", {}),
  getZeroclawRuntimeTarget: (): Promise<ZeroclawRuntimeTarget> =>
    invoke("get_zeroclaw_runtime_target", {}),
  setZeroclawModelPreference: (model: string | null): Promise<AppPreferences> =>
    invoke("set_zeroclaw_model_preference", { model }),
  setZeroclawDoctorUiPreference: (showUi: boolean): Promise<AppPreferences> =>
    invoke("set_zeroclaw_doctor_ui_preference", { showUi }),
  setRescueBotUiPreference: (showUi: boolean): Promise<AppPreferences> =>
    invoke("set_rescue_bot_ui_preference", { showUi }),
  setSshTransferSpeedUiPreference: (showUi: boolean): Promise<AppPreferences> =>
    invoke("set_ssh_transfer_speed_ui_preference", { showUi }),
  explainOperationError: (
    instanceId: string,
    operation: string,
    transport: "local" | "docker_local" | "remote_ssh",
    error: string,
    language?: string,
  ): Promise<{ message: string; summary: string; actions: string[]; structuredActions: GuidanceAction[]; source: string }> =>
    invoke("explain_operation_error", {
      instanceId,
      operation,
      transport,
      error,
      language: language ?? null,
    }),
  localOpenclawConfigExists: (openclawHome: string): Promise<boolean> =>
    invoke("local_openclaw_config_exists", { openclawHome }),
  deleteLocalInstanceHome: (openclawHome: string): Promise<boolean> =>
    invoke("delete_local_instance_home", { openclawHome }),
  ensureAccessProfile: (instanceId: string, transport: string): Promise<EnsureAccessResult> =>
    invoke("ensure_access_profile", { instanceId, transport }),
  recordInstallExperience: (sessionId: string, instanceId: string, goal: string): Promise<RecordInstallExperienceResult> =>
    invoke("record_install_experience", { sessionId, instanceId, goal }),
  installCreateSession: (
    method: "local" | "wsl2" | "docker" | "remote_ssh",
    options?: Record<string, unknown>,
  ): Promise<InstallSession> =>
    invoke("install_create_session", options ? { method, options } : { method }),
  installGetSession: (sessionId: string): Promise<InstallSession> =>
    invoke("install_get_session", { sessionId }),
  installListMethods: (): Promise<InstallMethodCapability[]> =>
    invoke("install_list_methods", {}),
  installDecideTarget: (goal: string, context?: Record<string, unknown>): Promise<InstallTargetDecision> =>
    invoke("install_decide_target", context ? { goal, context } : { goal }),
  installOrchestratorNext: (sessionId: string, goal: string): Promise<InstallOrchestratorDecision> =>
    invoke("install_orchestrator_next", { sessionId, goal }),
  installRunStep: (sessionId: string, step: "precheck" | "install" | "init" | "verify"): Promise<InstallStepResult> =>
    invoke("install_run_step", { sessionId, step }),
  getSystemStatus: (): Promise<SystemStatus> =>
    invoke("get_system_status", {}),
  listRegisteredInstances: (): Promise<RegisteredInstance[]> =>
    invoke("list_registered_instances", {}),
  discoverLocalInstances: (): Promise<DiscoveredInstance[]> =>
    invoke("discover_local_instances"),
  deleteRegisteredInstance: (instanceId: string): Promise<boolean> =>
    invoke("delete_registered_instance", { instanceId }),
  connectDockerInstance: (
    home: string,
    label?: string,
    instanceId?: string,
  ): Promise<RegisteredInstance> =>
    invoke("connect_docker_instance", { home, label: label ?? null, instanceId: instanceId ?? null }),
  connectLocalInstance: (
    home: string,
    label?: string,
    instanceId?: string,
  ): Promise<RegisteredInstance> =>
    invoke("connect_local_instance", { home, label: label ?? null, instanceId: instanceId ?? null }),
  connectSshInstance: (hostId: string): Promise<RegisteredInstance> =>
    invoke("connect_ssh_instance", { hostId }),
  migrateLegacyInstances: (
    legacyDockerInstances: DockerInstance[],
    legacyOpenTabIds: string[],
  ): Promise<{ importedSshHosts: number; importedDockerInstances: number; importedOpenTabInstances: number; totalInstances: number }> =>
    invoke("migrate_legacy_instances", { legacyDockerInstances, legacyOpenTabIds }),
  getInstanceStatus: (): Promise<InstanceStatus> =>
    invoke("get_status_light", {}),
  getStatusExtra: (): Promise<StatusExtra> =>
    invoke("get_status_extra", {}),
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
    invoke("delete_model_profile", { profileId }),
  testModelProfile: (profileId: string): Promise<boolean> =>
    invoke("test_model_profile", { profileId }),
  resolveProviderAuth: (provider: string): Promise<ProviderAuthSuggestion> =>
    invoke("resolve_provider_auth", { provider }),
  startZeroclawOauthLogin: (
    provider: string,
    profile?: string,
    instanceId?: string,
  ): Promise<ZeroclawOauthLoginStartResult> =>
    invoke("start_zeroclaw_oauth_login", {
      provider,
      profile: profile ?? null,
      instanceId: instanceId ?? null,
    }),
  completeZeroclawOauthLogin: (
    provider: string,
    redirectInput: string,
    profile?: string,
    instanceId?: string,
  ): Promise<ZeroclawOauthCompleteResult> =>
    invoke("complete_zeroclaw_oauth_login", {
      provider,
      redirectInput,
      profile: profile ?? null,
      instanceId: instanceId ?? null,
    }),
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
  precheckRegistry: (): Promise<PrecheckIssue[]> =>
    invoke("precheck_registry"),
  precheckInstance: (instanceId: string): Promise<PrecheckIssue[]> =>
    invoke("precheck_instance", { instanceId }),
  precheckTransport: (instanceId: string): Promise<PrecheckIssue[]> =>
    invoke("precheck_transport", { instanceId }),
  precheckAuth: (instanceId: string): Promise<PrecheckIssue[]> =>
    invoke("precheck_auth", { instanceId }),
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
  manageRescueBot: (action: RescueBotAction, profile?: string, rescuePort?: number): Promise<RescueBotManageResult> =>
    invoke("manage_rescue_bot", { action, profile: profile ?? null, rescuePort: rescuePort ?? null }),
  diagnosePrimaryViaRescue: (targetProfile?: string, rescueProfile?: string): Promise<RescuePrimaryDiagnosisResult> =>
    invoke("diagnose_primary_via_rescue", { targetProfile: targetProfile ?? null, rescueProfile: rescueProfile ?? null }),
  repairPrimaryViaRescue: (targetProfile?: string, rescueProfile?: string, issueIds?: string[]): Promise<RescuePrimaryRepairResult> =>
    invoke("repair_primary_via_rescue", { targetProfile: targetProfile ?? null, rescueProfile: rescueProfile ?? null, issueIds: issueIds ?? null }),
  setGlobalModel: (modelValue: string | null): Promise<boolean> =>
    invoke("set_global_model", { modelValue }),
  setAgentModel: (agentId: string, modelValue: string | null): Promise<boolean> =>
    invoke("set_agent_model", { agentId, modelValue }),
  listBindings: (): Promise<Binding[]> =>
    invoke("list_bindings", {}),
  // SSH host management
  listSshHosts: (): Promise<SshHost[]> =>
    invoke("list_ssh_hosts", {}),
  listSshConfigHosts: (): Promise<SshConfigHostSuggestion[]> =>
    invoke("list_ssh_config_hosts", {}),
  upsertSshHost: (host: SshHost): Promise<SshHost> =>
    invoke("upsert_ssh_host", { host }),
  deleteSshHost: (hostId: string): Promise<boolean> =>
    invoke("delete_ssh_host", { hostId }),

  // SSH connection
  sshConnect: (hostId: string): Promise<boolean> =>
    invoke("ssh_connect", { hostId }),
  sshConnectWithPassphrase: (hostId: string, passphrase: string): Promise<boolean> =>
    invoke("ssh_connect_with_passphrase", { hostId, passphrase }),
  sshDisconnect: (hostId: string): Promise<boolean> =>
    invoke("ssh_disconnect", { hostId }),
  sshStatus: (hostId: string): Promise<string> =>
    invoke("ssh_status", { hostId }),
  diagnoseSsh: (hostId: string, intent: SshIntent): Promise<SshDiagnosticReport> =>
    invoke("diagnose_ssh", { hostId, intent }),
  getSshTransferStats: (hostId: string): Promise<SshTransferStats> =>
    invoke("get_ssh_transfer_stats", { hostId }),

  // Remote business commands
  remoteReadRawConfig: (hostId: string): Promise<string> =>
    invoke("remote_read_raw_config", { hostId }),
  remoteGetInstanceStatus: (hostId: string): Promise<InstanceStatus> =>
    invoke("remote_get_system_status", { hostId }),
  remoteGetStatusExtra: (hostId: string): Promise<StatusExtra> =>
    invoke("remote_get_status_extra", { hostId }),
  remoteListAgentsOverview: (hostId: string): Promise<AgentOverview[]> =>
    invoke("remote_list_agents_overview", { hostId }),
  remoteListChannelsMinimal: (hostId: string): Promise<ChannelNode[]> =>
    invoke("remote_list_channels_minimal", { hostId }),
  remoteListBindings: (hostId: string): Promise<Binding[]> =>
    invoke("remote_list_bindings", { hostId }),
  remoteRestartGateway: (hostId: string): Promise<boolean> =>
    invoke("remote_restart_gateway", { hostId }),
  remoteManageRescueBot: (hostId: string, action: RescueBotAction, profile?: string, rescuePort?: number): Promise<RescueBotManageResult> =>
    invoke("remote_manage_rescue_bot", { hostId, action, profile: profile ?? null, rescuePort: rescuePort ?? null }),
  remoteDiagnosePrimaryViaRescue: (hostId: string, targetProfile?: string, rescueProfile?: string): Promise<RescuePrimaryDiagnosisResult> =>
    invoke("remote_diagnose_primary_via_rescue", { hostId, targetProfile: targetProfile ?? null, rescueProfile: rescueProfile ?? null }),
  remoteRepairPrimaryViaRescue: (hostId: string, targetProfile?: string, rescueProfile?: string, issueIds?: string[]): Promise<RescuePrimaryRepairResult> =>
    invoke("remote_repair_primary_via_rescue", { hostId, targetProfile: targetProfile ?? null, rescueProfile: rescueProfile ?? null, issueIds: issueIds ?? null }),
  remoteApplyConfigPatch: (hostId: string, patchTemplate: string, params: Record<string, string>): Promise<ApplyResult> =>
    invoke("remote_apply_config_patch", { hostId, patchTemplate, params }),
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
  remoteTestModelProfile: (hostId: string, profileId: string): Promise<boolean> =>
    invoke("remote_test_model_profile", { hostId, profileId }),
  remoteResolveApiKeys: (hostId: string): Promise<ResolvedApiKey[]> =>
    invoke("remote_resolve_api_keys", { hostId }),
  remoteSyncProfilesToLocalAuth: (hostId: string): Promise<RemoteAuthSyncResult> =>
    invoke("remote_sync_profiles_to_local_auth", { hostId }),
  pushRelatedSecretsToRemote: (hostId: string): Promise<RelatedSecretPushResult> =>
    invoke("push_related_secrets_to_remote", { hostId }),
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
    invoke("check_openclaw_update"),
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

  // Doctor Agent
  doctorConnect: (): Promise<void> =>
    invoke("doctor_connect"),
  doctorDisconnect: (): Promise<void> =>
    invoke("doctor_disconnect"),
  doctorStartDiagnosis: (context: string, sessionKey: string, agentId?: string, instanceId?: string): Promise<void> =>
    invoke("doctor_start_diagnosis", { context, sessionKey, agentId: agentId ?? "main", instanceId: instanceId ?? "local" }),
  doctorSendMessage: (message: string, sessionKey: string, agentId?: string, instanceId?: string): Promise<void> =>
    invoke("doctor_send_message", { message, sessionKey, agentId: agentId ?? "main", instanceId: instanceId ?? "local" }),
  doctorApproveInvoke: (invokeId: string, target: string, instanceId: string, sessionKey: string, agentId: string, domain?: string): Promise<Record<string, unknown>> =>
    invoke("doctor_approve_invoke", { invokeId, target, instanceId, sessionKey, agentId, domain }),
  doctorRejectInvoke: (invokeId: string, reason: string): Promise<void> =>
    invoke("doctor_reject_invoke", { invokeId, reason }),
  collectDoctorContext: (): Promise<string> =>
    invoke("collect_doctor_context"),
  collectDoctorContextRemote: (hostId: string): Promise<string> =>
    invoke("collect_doctor_context_remote", { hostId }),
  // Install Agent
  installStartSession: (context: string, sessionKey: string, agentId?: string, instanceId?: string): Promise<void> =>
    invoke("install_start_session", { context, sessionKey, agentId: agentId ?? "main", instanceId: instanceId ?? "local" }),
  installSendMessage: (message: string, sessionKey: string, agentId?: string, instanceId?: string): Promise<void> =>
    invoke("install_send_message", { message, sessionKey, agentId: agentId ?? "main", instanceId: instanceId ?? "local" }),
  // Logs
  readAppLog: (lines?: number): Promise<string> =>
    invoke("read_app_log", { lines }),
  readErrorLog: (lines?: number): Promise<string> =>
    invoke("read_error_log", { lines }),
  readGatewayLog: (lines?: number): Promise<string> =>
    invoke("read_gateway_log", { lines }),
  readGatewayErrorLog: (lines?: number): Promise<string> =>
    invoke("read_gateway_error_log", { lines }),
  remoteReadAppLog: (hostId: string, lines?: number): Promise<string> =>
    invoke("remote_read_app_log", { hostId, lines }),
  remoteReadErrorLog: (hostId: string, lines?: number): Promise<string> =>
    invoke("remote_read_error_log", { hostId, lines }),
  remoteReadGatewayLog: (hostId: string, lines?: number): Promise<string> =>
    invoke("remote_read_gateway_log", { hostId, lines }),
  remoteReadGatewayErrorLog: (hostId: string, lines?: number): Promise<string> =>
    invoke("remote_read_gateway_error_log", { hostId, lines }),
};
