use serde::Serialize;
use serde_json::{json, Map, Value};
use std::collections::BTreeSet;

use crate::execution_spec::{
    validate_execution_spec, ExecutionAction, ExecutionCapabilities, ExecutionMetadata,
    ExecutionResourceClaim, ExecutionResources, ExecutionSecrets, ExecutionSpec, ExecutionTarget,
};
use crate::recipe::{
    render_step_args, render_template_value, step_references_empty_param, validate, Recipe,
    RecipeParam, RecipeStep,
};
use crate::recipe_bundle::{
    validate_execution_spec_against_bundle, BundleCapabilities, BundleCompatibility,
    BundleExecution, BundleMetadata, BundleResources, BundleRunner, RecipeBundle,
};

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct RecipeSourceDocument {
    pub id: String,
    pub name: String,
    pub description: String,
    pub version: String,
    pub tags: Vec<String>,
    pub difficulty: String,
    pub params: Vec<RecipeParam>,
    pub steps: Vec<RecipeStep>,
    pub bundle: RecipeBundle,
    pub execution_spec_template: ExecutionSpec,
}

pub fn compile_recipe_to_spec(
    recipe: &Recipe,
    params: &Map<String, Value>,
) -> Result<ExecutionSpec, String> {
    let errors = validate(recipe, params);
    if !errors.is_empty() {
        return Err(errors.join(", "));
    }

    if recipe.execution_spec_template.is_some() {
        return compile_structured_recipe_to_spec(recipe, params);
    }

    compile_step_recipe_to_spec(recipe, params)
}

pub fn export_recipe_source(recipe: &Recipe) -> Result<String, String> {
    let execution_spec_template = build_recipe_spec_template(recipe)?;
    let bundle = canonical_recipe_bundle(recipe, &execution_spec_template);
    let document = RecipeSourceDocument {
        id: recipe.id.clone(),
        name: recipe.name.clone(),
        description: recipe.description.clone(),
        version: recipe.version.clone(),
        tags: recipe.tags.clone(),
        difficulty: recipe.difficulty.clone(),
        params: recipe.params.clone(),
        steps: recipe.steps.clone(),
        bundle,
        execution_spec_template,
    };
    serde_json::to_string_pretty(&document).map_err(|error| error.to_string())
}

pub(crate) fn build_recipe_spec_template(recipe: &Recipe) -> Result<ExecutionSpec, String> {
    if let Some(template) = &recipe.execution_spec_template {
        return Ok(template.clone());
    }
    build_step_recipe_template(recipe)
}

fn compile_structured_recipe_to_spec(
    recipe: &Recipe,
    params: &Map<String, Value>,
) -> Result<ExecutionSpec, String> {
    let template = recipe
        .execution_spec_template
        .as_ref()
        .ok_or_else(|| format!("recipe '{}' is missing executionSpecTemplate", recipe.id))?;
    let template_value = serde_json::to_value(template).map_err(|error| error.to_string())?;
    let rendered_template = render_template_value(&template_value, params);
    let mut spec: ExecutionSpec =
        serde_json::from_value(rendered_template).map_err(|error| error.to_string())?;

    filter_optional_structured_actions(recipe, params, &mut spec)?;
    normalize_recipe_spec(recipe, &mut spec, "structuredTemplate");

    if let Some((used_capabilities, claims)) = infer_recipe_action_requirements(&spec.actions) {
        spec.capabilities.used_capabilities = used_capabilities;
        spec.resources.claims = claims;
    }

    validate_recipe_spec(recipe, &spec)?;
    Ok(spec)
}

