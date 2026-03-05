export type Severity = "low" | "medium" | "high";

export interface ChannelNode {
  path: string;
  channelType: string | null;
  mode: string | null;
  allowlist: string[];
  model: string | null;
  hasModelField: boolean;
  displayName: string | null;
  nameStatus: string | null;
}

export interface DiscordGuildChannel {
  guildId: string;
  guildName: string;
  channelId: string;
  channelName: string;
  defaultAgentId?: string;
}

export interface RecipeParam {
  id: string;
  label: string;
  type: "string" | "number" | "boolean" | "textarea" | "discord_guild" | "discord_channel" | "model_profile" | "agent";
  required: boolean;
  pattern?: string;
  minLength?: number;
  maxLength?: number;
  placeholder?: string;
  dependsOn?: string;
  defaultValue?: string;
}

export interface RecipeStep {
  action: string;
  label: string;
  args: Record<string, unknown>;
}

export interface Recipe {
  id: string;
  name: string;
  description: string;
  version: string;
  tags: string[];
  difficulty: "easy" | "normal" | "advanced";
  params: RecipeParam[];
  steps: RecipeStep[];
}

export interface ChangeItem {
  path: string;
  op: string;
  risk: string;
  reason?: string;
}

export interface PreviewResult {
  recipeId: string;
  diff: string;
  configBefore: string;
  configAfter: string;
  changes: ChangeItem[];
  overwritesExisting: boolean;
  canRollback: boolean;
  impactLevel: string;
  warnings: string[];
}

export interface ApplyResult {
  ok: boolean;
  snapshotId?: string;
  configPath: string;
  backupPath?: string;
  warnings: string[];
  errors: string[];
}

export interface SystemStatus {
  healthy: boolean;
  configPath: string;
  openclawDir: string;
  clawpalDir: string;
  openclawVersion: string;
  activeAgents: number;
  snapshots: number;
  openclawUpdate?: {
    installedVersion: string;
    latestVersion?: string;
    upgradeAvailable: boolean;
    channel?: string;
    details?: string;
    source: string;
    checkedAt: string;
  };
  channels: {
    configuredChannels: number;
    channelModelOverrides: number;
    channelExamples: string[];
  };
  models: {
    globalDefaultModel?: string;
    agentOverrides: string[];
    channelOverrides: string[];
  };
  memory: {
    fileCount: number;
    totalBytes: number;
    files: { path: string; sizeBytes: number }[];
  };
  sessions: {
    totalSessionFiles: number;
    totalArchiveFiles: number;
    totalBytes: number;
    byAgent: { agent: string; sessionFiles: number; archiveFiles: number; totalBytes: number }[];
  };
}

export interface SessionFile {
  path: string;
  relativePath: string;
  agent: string;
  kind: "sessions" | "archive";
  sizeBytes: number;
}

export interface SessionAnalysis {
  agent: string;
  sessionId: string;
  filePath: string;
  sizeBytes: number;
  messageCount: number;
  userMessageCount: number;
  assistantMessageCount: number;
  lastActivity: string | null;
  ageDays: number;
  totalTokens: number;
  model: string | null;
  category: "empty" | "low_value" | "valuable";
  kind: string;
}

export interface AgentSessionAnalysis {
  agent: string;
  totalFiles: number;
  totalSizeBytes: number;
  emptyCount: number;
  lowValueCount: number;
  valuableCount: number;
  sessions: SessionAnalysis[];
}

export interface ModelProfile {
  id: string;
  name: string;
  provider: string;
  model: string;
  authRef: string;
  apiKey?: string;
  baseUrl?: string;
  description?: string;
  enabled: boolean;
}

export interface ModelCatalogModel {
  id: string;
  name?: string;
}

export interface ModelCatalogProvider {
  provider: string;
  baseUrl?: string;
  models: ModelCatalogModel[];
}

export interface ProviderAuthSuggestion {
  authRef: string | null;
  hasKey: boolean;
  source: string;
}

export interface ZeroclawOauthLoginStartResult {
  provider: string;
  profile: string;
  authRef: string;
  authorizeUrl: string;
  details: string;
}

export interface ZeroclawOauthCompleteResult {
  provider: string;
  profile: string;
  authRef: string;
  details: string;
}

export interface ResolvedApiKey {
  profileId: string;
  maskedKey: string;
}

export interface RemoteAuthSyncResult {
  totalRemoteProfiles: number;
  syncedProfiles: number;
  createdProfiles: number;
  updatedProfiles: number;
  resolvedKeys: number;
  unresolvedKeys: number;
  failedKeyResolves: number;
}

export interface RelatedSecretPushResult {
  totalRelatedProviders: number;
  resolvedSecrets: number;
  writtenSecrets: number;
  skippedProviders: number;
  failedProviders: number;
}

export interface AppPreferences {
  zeroclawModel: string | null;
  showZeroclawDoctorUi: boolean;
  showRescueBotUi: boolean;
  showSshTransferSpeedUi: boolean;
}

