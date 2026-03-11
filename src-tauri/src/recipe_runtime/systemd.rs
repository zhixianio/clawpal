use serde_json::Value;

use crate::execution_spec::ExecutionSpec;

#[derive(Debug, Clone, Default)]
pub struct SystemdRuntimePlan {
    pub unit_name: String,
    pub commands: Vec<Vec<String>>,
    pub resources: Vec<String>,
    pub warnings: Vec<String>,
}

pub fn materialize_job(spec: &ExecutionSpec) -> Result<SystemdRuntimePlan, String> {
    let command = extract_command(spec)?;
    let unit_name = job_unit_name(spec);

    Ok(SystemdRuntimePlan {
        unit_name: unit_name.clone(),
        commands: vec![build_systemd_run_command(&unit_name, &command, None)],
        resources: collect_resource_refs(spec),
        warnings: Vec::new(),
    })
}

pub fn materialize_service(spec: &ExecutionSpec) -> Result<SystemdRuntimePlan, String> {
    let command = extract_command(spec)?;
    let unit_name = service_unit_name(spec);

    Ok(SystemdRuntimePlan {
        unit_name: unit_name.clone(),
        commands: vec![build_systemd_run_command(
            &unit_name,
            &command,
            Some(&["--property=Restart=always", "--property=RestartSec=5s"]),
        )],
        resources: collect_resource_refs(spec),
        warnings: Vec::new(),
    })
}

pub fn materialize_schedule(spec: &ExecutionSpec) -> Result<SystemdRuntimePlan, String> {
    let command = extract_command(spec)?;
    let unit_name = job_unit_name(spec);
    let on_calendar = extract_schedule(spec)
        .as_deref()
        .ok_or_else(|| "schedule spec is missing desired_state.schedule.onCalendar".to_string())?
        .to_string();

    let mut resources = collect_resource_refs(spec);
    let launch_ref = format!("job/{}", sanitize_unit_fragment(spec_name(spec)));
    if !resources.iter().any(|resource| resource == &launch_ref) {
        resources.push(launch_ref);
    }

    Ok(SystemdRuntimePlan {
        unit_name: unit_name.clone(),
        commands: vec![build_systemd_run_command(
            &unit_name,
            &command,
            Some(&[
                "--timer-property=Persistent=true",
                &format!("--on-calendar={}", on_calendar),
            ]),
        )],
        resources,
        warnings: Vec::new(),
    })
}

pub fn materialize_attachment(spec: &ExecutionSpec) -> Result<SystemdRuntimePlan, String> {
    let unit_name = attachment_unit_name(spec);
    let mut commands = Vec::new();
    let mut warnings = Vec::new();

    if let Some(drop_in_name) = spec
        .desired_state
        .get("systemdDropIn")
        .and_then(|value| value.get("name"))
        .and_then(Value::as_str)
    {
        commands.push(vec![
            "systemctl".into(),
            "--user".into(),
            "edit".into(),
            "--drop-in".into(),
            drop_in_name.into(),
        ]);
    }

    if spec
        .desired_state
        .get("envPatch")
        .and_then(Value::as_object)
        .is_some()
    {
        commands.push(vec![
            "systemctl".into(),
            "--user".into(),
            "daemon-reload".into(),
        ]);
    }

    if commands.is_empty() {
        warnings.push(
            "attachment spec materialized without concrete systemdDropIn/envPatch operations"
                .into(),
        );
    }

    Ok(SystemdRuntimePlan {
        unit_name,
        commands,
        resources: collect_resource_refs(spec),
        warnings,
    })
}

fn build_systemd_run_command(
    unit_name: &str,
    command: &[String],
    extra_flags: Option<&[&str]>,
) -> Vec<String> {
    let mut cmd = vec![
        "systemd-run".into(),
        format!("--unit={}", unit_name),
        "--collect".into(),
        "--service-type=exec".into(),
    ];
    if let Some(flags) = extra_flags {
        cmd.extend(flags.iter().map(|flag| flag.to_string()));
    }
    cmd.push("--".into());
    cmd.extend(command.iter().cloned());
    cmd
}

fn collect_resource_refs(spec: &ExecutionSpec) -> Vec<String> {
    let mut resources = Vec::new();

    for claim in &spec.resources.claims {
        if let Some(id) = &claim.id {
            push_unique(&mut resources, id.clone());
        }
        if let Some(target) = &claim.target {
            push_unique(&mut resources, target.clone());
        }
        if let Some(path) = &claim.path {
            push_unique(&mut resources, path.clone());
        }
    }

    if let Some(schedule_id) = spec
        .desired_state
        .get("schedule")
        .and_then(|value| value.get("id"))
        .and_then(Value::as_str)
    {
        push_unique(&mut resources, schedule_id.to_string());
    }

    resources
}

fn extract_command(spec: &ExecutionSpec) -> Result<Vec<String>, String> {
    if let Some(command) = extract_command_from_value(spec.desired_state.get("command")) {
        return Ok(command);
    }
    if let Some(command) = spec
        .desired_state
        .get("job")
        .and_then(|value| value.get("command"))
        .and_then(|value| extract_command_from_value(Some(value)))
    {
        return Ok(command);
    }
    for action in &spec.actions {
        if let Some(command) = action
            .args
            .get("command")
            .and_then(|value| extract_command_from_value(Some(value)))
        {
            return Ok(command);
        }
    }

    Err("execution spec is missing a concrete command payload".into())
}

fn extract_command_from_value(value: Option<&Value>) -> Option<Vec<String>> {
    value
        .and_then(Value::as_array)
        .map(|parts| {
            parts
                .iter()
                .filter_map(|part| part.as_str().map(|text| text.to_string()))
                .collect::<Vec<_>>()
        })
        .filter(|parts| !parts.is_empty())
}

fn extract_schedule(spec: &ExecutionSpec) -> Option<String> {
    spec.desired_state
        .get("schedule")
        .and_then(|value| value.get("onCalendar"))
        .and_then(Value::as_str)
        .map(|value| value.to_string())
        .or_else(|| {
            spec.actions.iter().find_map(|action| {
                action
                    .args
                    .get("onCalendar")
                    .and_then(Value::as_str)
                    .map(|value| value.to_string())
            })
        })
}

fn job_unit_name(spec: &ExecutionSpec) -> String {
    format!("clawpal-job-{}", sanitize_unit_fragment(spec_name(spec)))
}

fn service_unit_name(spec: &ExecutionSpec) -> String {
    format!(
        "clawpal-service-{}",
        sanitize_unit_fragment(spec_name(spec))
    )
}

fn attachment_unit_name(spec: &ExecutionSpec) -> String {
    format!(
        "clawpal-attachment-{}",
        sanitize_unit_fragment(spec_name(spec))
    )
}

fn spec_name(spec: &ExecutionSpec) -> &str {
    spec.metadata
        .name
        .as_deref()
        .filter(|value| !value.trim().is_empty())
        .unwrap_or("spec")
}

fn sanitize_unit_fragment(input: &str) -> String {
    let sanitized: String = input
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() {
                ch.to_ascii_lowercase()
            } else {
                '-'
            }
        })
        .collect();
    let collapsed = sanitized
        .split('-')
        .filter(|segment| !segment.is_empty())
        .collect::<Vec<_>>()
        .join("-");
    if collapsed.is_empty() {
        "spec".into()
    } else {
        collapsed
    }
}

fn push_unique(values: &mut Vec<String>, next: String) {
    if !values.iter().any(|existing| existing == &next) {
        values.push(next);
    }
}
