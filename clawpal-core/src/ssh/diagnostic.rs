use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum SshStage {
    ResolveHostConfig,
    TcpReachability,
    HostKeyVerification,
    AuthNegotiation,
    SessionOpen,
    RemoteExec,
    SftpRead,
    SftpWrite,
    SftpRemove,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SshIntent {
    Connect,
    Exec,
    SftpRead,
    SftpWrite,
    SftpRemove,
    InstallStep,
    DoctorRemote,
    HealthCheck,
}

impl SshIntent {
    pub fn from_str(raw: &str) -> Option<Self> {
        let normalized = raw.trim().to_ascii_lowercase();
        match normalized.as_str() {
            "connect" => Some(Self::Connect),
            "exec" => Some(Self::Exec),
            "sftp_read" => Some(Self::SftpRead),
            "sftp_write" => Some(Self::SftpWrite),
            "sftp_remove" => Some(Self::SftpRemove),
            "install_step" => Some(Self::InstallStep),
            "doctor_remote" => Some(Self::DoctorRemote),
            "health_check" => Some(Self::HealthCheck),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SshDiagnosticStatus {
    Ok,
    Degraded,
    Failed,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum SshErrorCode {
    HostUnreachable,
    ConnectionRefused,
    Timeout,
    HostKeyFailed,
    KeyfileMissing,
    PassphraseRequired,
    AuthFailed,
    RemoteCommandFailed,
    SftpPermissionDenied,
    SessionStale,
    Unknown,
}

impl SshErrorCode {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::HostUnreachable => "SSH_HOST_UNREACHABLE",
            Self::ConnectionRefused => "SSH_CONNECTION_REFUSED",
            Self::Timeout => "SSH_TIMEOUT",
            Self::HostKeyFailed => "SSH_HOST_KEY_FAILED",
            Self::KeyfileMissing => "SSH_KEYFILE_MISSING",
            Self::PassphraseRequired => "SSH_PASSPHRASE_REQUIRED",
            Self::AuthFailed => "SSH_AUTH_FAILED",
            Self::RemoteCommandFailed => "SSH_REMOTE_COMMAND_FAILED",
            Self::SftpPermissionDenied => "SSH_SFTP_PERMISSION_DENIED",
            Self::SessionStale => "SSH_SESSION_STALE",
            Self::Unknown => "SSH_UNKNOWN",
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum SshRepairAction {
    PromptPassphrase,
    RetryWithBackoff,
    SwitchAuthMethodToSshConfig,
    SuggestKnownHostsBootstrap,
    SuggestAuthorizedKeysCheck,
    SuggestPortHostValidation,
    ReconnectSession,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct SshEvidence {
    pub kind: String,
    pub value: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct SshDiagnosticReport {
    pub stage: SshStage,
    pub intent: SshIntent,
    pub status: SshDiagnosticStatus,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error_code: Option<SshErrorCode>,
    pub summary: String,
    pub evidence: Vec<SshEvidence>,
    pub repair_plan: Vec<SshRepairAction>,
    pub confidence: f32,
}

impl SshDiagnosticReport {
    pub fn success(stage: SshStage, intent: SshIntent, summary: impl Into<String>) -> Self {
        Self {
            stage,
            intent,
            status: SshDiagnosticStatus::Ok,
            error_code: None,
            summary: summary.into(),
            evidence: Vec::new(),
            repair_plan: Vec::new(),
            confidence: 1.0,
        }
    }
}

pub fn from_any_error(stage: SshStage, intent: SshIntent, raw_error: impl Into<String>) -> SshDiagnosticReport {
    let raw = raw_error.into();
    let lowered = raw.to_ascii_lowercase();
    let (error_code, confidence) = classify_error_code(stage, &lowered);
    let repair_plan = repair_plan_for_error(error_code);
    let summary = match error_code {
        SshErrorCode::HostUnreachable => "SSH target host is unreachable or cannot be resolved",
        SshErrorCode::ConnectionRefused => "SSH connection was refused by target host",
        SshErrorCode::Timeout => "SSH operation timed out",
        SshErrorCode::HostKeyFailed => "SSH host key verification failed",
        SshErrorCode::KeyfileMissing => "SSH private key file is missing or unreadable",
        SshErrorCode::PassphraseRequired => "SSH key passphrase is required",
        SshErrorCode::AuthFailed => "SSH authentication failed",
        SshErrorCode::RemoteCommandFailed => "Remote SSH command failed",
        SshErrorCode::SftpPermissionDenied => "SFTP access denied by remote host",
        SshErrorCode::SessionStale => "SSH session became stale and needs reconnection",
        SshErrorCode::Unknown => "Unknown SSH failure",
    };
    SshDiagnosticReport {
        stage,
        intent,
        status: SshDiagnosticStatus::Failed,
        error_code: Some(error_code),
        summary: summary.to_string(),
        evidence: vec![SshEvidence {
            kind: "rawError".to_string(),
            value: raw,
        }],
        repair_plan,
        confidence,
    }
}

fn classify_error_code(stage: SshStage, lowered: &str) -> (SshErrorCode, f32) {
    if looks_like_host_unreachable(lowered) {
        return (SshErrorCode::HostUnreachable, 0.96);
    }
    if lowered.contains("connection refused") {
        return (SshErrorCode::ConnectionRefused, 0.97);
    }
    if looks_like_timeout(lowered) {
        return (SshErrorCode::Timeout, 0.93);
    }
    if looks_like_host_key_failure(lowered) {
        return (SshErrorCode::HostKeyFailed, 0.97);
    }
    if looks_like_keyfile_missing(lowered) {
        return (SshErrorCode::KeyfileMissing, 0.92);
    }
    if looks_like_passphrase_required(lowered) {
        return (SshErrorCode::PassphraseRequired, 0.94);
    }
    if looks_like_sftp_permission_denied(stage, lowered) {
        return (SshErrorCode::SftpPermissionDenied, 0.96);
    }
    if looks_like_session_stale(lowered) {
        return (SshErrorCode::SessionStale, 0.9);
    }
    if looks_like_auth_failure(lowered) {
        return (SshErrorCode::AuthFailed, 0.9);
    }
    if looks_like_remote_command_failure(stage, lowered) {
        return (SshErrorCode::RemoteCommandFailed, 0.82);
    }
    (SshErrorCode::Unknown, 0.55)
}

fn looks_like_host_unreachable(lowered: &str) -> bool {
    lowered.contains("name or service not known")
        || lowered.contains("nodename nor servname provided")
        || lowered.contains("temporary failure in name resolution")
        || lowered.contains("no address associated with hostname")
        || lowered.contains("failed to lookup address information")
        || lowered.contains("unknown host")
        || lowered.contains("hostname was not found")
        || lowered.contains("getaddrinfo")
        || lowered.contains("host unreachable")
}

fn looks_like_timeout(lowered: &str) -> bool {
    lowered.contains("timed out")
        || lowered.contains("timeout")
        || lowered.contains("connection timed out")
}

fn looks_like_host_key_failure(lowered: &str) -> bool {
    lowered.contains("host key verification failed")
        || lowered.contains("remote host identification has changed")
}

fn looks_like_keyfile_missing(lowered: &str) -> bool {
    let has_not_found = lowered.contains("no such file")
        || lowered.contains("not found")
        || lowered.contains("cannot find")
        || lowered.contains("could not open");
    let has_key_hint = lowered.contains("key")
        || lowered.contains("identityfile")
        || lowered.contains("id_rsa")
        || lowered.contains("id_ed25519");
    has_not_found && has_key_hint
}

fn looks_like_passphrase_required(lowered: &str) -> bool {
    lowered.contains("passphrase")
        || lowered.contains("key is encrypted")
        || lowered.contains("can't open /dev/tty")
        || lowered.contains("agent refused operation")
        || lowered.contains("authentication agent")
        || lowered.contains("sign_and_send_pubkey")
}

fn looks_like_auth_failure(lowered: &str) -> bool {
    lowered.contains("permission denied")
        || lowered.contains("authentication failed")
        || lowered.contains("public key authentication failed")
}

fn looks_like_session_stale(lowered: &str) -> bool {
    lowered.contains("ssh open channel failed")
        || lowered.contains("failed to open channel")
        || lowered.contains("connection reset")
        || lowered.contains("broken pipe")
        || lowered.contains("connection closed")
        || lowered.contains("no connection for id")
}

fn looks_like_remote_command_failure(stage: SshStage, lowered: &str) -> bool {
    matches!(stage, SshStage::RemoteExec | SshStage::SftpRemove)
        && (lowered.contains("command failed")
            || lowered.contains("exit code")
            || lowered.contains("cat exited with code"))
}

fn looks_like_sftp_permission_denied(stage: SshStage, lowered: &str) -> bool {
    matches!(stage, SshStage::SftpRead | SshStage::SftpWrite | SshStage::SftpRemove)
        && lowered.contains("permission denied")
}

fn repair_plan_for_error(code: SshErrorCode) -> Vec<SshRepairAction> {
    match code {
        SshErrorCode::HostUnreachable => vec![
            SshRepairAction::SuggestPortHostValidation,
            SshRepairAction::RetryWithBackoff,
        ],
        SshErrorCode::ConnectionRefused => vec![
            SshRepairAction::SuggestPortHostValidation,
            SshRepairAction::RetryWithBackoff,
        ],
        SshErrorCode::Timeout => vec![
            SshRepairAction::RetryWithBackoff,
            SshRepairAction::SuggestPortHostValidation,
        ],
        SshErrorCode::HostKeyFailed => vec![SshRepairAction::SuggestKnownHostsBootstrap],
        SshErrorCode::KeyfileMissing => vec![SshRepairAction::SwitchAuthMethodToSshConfig],
        SshErrorCode::PassphraseRequired => vec![SshRepairAction::PromptPassphrase],
        SshErrorCode::AuthFailed => vec![
            SshRepairAction::SuggestAuthorizedKeysCheck,
            SshRepairAction::SwitchAuthMethodToSshConfig,
        ],
        SshErrorCode::RemoteCommandFailed => vec![SshRepairAction::ReconnectSession],
        SshErrorCode::SftpPermissionDenied => vec![SshRepairAction::SuggestAuthorizedKeysCheck],
        SshErrorCode::SessionStale => vec![
            SshRepairAction::ReconnectSession,
            SshRepairAction::RetryWithBackoff,
        ],
        SshErrorCode::Unknown => vec![SshRepairAction::RetryWithBackoff],
    }
}

#[cfg(test)]
mod tests {
    use super::{from_any_error, SshErrorCode, SshIntent, SshStage};

    #[test]
    fn classify_host_unreachable() {
        let report = from_any_error(
            SshStage::TcpReachability,
            SshIntent::Connect,
            "failed to lookup address information",
        );
        assert_eq!(report.error_code, Some(SshErrorCode::HostUnreachable));
    }

    #[test]
    fn classify_connection_refused() {
        let report = from_any_error(
            SshStage::TcpReachability,
            SshIntent::Connect,
            "connection refused",
        );
        assert_eq!(report.error_code, Some(SshErrorCode::ConnectionRefused));
    }

    #[test]
    fn classify_timeout() {
        let report = from_any_error(
            SshStage::RemoteExec,
            SshIntent::Exec,
            "operation timed out after 20s",
        );
        assert_eq!(report.error_code, Some(SshErrorCode::Timeout));
    }

    #[test]
    fn classify_host_key_failure() {
        let report = from_any_error(
            SshStage::HostKeyVerification,
            SshIntent::Connect,
            "Host key verification failed",
        );
        assert_eq!(report.error_code, Some(SshErrorCode::HostKeyFailed));
    }

    #[test]
    fn classify_key_missing() {
        let report = from_any_error(
            SshStage::AuthNegotiation,
            SshIntent::Connect,
            "Could not open key file /tmp/id_ed25519: no such file",
        );
        assert_eq!(report.error_code, Some(SshErrorCode::KeyfileMissing));
    }

    #[test]
    fn classify_passphrase_required() {
        let report = from_any_error(
            SshStage::AuthNegotiation,
            SshIntent::Connect,
            "key is encrypted; passphrase required",
        );
        assert_eq!(report.error_code, Some(SshErrorCode::PassphraseRequired));
    }

    #[test]
    fn classify_auth_failed() {
        let report = from_any_error(
            SshStage::AuthNegotiation,
            SshIntent::Connect,
            "permission denied (publickey)",
        );
        assert_eq!(report.error_code, Some(SshErrorCode::AuthFailed));
    }

    #[test]
    fn classify_sftp_permission_denied() {
        let report = from_any_error(
            SshStage::SftpWrite,
            SshIntent::SftpWrite,
            "sftp failed: permission denied",
        );
        assert_eq!(report.error_code, Some(SshErrorCode::SftpPermissionDenied));
    }

    #[test]
    fn classify_session_stale() {
        let report = from_any_error(
            SshStage::SessionOpen,
            SshIntent::Exec,
            "ssh open channel failed: channel closed",
        );
        assert_eq!(report.error_code, Some(SshErrorCode::SessionStale));
    }

    #[test]
    fn classify_remote_command_failed() {
        let report = from_any_error(
            SshStage::RemoteExec,
            SshIntent::Exec,
            "command failed: exit code 127",
        );
        assert_eq!(report.error_code, Some(SshErrorCode::RemoteCommandFailed));
    }
}
