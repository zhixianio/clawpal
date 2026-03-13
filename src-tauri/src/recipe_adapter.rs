use serde::Serialize;
use serde_json::{json, Map, Value};
use std::collections::BTreeSet;

use crate::execution_spec::{
    validate_execution_spec, ExecutionAction, ExecutionCapabilities, ExecutionMetadata,
    ExecutionResourceClaim, ExecutionResources, ExecutionSecrets, ExecutionSpec, ExecutionTarget,
};
use crate::recipe::{
    render_step_args, render_template_value, step_references_empty_param, validate, Recipe,
    RecipeParam, RecipePresentation, RecipeStep,
};
use crate::recipe_action_catalog::find_recipe_action as find_recipe_action_catalog_entry;
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
    #[serde(skip_serializing_if = "Option::is_none")]
    pub presentation: Option<RecipePresentation>,
    pub params: Vec<RecipeParam>,
    pub steps: Vec<RecipeStep>,
    #[serde(skip_serializing_if = "Option::is_none", rename = "clawpalPresetMaps")]
    pub clawpal_preset_maps: Option<Map<String, Value>>,
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
        presentation: recipe.presentation.clone(),
        params: recipe.params.clone(),
        steps: recipe.steps.clone(),
        clawpal_preset_maps: recipe.clawpal_preset_maps.clone(),
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
    let rendered_template =
        render_template_value(&template_value, params, recipe.clawpal_preset_maps.as_ref());
    let mut spec: ExecutionSpec =
        serde_json::from_value(rendered_template).map_err(|error| error.to_string())?;

    filter_optional_structured_actions(recipe, params, &mut spec)?;
    validate_recipe_action_kinds(&spec.actions)?;
    normalize_recipe_spec(recipe, Some(params), &mut spec, "structuredTemplate");

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

        let rendered_args =
            render_step_args(&step.args, params, recipe.clawpal_preset_maps.as_ref());
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

    normalize_recipe_spec(recipe, Some(params), &mut spec, "stepAdapter");
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

    normalize_recipe_spec(recipe, None, &mut spec, "stepTemplate");
    Ok(spec)
}

fn build_recipe_presentation_source(
    recipe: &Recipe,
    params: Option<&Map<String, Value>>,
) -> Option<Value> {
    let presentation = recipe.presentation.as_ref()?;
    let raw_value = serde_json::to_value(presentation).ok()?;
    Some(match params {
        Some(params) => {
            render_template_value(&raw_value, params, recipe.clawpal_preset_maps.as_ref())
        }
        None => raw_value,
    })
}

