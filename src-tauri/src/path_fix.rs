use std::env;
use std::ffi::OsString;
use std::fs;
use std::path::PathBuf;

use crate::logging::{log_error, log_info};

/// Ensure `openclaw` and `node` are discoverable on PATH.
/// On non-macOS platforms this is a no-op.
pub fn ensure_tool_paths() {
    #[cfg(target_os = "macos")]
    ensure_tool_paths_macos();
}

// ── macOS implementation ────────────────────────────────────────────

#[cfg(target_os = "macos")]
fn ensure_tool_paths_macos() {
    // Step 1: try fix_path_env (sources shell profile)
    match fix_path_env::fix() {
        Ok(_) => log_info("fix_path_env::fix() succeeded"),
        Err(e) => log_error(&format!("fix_path_env::fix() failed: {e}")),
    }

    let need_openclaw = find_on_path("openclaw").is_none();
    let need_node = find_on_path("node").is_none();

    if need_openclaw || need_node {
        log_info(&format!(
            "PATH补全: openclaw missing={need_openclaw}, node missing={need_node}"
        ));

        let candidates = candidate_bin_dirs();
        let current_path = env::var("PATH").unwrap_or_default();

        // Collect dirs that exist and contain a needed binary
        let extra: Vec<PathBuf> = candidates
            .into_iter()
            .filter(|d| d.is_dir())
            .filter(|d| {
                (need_openclaw && d.join("openclaw").is_file())
                    || (need_node && d.join("node").is_file())
            })
            .collect();

        if !extra.is_empty() {
            let new_path = dedup_prepend_path(&extra, &current_path);
            // SAFETY: called from main() before any threads are spawned.
            unsafe { env::set_var("PATH", &new_path) };
            log_info(&format!("PATH prepended with: {:?}", extra));
        }
    }

    // Final status
    match find_on_path("openclaw") {
        Some(p) => log_info(&format!("openclaw found: {}", p.display())),
        None => log_error("openclaw NOT found on PATH after fix"),
    }
    match find_on_path("node") {
        Some(p) => log_info(&format!("node found: {}", p.display())),
        None => log_error("node NOT found on PATH after fix"),
    }
}

// ── Pure helper functions (testable) ────────────────────────────────