export interface SshTransferStats {
  hostId: string;
  uploadBytesPerSec: number;
  downloadBytesPerSec: number;
  totalUploadBytes: number;
  totalDownloadBytes: number;
  updatedAtMs: number;
}

export interface ZeroclawUsageStats {
  totalCalls: number;
  usageCalls: number;
  promptTokens: number;
  completionTokens: number;
  totalTokens: number;
  lastUpdatedMs: number;
}

export interface ZeroclawRuntimeTarget {
  provider: string | null;
  model: string | null;
  source: "preferred" | "auto" | "provider_only" | "unavailable" | string;
  preferredModel: string | null;
  providerOrder: string[];
}

export interface HistoryItem {
  id: string;
  recipeId?: string;
  createdAt: string;
  source: string;
  canRollback: boolean;
  rollbackOf?: string;
}

export interface DoctorIssue {
  id: string;
  code: string;
  severity: "error" | "warn" | "info";
  message: string;
  autoFixable: boolean;
  fixHint?: string;
}

export interface DoctorReport {
  ok: boolean;
  score: number;
  issues: DoctorIssue[];
}

export interface GuidanceAction {
  label: string;
  actionType: "inline_fix" | "doctor_handoff";
  tool?: string;
  args?: string;
  invokeType?: string;
  context?: string;
}

export interface PrecheckIssue {
  code: string;
  severity: "error" | "warn";
  message: string;
  autoFixable: boolean;
}

export interface AgentOverview {
  id: string;
  name?: string;
  emoji?: string;
  model: string | null;
  channels: string[];
  online: boolean;
  workspace?: string;
}

export interface InstanceStatus {
  healthy: boolean;
  activeAgents: number;
  globalDefaultModel?: string;
  fallbackModels?: string[];
}

export interface StatusExtra {
  openclawVersion?: string;
  duplicateInstalls?: string[];
}

export interface Binding {
  agentId: string;
  match: { channel: string; peer?: { id: string; kind: string } };
}

export interface BackupInfo {
  name: string;
  path: string;
  createdAt: string;
  sizeBytes: number;
}

export interface SshHost {
  id: string;
  label: string;
  host: string;
  port: number;
  username: string;
  authMethod: "key" | "ssh_config" | "password";
  keyPath?: string;
  password?: string;
  passphrase?: string;
}

export interface SshConfigHostSuggestion {
  hostAlias: string;
  hostName?: string;
  user?: string;
  port?: number;
  identityFile?: string;
}

export interface DockerInstance {
  id: string;
  label: string;
  projectDir?: string;
  openclawHome?: string;
  clawpalDataDir?: string;
}

export interface RegisteredInstance {
  id: string;
  instanceType: "local" | "docker" | "remote_ssh";
  label: string;
  openclawHome?: string | null;
  clawpalDataDir?: string | null;
}

export interface DiscoveredInstance {
  id: string;
  instanceType: string;
  label: string;
  homePath: string;
  source: string;
  containerName?: string;
  alreadyRegistered: boolean;
}

export interface SshExecResult {
  stdout: string;
  stderr: string;
  exitCode: number;
}

export interface SftpEntry {
  name: string;
  isDir: boolean;
  size: number;
}

export type RescueBotAction = "set" | "activate" | "status" | "deactivate" | "unset";

export interface RescueBotCommandResult {
  command: string[];
  output: {
    stdout: string;
    stderr: string;
    exitCode: number;
  };
}

export interface RescueBotManageResult {
  action: RescueBotAction;
  profile: string;
  mainPort: number;
  rescuePort: number;
  minRecommendedPort: number;
  wasAlreadyConfigured: boolean;
  commands: RescueBotCommandResult[];
}

export interface RescuePrimaryCheckItem {
  id: string;
  title: string;
  ok: boolean;
  detail: string;
}

export interface RescuePrimaryIssue {
  id: string;
  code: string;
  severity: "error" | "warn" | "info";
  message: string;
  autoFixable: boolean;
  fixHint?: string;
  source: "rescue" | "primary";
}

export interface RescuePrimaryDiagnosisResult {
  status: "healthy" | "degraded" | "broken";
  checkedAt: string;
  targetProfile: string;
  rescueProfile: string;
  rescueConfigured: boolean;
  rescuePort?: number;
  checks: RescuePrimaryCheckItem[];
  issues: RescuePrimaryIssue[];
}

export interface RescuePrimaryRepairStep {
  id: string;
  title: string;
  ok: boolean;
  detail: string;
  command?: string[];
}

export interface RescuePrimaryRepairResult {
  attemptedAt: string;
  targetProfile: string;
  rescueProfile: string;
  selectedIssueIds: string[];
  appliedIssueIds: string[];
  skippedIssueIds: string[];
  failedIssueIds: string[];
  steps: RescuePrimaryRepairStep[];
  before: RescuePrimaryDiagnosisResult;
  after: RescuePrimaryDiagnosisResult;
}

