use std::env;
use std::fs;
use std::path::{Path, PathBuf};

use dirs::home_dir;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OpenClawPaths {
    pub openclaw_dir: PathBuf,
    pub config_path: PathBuf,
    pub base_dir: PathBuf,
    pub clawpal_dir: PathBuf,
    pub history_dir: PathBuf,
    pub metadata_path: PathBuf,
    pub recipe_runtime_dir: PathBuf,
}

fn expand_user_path(raw: &str) -> PathBuf {
    if let Some(rest) = raw.strip_prefix("~/") {
        if let Some(home) = home_dir() {
            return home.join(rest);
        }
    }
    PathBuf::from(raw)
}

fn env_path(name: &str) -> Option<PathBuf> {
    env::var(name)
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .map(|value| expand_user_path(&value))
}

fn derive_openclaw_dir_from_override(home_override: PathBuf) -> PathBuf {
    home_override.join(".openclaw")
}

pub fn resolve_paths() -> OpenClawPaths {
    let home = home_dir().unwrap_or_else(|| Path::new(".").to_path_buf());
    let active_override = crate::cli_runner::get_active_openclaw_home_override().map(PathBuf::from);
    let active_clawpal_data =
        crate::cli_runner::get_active_clawpal_data_override().map(PathBuf::from);
    let openclaw_dir = if let Some(home_override) = active_override {
        derive_openclaw_dir_from_override(home_override)
    } else {
        env_path("CLAWPAL_OPENCLAW_DIR")
            .or_else(|| env_path("OPENCLAW_HOME"))
            .unwrap_or_else(|| home.join(".openclaw"))
    };
    let clawpal_dir = active_clawpal_data
        .or_else(|| env_path("CLAWPAL_DATA_DIR"))
        .unwrap_or_else(|| home.join(".clawpal"));

    // Migrate: ~/.openclaw/.clawpal → ~/.clawpal
    let legacy_dir = openclaw_dir.join(".clawpal");
    if legacy_dir.is_dir() {
        if !clawpal_dir.exists() {
            // New dir doesn't exist yet — just rename
            if let Err(e) = fs::rename(&legacy_dir, &clawpal_dir) {
                eprintln!(
                    "Failed to migrate {:?} → {:?}: {}",
                    legacy_dir, clawpal_dir, e
                );
            }
        } else {
            // Both exist (ensure_dirs created new one first) — remove stale legacy
            let _ = fs::remove_dir_all(&legacy_dir);
        }
    }

    let config_path = openclaw_dir.join("openclaw.json");
    let history_dir = clawpal_dir.join("history");
    let metadata_path = clawpal_dir.join("metadata.json");
    let recipe_runtime_dir = clawpal_dir.join("recipe-runtime");

    OpenClawPaths {
        openclaw_dir: openclaw_dir.clone(),
        config_path,
        base_dir: openclaw_dir.clone(),
        clawpal_dir,
        history_dir,
        metadata_path,
        recipe_runtime_dir,
    }
}
