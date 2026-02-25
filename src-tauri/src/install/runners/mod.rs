use crate::install::types::{InstallMethod, InstallStep};
use serde_json::Value;
use std::collections::HashMap;
use std::process::Command;

pub mod docker;
pub mod local;
pub mod remote_ssh;
pub mod wsl2;

#[derive(Clone, Debug)]
pub struct RunnerOutput {
    pub summary: String,
    pub details: String,
    pub commands: Vec<String>,
    pub artifacts: HashMap<String, Value>,
}

#[derive(Clone, Debug)]
pub struct RunnerFailure {
    pub error_code: String,
    pub summary: String,
    pub details: String,
    pub commands: Vec<String>,
}

#[derive(Clone, Debug)]
pub struct CommandResult {
    pub command_line: String,
    pub stdout: String,
    pub stderr: String,
    pub exit_code: i32,
}

fn format_command(program: &str, args: &[&str]) -> String {
    if args.is_empty() {
        return program.to_string();
    }
    format!("{} {}", program, args.join(" "))
}

pub fn classify_error_code(raw: &str) -> String {
    let msg = raw.to_lowercase();
    if msg.contains("not found") || msg.contains("no such file") {
        return "env_missing".to_string();
    }
    if msg.contains("permission denied") || msg.contains("operation not permitted") {
        return "permission_denied".to_string();
    }
    if msg.contains("timed out")
        || msg.contains("temporary failure")
        || msg.contains("could not resolve")
        || msg.contains("failed to connect")
        || msg.contains("network")
    {
        return "network_error".to_string();
    }
    "command_failed".to_string()
}

pub fn run_command(program: &str, args: &[&str]) -> Result<CommandResult, RunnerFailure> {
    let command_line = format_command(program, args);
    let output = Command::new(program)
        .args(args)
        .output()
        .map_err(|e| RunnerFailure {
            error_code: classify_error_code(&e.to_string()),
            summary: format!("Command execution failed: {program}"),
            details: e.to_string(),
            commands: vec![command_line.clone()],
        })?;

    let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
    let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
    let exit_code = output.status.code().unwrap_or(-1);

    if exit_code != 0 {
        let details = if stderr.is_empty() {
            format!("exit code {exit_code}: {stdout}")
        } else {
            format!("exit code {exit_code}: {stderr}")
        };
        return Err(RunnerFailure {
            error_code: classify_error_code(&details),
            summary: format!("Command failed: {program}"),
            details,
            commands: vec![command_line],
        });
    }

    Ok(CommandResult {
        command_line,
        stdout,
        stderr,
        exit_code,
    })
}

pub fn run_step(
    method: &InstallMethod,
    step: &InstallStep,
    artifacts: &HashMap<String, Value>,
) -> Result<RunnerOutput, RunnerFailure> {
    match method {
        InstallMethod::Local => local::run_step(step, artifacts),
        InstallMethod::Wsl2 => wsl2::run_step(step, artifacts),
        InstallMethod::Docker => docker::run_step(step, artifacts),
        InstallMethod::RemoteSsh => Err(RunnerFailure {
            error_code: "validation_failed".to_string(),
            summary: "remote runner requires ssh connection pool".to_string(),
            details: "Use async remote runner path".to_string(),
            commands: vec![],
        }),
    }
}