/// Return candidate directories where `openclaw` or `node` might live.
fn candidate_bin_dirs() -> Vec<PathBuf> {
    let home = match dirs::home_dir() {
        Some(h) => h,
        None => return vec![],
    };

    let mut dirs = vec![
        home.join(".local/bin"),
        PathBuf::from("/opt/homebrew/bin"),
        PathBuf::from("/usr/local/bin"),
        home.join(".bun/bin"),
        home.join(".volta/bin"),
        home.join("Library/pnpm"),
        home.join(".cargo/bin"),
    ];

    // NVM: pick the latest node version
    let nvm_dir = env::var("NVM_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|_| home.join(".nvm"));
    if let Some(nvm_bin) = latest_nvm_node_bin(&nvm_dir) {
        dirs.push(nvm_bin);
    }

    // FNM: prefer default alias, fallback to latest installed version.
    if let Some(fnm_bin) = latest_fnm_node_bin(&home) {
        dirs.push(fnm_bin);
    }

    dirs
}

/// Find the `bin/` directory of the latest node version installed via NVM.
fn latest_nvm_node_bin(nvm_dir: &PathBuf) -> Option<PathBuf> {
    // Try alias/default first (symlink to a version)
    let alias_default = nvm_dir.join("alias/default");
    if alias_default.exists() {
        if let Ok(target) = fs::read_to_string(&alias_default) {
            let version = target.trim();
            let bin = nvm_dir.join("versions/node").join(version).join("bin");
            if bin.is_dir() {
                return Some(bin);
            }
        }
    }

    // Fallback: scan versions/node/ and pick the highest semver
    let versions_dir = nvm_dir.join("versions/node");
    let mut versions: Vec<(Vec<u64>, PathBuf)> = Vec::new();

    if let Ok(entries) = fs::read_dir(&versions_dir) {
        for entry in entries.flatten() {
            let name = entry.file_name();
            let name_str = name.to_string_lossy();
            let trimmed = name_str.strip_prefix('v').unwrap_or(&name_str);
            let parts: Vec<u64> = trimmed.split('.').filter_map(|s| s.parse().ok()).collect();
            if parts.len() == 3 {
                let bin = entry.path().join("bin");
                if bin.is_dir() {
                    versions.push((parts, bin));
                }
            }
        }
    }

    versions.sort_by(|a, b| a.0.cmp(&b.0));
    versions.into_iter().last().map(|(_, path)| path)
}

/// Find a likely Node `bin/` directory managed by FNM.
///
/// Preference order:
/// 1. `aliases/default/bin` under known FNM roots
/// 2. Latest semver under `node-versions/*/installation/bin`
fn latest_fnm_node_bin(home: &PathBuf) -> Option<PathBuf> {
    let mut roots: Vec<PathBuf> = Vec::new();

    if let Ok(fnm_dir) = env::var("FNM_DIR") {
        roots.push(PathBuf::from(fnm_dir));
    }
    roots.push(home.join(".fnm"));
    roots.push(home.join("Library/Application Support/fnm"));

    let mut dedup_roots = Vec::new();
    let mut seen_roots = std::collections::HashSet::new();
    for root in roots {
        if seen_roots.insert(root.clone()) {
            dedup_roots.push(root);
        }
    }

    for root in &dedup_roots {
        let alias_default = root.join("aliases/default/bin");
        if alias_default.is_dir() {
            return Some(alias_default);
        }
    }

    let mut versions: Vec<(Vec<u64>, PathBuf)> = Vec::new();
    for root in &dedup_roots {
        let versions_dir = root.join("node-versions");
        let Ok(entries) = fs::read_dir(&versions_dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let name = entry.file_name();
            let name_str = name.to_string_lossy();
            let trimmed = name_str.strip_prefix('v').unwrap_or(&name_str);
            let parts: Vec<u64> = trimmed.split('.').filter_map(|s| s.parse().ok()).collect();
            if parts.len() != 3 {
                continue;
            }
            let bin = entry.path().join("installation/bin");
            if bin.is_dir() {
                versions.push((parts, bin));
            }
        }
    }

    versions.sort_by(|a, b| a.0.cmp(&b.0));
    versions.into_iter().last().map(|(_, path)| path)
}

/// Search PATH for a binary by name. Returns the full path if found.
fn find_on_path(binary: &str) -> Option<PathBuf> {
    let path_var = env::var_os("PATH")?;
    find_in_dirs(binary, &env::split_paths(&path_var).collect::<Vec<_>>())
}

/// Pure function: return the first directory that contains `binary`.
fn find_in_dirs(binary: &str, dirs: &[PathBuf]) -> Option<PathBuf> {
    for dir in dirs {
        let candidate = dir.join(binary);
        if candidate.is_file() {
            return Some(candidate);
        }
    }
    None
}

/// Pure function: prepend `extra` dirs to `current` PATH, deduplicating.
fn dedup_prepend_path(extra: &[PathBuf], current: &str) -> OsString {
    let current_dirs: Vec<PathBuf> = env::split_paths(current).collect();
    let mut seen = std::collections::HashSet::new();
    let mut result: Vec<PathBuf> = Vec::new();

    // Add extra dirs first (prepend)
    for d in extra {
        if seen.insert(d.clone()) {
            result.push(d.clone());
        }
    }
    // Then existing dirs
    for d in current_dirs {
        if seen.insert(d.clone()) {
            result.push(d);
        }
    }

    env::join_paths(result).unwrap_or_else(|_| OsString::from(current))
}

// ── Tests ───────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn candidate_bin_dirs_is_nonempty() {
        let dirs = candidate_bin_dirs();
        assert!(!dirs.is_empty());
        // Should always include .local/bin
        assert!(dirs.iter().any(|d| d.ends_with(".local/bin")));
    }

    #[test]
    fn find_in_dirs_existing() {
        let dir = std::env::temp_dir();
        let marker = dir.join("__clawpal_test_bin__");
        std::fs::write(&marker, "").unwrap();
        let result = find_in_dirs("__clawpal_test_bin__", &[dir.clone()]);
        std::fs::remove_file(&marker).ok();
        assert!(result.is_some());
        assert_eq!(result.unwrap(), dir.join("__clawpal_test_bin__"));
    }

    #[test]
    fn find_in_dirs_nonexistent() {
        let result = find_in_dirs(
            "nonexistent_binary_xyz_12345",
            &[PathBuf::from("/usr/bin"), PathBuf::from("/usr/local/bin")],
        );
        assert!(result.is_none());
    }

    #[test]
    fn dedup_prepend_preserves_order_and_deduplicates() {
        let extra = vec![
            PathBuf::from("/extra/a"),
            PathBuf::from("/extra/b"),
        ];
        let current = "/existing/x:/extra/a:/existing/y";
        let result = dedup_prepend_path(&extra, current);
        let result_str = result.to_string_lossy();

        let parts: Vec<&str> = result_str.split(':').collect();
        assert_eq!(parts, vec!["/extra/a", "/extra/b", "/existing/x", "/existing/y"]);
    }

    #[test]
    fn dedup_prepend_empty_extra() {
        let result = dedup_prepend_path(&[], "/a:/b");
        assert_eq!(result.to_string_lossy(), "/a:/b");
    }
}