// Cron

export type WatchdogJobStatus = "ok" | "pending" | "triggered" | "retrying" | "escalated";

export interface CronSchedule {
  kind: "cron" | "every" | "at";
  expr?: string;
  tz?: string;
  everyMs?: number;
  at?: string;
}

export interface CronJobState {
  lastRunAtMs?: number;
  lastStatus?: string;
  lastError?: string;
}

export interface CronJobDelivery {
  mode?: string;
  channel?: string;
  to?: string;
}

export interface CronJob {
  jobId: string;
  name: string;
  schedule: CronSchedule;
  sessionTarget: "main" | "isolated";
  agentId?: string;
  enabled: boolean;
  description?: string;
  state?: CronJobState;
  delivery?: CronJobDelivery;
}

export interface CronRun {
  jobId: string;
  startedAt: string;
  endedAt?: string;
  outcome: string;
  error?: string;
  ts?: number;
  runAtMs?: number;
  durationMs?: number;
  summary?: string;
}

export interface WatchdogJobState {
  status: WatchdogJobStatus;
  lastScheduledAt?: string;
  lastRunAt?: string | null;
  retries: number;
  lastError?: string;
  escalatedAt?: string;
}

export interface WatchdogStatus {
  pid: number;
  startedAt: string;
  lastCheckAt: string;
  gatewayHealthy: boolean;
  jobs: Record<string, WatchdogJobState>;
}

// Command Queue

export interface PendingCommand {
  id: string;
  label: string;
  command: string[];
  createdAt: string;
}

export interface PreviewQueueResult {
  commands: PendingCommand[];
  configBefore: string;
  configAfter: string;
  errors: string[];
}

// Doctor Agent

export interface DoctorInvoke {
  id: string;
  command: string;
  args: Record<string, unknown>;
  type: "read" | "write";
}

export interface DiagnosisCitation {
  url: string;
  section?: string;
}

export interface DiagnosisReportItem {
  problem: string;
  severity: "error" | "warn" | "info";
  fix_options: string[];
  root_cause_hypothesis?: string;
  fix_steps?: string[];
  confidence?: number;
  citations?: DiagnosisCitation[];
  version_awareness?: string;
  action?: { tool: string; args: string; instance?: string; reason?: string };
}

export interface DoctorChatMessage {
  id: string;
  role: "assistant" | "user" | "tool-call" | "tool-result";
  content: string;
  invoke?: DoctorInvoke;
  invokeResult?: unknown;
  invokeId?: string;
  status?: "pending" | "approved" | "rejected" | "auto";
  diagnosisReport?: { items: DiagnosisReportItem[] };
  /** Epoch milliseconds when the message was created. */
  timestamp?: number;
}

export interface ApplyQueueResult {
  ok: boolean;
  appliedCount: number;
  totalCount: number;
  error: string | null;
  rolledBack: boolean;
}

export type InstallMethod = "local" | "wsl2" | "docker" | "remote_ssh";

export type InstallState =
  | "idle"
  | "selected_method"
  | "precheck_running"
  | "precheck_failed"
  | "precheck_passed"
  | "install_running"
  | "install_failed"
  | "install_passed"
  | "init_running"
  | "init_failed"
  | "init_passed"
  | "verify_running"
  | "verify_failed"
  | "ready";

export type InstallStep = "precheck" | "install" | "init" | "verify";

export interface InstallLogEntry {
  at: string;
  level: string;
  message: string;
}

export interface InstallSession {
  id: string;
  method: InstallMethod;
  state: InstallState;
  current_step: InstallStep | null;
  logs: InstallLogEntry[];
  artifacts: Record<string, unknown>;
  created_at: string;
  updated_at: string;
}

export interface InstallStepResult {
  ok: boolean;
  summary: string;
  details: string;
  commands: string[];
  artifacts: Record<string, unknown>;
  next_step: string | null;
  error_code: string | null;
}

export interface InstallMethodCapability {
  method: InstallMethod;
  available: boolean;
  hint: string | null;
}

export interface InstallOrchestratorDecision {
  step: string | null;
  reason: string;
  source: string;
  errorCode?: string | null;
  actionHint?: string | null;
}

export interface InstallUiAction {
  id: string;
  kind: string;
  label: string;
  payload?: Record<string, unknown>;
}

export interface InstallTargetDecision {
  method: InstallMethod | null;
  reason: string;
  source: string;
  requiresSshHost: boolean;
  requiredFields?: string[];
  uiActions?: InstallUiAction[];
  errorCode?: string | null;
  actionHint?: string | null;
}

export interface EnsureAccessResult {
  instanceId: string;
  transport: string;
  workingChain: string[];
  usedLegacyFallback: boolean;
  profileReused: boolean;
}

export interface RecordInstallExperienceResult {
  saved: boolean;
  totalCount: number;
}
