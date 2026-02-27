use std::path::PathBuf;
use std::process::Command;
use std::{
    collections::hash_map::DefaultHasher,
    hash::{Hash, Hasher},
};

use crate::models::{resolve_paths, OpenClawPaths};

use super::sanitize::sanitize_output;

fn sanitize_instance_namespace(raw: &str) -> String {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return "unknown-0000000000000000".to_string();
    }

    let mut normalized = String::with_capacity(trimmed.len());
    let mut last_underscore = false;
    for ch in trimmed.chars() {
        let mapped = if ch.is_ascii_alphanumeric() {
            ch.to_ascii_lowercase()
        } else if ch == '-' || ch == '_' {
            ch
        } else {
            '_'
        };
        if mapped == '_' {
            if !last_underscore {
                normalized.push('_');
                last_underscore = true;
            }
        } else {
            normalized.push(mapped);
            last_underscore = false;
        }
    }

    let mut base = normalized.trim_matches('_').to_string();
    if base.is_empty() {
        base = "unknown".to_string();
    }
    if base.len() > 48 {
        base.truncate(48);
    }

    let mut hasher = DefaultHasher::new();
    trimmed.hash(&mut hasher);
    let suffix = hasher.finish();
    format!("{base}-{suffix:016x}")
}

fn doctor_sidecar_config_dir(instance_id: &str, session_scope: &str) -> Result<PathBuf, String> {
    let bucket = sanitize_instance_namespace(&format!("{instance_id}::{session_scope}"));
    let dir = resolve_paths()
        .clawpal_dir
        .join("zeroclaw-sidecar")
        .join("instances")
        .join(bucket);
    std::fs::create_dir_all(&dir).map_err(|e| format!("failed to create zeroclaw config dir: {e}"))?;
    Ok(dir)
}

fn platform_sidecar_dir_name() -> &'static str {
    if cfg!(target_os = "macos") && cfg!(target_arch = "aarch64") {
        "darwin-aarch64"
    } else if cfg!(target_os = "macos") && cfg!(target_arch = "x86_64") {
        "darwin-x64"
    } else if cfg!(target_os = "linux") && cfg!(target_arch = "x86_64") {
        "linux-x64"
    } else if cfg!(target_os = "windows") && cfg!(target_arch = "x86_64") {
        "windows-x64"
    } else {
        "unknown"
    }
}

fn zeroclaw_file_name() -> &'static str {
    if cfg!(target_os = "windows") { "zeroclaw.exe" } else { "zeroclaw" }
}

fn resolve_zeroclaw_command_path() -> Option<PathBuf> {
    if let Ok(raw) = std::env::var("CLAWPAL_ZEROCLAW_BIN") {
        let trimmed = raw.trim();
        if !trimmed.is_empty() {
            let p = PathBuf::from(trimmed);
            if p.exists() {
                return Some(p);
            }
        }
    }

    let exe = std::env::current_exe().ok()?;
    let exe_dir = exe.parent()?.to_path_buf();
    let cwd = std::env::current_dir().ok()?;
    let bin_name = zeroclaw_file_name();
    let platform_dir = platform_sidecar_dir_name();
    let mut candidates: Vec<PathBuf> = vec![
        cwd.join("src-tauri").join("resources").join("zeroclaw").join(platform_dir).join(bin_name),
        cwd.join("resources").join("zeroclaw").join(platform_dir).join(bin_name),
        cwd.parent().unwrap_or(&cwd).join("src-tauri").join("resources").join("zeroclaw").join(platform_dir).join(bin_name),
        exe_dir.join("../Resources/zeroclaw").join(platform_dir).join(bin_name),
        exe_dir.join("resources").join("zeroclaw").join(platform_dir).join(bin_name),
        exe_dir.join(bin_name),
    ];
    candidates.dedup();
    candidates.into_iter().find(|p| p.exists())
}

fn collect_provider_api_keys_for_doctor() -> std::collections::HashMap<String, String> {
    let keys = crate::commands::collect_provider_api_keys_for_internal();
    if !keys.is_empty() {
        return keys;
    }

    // Fallback for docker-local and other overridden contexts:
    // if instance-specific data has no profiles yet, reuse host default profiles.
    let current = resolve_paths();
    let Some(home) = dirs::home_dir() else {
        return keys;
    };
    let default_clawpal = home.join(".clawpal");
    let default_openclaw = home.join(".openclaw");
    if current.clawpal_dir == default_clawpal {
        return keys;
    }
    let fallback = OpenClawPaths {
        openclaw_dir: default_openclaw.clone(),
        config_path: default_openclaw.join("openclaw.json"),
        base_dir: default_openclaw,
        clawpal_dir: default_clawpal.clone(),
        history_dir: default_clawpal.join("history"),
        metadata_path: default_clawpal.join("metadata.json"),
    };
    crate::commands::collect_provider_api_keys_from_paths(&fallback)
}

