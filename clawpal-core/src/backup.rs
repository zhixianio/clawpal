use regex::Regex;

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct BackupEntry {
    pub path: String,
    pub size_bytes: u64,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct BackupResult {
    pub size_bytes: u64,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct UpgradeResult {
    pub detected_versions: Vec<String>,
}

pub fn parse_backup_list(du_output: &str) -> Vec<BackupEntry> {
    du_output
        .lines()
        .filter_map(|line| {
            let parts: Vec<&str> = line.splitn(2, '\t').collect();
            if parts.len() != 2 {
                return None;
            }
            let size_kb = parts[0].trim().parse::<u64>().ok().unwrap_or(0);
            let path = parts[1].trim().trim_end_matches('/').to_string();
            Some(BackupEntry {
                path,
                size_bytes: size_kb * 1024,
            })
        })
        .collect()
}

pub fn parse_backup_result(output: &str) -> BackupResult {
    let size_bytes = output
        .trim()
        .lines()
        .last()
        .and_then(|l| l.trim().parse::<u64>().ok())
        .unwrap_or(0);
    BackupResult { size_bytes }
}

pub fn parse_upgrade_result(output: &str) -> UpgradeResult {
    let mut versions = Vec::new();
    let re = Regex::new(r"openclaw\s+([0-9]+\.[0-9]+\.[0-9]+)").expect("regex");
    for cap in re.captures_iter(output) {
        if let Some(v) = cap.get(1) {
            let ver = v.as_str().to_string();
            if !versions.contains(&ver) {
                versions.push(ver);
            }
        }
    }
    UpgradeResult {
        detected_versions: versions,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_backup_list_reads_du_lines() {
        let out = parse_backup_list("10\t/home/a\n0\t/home/b\n");
        assert_eq!(out.len(), 2);
        assert_eq!(out[0].size_bytes, 10 * 1024);
    }

    #[test]
    fn parse_backup_result_reads_last_line_number() {
        let out = parse_backup_result("log\n123\n");
        assert_eq!(out.size_bytes, 123);
    }

    #[test]
    fn parse_upgrade_result_extracts_versions() {
        let out = parse_upgrade_result("openclaw 0.2.0\nfoo\nopenclaw 0.3.1");
        assert_eq!(out.detected_versions, vec!["0.2.0", "0.3.1"]);
    }

    #[test]
    fn parse_backup_list_empty_input() {
        let out = parse_backup_list("");
        assert!(out.is_empty());
    }

    #[test]
    fn parse_backup_list_strips_trailing_slash() {
        let out = parse_backup_list("50\t/home/user/backup/\n");
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].path, "/home/user/backup");
        assert_eq!(out[0].size_bytes, 50 * 1024);
    }

    #[test]
    fn parse_backup_list_skips_malformed_lines() {
        let out = parse_backup_list("no tab here\n10\t/valid\n");
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].path, "/valid");
    }

    #[test]
    fn parse_backup_result_empty_input() {
        let out = parse_backup_result("");
        assert_eq!(out.size_bytes, 0);
    }

    #[test]
    fn parse_backup_result_non_numeric_last_line() {
        let out = parse_backup_result("done\ncomplete\n");
        assert_eq!(out.size_bytes, 0);
    }

    #[test]
    fn parse_upgrade_result_no_versions() {
        let out = parse_upgrade_result("nothing relevant here");
        assert!(out.detected_versions.is_empty());
    }

    #[test]
    fn parse_upgrade_result_deduplicates() {
        let out = parse_upgrade_result("openclaw 1.0.0\nupgraded\nopenclaw 1.0.0\nopenclaw 1.1.0");
        assert_eq!(out.detected_versions, vec!["1.0.0", "1.1.0"]);
    }

    #[test]
    fn parse_backup_list_zero_size() {
        let out = parse_backup_list("0\t/empty/dir\n");
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].size_bytes, 0);
    }
}
