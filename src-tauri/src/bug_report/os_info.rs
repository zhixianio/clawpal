#[cfg(any(target_os = "macos", target_os = "windows"))]
use std::process::Command;
#[cfg(any(target_os = "macos", target_os = "linux"))]
use std::sync::OnceLock;

#[cfg(target_os = "macos")]
pub fn os_version_string() -> String {
    static VERSION: OnceLock<String> = OnceLock::new();
    VERSION
        .get_or_init(|| {
            Command::new("sw_vers")
                .arg("-productVersion")
                .output()
                .ok()
                .and_then(|output| {
                    if output.status.success() {
                        String::from_utf8(output.stdout)
                            .ok()
                            .map(|value| value.trim().to_string())
                    } else {
                        None
                    }
                })
                .filter(|value| !value.is_empty())
                .unwrap_or_else(|| "unknown".to_string())
        })
        .clone()
}

#[cfg(target_os = "linux")]
pub fn os_version_string() -> String {
    static VERSION: OnceLock<String> = OnceLock::new();
    VERSION
        .get_or_init(|| {
            std::fs::read_to_string("/etc/os-release")
                .ok()
                .and_then(|content| {
                    content
                        .lines()
                        .find_map(|line| line.strip_prefix("PRETTY_NAME="))
                        .map(|raw| raw.trim_matches('"').to_string())
                })
                .filter(|value| !value.is_empty())
                .unwrap_or_else(|| "unknown".to_string())
        })
        .clone()
}

#[cfg(target_os = "windows")]
pub fn os_version_string() -> String {
    fn read_registry_version() -> Option<String> {
        let output = Command::new("reg")
            .args([
                "query",
                "HKEY_LOCAL_MACHINE\\SOFTWARE\\Microsoft\\Windows NT\\CurrentVersion",
                "/v",
                "CurrentBuildNumber",
            ])
            .output()
            .ok()?;
        if !output.status.success() {
            return None;
        }
        let stdout = String::from_utf8(output.stdout).ok()?;
        stdout
            .lines()
            .find_map(|line| {
                if line.contains("CurrentBuildNumber") {
                    Some(line.split_whitespace().last()?.to_string())
                } else {
                    None
                }
            })
            .filter(|value| !value.is_empty())
            .map(|build| format!("Windows build {build}"))
    }

    fn read_winver() -> Option<String> {
        let output = Command::new("cmd").args(["/C", "ver"]).output().ok()?;
        if !output.status.success() {
            return None;
        }
        let stdout = String::from_utf8(output.stdout).ok()?;
        let trimmed = stdout.trim().to_string();
        if trimmed.is_empty() {
            None
        } else {
            Some(trimmed)
        }
    }

    read_registry_version()
        .or_else(read_winver)
        .or_else(|| std::env::var("OS").ok())
        .filter(|value| !value.trim().is_empty())
        .unwrap_or_else(|| "unknown".to_string())
}

#[cfg(not(any(target_os = "macos", target_os = "linux", target_os = "windows")))]
pub fn os_version_string() -> String {
    "unknown".to_string()
}

#[cfg(test)]
mod tests {
    #[cfg(target_os = "linux")]
    #[test]
    fn linux_pretty_name_parser_handles_quotes() {
        let sample = "NAME=\"Ubuntu\"\nPRETTY_NAME=\"Ubuntu 24.04.2 LTS\"\n";
        let parsed = sample
            .lines()
            .find_map(|line| line.strip_prefix("PRETTY_NAME="))
            .map(|raw| raw.trim_matches('"').to_string());
        assert_eq!(parsed.as_deref(), Some("Ubuntu 24.04.2 LTS"));
    }
}
