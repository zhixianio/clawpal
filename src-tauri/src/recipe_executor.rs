use serde::{Deserialize, Serialize};
use serde_json::Value;
use uuid::Uuid;

use crate::execution_spec::ExecutionSpec;
use crate::recipe_runtime::systemd;
use crate::recipe_store::Artifact as RecipeRuntimeArtifact;

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase", default)]
pub struct MaterializedExecutionPlan {
    pub execution_kind: String,
    pub unit_name: String,
    pub commands: Vec<Vec<String>>,
    pub resources: Vec<String>,
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase", default)]
pub struct ExecutionRoute {
    pub runner: String,
    pub target_kind: String,
    pub host_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExecuteRecipeRequest {
    pub spec: ExecutionSpec,
    #[serde(default)]
    pub source_origin: Option<String>,
    #[serde(default)]
    pub source_text: Option<String>,
    #[serde(default)]
    pub workspace_slug: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExecuteRecipePrepared {
    pub run_id: String,
    pub route: ExecutionRoute,
    pub plan: MaterializedExecutionPlan,
    pub summary: String,
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExecuteRecipeResult {
    pub run_id: String,
    pub instance_id: String,
    pub summary: String,
    pub warnings: Vec<String>,
}

fn has_command_value(value: Option<&Value>) -> bool {
    value
        .and_then(Value::as_array)
        .is_some_and(|parts| !parts.is_empty())
}

fn has_structured_job_command(spec: &ExecutionSpec) -> bool {
    has_command_value(spec.desired_state.get("command"))
        || spec
            .desired_state
            .get("job")
            .and_then(|value| value.get("command"))
            .and_then(Value::as_array)
            .is_some_and(|parts| !parts.is_empty())
        || spec.actions.iter().any(|action| {
            action
                .args
                .get("command")
                .and_then(Value::as_array)
                .is_some_and(|parts| !parts.is_empty())
        })
}

fn has_structured_schedule(spec: &ExecutionSpec) -> bool {
    spec.desired_state
        .get("schedule")
        .and_then(|value| value.get("onCalendar"))
        .and_then(Value::as_str)
        .map(str::trim)
        .is_some_and(|value| !value.is_empty())
        || spec.actions.iter().any(|action| {
            action
                .args
                .get("onCalendar")
                .and_then(Value::as_str)
                .map(str::trim)
                .is_some_and(|value| !value.is_empty())
        })
}

fn has_structured_attachment_state(spec: &ExecutionSpec) -> bool {
    spec.desired_state
        .get("systemdDropIn")
        .and_then(Value::as_object)
        .is_some()
        || spec
            .desired_state
            .get("envPatch")
            .and_then(Value::as_object)
            .is_some()
}

fn collect_claim_resource_refs(spec: &ExecutionSpec) -> Vec<String> {
    let mut refs = Vec::new();
    for claim in &spec.resources.claims {
        for value in [&claim.id, &claim.target, &claim.path] {
            if let Some(value) = value
                .as_deref()
                .map(str::trim)
                .filter(|value| !value.is_empty())
            {
                if !refs.iter().any(|existing| existing == value) {
                    refs.push(value.to_string());
                }
            }
        }
    }
    refs
}

fn action_only_materialized_plan(spec: &ExecutionSpec) -> MaterializedExecutionPlan {
    MaterializedExecutionPlan {
        execution_kind: spec.execution.kind.clone(),
        unit_name: String::new(),
        commands: Vec::new(),
        resources: collect_claim_resource_refs(spec),
        warnings: Vec::new(),
    }
}

fn summary_subject(spec: &ExecutionSpec, plan: &MaterializedExecutionPlan) -> String {
    if !plan.unit_name.trim().is_empty() {
        return plan.unit_name.clone();
    }

    spec.metadata
        .name
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(|value| value.to_string())
        .unwrap_or_else(|| "recipe".into())
}

pub fn materialize_execution_plan(
    spec: &ExecutionSpec,
) -> Result<MaterializedExecutionPlan, String> {
    match spec.execution.kind.as_str() {
        "job" if has_structured_job_command(spec) => {
            let runtime_plan = systemd::materialize_job(spec)?;
            Ok(MaterializedExecutionPlan {
                execution_kind: spec.execution.kind.clone(),
                unit_name: runtime_plan.unit_name,
                commands: runtime_plan.commands,
                resources: runtime_plan.resources,
                warnings: runtime_plan.warnings,
            })
        }
        "service" if has_structured_job_command(spec) => {
            let runtime_plan = systemd::materialize_service(spec)?;
            Ok(MaterializedExecutionPlan {
                execution_kind: spec.execution.kind.clone(),
                unit_name: runtime_plan.unit_name,
                commands: runtime_plan.commands,
                resources: runtime_plan.resources,
                warnings: runtime_plan.warnings,
            })
        }
        "schedule" if has_structured_job_command(spec) && has_structured_schedule(spec) => {
            let runtime_plan = systemd::materialize_schedule(spec)?;
            Ok(MaterializedExecutionPlan {
                execution_kind: spec.execution.kind.clone(),
                unit_name: runtime_plan.unit_name,
                commands: runtime_plan.commands,
                resources: runtime_plan.resources,
                warnings: runtime_plan.warnings,
            })
        }
        "attachment" if has_structured_attachment_state(spec) => {
            let runtime_plan = systemd::materialize_attachment(spec)?;
            Ok(MaterializedExecutionPlan {
                execution_kind: spec.execution.kind.clone(),
                unit_name: runtime_plan.unit_name,
                commands: runtime_plan.commands,
                resources: runtime_plan.resources,
                warnings: runtime_plan.warnings,
            })
        }
        "job" | "attachment" if !spec.actions.is_empty() => Ok(action_only_materialized_plan(spec)),
        other => Err(format!("unsupported execution kind: {}", other)),
    }
}

pub fn route_execution(target: &Value) -> Result<ExecutionRoute, String> {
    let target_kind = target
        .get("kind")
        .and_then(Value::as_str)
        .unwrap_or("local")
        .to_string();

    match target_kind.as_str() {
        "local" | "docker_local" => Ok(ExecutionRoute {
            runner: "local".into(),
            target_kind,
            host_id: None,
        }),
        "remote" | "remote_ssh" => Ok(ExecutionRoute {
            runner: "remote_ssh".into(),
            target_kind,
            host_id: target
                .get("hostId")
                .and_then(Value::as_str)
                .map(|value| value.to_string()),
        }),
        other => Err(format!("unsupported execution target kind: {}", other)),
    }
}

fn push_unique_artifact(
    artifacts: &mut Vec<RecipeRuntimeArtifact>,
    artifact: RecipeRuntimeArtifact,
) {
    if !artifacts.iter().any(|existing| {
        existing.kind == artifact.kind
            && existing.label == artifact.label
            && existing.path == artifact.path
    }) {
        artifacts.push(artifact);
    }
}

fn push_unique_command(commands: &mut Vec<Vec<String>>, command: Vec<String>) {
    if !commands.iter().any(|existing| existing == &command) {
        commands.push(command);
    }
}

pub fn build_runtime_artifacts(
    spec: &ExecutionSpec,
    prepared: &ExecuteRecipePrepared,
) -> Vec<RecipeRuntimeArtifact> {
    let mut artifacts = Vec::new();
    let unit_name = prepared.plan.unit_name.trim();

    match spec.execution.kind.as_str() {
        "job" | "service" if !unit_name.is_empty() => {
            push_unique_artifact(
                &mut artifacts,
                RecipeRuntimeArtifact {
                    id: format!("{}:unit", prepared.run_id),
                    kind: "systemdUnit".into(),
                    label: prepared.plan.unit_name.clone(),
                    path: Some(prepared.plan.unit_name.clone()),
                },
            );
        }
        "schedule" if !unit_name.is_empty() => {
            push_unique_artifact(
                &mut artifacts,
                RecipeRuntimeArtifact {
                    id: format!("{}:unit", prepared.run_id),
                    kind: "systemdUnit".into(),
                    label: prepared.plan.unit_name.clone(),
                    path: Some(prepared.plan.unit_name.clone()),
                },
            );
            push_unique_artifact(
                &mut artifacts,
                RecipeRuntimeArtifact {
                    id: format!("{}:timer", prepared.run_id),
                    kind: "systemdTimer".into(),
                    label: format!("{}.timer", prepared.plan.unit_name),
                    path: Some(format!("{}.timer", prepared.plan.unit_name)),
                },
            );
        }
        "attachment" => {
            if systemd::render_env_patch_dropin_content(spec).is_some() {
                push_unique_artifact(
                    &mut artifacts,
                    RecipeRuntimeArtifact {
                        id: format!("{}:daemon-reload", prepared.run_id),
                        kind: "systemdDaemonReload".into(),
                        label: "systemctl --user daemon-reload".into(),
                        path: None,
                    },
                );
            }

            if let Some(path) = systemd::env_patch_dropin_path(spec) {
                if let Some(target) = systemd::attachment_target_unit(spec) {
                    let name = systemd::env_patch_dropin_name(spec);
                    push_unique_artifact(
                        &mut artifacts,
                        RecipeRuntimeArtifact {
                            id: format!("{}:env-dropin", prepared.run_id),
                            kind: "systemdDropIn".into(),
                            label: format!("{}:{}", target, name),
                            path: Some(path),
                        },
                    );
                }
            }

            if let Some(drop_in) = spec
                .desired_state
                .get("systemdDropIn")
                .and_then(Value::as_object)
            {
                let target = drop_in
                    .get("unit")
                    .or_else(|| drop_in.get("target"))
                    .and_then(Value::as_str)
                    .map(str::trim)
                    .filter(|value| !value.is_empty());
                let name = drop_in
                    .get("name")
                    .and_then(Value::as_str)
                    .map(str::trim)
                    .filter(|value| !value.is_empty());
                if let (Some(target), Some(name)) = (target, name) {
                    push_unique_artifact(
                        &mut artifacts,
                        RecipeRuntimeArtifact {
                            id: format!("{}:dropin", prepared.run_id),
                            kind: "systemdDropIn".into(),
                            label: format!("{}:{}", target, name),
                            path: Some(format!("~/.config/systemd/user/{}.d/{}", target, name)),
                        },
                    );
                }
            }
        }
        _ => {}
    }

    artifacts
}

pub fn build_cleanup_commands(artifacts: &[RecipeRuntimeArtifact]) -> Vec<Vec<String>> {
    let mut commands = Vec::new();

    for artifact in artifacts {
        match artifact.kind.as_str() {
            "systemdUnit" | "systemdTimer" => {
                let target = artifact
                    .path
                    .as_deref()
                    .filter(|value| !value.trim().is_empty())
                    .unwrap_or(&artifact.label);
                push_unique_command(
                    &mut commands,
                    vec![
                        "systemctl".into(),
                        "--user".into(),
                        "stop".into(),
                        target.to_string(),
                    ],
                );
                push_unique_command(
                    &mut commands,
                    vec![
                        "systemctl".into(),
                        "--user".into(),
                        "reset-failed".into(),
                        target.to_string(),
                    ],
                );
            }
            "systemdDaemonReload" => {
                push_unique_command(
                    &mut commands,
                    vec!["systemctl".into(), "--user".into(), "daemon-reload".into()],
                );
            }
            _ => {}
        }
    }

    commands
}

pub fn execute_recipe(request: ExecuteRecipeRequest) -> Result<ExecuteRecipePrepared, String> {
    let plan = materialize_execution_plan(&request.spec)?;
    let route = route_execution(&request.spec.target)?;
    let operation_count = if !plan.commands.is_empty() {
        plan.commands.len()
    } else {
        request.spec.actions.len()
    };
    let operation_label = if !plan.commands.is_empty() {
        "command"
    } else {
        "action"
    };
    let summary = format!(
        "{} via {} ({} {}{})",
        summary_subject(&request.spec, &plan),
        route.runner,
        operation_count,
        operation_label,
        if operation_count == 1 { "" } else { "s" }
    );

    let warnings = plan.warnings.clone();

    Ok(ExecuteRecipePrepared {
        run_id: Uuid::new_v4().to_string(),
        route,
        plan,
        summary,
        warnings,
    })
}