fn normalize_recipe_spec(
    recipe: &Recipe,
    params: Option<&Map<String, Value>>,
    spec: &mut ExecutionSpec,
    compiler: &str,
) {
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
    if let Some(presentation) = build_recipe_presentation_source(recipe, params) {
        source.insert("recipePresentation".into(), presentation);
    }
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
        let entry = find_recipe_action_catalog_entry(kind)?;
        if !entry.runner_supported {
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
    let action_entry = find_recipe_action_catalog_entry(step.action.as_str())
        .ok_or_else(|| format!("recipe action '{}' is not recognized", step.action))?;
    if !action_entry.runner_supported {
        return Err(format!(
            "recipe action '{}' is documented but not supported by the Recipe runner",
            step.action
        ));
    }

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

fn validate_recipe_action_kinds(actions: &[ExecutionAction]) -> Result<(), String> {
    for action in actions {
        let kind = action
            .kind
            .as_deref()
            .ok_or_else(|| "recipe action is missing kind".to_string())?;
        let entry = find_recipe_action_catalog_entry(kind)
            .ok_or_else(|| format!("recipe action '{}' is not recognized", kind))?;
        if !entry.runner_supported {
            return Err(format!(
                "recipe action '{}' is documented but not supported by the Recipe runner",
                kind
            ));
        }
    }
    Ok(())
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
        "delete_agent" => {
            push_capability(used_capabilities, "agent.manage");
            push_optional_id_claim(claims, "agent", rendered_args.get("agentId"));
        }
        "setup_identity" => {
            push_capability(used_capabilities, "agent.identity.write");
            push_optional_id_claim(claims, "agent", rendered_args.get("agentId"));
        }
        "set_agent_identity" => {
            push_capability(used_capabilities, "agent.identity.write");
            push_optional_id_claim(claims, "agent", rendered_args.get("agentId"));
        }
        "set_agent_persona" | "clear_agent_persona" => {
            push_capability(used_capabilities, "agent.identity.write");
            push_optional_id_claim(claims, "agent", rendered_args.get("agentId"));
        }
        "bind_agent" => {
            push_capability(used_capabilities, "binding.manage");
            let channel_id = rendered_args
                .get("binding")
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
        "unbind_agent" => {
            push_capability(used_capabilities, "binding.manage");
            let channel_id = rendered_args
                .get("binding")
                .and_then(Value::as_str)
                .map(|value| value.to_string());
            push_claim(
                claims,
                ExecutionResourceClaim {
                    kind: "channel".into(),
                    id: channel_id,
                    target: None,
                    path: None,
                },
            );
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
        "unbind_channel" => {
            push_capability(used_capabilities, "binding.manage");
            let channel_id = rendered_args
                .get("peerId")
                .and_then(Value::as_str)
                .map(|value| value.to_string());
            push_claim(
                claims,
                ExecutionResourceClaim {
                    kind: "channel".into(),
                    id: channel_id,
                    target: None,
                    path: None,
                },
            );
        }
        "set_agent_model" => {
            push_capability(used_capabilities, "model.manage");
            if rendered_args
                .get("ensureProfile")
                .and_then(Value::as_bool)
                .unwrap_or(true)
            {
                push_capability(used_capabilities, "secret.sync");
            }
            push_optional_id_claim(claims, "agent", rendered_args.get("agentId"));
            push_optional_id_claim(claims, "modelProfile", rendered_args.get("profileId"));
        }
        "set_channel_persona" | "clear_channel_persona" => {
            push_capability(used_capabilities, "config.write");
            let channel_id = rendered_args
                .get("peerId")
                .and_then(Value::as_str)
                .map(|value| value.to_string());
            push_claim(
                claims,
                ExecutionResourceClaim {
                    kind: "channel".into(),
                    id: channel_id,
                    target: None,
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
        "set_config_value" | "unset_config_value" => {
            push_capability(used_capabilities, "config.write");
            push_claim(
                claims,
                ExecutionResourceClaim {
                    kind: "file".into(),
                    id: action_string(rendered_args.get("path")),
                    target: None,
                    path: action_string(rendered_args.get("path")),
                },
            );
        }
        "set_default_model" => {
            push_capability(used_capabilities, "model.manage");
            push_optional_id_claim(claims, "modelProfile", rendered_args.get("modelOrAlias"));
        }
        "upsert_markdown_document" => {
            push_capability(used_capabilities, "document.write");
            if let Some(path) = document_target_claim_path(rendered_args) {
                push_claim(
                    claims,
                    ExecutionResourceClaim {
                        kind: "document".into(),
                        id: None,
                        target: None,
                        path: Some(path),
                    },
                );
            }
        }
        "delete_markdown_document" => {
            push_capability(used_capabilities, "document.delete");
            if let Some(path) = document_target_claim_path(rendered_args) {
                push_claim(
                    claims,
                    ExecutionResourceClaim {
                        kind: "document".into(),
                        id: None,
                        target: None,
                        path: Some(path),
                    },
                );
            }
        }
        "ensure_model_profile" => {
            push_capability(used_capabilities, "model.manage");
            push_capability(used_capabilities, "secret.sync");
            push_optional_id_claim(claims, "modelProfile", rendered_args.get("profileId"));
        }
        "delete_model_profile" => {
            push_capability(used_capabilities, "model.manage");
            push_optional_id_claim(claims, "modelProfile", rendered_args.get("profileId"));
            if action_bool(rendered_args.get("deleteAuthRef")) {
                if let Some(auth_ref) = action_string(rendered_args.get("authRef")) {
                    push_claim(
                        claims,
                        ExecutionResourceClaim {
                            kind: "authProfile".into(),
                            id: Some(auth_ref),
                            target: None,
                            path: None,
                        },
                    );
                }
            }
        }
        "ensure_provider_auth" => {
            push_capability(used_capabilities, "auth.manage");
            push_capability(used_capabilities, "secret.sync");
            let auth_ref = action_string(rendered_args.get("authRef")).or_else(|| {
                action_string(rendered_args.get("provider"))
                    .map(|provider| format!("{}:default", provider.trim().to_ascii_lowercase()))
            });
            push_claim(
                claims,
                ExecutionResourceClaim {
                    kind: "authProfile".into(),
                    id: auth_ref,
                    target: None,
                    path: None,
                },
            );
        }
        "delete_provider_auth" => {
            push_capability(used_capabilities, "auth.manage");
            push_optional_id_claim(claims, "authProfile", rendered_args.get("authRef"));
        }
        "apply_secrets_plan" => {
            push_capability(used_capabilities, "auth.manage");
            push_capability(used_capabilities, "secret.sync");
            push_claim(
                claims,
                ExecutionResourceClaim {
                    kind: "file".into(),
                    id: action_string(rendered_args.get("fromPath")),
                    target: None,
                    path: action_string(rendered_args.get("fromPath")),
                },
            );
        }
        _ => {}
    }
}

fn document_target_claim_path(rendered_args: &Map<String, Value>) -> Option<String> {
    let target = rendered_args.get("target")?.as_object()?;
    let scope = target.get("scope").and_then(Value::as_str)?.trim();
    let path = target.get("path").and_then(Value::as_str)?.trim();
    if scope.is_empty() || path.is_empty() {
        return None;
    }

    if scope == "agent" {
        let agent_id = target.get("agentId").and_then(Value::as_str)?.trim();
        if agent_id.is_empty() {
            return None;
        }
        return Some(format!("agent:{agent_id}/{path}"));
    }

    Some(format!("{scope}:{path}"))
}

fn push_capability(target: &mut Vec<String>, capability: &str) {
    if !target.iter().any(|item| item == capability) {
        target.push(capability.into());
    }
}

fn action_string(value: Option<&Value>) -> Option<String> {
    value.and_then(|value| match value {
        Value::String(text) => {
            let trimmed = text.trim();
            if trimmed.is_empty() {
                None
            } else {
                Some(trimmed.to_string())
            }
        }
        _ => None,
    })
}

fn action_bool(value: Option<&Value>) -> bool {
    match value {
        Some(Value::Bool(value)) => *value,
        Some(Value::String(value)) => value.trim().eq_ignore_ascii_case("true"),
        _ => false,
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
