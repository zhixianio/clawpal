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
  options?: Array<{ value: string; label: string }>;
}

export interface RecipeStep {
  action: string;
  label: string;
  args: Record<string, unknown>;
}

export interface RecipePresentation {
  resultSummary?: string;
}

export interface Recipe {
  id: string;
  name: string;
  description: string;
  version: string;
  tags: string[];
  difficulty: "easy" | "normal" | "advanced";
  presentation?: RecipePresentation;
  params: RecipeParam[];
  steps: RecipeStep[];
}

export interface RecipeWorkspaceEntry {
  slug: string;
  path: string;
  recipeId?: string;
  version?: string;
  sourceKind?: "bundled" | "localImport" | "remoteUrl";
  bundledVersion?: string;
  bundledState?:
    | "missing"
    | "upToDate"
    | "updateAvailable"
    | "localModified"
    | "conflictedUpdate";
  trustLevel: "trusted" | "caution" | "untrusted";
  riskLevel: "low" | "medium" | "high";
  approvalRequired: boolean;
}

export interface RecipeActionCatalogEntry {
  kind: string;
  title: string;
  group: string;
  category: string;
  backend: string;
  description: string;
  readOnly: boolean;
  interactive: boolean;
  runnerSupported: boolean;
  recommended: boolean;
  cliCommand?: string;
  legacyAliasOf?: string;
  capabilities: string[];
  resourceKinds: string[];
}

export interface RecipeSourceSaveResult {
  slug: string;
  path: string;
}

export interface ImportedRecipe {
  slug: string;
  recipeId: string;
  path: string;
}

export interface SkippedRecipeImport {
  recipeDir: string;
  reason: string;
}

export interface RecipeLibraryImportResult {
  imported: ImportedRecipe[];
  skipped: SkippedRecipeImport[];
  warnings: string[];
}

export type RecipeImportSourceKind =
  | "localFile"
  | "localRecipeDirectory"
  | "localRecipeLibrary"
  | "remoteUrl";

export interface RecipeImportConflict {
  slug: string;
  recipeId: string;
  path: string;
}

export interface SkippedRecipeSourceImport {
  source: string;
  reason: string;
}

export interface RecipeSourceImportResult {
  sourceKind?: RecipeImportSourceKind | null;
  imported: ImportedRecipe[];
  skipped: SkippedRecipeSourceImport[];
  warnings: string[];
  conflicts: RecipeImportConflict[];
}

export interface RecipeSourceDiagnostic {
  category: string;
  severity: string;
  recipeId?: string;
  path?: string;
  message: string;
}

export interface RecipeSourceDiagnostics {
  errors: RecipeSourceDiagnostic[];
  warnings: RecipeSourceDiagnostic[];
}

export type RecipeEditorOrigin = "builtin" | "workspace" | "external";

export interface RecipeStudioDraft {
  recipeId: string;
  recipeName: string;
  source: string;
  origin: RecipeEditorOrigin;
  workspaceSlug?: string;
}

export interface RecipeEditorActionRow {
  kind: string;
  name: string;
  argsText: string;
}

