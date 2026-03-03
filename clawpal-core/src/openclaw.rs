use std::collections::HashMap;
use std::path::Path;
use std::process::Command;
use std::sync::OnceLock;

use serde::{Deserialize, Serialize};
use serde_json::Value;
use thiserror::Error;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CliOutput {
    pub stdout: String,
    pub stderr: String,
    pub exit_code: i32,
}

#[derive(Debug, Clone)]
pub struct OpenclawCli {
    bin: String,
}

#[derive(Debug, Error)]
pub enum OpenclawError {
    #[error("failed to run openclaw: {0}")]
    Spawn(#[from] std::io::Error),
    #[error("openclaw command failed ({exit_code}): {details}")]
    CommandFailed { exit_code: i32, details: String },
    #[error("no json found in output: {0}")]
    NoJson(String),
    #[error("failed to parse json: {0}")]
    ParseJson(#[from] serde_json::Error),
}

pub type Result<T> = std::result::Result<T, OpenclawError>;

pub fn resolve_openclaw_bin() -> &'static str {
    static BIN: OnceLock<String> = OnceLock::new();
    BIN.get_or_init(|| {
        if find_in_path("openclaw") {
            return "openclaw".to_string();
        }

        let home = std::env::var("HOME").unwrap_or_default();
        let candidates = [
            "/opt/homebrew/bin/openclaw".to_string(),
            "/usr/local/bin/openclaw".to_string(),
            format!("{home}/.npm-global/bin/openclaw"),
            format!("{home}/.local/bin/openclaw"),
        ];

        let nvm_dir = std::env::var("NVM_DIR").unwrap_or_else(|_| format!("{home}/.nvm"));
        let nvm_pattern = format!("{nvm_dir}/versions/node");
        let mut nvm_candidates = Vec::new();
        if let Ok(entries) = std::fs::read_dir(&nvm_pattern) {
            for entry in entries.flatten() {
                let path = entry.path().join("bin/openclaw");
                if path.exists() {
                    nvm_candidates.push(path.to_string_lossy().to_string());
                }
            }
        }

        for candidate in candidates.iter().chain(nvm_candidates.iter()) {
            if Path::new(candidate).is_file() {
                if let Some(dir) = Path::new(candidate).parent() {
                    if let Ok(current_path) = std::env::var("PATH") {
                        let dir_str = dir.to_string_lossy();
                        let already_in_path = std::env::split_paths(&current_path)
                            .any(|path| path == Path::new(dir_str.as_ref()));
                        if !already_in_path {
                            std::env::set_var("PATH", format!("{dir_str}:{current_path}"));
                        }
                    }
                }
                return candidate.clone();
            }
        }

        "openclaw".to_string()
    })
}

impl OpenclawCli {
    pub fn new() -> Self {
        Self {
            bin: resolve_openclaw_bin().to_string(),
        }
    }

    pub fn with_bin(bin: impl Into<String>) -> Self {
        Self { bin: bin.into() }
    }

    pub fn run(&self, args: &[&str]) -> Result<CliOutput> {
        self.run_with_env(args, None)
    }

    pub fn run_with_env(
        &self,
        args: &[&str],
        env: Option<&HashMap<String, String>>,
    ) -> Result<CliOutput> {
        let mut cmd = Command::new(&self.bin);
        cmd.args(args)
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped());

        if let Some(env_vars) = env {
            for (key, value) in env_vars {
                cmd.env(key, value);
            }
        }

        let output = cmd.output()?;
        Ok(CliOutput {
            stdout: String::from_utf8_lossy(&output.stdout)
                .trim_end()
                .to_string(),
            stderr: String::from_utf8_lossy(&output.stderr)
                .trim_end()
                .to_string(),
            exit_code: output.status.code().unwrap_or(-1),
        })
    }
}

impl Default for OpenclawCli {
    fn default() -> Self {
        Self::new()
    }
}

