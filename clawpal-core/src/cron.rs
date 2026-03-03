use serde_json::Value;

pub type CronJob = Value;
pub type CronRun = Value;

pub fn parse_cron_jobs(json: &str) -> Result<Vec<CronJob>, String> {
    let parsed: Value = serde_json::from_str(json).unwrap_or(Value::Array(vec![]));
    let jobs = if let Some(arr) = parsed.pointer("/jobs") {
        arr.clone()
    } else {
        parsed
    };

    match jobs {
        Value::Array(arr) => Ok(arr
            .into_iter()
            .map(|mut v| {
                if let Value::Object(ref mut obj) = v {
                    if let Some(id) = obj.get("id").cloned() {
                        obj.entry("jobId".to_string()).or_insert(id);
                    }
                }
                v
            })
            .collect()),
        Value::Object(map) => Ok(map
            .into_iter()
            .map(|(k, mut v)| {
                if let Value::Object(ref mut obj) = v {
                    obj.entry("jobId".to_string())
                        .or_insert(Value::String(k.clone()));
                    obj.entry("id".to_string()).or_insert(Value::String(k));
                }
                v
            })
            .collect()),
        _ => Ok(vec![]),
    }
}

pub fn parse_cron_runs(jsonl: &str) -> Result<Vec<CronRun>, String> {
    let mut runs: Vec<Value> = jsonl
        .lines()
        .filter(|l| !l.trim().is_empty())
        .map(|l| {
            serde_json::from_str::<Value>(l)
                .map_err(|e| format!("Failed to parse cron run line: {e}"))
        })
        .collect::<Result<Vec<_>, _>>()?;
    runs.reverse();
    Ok(runs)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_cron_jobs_supports_wrapper() {
        let raw = r#"{"version":1,"jobs":[{"id":"j1","expr":"* * * * *"}]}"#;
        let out = parse_cron_jobs(raw).expect("parse");
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].get("jobId").and_then(Value::as_str), Some("j1"));
    }

    #[test]
    fn parse_cron_runs_parses_jsonl() {
        let raw = "{\"runId\":\"1\"}\n{\"runId\":\"2\"}\n";
        let out = parse_cron_runs(raw).expect("parse");
        assert_eq!(out.len(), 2);
        assert_eq!(out[0].get("runId").and_then(Value::as_str), Some("2"));
    }

    #[test]
    fn parse_cron_jobs_plain_array() {
        let raw = r#"[{"id":"j1","expr":"0 * * * *"},{"id":"j2","expr":"*/5 * * * *"}]"#;
        let out = parse_cron_jobs(raw).expect("parse");
        assert_eq!(out.len(), 2);
        assert_eq!(out[0].get("jobId").and_then(Value::as_str), Some("j1"));
        assert_eq!(out[1].get("jobId").and_then(Value::as_str), Some("j2"));
    }

    #[test]
    fn parse_cron_jobs_object_map() {
        let raw = r#"{"myJob":{"expr":"0 0 * * *"},"other":{"expr":"*/10 * * * *"}}"#;
        let out = parse_cron_jobs(raw).expect("parse");
        assert_eq!(out.len(), 2);
        // Each entry should have both id and jobId
        for job in &out {
            assert!(job.get("jobId").is_some());
            assert!(job.get("id").is_some());
        }
    }

    #[test]
    fn parse_cron_jobs_empty_input() {
        let out = parse_cron_jobs("").expect("parse");
        assert!(out.is_empty());
    }

    #[test]
    fn parse_cron_jobs_invalid_json_returns_empty() {
        let out = parse_cron_jobs("not json at all").expect("parse");
        assert!(out.is_empty());
    }

    #[test]
    fn parse_cron_runs_empty_input() {
        let out = parse_cron_runs("").expect("parse");
        assert!(out.is_empty());
    }

    #[test]
    fn parse_cron_runs_skips_empty_lines() {
        let raw = "\n{\"runId\":\"1\"}\n\n{\"runId\":\"2\"}\n\n";
        let out = parse_cron_runs(raw).expect("parse");
        assert_eq!(out.len(), 2);
    }

    #[test]
    fn parse_cron_runs_reverses_order() {
        let raw = "{\"runId\":\"first\"}\n{\"runId\":\"second\"}\n{\"runId\":\"third\"}\n";
        let out = parse_cron_runs(raw).expect("parse");
        assert_eq!(out[0].get("runId").and_then(Value::as_str), Some("third"));
        assert_eq!(out[2].get("runId").and_then(Value::as_str), Some("first"));
    }

    #[test]
    fn parse_cron_jobs_preserves_existing_job_id() {
        // If jobId already exists, id should not overwrite it
        let raw = r#"[{"id":"j1","jobId":"existing"}]"#;
        let out = parse_cron_jobs(raw).expect("parse");
        assert_eq!(
            out[0].get("jobId").and_then(Value::as_str),
            Some("existing")
        );
    }
}