fn compile_step_recipe_to_spec(
    recipe: &Recipe,
    params: &Map<String, Value>,
) -> Result<ExecutionSpec, String> {
    let mut used_capabilities = Vec::new();
    let mut claims = Vec::new();
    let mut actions = Vec::new();

    for step in &recipe.steps {
        if step_references_empty_param(step, params) {
            continue;
        }

        let rendered_args = render_step_args(&step.args, params);
        collect_action_requirements(
            step.action.as_str(),
            &rendered_args,
            &mut used_capabilities,
            &mut claims,
        );
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

    let mut spec = ExecutionSpec {
        api_version: "strategy.platform/v1".into(),
        kind: "ExecutionSpec".into(),
        metadata: ExecutionMetadata {
            name: Some(recipe.id.clone()),
            digest: None,
        },
        source: Value::Object(Map::new()),
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
    };

    normalize_recipe_spec(recipe, &mut spec, "stepAdapter");
    validate_recipe_spec(recipe, &spec)?;
    Ok(spec)
}

fn build_step_recipe_template(recipe: &Recipe) -> Result<ExecutionSpec, String> {
    let mut used_capabilities = Vec::new();
    let mut claims = Vec::new();
    let mut actions = Vec::new();

    for step in &recipe.steps {
        collect_action_requirements(
            step.action.as_str(),
            &step.args,
            &mut used_capabilities,
            &mut claims,
        );
        actions.push(build_recipe_action(step, step.args.clone())?);
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
        source: Value::Object(Map::new()),
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

fn normalize_recipe_spec(recipe: &Recipe, spec: &mut ExecutionSpec, compiler: &str) {
    if spec.metadata.name.is_none() {
        spec.metadata.name = Some(recipe.id.clone());
    }

    let mut source = spec.source.as_object().cloned().unwrap_or_default();
    source.insert("recipeId".into(), Value::String(recipe.id.clone()));
    source.insert(
        "recipeVersion".into(),
        Value::String(recipe.version.clone()),
    );
    source.insert("recipeCompiler".into(), Value::String(compiler.into()));
    spec.source = Value::Object(source);

    if let Some(desired_state) = spec.desired_state.as_object_mut() {
        desired_state.insert("actionCount".into(), json!(spec.actions.len()));
    } else {
        spec.desired_state = json!({
            "actionCount": spec.actions.len(),
        });
    }

    if spec.outputs.is_empty() {
        spec.outputs.push(json!({
            "kind": "recipe-summary",
            "recipeId": recipe.id,
        }));
    }
}

fn validate_recipe_spec(recipe: &Recipe, spec: &ExecutionSpec) -> Result<(), String> {
    if let Some(bundle) = &recipe.bundle {
        validate_execution_spec_against_bundle(bundle, spec)
    } else {
        validate_execution_spec(spec)
    }
}

pub(crate) fn canonical_recipe_bundle(recipe: &Recipe, spec: &ExecutionSpec) -> RecipeBundle {
    if let Some(bundle) = &recipe.bundle {
        return bundle.clone();
    }

    let allowed_capabilities = spec
        .capabilities
        .used_capabilities
        .iter()
        .cloned()
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect();
    let supported_resource_kinds = spec
        .resources
        .claims
        .iter()
        .map(|claim| claim.kind.clone())
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect();

    RecipeBundle {
        api_version: "strategy.platform/v1".into(),
        kind: "StrategyBundle".into(),
        metadata: BundleMetadata {
            name: Some(recipe.id.clone()),
            version: Some(recipe.version.clone()),
            description: Some(recipe.description.clone()),
        },
        compatibility: BundleCompatibility::default(),
        inputs: Vec::new(),
        capabilities: BundleCapabilities {
            allowed: allowed_capabilities,
        },
        resources: BundleResources {
            supported_kinds: supported_resource_kinds,
        },
        execution: BundleExecution {
            supported_kinds: vec![spec.execution.kind.clone()],
        },
        runner: BundleRunner::default(),
        outputs: spec.outputs.clone(),
    }
}

fn filter_optional_structured_actions(
    recipe: &Recipe,
    params: &Map<String, Value>,
    spec: &mut ExecutionSpec,
) -> Result<(), String> {
    let skipped_step_indices: BTreeSet<usize> = recipe
        .steps
        .iter()
        .enumerate()
        .filter(|(_, step)| step_references_empty_param(step, params))
        .map(|(index, _)| index)
        .collect();
    if skipped_step_indices.is_empty() {
        return Ok(());
    }

    if spec.actions.len() != recipe.steps.len() {
        return Err(format!(
            "recipe '{}' executionSpecTemplate must align actions with UI steps for optional step elision",
            recipe.id
        ));
    }

    spec.actions = spec
        .actions
        .iter()
        .enumerate()
        .filter_map(|(index, action)| {
            if skipped_step_indices.contains(&index) {
                None
            } else {
                Some(action.clone())
            }
        })
        .collect();
    Ok(())
}

fn infer_recipe_action_requirements(
    actions: &[ExecutionAction],
) -> Option<(Vec<String>, Vec<ExecutionResourceClaim>)> {
    let mut used_capabilities = Vec::new();
    let mut claims = Vec::new();

    for action in actions {
        let kind = action.kind.as_deref()?;
        let args = action.args.as_object()?;
        if !matches!(
            kind,
            "create_agent" | "setup_identity" | "bind_channel" | "config_patch"
        ) {
            return None;
        }

        collect_action_requirements(kind, args, &mut used_capabilities, &mut claims);
    }

    Some((used_capabilities, claims))
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

fn collect_action_requirements(
    action_kind: &str,
    rendered_args: &Map<String, Value>,
    used_capabilities: &mut Vec<String>,
    claims: &mut Vec<ExecutionResourceClaim>,
) {
    match action_kind {
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