pub fn parse_json_output(output: &CliOutput) -> Result<Value> {
    if output.exit_code != 0 {
        let details = if !output.stderr.is_empty() {
            output.stderr.clone()
        } else {
            output.stdout.clone()
        };
        return Err(OpenclawError::CommandFailed {
            exit_code: output.exit_code,
            details,
        });
    }

    let raw = &output.stdout;
    let last_brace = raw.rfind('}');
    let last_bracket = raw.rfind(']');
    let end = match (last_brace, last_bracket) {
        (Some(a), Some(b)) => Some(a.max(b)),
        (Some(a), None) => Some(a),
        (None, Some(b)) => Some(b),
        (None, None) => None,
    };
    let start = match end {
        Some(e) => {
            let closer = raw.as_bytes()[e];
            let opener = if closer == b']' { b'[' } else { b'{' };
            let mut depth = 0i32;
            let mut pos = None;
            for i in (0..=e).rev() {
                let ch = raw.as_bytes()[i];
                if ch == closer {
                    depth += 1;
                } else if ch == opener {
                    depth -= 1;
                }
                if depth == 0 {
                    pos = Some(i);
                    break;
                }
            }
            pos
        }
        None => None,
    };

    let start = start.ok_or_else(|| OpenclawError::NoJson(raw.to_string()))?;
    let end = end.expect("end exists when start exists");
    let json_str = &raw[start..=end];
    Ok(serde_json::from_str(json_str)?)
}

fn find_in_path(bin: &str) -> bool {
    std::env::var_os("PATH")
        .map(|paths| std::env::split_paths(&paths).any(|dir| dir.join(bin).is_file()))
        .unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use uuid::Uuid;

    #[cfg(unix)]
    fn create_fake_openclaw_script(body: &str) -> String {
        use std::io::Write;
        use std::os::unix::fs::PermissionsExt;

        let dir =
            std::env::temp_dir().join(format!("clawpal-core-openclaw-test-{}", Uuid::new_v4()));
        fs::create_dir_all(&dir).expect("create temp dir");
        let path = dir.join("fake-openclaw.sh");
        // Open → write → fsync → close explicitly to avoid ETXTBSY on exec.
        {
            let mut f = fs::File::create(&path).expect("create script");
            f.write_all(body.as_bytes()).expect("write script");
            f.sync_all().expect("sync script");
        }
        fs::set_permissions(&path, fs::Permissions::from_mode(0o755)).expect("chmod");
        path.to_string_lossy().to_string()
    }

    #[test]
    fn resolve_openclaw_bin_returns_non_empty_path() {
        assert!(!resolve_openclaw_bin().is_empty());
    }

    #[test]
    #[cfg(unix)]
    fn run_executes_binary_and_returns_output() {
        let bin = create_fake_openclaw_script("#!/bin/sh\necho '{\"ok\":true}'\n");
        let cli = OpenclawCli::with_bin(bin);
        let output = cli.run(&["status"]).expect("run");
        assert_eq!(output.exit_code, 0);
        assert!(output.stdout.contains("\"ok\":true"));
    }

    #[test]
    #[cfg(unix)]
    fn run_with_env_passes_environment_variables() {
        let bin = create_fake_openclaw_script("#!/bin/sh\necho \"$CLAWPAL_TEST_ENV\"\n");
        let cli = OpenclawCli::with_bin(bin);
        let mut env = HashMap::new();
        env.insert("CLAWPAL_TEST_ENV".to_string(), "hello".to_string());
        let output = cli.run_with_env(&[], Some(&env)).expect("run_with_env");
        assert_eq!(output.stdout, "hello");
    }

    #[test]
    fn parse_json_output_extracts_payload_with_noise() {
        let output = CliOutput {
            stdout: "warn line\n{\"a\":1}".to_string(),
            stderr: String::new(),
            exit_code: 0,
        };
        let value = parse_json_output(&output).expect("parse");
        assert_eq!(value["a"], 1);
    }
}
