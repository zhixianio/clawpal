use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::execution_spec::ExecutionSpec;
use crate::recipe_runtime::systemd;

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

pub fn materialize_execution_plan(
    spec: &ExecutionSpec,
) -> Result<MaterializedExecutionPlan, String> {
    let runtime_plan = match spec.execution.kind.as_str() {
        "job" => systemd::materialize_job(spec)?,
        "service" => systemd::materialize_service(spec)?,
        "schedule" => systemd::materialize_schedule(spec)?,
        "attachment" => systemd::materialize_attachment(spec)?,
        other => return Err(format!("unsupported execution kind: {}", other)),
    };

    Ok(MaterializedExecutionPlan {
        execution_kind: spec.execution.kind.clone(),
        unit_name: runtime_plan.unit_name,
        commands: runtime_plan.commands,
        resources: runtime_plan.resources,
        warnings: runtime_plan.warnings,
    })
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
