use serde_json::Value;
use std::collections::HashSet;

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct GuildChannel {
    pub guild_id: String,
    pub guild_name: String,
    pub channel_id: String,
    pub channel_name: String,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ChannelWithBinding {
    pub guild_id: String,
    pub guild_name: String,
    pub channel_id: String,
    pub channel_name: String,
    pub agent_id: Option<String>,
}

pub fn parse_guild_channels(raw: &str) -> Result<Vec<GuildChannel>, String> {
    let cfg: Value =
        serde_json::from_str(raw).map_err(|e| format!("Failed to parse discord config: {e}"))?;
    let discord_cfg = cfg.get("channels").and_then(|c| c.get("discord"));

    let mut out = Vec::new();
    let mut seen = HashSet::new();

    let mut collect_guilds = |guilds: &serde_json::Map<String, Value>| {
        for (guild_id, guild_val) in guilds {
            let guild_name = guild_val
                .get("slug")
                .or_else(|| guild_val.get("name"))
                .and_then(Value::as_str)
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
                .unwrap_or_else(|| guild_id.clone());

            if let Some(channels) = guild_val.get("channels").and_then(Value::as_object) {
                for (channel_id, _) in channels {
                    if channel_id.contains('*') || channel_id.contains('?') {
                        continue;
                    }
                    let key = format!("{guild_id}::{channel_id}");
                    if !seen.insert(key) {
                        continue;
                    }
                    out.push(GuildChannel {
                        guild_id: guild_id.clone(),
                        guild_name: guild_name.clone(),
                        channel_id: channel_id.clone(),
                        channel_name: channel_id.clone(),
                    });
                }
            }
        }
    };

    if let Some(guilds) = discord_cfg
        .and_then(|d| d.get("guilds"))
        .and_then(Value::as_object)
    {
        collect_guilds(guilds);
    }

    if let Some(accounts) = discord_cfg
        .and_then(|d| d.get("accounts"))
        .and_then(Value::as_object)
    {
        for (_account_id, account_val) in accounts {
            if let Some(guilds) = account_val.get("guilds").and_then(Value::as_object) {
                collect_guilds(guilds);
            }
        }
    }

    if let Some(bindings) = cfg.get("bindings").and_then(Value::as_array) {
        for b in bindings {
            let m = match b.get("match") {
                Some(m) => m,
                None => continue,
            };
            if m.get("channel").and_then(Value::as_str) != Some("discord") {
                continue;
            }
            let guild_id = match m.get("guildId") {
                Some(Value::String(s)) => s.clone(),
                Some(Value::Number(n)) => n.to_string(),
                _ => continue,
            };
            let channel_id = match m.pointer("/peer/id") {
                Some(Value::String(s)) => s.clone(),
                Some(Value::Number(n)) => n.to_string(),
                _ => continue,
            };
            let key = format!("{guild_id}::{channel_id}");
            if !seen.insert(key) {
                continue;
            }
            out.push(GuildChannel {
                guild_id: guild_id.clone(),
                guild_name: guild_id.clone(),
                channel_id: channel_id.clone(),
                channel_name: channel_id,
            });
        }
    }

    Ok(out)
}

pub fn merge_channel_bindings(
    channels: &[GuildChannel],
    bindings: &str,
) -> Vec<ChannelWithBinding> {
    let parsed = parse_bindings(bindings).unwrap_or_default();
    channels
        .iter()
        .map(|c| {
            let agent_id = parsed.iter().find_map(|b| {
                let m = b.get("match")?;
                if m.get("channel").and_then(Value::as_str) != Some("discord") {
                    return None;
                }
                let gid = m.get("guildId").and_then(Value::as_str)?;
                let cid = m.pointer("/peer/id").and_then(Value::as_str)?;
                if gid == c.guild_id && cid == c.channel_id {
                    b.get("agentId")
                        .and_then(Value::as_str)
                        .map(ToString::to_string)
                } else {
                    None
                }
            });
            ChannelWithBinding {
                guild_id: c.guild_id.clone(),
                guild_name: c.guild_name.clone(),
                channel_id: c.channel_id.clone(),
                channel_name: c.channel_name.clone(),
                agent_id,
            }
        })
        .collect()
}

pub fn parse_bindings(raw: &str) -> Result<Vec<Value>, String> {
    let value: Value =
        serde_json::from_str(raw).map_err(|e| format!("Failed to parse bindings: {e}"))?;
    Ok(value.as_array().cloned().unwrap_or_default())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_guild_channels_reads_structured_config() {
        let raw = r#"{
          "channels": {"discord": {"guilds": {"g1": {"channels": {"c1": {}}}}}},
          "bindings": []
        }"#;
        let out = parse_guild_channels(raw).expect("parse");
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].guild_id, "g1");
    }

    #[test]
    fn merge_channel_bindings_sets_agent_id() {
        let channels = vec![GuildChannel {
            guild_id: "g".into(),
            guild_name: "g".into(),
            channel_id: "c".into(),
            channel_name: "c".into(),
        }];
        let bindings =
            r#"[{"match":{"channel":"discord","guildId":"g","peer":{"id":"c"}},"agentId":"main"}]"#;
        let out = merge_channel_bindings(&channels, bindings);
        assert_eq!(out[0].agent_id.as_deref(), Some("main"));
    }

    #[test]
    fn parse_bindings_returns_array() {
        let out = parse_bindings("[{\"a\":1}]").expect("parse");
        assert_eq!(out.len(), 1);
    }

    #[test]
    fn parse_guild_channels_from_accounts() {
        let raw = r#"{
          "channels": {
            "discord": {
              "accounts": {
                "acc1": {
                  "guilds": {
                    "g2": {"channels": {"c2": {}, "c3": {}}}
                  }
                }
              }
            }
          },
          "bindings": []
        }"#;
        let out = parse_guild_channels(raw).expect("parse");
        assert_eq!(out.len(), 2);
        assert!(out.iter().any(|c| c.channel_id == "c2"));
        assert!(out.iter().any(|c| c.channel_id == "c3"));
    }

    #[test]
    fn parse_guild_channels_uses_slug_as_name() {
        let raw = r#"{
          "channels": {"discord": {"guilds": {"g1": {"slug": "My Server", "channels": {"c1": {}}}}}},
          "bindings": []
        }"#;
        let out = parse_guild_channels(raw).expect("parse");
        assert_eq!(out[0].guild_name, "My Server");
    }

    #[test]
    fn parse_guild_channels_uses_name_fallback() {
        let raw = r#"{
          "channels": {"discord": {"guilds": {"g1": {"name": "Named Server", "channels": {"c1": {}}}}}},
          "bindings": []
        }"#;
        let out = parse_guild_channels(raw).expect("parse");
        assert_eq!(out[0].guild_name, "Named Server");
    }

    #[test]
    fn parse_guild_channels_falls_back_to_guild_id() {
        let raw = r#"{
          "channels": {"discord": {"guilds": {"12345": {"channels": {"c1": {}}}}}},
          "bindings": []
        }"#;
        let out = parse_guild_channels(raw).expect("parse");
        assert_eq!(out[0].guild_name, "12345");
    }

    #[test]
    fn parse_guild_channels_skips_wildcard_channels() {
        let raw = r#"{
          "channels": {"discord": {"guilds": {"g1": {"channels": {"*": {}, "c1": {}}}}}},
          "bindings": []
        }"#;
        let out = parse_guild_channels(raw).expect("parse");
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].channel_id, "c1");
    }

    #[test]
    fn parse_guild_channels_deduplicates() {
        let raw = r#"{
          "channels": {
            "discord": {
              "guilds": {"g1": {"channels": {"c1": {}}}},
              "accounts": {"a1": {"guilds": {"g1": {"channels": {"c1": {}}}}}}
            }
          },
          "bindings": []
        }"#;
        let out = parse_guild_channels(raw).expect("parse");
        assert_eq!(out.len(), 1);
    }

    #[test]
    fn parse_guild_channels_from_bindings() {
        let raw = r#"{
          "channels": {"discord": {}},
          "bindings": [
            {"match":{"channel":"discord","guildId":"g1","peer":{"id":"c1"}},"agentId":"main"},
            {"match":{"channel":"telegram","guildId":"g2","peer":{"id":"c2"}},"agentId":"main"}
          ]
        }"#;
        let out = parse_guild_channels(raw).expect("parse");
        // Only discord binding should be collected
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].guild_id, "g1");
        assert_eq!(out[0].channel_id, "c1");
    }

    #[test]
    fn parse_guild_channels_bindings_numeric_ids() {
        let raw = r#"{
          "channels": {},
          "bindings": [
            {"match":{"channel":"discord","guildId":12345,"peer":{"id":67890}},"agentId":"main"}
          ]
        }"#;
        let out = parse_guild_channels(raw).expect("parse");
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].guild_id, "12345");
        assert_eq!(out[0].channel_id, "67890");
    }

    #[test]
    fn parse_guild_channels_empty_config() {
        let raw = r#"{"channels":{},"bindings":[]}"#;
        let out = parse_guild_channels(raw).expect("parse");
        assert!(out.is_empty());
    }

    #[test]
    fn parse_guild_channels_invalid_json() {
        let result = parse_guild_channels("not json");
        assert!(result.is_err());
    }

    #[test]
    fn merge_channel_bindings_no_match() {
        let channels = vec![GuildChannel {
            guild_id: "g1".into(),
            guild_name: "g1".into(),
            channel_id: "c1".into(),
            channel_name: "c1".into(),
        }];
        let bindings = r#"[{"match":{"channel":"discord","guildId":"g2","peer":{"id":"c2"}},"agentId":"other"}]"#;
        let out = merge_channel_bindings(&channels, bindings);
        assert_eq!(out.len(), 1);
        assert!(out[0].agent_id.is_none());
    }

    #[test]
    fn merge_channel_bindings_invalid_bindings() {
        let channels = vec![GuildChannel {
            guild_id: "g".into(),
            guild_name: "g".into(),
            channel_id: "c".into(),
            channel_name: "c".into(),
        }];
        let out = merge_channel_bindings(&channels, "invalid json");
        assert_eq!(out.len(), 1);
        assert!(out[0].agent_id.is_none());
    }

    #[test]
    fn parse_bindings_empty_array() {
        let out = parse_bindings("[]").expect("parse");
        assert!(out.is_empty());
    }

    #[test]
    fn parse_bindings_non_array() {
        let out = parse_bindings("{}").expect("parse");
        assert!(out.is_empty());
    }

    #[test]
    fn parse_bindings_invalid_json() {
        let result = parse_bindings("not json");
        assert!(result.is_err());
    }
}