fn zeroclaw_env_pairs_from_clawpal() -> Vec<(String, String)> {
    let provider_keys = collect_provider_api_keys_for_doctor();
    let mut out = Vec::<(String, String)>::new();
    for (provider, key) in provider_keys {
        match provider.as_str() {
            "openrouter" => out.push(("OPENROUTER_API_KEY".to_string(), key)),
            "openai" | "openai-codex" => out.push(("OPENAI_API_KEY".to_string(), key)),
            "anthropic" => out.push(("ANTHROPIC_API_KEY".to_string(), key)),
            "gemini" | "google" => out.push(("GEMINI_API_KEY".to_string(), key)),
            _ => {}
        }
    }
    out
}

fn pick_zeroclaw_provider(env_pairs: &[(String, String)]) -> Option<&'static str> {
    if env_pairs.iter().any(|(k, _)| k == "OPENROUTER_API_KEY") {
        return Some("openrouter");
    }
    if env_pairs.iter().any(|(k, _)| k == "OPENAI_API_KEY") {
        return Some("openai");
    }
    if env_pairs.iter().any(|(k, _)| k == "ANTHROPIC_API_KEY") {
        return Some("anthropic");
    }
    None
}

fn default_model_for_provider(provider: &str) -> Option<&'static str> {
    match provider {
        "anthropic" => Some("claude-3-7-sonnet-latest"),
        "openai" => Some("gpt-4o-mini"),
        "openrouter" => Some("anthropic/claude-3.5-sonnet"),
        _ => None,
    }
}

fn candidate_models_for_provider(provider: &str) -> Vec<String> {
    let mut out = Vec::<String>::new();
    if let Ok(profiles) = crate::commands::list_model_profiles() {
        for p in profiles
            .into_iter()
            .filter(|p| p.enabled && p.provider.trim().eq_ignore_ascii_case(provider))
        {
            let mut model = p.model.trim().to_string();
            if model.is_empty() {
                continue;
            }
            if provider != "openrouter" {
                if let Some((_, tail)) = model.split_once('/') {
                    model = tail.to_string();
                }
            }
            if !out.contains(&model) {
                out.push(model);
            }
        }
    }
    if let Some(default_model) = default_model_for_provider(provider) {
        let d = default_model.to_string();
        if !out.contains(&d) {
            out.push(d);
        }
    }
    out
}

pub fn run_zeroclaw_message(
    message: &str,
    instance_id: &str,
    session_scope: &str,
) -> Result<String, String> {
    let cmd = resolve_zeroclaw_command_path()
        .ok_or_else(|| "zeroclaw binary not found in bundled resources".to_string())?;
    let cfg = doctor_sidecar_config_dir(instance_id, session_scope)?;
    let env_pairs = zeroclaw_env_pairs_from_clawpal();
    if env_pairs.is_empty() {
        return Err("No compatible API key found in ClawPal model profiles for zeroclaw.".to_string());
    }
    let cfg_arg = cfg.to_string_lossy().to_string();
    let mut base_args = vec![
        "--config-dir".to_string(),
        cfg_arg,
        "agent".to_string(),
        "-m".to_string(),
        message.to_string(),
    ];
    let mut model_candidates = Vec::<String>::new();
    if let Some(provider) = pick_zeroclaw_provider(&env_pairs) {
        base_args.push("-p".to_string());
        base_args.push(provider.to_string());
        model_candidates = candidate_models_for_provider(provider);
    }
    let mut last_error = String::new();
    let try_once = |args: Vec<String>| -> Result<String, String> {
        let output = Command::new(&cmd)
            .envs(env_pairs.clone())
            .args(args)
            .output()
            .map_err(|e| format!("failed to run zeroclaw sidecar: {e}"))?;
        let stdout = sanitize_output(&String::from_utf8_lossy(&output.stdout));
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        if !output.status.success() {
            let msg = if !stderr.is_empty() { stderr } else { stdout };
            return Err(format!("zeroclaw sidecar failed: {msg}"));
        }
        if !stdout.is_empty() {
            return Ok(stdout);
        }
        Ok("(zeroclaw returned no output)".to_string())
    };
    for model in model_candidates {
        let mut args = base_args.clone();
        args.push("--model".to_string());
        args.push(model);
        match try_once(args) {
            Ok(v) => return Ok(v),
            Err(e) => {
                let lower = e.to_ascii_lowercase();
                last_error = e;
                if lower.contains("not_found_error") || lower.contains("model:") {
                    continue;
                }
                return Err(last_error);
            }
        }
    }
    match try_once(base_args) {
        Ok(v) => Ok(v),
        Err(e) => {
            if !last_error.is_empty() {
                Err(format!("{e}; previous model error: {last_error}"))
            } else {
                Err(e)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::sanitize_instance_namespace;

    #[test]
    fn instance_namespace_is_stable_for_same_instance() {
        let a = sanitize_instance_namespace("docker:local");
        let b = sanitize_instance_namespace("docker:local");
        assert_eq!(a, b);
    }

    #[test]
    fn instance_namespace_is_isolated_across_instances() {
        let local = sanitize_instance_namespace("local");
        let docker = sanitize_instance_namespace("docker:local");
        assert_ne!(local, docker);
        assert!(!docker.contains(':'));
        assert!(!docker.contains('/'));
    }

    #[test]
    fn instance_namespace_is_isolated_across_sessions() {
        let a = sanitize_instance_namespace("vm1::session-a");
        let b = sanitize_instance_namespace("vm1::session-b");
        assert_ne!(a, b);
    }
}
