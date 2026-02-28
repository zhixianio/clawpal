use std::collections::HashSet;
use std::path::PathBuf;
use std::process::Command;

use serde::Serialize;

use clawpal_core::instance::InstanceRegistry;

/// A Docker instance or data-dir discovered on the local machine.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DiscoveredInstance {
    pub id: String,
    /// Always "docker" for now.
    pub instance_type: String,
    pub label: String,
    pub home_path: String,
    /// "container" if found via `docker ps`, "data_dir" if found via ~/.clawpal/ scan.
    pub source: String,
    pub container_name: Option<String>,
    pub already_registered: bool,
}

/// Convert a container name to a URL-safe slug.
///
/// Strips leading `/`, lowercases, and replaces non-alphanumeric chars with `-`.
fn slug_from_name(name: &str) -> String {
    let trimmed = name.strip_prefix('/').unwrap_or(name);
    let mut slug = String::with_capacity(trimmed.len());
    for ch in trimmed.chars() {
        if ch.is_ascii_alphanumeric() || ch == '-' {
            slug.push(ch.to_ascii_lowercase());
        } else {
            // Collapse repeated dashes
            if slug.ends_with('-') {
                continue;
            }
            slug.push('-');
        }
    }
    slug.trim_matches('-').to_string()
}

/// Discover local Docker instances that are either running as containers
/// or exist as data directories under `~/.clawpal/`.
#[tauri::command]
pub async fn discover_local_instances() -> Result<Vec<DiscoveredInstance>, String> {
    tauri::async_runtime::spawn_blocking(|| discover_blocking())
        .await
        .map_err(|e| e.to_string())?
}

fn discover_blocking() -> Result<Vec<DiscoveredInstance>, String> {
    // 1. Load registry for already_registered check
    let registered_ids: HashSet<String> = InstanceRegistry::load()
        .map(|r| r.ids().into_iter().collect())
        .unwrap_or_default();

    let mut results: Vec<DiscoveredInstance> = Vec::new();
    let mut seen_home_paths: HashSet<String> = HashSet::new();

    // 2. Scan Docker containers
    if let Ok(containers) = scan_docker_containers() {
        for inst in containers {
            if seen_home_paths.contains(&inst.home_path) {
                continue;
            }
            seen_home_paths.insert(inst.home_path.clone());
            results.push(inst);
        }
    }

    // 3. Scan ~/.clawpal/ data directories
    if let Ok(data_dirs) = scan_data_dirs() {
        for inst in data_dirs {
            if seen_home_paths.contains(&inst.home_path) {
                continue;
            }
            seen_home_paths.insert(inst.home_path.clone());
            results.push(inst);
        }
    }

    // 4. Mark already_registered
    for inst in &mut results {
        inst.already_registered = registered_ids.contains(&inst.id);
    }

    Ok(results)
}

/// Run `docker ps --format '{{json .}}'` and parse matching containers.
fn scan_docker_containers() -> Result<Vec<DiscoveredInstance>, String> {
    let output = Command::new("docker")
        .args(["ps", "--format", "{{json .}}"])
        .output()
        .map_err(|e| format!("failed to run docker ps: {e}"))?;

    if !output.status.success() {
        return Err(format!(
            "docker ps failed: {}",
            String::from_utf8_lossy(&output.stderr)
        ));
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    let mut instances = Vec::new();

    for line in stdout.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }

        let container: serde_json::Value =
            serde_json::from_str(line).map_err(|e| format!("failed to parse docker JSON: {e}"))?;

        let names = container
            .get("Names")
            .and_then(|v| v.as_str())
            .unwrap_or_default();
        let labels = container
            .get("Labels")
            .and_then(|v| v.as_str())
            .unwrap_or_default();

        let names_lower = names.to_lowercase();
        let labels_lower = labels.to_lowercase();

        let is_match = names_lower.contains("openclaw")
            || names_lower.contains("clawpal")
            || labels_lower.contains("com.clawpal");

        if !is_match {
            continue;
        }

        let slug = slug_from_name(names);
        let id = format!("docker:{slug}");

        // Try to extract home path from labels (com.clawpal.home=<path>),
        // otherwise derive from the container name.
        let home_path = extract_label_value(labels, "com.clawpal.home")
            .unwrap_or_else(|| derive_home_path_from_name(&slug));

        let label = names
            .strip_prefix('/')
            .unwrap_or(names)
            .to_string();

        instances.push(DiscoveredInstance {
            id,
            instance_type: "docker".to_string(),
            label,
            home_path,
            source: "container".to_string(),
            container_name: Some(names.to_string()),
            already_registered: false,
        });
    }

    Ok(instances)
}

/// Extract a specific key from a Docker labels string (comma-separated key=value pairs).
fn extract_label_value(labels: &str, key: &str) -> Option<String> {
    for pair in labels.split(',') {
        let pair = pair.trim();
        if let Some(val) = pair.strip_prefix(key) {
            if let Some(val) = val.strip_prefix('=') {
                return Some(val.to_string());
            }
        }
    }
    None
}

/// Derive a home path from a slug: `~/.clawpal/docker-{slug}`.
fn derive_home_path_from_name(slug: &str) -> String {
    let home = dirs::home_dir().unwrap_or_else(|| PathBuf::from("."));
    home.join(".clawpal")
        .join(format!("docker-{slug}"))
        .to_string_lossy()
        .to_string()
}

/// Scan `~/.clawpal/` for subdirectories starting with "docker-" that contain
/// `openclaw.json` or `docker-compose.yml`/`.yaml`.
fn scan_data_dirs() -> Result<Vec<DiscoveredInstance>, String> {
    let home = dirs::home_dir().ok_or("cannot determine home directory")?;
    let clawpal_dir = home.join(".clawpal");

    if !clawpal_dir.is_dir() {
        return Ok(Vec::new());
    }

    let entries =
        std::fs::read_dir(&clawpal_dir).map_err(|e| format!("failed to read ~/.clawpal: {e}"))?;

    let mut instances = Vec::new();

    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().to_string();
        if !name.starts_with("docker-") {
            continue;
        }

        let path = entry.path();
        if !path.is_dir() {
            continue;
        }

        let has_marker = path.join("openclaw.json").exists()
            || path.join("docker-compose.yml").exists()
            || path.join("docker-compose.yaml").exists();

        if !has_marker {
            continue;
        }

        let slug = name
            .strip_prefix("docker-")
            .unwrap_or(&name)
            .to_string();
        let id = format!("docker:{slug}");
        let home_path = path.to_string_lossy().to_string();

        instances.push(DiscoveredInstance {
            id,
            instance_type: "docker".to_string(),
            label: slug.clone(),
            home_path,
            source: "data_dir".to_string(),
            container_name: None,
            already_registered: false,
        });
    }

    Ok(instances)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slug_from_name_basic() {
        assert_eq!(slug_from_name("/my-project"), "my-project");
        assert_eq!(slug_from_name("My Project"), "my-project");
        assert_eq!(slug_from_name("/openclaw_dev"), "openclaw-dev");
        assert_eq!(slug_from_name("///foo///bar"), "foo-bar");
    }

    #[test]
    fn extract_label_value_works() {
        let labels = "com.clawpal=true,com.clawpal.home=/data/oc,other=val";
        assert_eq!(
            extract_label_value(labels, "com.clawpal.home"),
            Some("/data/oc".to_string())
        );
        assert_eq!(extract_label_value(labels, "missing"), None);
    }
}
