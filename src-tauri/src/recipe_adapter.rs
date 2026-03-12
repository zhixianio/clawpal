use serde_json::{json, Map, Value};

use crate::execution_spec::{
    ExecutionAction, ExecutionCapabilities, ExecutionMetadata, ExecutionResourceClaim,
    ExecutionResources, ExecutionSecrets, ExecutionSpec, ExecutionTarget,
};
use crate::recipe::{render_step_args, step_references_empty_param, validate, Recipe, RecipeStep};

pub fn compile_recipe_to_spec(
    recipe: &Recipe,
    params: &Map<String, Value>,
) -> Result<ExecutionSpec, String> {
    let errors = validate(recipe, params);
    if !errors.is_empty() {
        return Err(errors.join(", "));
    }

    let mut used_capabilities = Vec::new();
    let mut claims = Vec::new();
    let mut actions = Vec::new();

    for step in &recipe.steps {
        if step_references_empty_param(step, params) {
            continue;
        }

        let rendered_args = render_step_args(&step.args, params);
        collect_step_requirements(step, &rendered_args, &mut used_capabilities, &mut claims);
        actions.push(build_recipe_action(step, rendered_args)?);
    }

    let execution_kind = if actions
        .iter()
        .all(|action| action.kind.as_deref() == Some("config_patch"))
    {
        "attachment"
    } else {
        "job"
    };

    Ok(ExecutionSpec {
        api_version: "strategy.platform/v1".into(),
        kind: "ExecutionSpec".into(),
        metadata: ExecutionMetadata {
            name: Some(recipe.id.clone()),
            digest: None,
        },
        source: json!({
            "recipeId": recipe.id,
            "recipeVersion": recipe.version,
            "recipeCompiler": "stepAdapter",
        }),
        target: Value::Object(Map::new()),
        execution: ExecutionTarget {
            kind: execution_kind.into(),
        },
        capabilities: ExecutionCapabilities { used_capabilities },
        resources: ExecutionResources { claims },
        secrets: ExecutionSecrets::default(),
        desired_state: json!({
            "actionCount": actions.len(),
        }),
        actions,
        outputs: vec![json!({
            "kind": "recipe-summary",
            "recipeId": recipe.id,
        })],
    })
}

fn build_recipe_action(
    step: &RecipeStep,
    mut rendered_args: Map<String, Value>,
) -> Result<ExecutionAction, String> {
    let args = if step.action == "config_patch" {
        let mut action_args = Map::new();
        if let Some(Value::String(patch_template)) = rendered_args.remove("patchTemplate") {
            let patch: Value =
                json5::from_str(&patch_template).map_err(|error| error.to_string())?;
            action_args.insert("patchTemplate".into(), Value::String(patch_template));
            action_args.insert("patch".into(), patch);
        }
        action_args.extend(rendered_args);
        Value::Object(action_args)
    } else {
        Value::Object(rendered_args)
    };

    Ok(ExecutionAction {
        kind: Some(step.action.clone()),
        name: Some(step.label.clone()),
        args,
    })
}

fn collect_step_requirements(
    step: &RecipeStep,
    rendered_args: &Map<String, Value>,
    used_capabilities: &mut Vec<String>,
    claims: &mut Vec<ExecutionResourceClaim>,
) {
    match step.action.as_str() {
        "create_agent" => {
            push_capability(used_capabilities, "agent.manage");
            push_optional_id_claim(claims, "agent", rendered_args.get("agentId"));
        }
        "setup_identity" => {
            push_capability(used_capabilities, "agent.identity.write");
            push_optional_id_claim(claims, "agent", rendered_args.get("agentId"));
        }
        "bind_channel" => {
            push_capability(used_capabilities, "binding.manage");
            let channel_id = rendered_args
                .get("peerId")
                .and_then(Value::as_str)
                .map(|value| value.to_string());
            let agent_id = rendered_args
                .get("agentId")
                .and_then(Value::as_str)
                .map(|value| value.to_string());
            push_claim(
                claims,
                ExecutionResourceClaim {
                    kind: "channel".into(),
                    id: channel_id,
                    target: agent_id,
                    path: None,
                },
            );
        }
        "config_patch" => {
            push_capability(used_capabilities, "config.write");
            push_claim(
                claims,
                ExecutionResourceClaim {
                    kind: "file".into(),
                    id: Some("openclaw.config".into()),
                    target: None,
                    path: Some("openclaw.config".into()),
                },
            );
        }
        _ => {}
    }
}

fn push_capability(target: &mut Vec<String>, capability: &str) {
    if !target.iter().any(|item| item == capability) {
        target.push(capability.into());
    }
}

fn push_optional_id_claim(
    claims: &mut Vec<ExecutionResourceClaim>,
    kind: &str,
    id: Option<&Value>,
) {
    let id = id.and_then(Value::as_str).map(|value| value.to_string());
    push_claim(
        claims,
        ExecutionResourceClaim {
            kind: kind.into(),
            id,
            target: None,
            path: None,
        },
    );
}

fn push_claim(claims: &mut Vec<ExecutionResourceClaim>, next: ExecutionResourceClaim) {
    let exists = claims.iter().any(|claim| {
        claim.kind == next.kind
            && claim.id == next.id
            && claim.target == next.target
            && claim.path == next.path
    });
    if !exists {
        claims.push(next);
    }
}
