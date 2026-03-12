use serde_json::{json, Value};

use crate::commands::INTERNAL_SYSTEMD_DROPIN_WRITE_COMMAND;
use crate::execution_spec::{
    ExecutionAction, ExecutionCapabilities, ExecutionMetadata, ExecutionResourceClaim,
    ExecutionResources, ExecutionSecrets, ExecutionSpec, ExecutionTarget,
};
use crate::recipe_executor::{
    build_cleanup_commands, build_runtime_artifacts, execute_recipe, materialize_execution_plan,
    route_execution, ExecuteRecipeRequest,
};
use crate::recipe_store::Artifact;

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

fn sample_execution_request() -> ExecuteRecipeRequest {
    ExecuteRecipeRequest {
        spec: sample_job_spec(),
        source_origin: None,
        source_text: None,
        workspace_slug: None,
    }
}

fn sample_attachment_spec() -> ExecutionSpec {
    ExecutionSpec {
        api_version: "strategy.platform/v1".into(),
        kind: "ExecutionSpec".into(),
        metadata: ExecutionMetadata {
            name: Some("gateway-env".into()),
            digest: None,
        },
        source: Value::Null,
        target: json!({ "kind": "local" }),
        execution: ExecutionTarget {
            kind: "attachment".into(),
        },
        capabilities: ExecutionCapabilities {
            used_capabilities: vec!["service.manage".into()],
        },
        resources: ExecutionResources {
            claims: vec![ExecutionResourceClaim {
                kind: "service".into(),
                id: Some("openclaw-gateway".into()),
                target: Some("openclaw-gateway.service".into()),
                path: None,
            }],
        },
        secrets: ExecutionSecrets::default(),
        desired_state: json!({
            "systemdDropIn": {
                "unit": "openclaw-gateway.service",
                "name": "10-channel.conf",
                "content": "[Service]\nEnvironment=OPENCLAW_CHANNEL=discord\n",
            },
            "envPatch": {
                "OPENCLAW_CHANNEL": "discord",
            }
        }),
        actions: vec![ExecutionAction {
            kind: Some("attachment".into()),
            name: Some("Apply gateway env".into()),
            args: json!({}),
        }],
        outputs: vec![],
    }
}

