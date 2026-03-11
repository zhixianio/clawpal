use serde_json::{json, Value};

use crate::execution_spec::{
    ExecutionAction, ExecutionCapabilities, ExecutionMetadata, ExecutionResourceClaim,
    ExecutionResources, ExecutionSecrets, ExecutionSpec, ExecutionTarget,
};
use crate::recipe_executor::{materialize_execution_plan, route_execution};

fn sample_target(kind: &str) -> Value {
    match kind {
        "remote" => json!({
            "kind": "remote",
            "hostId": "ssh:prod-a",
        }),
        _ => json!({
            "kind": "local",
        }),
    }
}

fn sample_job_spec() -> ExecutionSpec {
    ExecutionSpec {
        api_version: "strategy.platform/v1".into(),
        kind: "ExecutionSpec".into(),
        metadata: ExecutionMetadata {
            name: Some("hourly-health-check".into()),
            digest: None,
        },
        source: Value::Null,
        target: json!({ "kind": "local" }),
        execution: ExecutionTarget { kind: "job".into() },
        capabilities: ExecutionCapabilities {
            used_capabilities: vec!["service.manage".into()],
        },
        resources: ExecutionResources {
            claims: vec![ExecutionResourceClaim {
                kind: "service".into(),
                id: Some("openclaw-gateway".into()),
                target: None,
                path: None,
            }],
        },
        secrets: ExecutionSecrets::default(),
        desired_state: json!({
            "command": ["openclaw", "doctor", "run"],
        }),
        actions: vec![ExecutionAction {
            kind: Some("job".into()),
            name: Some("Run doctor".into()),
            args: json!({
                "command": ["openclaw", "doctor", "run"],
            }),
        }],
        outputs: vec![],
    }
}

fn sample_schedule_spec() -> ExecutionSpec {
    ExecutionSpec {
        api_version: "strategy.platform/v1".into(),
        kind: "ExecutionSpec".into(),
        metadata: ExecutionMetadata {
            name: Some("hourly-reconcile".into()),
            digest: None,
        },
        source: Value::Null,
        target: json!({ "kind": "local" }),
        execution: ExecutionTarget {
            kind: "schedule".into(),
        },
        capabilities: ExecutionCapabilities {
            used_capabilities: vec!["service.manage".into()],
        },
        resources: ExecutionResources {
            claims: vec![ExecutionResourceClaim {
                kind: "service".into(),
                id: Some("schedule/hourly".into()),
                target: Some("job/hourly-reconcile".into()),
                path: None,
            }],
        },
        secrets: ExecutionSecrets::default(),
        desired_state: json!({
            "schedule": {
                "id": "schedule/hourly",
                "onCalendar": "hourly",
            },
            "job": {
                "command": ["openclaw", "doctor", "run"],
            }
        }),
        actions: vec![ExecutionAction {
            kind: Some("schedule".into()),
            name: Some("Run hourly reconcile".into()),
            args: json!({
                "command": ["openclaw", "doctor", "run"],
                "onCalendar": "hourly",
            }),
        }],
        outputs: vec![],
    }
}

#[test]
fn job_spec_materializes_to_systemd_run_command() {
    let spec = sample_job_spec();
    let plan = materialize_execution_plan(&spec).expect("materialize execution plan");

    assert!(plan
        .commands
        .iter()
        .any(|cmd| cmd.join(" ").contains("systemd-run")));
}

#[test]
fn schedule_spec_references_job_launch_ref() {
    let spec = sample_schedule_spec();
    let plan = materialize_execution_plan(&spec).expect("materialize execution plan");

    assert!(plan
        .resources
        .iter()
        .any(|ref_id| ref_id == "schedule/hourly"));
}

#[test]
fn local_target_uses_local_runner() {
    let route = route_execution(&sample_target("local")).expect("route execution");

    assert_eq!(route.runner, "local");
}

#[test]
fn remote_target_uses_remote_ssh_runner() {
    let route = route_execution(&sample_target("remote")).expect("route execution");

    assert_eq!(route.runner, "remote_ssh");
}