export interface RecipeEditorModel {
  id: string;
  name: string;
  description: string;
  version: string;
  tagsText: string;
  difficulty: Recipe["difficulty"];
  params: RecipeParam[];
  steps: RecipeStep[];
  actionRows: RecipeEditorActionRow[];
  bundleCapabilities: string[];
  bundleResources: string[];
  executionKind: RecipeExecutionKind;
  sourceDocument: unknown;
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

export type RecipeExecutionKind = "job" | "service" | "schedule" | "attachment";

export interface RecipeBundle {
  apiVersion: string;
  kind: string;
  metadata: {
    name?: string;
    version?: string;
    description?: string;
  };
  compatibility: {
    minRunnerVersion?: string;
    targetPlatforms?: string[];
  };
  inputs: Record<string, unknown>[];
  capabilities: {
    allowed: string[];
  };
  resources: {
    supportedKinds: string[];
  };
  execution: {
    supportedKinds: RecipeExecutionKind[];
  };
  runner: {
    name?: string;
    version?: string;
  };
  outputs: Record<string, unknown>[];
}

export interface ExecutionResourceClaim {
  kind: string;
  id?: string;
  target?: string;
  path?: string;
}

export interface ExecutionSecretBinding {
  id: string;
  source: string;
  mount?: string;
}

export interface ExecutionSpec {
  apiVersion: string;
  kind: string;
  metadata: {
    name?: string;
    digest?: string;
  };
  source: Record<string, unknown>;
  target: Record<string, unknown>;
  execution: {
    kind: RecipeExecutionKind;
  };
  capabilities: {
    usedCapabilities: string[];
  };
  resources: {
    claims: ExecutionResourceClaim[];
  };
  secrets: {
    bindings: ExecutionSecretBinding[];
  };
  desiredState: Record<string, unknown>;
  actions: Record<string, unknown>[];
  outputs: Record<string, unknown>[];
}

export interface RecipePlanSummary {
  recipeId: string;
  recipeName: string;
  executionKind: RecipeExecutionKind;
  actionCount: number;
  skippedStepCount: number;
}

export interface RecipePlan {
  summary: RecipePlanSummary;
  usedCapabilities: string[];
  concreteClaims: ExecutionResourceClaim[];
  executionSpecDigest: string;
  executionSpec: ExecutionSpec;
  warnings: string[];
}

export type RecipeSourceOrigin = "saved" | "draft";

export interface ExecuteRecipeRequest {
  spec: ExecutionSpec;
  sourceOrigin?: RecipeSourceOrigin;
  sourceText?: string;
  workspaceSlug?: string;
}

export interface ExecuteRecipeResult {
  runId: string;
  instanceId: string;
  summary: string;
  warnings: string[];
}

export interface RecipeRuntimeArtifact {
  id: string;
  kind: string;
  label: string;
  path?: string;
}

export interface RecipeRuntimeRun {
  id: string;
  instanceId: string;
  recipeId: string;
  executionKind: string;
  runner: string;
  status: string;
  summary: string;
  startedAt: string;
  finishedAt?: string;
  artifacts: RecipeRuntimeArtifact[];
  resourceClaims: ExecutionResourceClaim[];
  warnings: string[];
  sourceOrigin?: string;
  sourceDigest?: string;
  workspacePath?: string;
}

export interface RecipeRuntimeInstance {
  id: string;
  recipeId: string;
  executionKind: string;
  runner: string;
  status: string;
  lastRunId?: string;
  updatedAt: string;
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

export interface ResolvedApiKey {
  profileId: string;
  maskedKey: string;
  credentialKind?: "oauth" | "env_ref" | "manual" | "unset";
  authRef?: string | null;
  resolved?: boolean;
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

export interface ProfilePushResult {
  requestedProfiles: number;
  pushedProfiles: number;
  writtenModelEntries: number;
  writtenAuthEntries: number;
  blockedProfiles: number;
}

export interface RelatedSecretPushResult {
  totalRelatedProviders: number;
  resolvedSecrets: number;
  writtenSecrets: number;
  skippedProviders: number;
  failedProviders: number;
}

export interface AppPreferences {
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

export type BugReportBackend = "sentry";
export type BugReportSeverity = "info" | "warn" | "error" | "critical";

export interface BugReportSettings {
  enabled: boolean;
  backend: BugReportBackend;
  endpoint: string | null;
  severityThreshold: BugReportSeverity;
  maxReportsPerHour: number;
}

export interface BugReportStats {
  sessionId: string;
  totalSent: number;
  sentLastHour: number;
  droppedRateLimited: number;
  sendFailures: number;
  lastSentAt: string | null;
  persistedPending: number;
  deadLetterCount: number;
}

export interface HistoryItem {
  id: string;
  recipeId?: string;
  createdAt: string;
  source: string;
  canRollback: boolean;
  runId?: string;
  rollbackOf?: string;
  artifacts?: RecipeRuntimeArtifact[];
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
  sshDiagnostic?: SshDiagnosticReport | null;
}

export type SshConnectionQuality = "excellent" | "good" | "fair" | "poor" | "unknown";
export type SshConnectionBottleneckStage = "connect" | "gateway" | "config" | "agents" | "version" | "other";
export type SshConnectionProbeStatus = "success" | "failed" | "interactive_required";
export type SshConnectionStageKey = "connect" | "gateway" | "config" | "agents" | "version";
export type SshConnectionStageStatus = "ok" | "failed" | "not_run" | "reused" | "interactive_required";
export type SshConnectionProbePhase = "start" | "success" | "failed" | "reused" | "interactive_required" | "completed";

export interface SshConnectionStageMetric {
  key: SshConnectionStageKey;
  latencyMs: number;
  status: SshConnectionStageStatus;
  note?: string | null;
}

export interface SshProbeProgressEvent {
  hostId: string;
  requestId: string;
  stage: SshConnectionStageKey;
  phase: SshConnectionProbePhase;
  latencyMs?: number | null;
  note?: string | null;
}

export interface SshConnectionProfile {
  probeStatus?: SshConnectionProbeStatus;
  reusedExistingConnection?: boolean;
  status: InstanceStatus;
  connectLatencyMs: number;
  gatewayLatencyMs: number;
  configLatencyMs: number;
  agentsLatencyMs?: number;
  versionLatencyMs: number;
  totalLatencyMs: number;
  quality: SshConnectionQuality;
  qualityScore: number;
  bottleneck: {
    stage: SshConnectionBottleneckStage;
    latencyMs: number;
  };
  stages?: SshConnectionStageMetric[];
}

export interface StatusExtra {
  openclawVersion?: string;
  duplicateInstalls?: string[];
}

export interface InstanceConfigSnapshot {
  globalDefaultModel?: string;
  fallbackModels: string[];
  agents: AgentOverview[];
}

export interface InstanceRuntimeSnapshot {
  status: InstanceStatus;
  agents: AgentOverview[];
  globalDefaultModel?: string;
  fallbackModels: string[];
}

export interface ChannelsConfigSnapshot {
  channels: ChannelNode[];
  bindings: Binding[];
}

export interface ChannelsRuntimeSnapshot {
  channels: ChannelNode[];
  bindings: Binding[];
  agents: AgentOverview[];
}

export interface CronConfigSnapshot {
  jobs: CronJob[];
}

export interface CronRuntimeSnapshot {
  jobs: CronJob[];
  watchdog: WatchdogStatus & { alive: boolean; deployed: boolean };
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

export type SshStage =
  | "resolveHostConfig"
  | "tcpReachability"
  | "hostKeyVerification"
  | "authNegotiation"
  | "sessionOpen"
  | "remoteExec"
  | "sftpRead"
  | "sftpWrite"
  | "sftpRemove";

export type SshIntent =
  | "connect"
  | "exec"
  | "sftp_read"
  | "sftp_write"
  | "sftp_remove"
  | "install_step"
  | "doctor_remote"
  | "health_check";

export type SshDiagnosticStatus = "ok" | "degraded" | "failed";

export type SshErrorCode =
  | "SSH_HOST_UNREACHABLE"
  | "SSH_CONNECTION_REFUSED"
  | "SSH_TIMEOUT"
  | "SSH_HOST_KEY_FAILED"
  | "SSH_KEYFILE_MISSING"
  | "SSH_PASSPHRASE_REQUIRED"
  | "SSH_AUTH_FAILED"
  | "SSH_REMOTE_COMMAND_FAILED"
  | "SSH_SFTP_PERMISSION_DENIED"
  | "SSH_SESSION_STALE"
  | "SSH_UNKNOWN";

export type SshRepairAction =
  | "promptPassphrase"
  | "retryWithBackoff"
  | "switchAuthMethodToSshConfig"
  | "suggestKnownHostsBootstrap"
  | "suggestAuthorizedKeysCheck"
  | "suggestPortHostValidation"
  | "reconnectSession";

export interface SshEvidence {
  kind: string;
  value: string;
}

export interface SshDiagnosticReport {
  stage: SshStage;
  intent: SshIntent;
  status: SshDiagnosticStatus;
  errorCode?: SshErrorCode | null;
  summary: string;
  evidence: SshEvidence[];
  repairPlan: SshRepairAction[];
  confidence: number;
}

export interface SshCommandError {
  message: string;
  diagnostic: SshDiagnosticReport;
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
export type RescueBotRuntimeState =
  | "unconfigured"
  | "configured_inactive"
  | "active"
  | "checking"
  | "error";

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
  configured: boolean;
  active: boolean;
  runtimeState: RescueBotRuntimeState;
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

export interface RescueDocHypothesis {
  title: string;
  reason: string;
  score: number;
}

export interface RescueDocCitation {
  url: string;
  section: string;
}

export interface RescuePrimarySummary {
  status: "healthy" | "degraded" | "broken" | "inactive";
  headline: string;
  recommendedAction: string;
  fixableIssueCount: number;
  selectedFixIssueIds: string[];
  rootCauseHypotheses?: RescueDocHypothesis[];
  fixSteps?: string[];
  confidence?: number;
  citations?: RescueDocCitation[];
  versionAwareness?: string;
}

export interface RescuePrimarySectionItem {
  id: string;
  label: string;
  status: "ok" | "warn" | "error" | "info" | "inactive";
  detail: string;
  autoFixable: boolean;
  issueId?: string | null;
}

export interface RescuePrimarySectionResult {
  key: "gateway" | "models" | "tools" | "agents" | "channels";
  title: string;
  status: "healthy" | "degraded" | "broken" | "inactive";
  summary: string;
  docsUrl: string;
  items: RescuePrimarySectionItem[];
  rootCauseHypotheses?: RescueDocHypothesis[];
  fixSteps?: string[];
  confidence?: number;
  citations?: RescueDocCitation[];
  versionAwareness?: string;
}

export interface RescuePrimaryDiagnosisResult {
  status: "healthy" | "degraded" | "broken" | "inactive";
  checkedAt: string;
  targetProfile: string;
  rescueProfile: string;
  rescueConfigured: boolean;
  rescuePort?: number;
  summary: RescuePrimarySummary;
  sections: RescuePrimarySectionResult[];
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

export interface RescuePrimaryPendingAction {
  kind: "tempProviderSetup";
  reason: string;
  tempProviderProfileId?: string | null;
}

export interface RescuePrimaryRepairResult {
  status: "completed" | "needsTempProviderSetup";
  attemptedAt: string;
  targetProfile: string;
  rescueProfile: string;
  selectedIssueIds: string[];
  appliedIssueIds: string[];
  skippedIssueIds: string[];
  failedIssueIds: string[];
  pendingAction?: RescuePrimaryPendingAction | null;
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
  warnings: string[];
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
  ssh_diagnostic?: SshDiagnosticReport | null;
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