fn sample_action_recipe_spec() -> ExecutionSpec {
    ExecutionSpec {
        api_version: "strategy.platform/v1".into(),
        kind: "ExecutionSpec".into(),
        metadata: ExecutionMetadata {
            name: Some("discord-channel-persona".into()),
            digest: None,
        },
        source: json!({
            "recipeId": "discord-channel-persona",
            "recipeVersion": "1.0.0",
        }),
        target: json!({ "kind": "local" }),
        execution: ExecutionTarget { kind: "job".into() },
        capabilities: ExecutionCapabilities {
            used_capabilities: vec!["config.write".into()],
        },
        resources: ExecutionResources::default(),
        secrets: ExecutionSecrets::default(),
        desired_state: json!({
            "actionCount": 1,
        }),
        actions: vec![ExecutionAction {
            kind: Some("config_patch".into()),
            name: Some("Set channel persona".into()),
            args: json!({
                "patch": {
                    "channels": {
                        "discord": {
                            "guilds": {
                                "guild-1": {
                                    "channels": {
                                        "channel-1": {
                                            "systemPrompt": "Keep answers concise"
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }),
        }],
        outputs: vec![json!({
            "kind": "recipe-summary",
            "recipeId": "discord-channel-persona",
        })],
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

#[test]
fn execute_recipe_returns_run_id_and_summary() {
    let result = execute_recipe(sample_execution_request()).expect("execute recipe");

    assert!(!result.run_id.is_empty());
    assert!(!result.summary.is_empty());
}

#[test]
fn action_recipe_spec_can_prepare_without_command_payload() {
    let result = execute_recipe(ExecuteRecipeRequest {
        spec: sample_action_recipe_spec(),
        source_origin: None,
        source_text: None,
        workspace_slug: None,
    })
    .expect("prepare action recipe execution");

    assert!(!result.run_id.is_empty());
    assert!(result.summary.contains("discord-channel-persona"));
}

#[test]
fn attachment_spec_materializes_dropin_write_and_daemon_reload() {
    let spec = sample_attachment_spec();
    let plan = materialize_execution_plan(&spec).expect("materialize attachment execution plan");

    assert_eq!(
        plan.commands[0],
        vec![
            INTERNAL_SYSTEMD_DROPIN_WRITE_COMMAND.to_string(),
            "openclaw-gateway.service".to_string(),
            "10-channel.conf".to_string(),
            "[Service]\nEnvironment=OPENCLAW_CHANNEL=discord\n".to_string(),
        ]
    );
    assert!(plan.commands.iter().any(|command| {
        command
            == &vec![
                INTERNAL_SYSTEMD_DROPIN_WRITE_COMMAND.to_string(),
                "openclaw-gateway.service".to_string(),
                "90-clawpal-env-gateway-env.conf".to_string(),
                "[Service]\nEnvironment=\"OPENCLAW_CHANNEL=discord\"\n".to_string(),
            ]
    }));
    assert!(plan.commands.iter().any(|command| {
        command
            == &vec![
                "systemctl".to_string(),
                "--user".to_string(),
                "daemon-reload".to_string(),
            ]
    }));
}

#[test]
fn schedule_execution_builds_unit_and_timer_artifacts() {
    let spec = sample_schedule_spec();
    let prepared = execute_recipe(ExecuteRecipeRequest {
        spec: spec.clone(),
        source_origin: None,
        source_text: None,
        workspace_slug: None,
    })
    .expect("prepare schedule execution");

    let artifacts = build_runtime_artifacts(&spec, &prepared);

    assert!(artifacts.iter().any(
        |artifact| artifact.kind == "systemdUnit" && artifact.label == prepared.plan.unit_name
    ));
    assert!(artifacts
        .iter()
        .any(|artifact| artifact.kind == "systemdTimer"));
}

#[test]
fn attachment_execution_builds_dropin_and_reload_artifacts() {
    let spec = sample_attachment_spec();
    let prepared = execute_recipe(ExecuteRecipeRequest {
        spec: spec.clone(),
        source_origin: None,
        source_text: None,
        workspace_slug: None,
    })
    .expect("prepare attachment execution");

    let artifacts = build_runtime_artifacts(&spec, &prepared);

    assert!(artifacts
        .iter()
        .any(|artifact| artifact.kind == "systemdDropIn"
            && artifact.path.as_deref()
                == Some("~/.config/systemd/user/openclaw-gateway.service.d/10-channel.conf")));
    assert!(artifacts
        .iter()
        .any(|artifact| artifact.kind == "systemdDropIn"
            && artifact.path.as_deref()
                == Some("~/.config/systemd/user/openclaw-gateway.service.d/90-clawpal-env-gateway-env.conf")));
    assert!(artifacts
        .iter()
        .any(|artifact| artifact.kind == "systemdDaemonReload"));
}

#[test]
fn cleanup_commands_stop_and_reset_failed_for_systemd_artifacts() {
    let commands = build_cleanup_commands(&[
        Artifact {
            id: "run_01:unit".into(),
            kind: "systemdUnit".into(),
            label: "clawpal-job-hourly".into(),
            path: Some("clawpal-job-hourly".into()),
        },
        Artifact {
            id: "run_01:timer".into(),
            kind: "systemdTimer".into(),
            label: "clawpal-job-hourly.timer".into(),
            path: Some("clawpal-job-hourly.timer".into()),
        },
    ]);

    assert_eq!(
        commands,
        vec![
            vec![
                String::from("systemctl"),
                String::from("--user"),
                String::from("stop"),
                String::from("clawpal-job-hourly"),
            ],
            vec![
                String::from("systemctl"),
                String::from("--user"),
                String::from("reset-failed"),
                String::from("clawpal-job-hourly"),
            ],
            vec![
                String::from("systemctl"),
                String::from("--user"),
                String::from("stop"),
                String::from("clawpal-job-hourly.timer"),
            ],
            vec![
                String::from("systemctl"),
                String::from("--user"),
                String::from("reset-failed"),
                String::from("clawpal-job-hourly.timer"),
            ],
        ]
    );
}
