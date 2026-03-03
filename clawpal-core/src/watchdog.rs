use serde_json::Value;

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct WatchdogStatus {
    pub alive: bool,
    pub deployed: bool,
    #[serde(flatten)]
    pub extra: serde_json::Map<String, Value>,
}

pub fn parse_watchdog_status(status_json: &str, ps_output: &str) -> WatchdogStatus {
    let alive = ps_output.trim() == "alive";
    let mut extra = match serde_json::from_str::<Value>(status_json) {
        Ok(Value::Object(map)) => map,
        _ => serde_json::Map::new(),
    };
    extra.insert("alive".to_string(), Value::Bool(alive));
    let deployed = extra
        .get("deployed")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    WatchdogStatus {
        alive,
        deployed,
        extra,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_watchdog_status_merges_alive_flag() {
        let out = parse_watchdog_status("{\"foo\":1}", "alive");
        assert!(out.alive);
        assert_eq!(out.extra.get("foo").and_then(Value::as_i64), Some(1));
        assert_eq!(out.extra.get("alive").and_then(Value::as_bool), Some(true));
    }

    #[test]
    fn parse_watchdog_status_dead_when_not_alive() {
        let out = parse_watchdog_status("{}", "dead");
        assert!(!out.alive);
        assert_eq!(out.extra.get("alive").and_then(Value::as_bool), Some(false));
    }

    #[test]
    fn parse_watchdog_status_deployed_flag() {
        let out = parse_watchdog_status("{\"deployed\":true}", "alive");
        assert!(out.deployed);

        let out2 = parse_watchdog_status("{\"deployed\":false}", "alive");
        assert!(!out2.deployed);
    }

    #[test]
    fn parse_watchdog_status_deployed_defaults_false() {
        let out = parse_watchdog_status("{}", "alive");
        assert!(!out.deployed);
    }

    #[test]
    fn parse_watchdog_status_invalid_json() {
        let out = parse_watchdog_status("not json", "alive");
        assert!(out.alive);
        assert!(!out.deployed);
        // extra should be mostly empty (just the alive flag)
        assert_eq!(out.extra.len(), 1);
    }

    #[test]
    fn parse_watchdog_status_empty_ps_output() {
        let out = parse_watchdog_status("{}", "");
        assert!(!out.alive);
    }

    #[test]
    fn parse_watchdog_status_whitespace_ps_output() {
        // "alive\n" trimmed should match "alive"
        let out = parse_watchdog_status("{}", "alive\n");
        // After trim: "alive\n".trim() = "alive" — but the function trims
        // Actually let me check — the function uses `ps_output.trim() == "alive"`
        assert!(out.alive);
    }
}
