use serde_json::{Map, Value};

use crate::recipe::{builtin_recipes, validate_recipe_source, Recipe, RecipeParam, RecipeStep};
use crate::recipe_adapter::{compile_recipe_to_spec, export_recipe_source};

fn sample_params() -> Map<String, Value> {
    let mut params = Map::new();
    params.insert("agent_id".into(), Value::String("bot-alpha".into()));
    params.insert("model".into(), Value::String("__default__".into()));
    params.insert("guild_id".into(), Value::String("guild-1".into()));
    params.insert("channel_id".into(), Value::String("channel-1".into()));
    params.insert("independent".into(), Value::String("true".into()));
    params.insert("name".into(), Value::String("Bot Alpha".into()));
    params.insert("emoji".into(), Value::String(":claw:".into()));
    params.insert(
        "persona".into(),
        Value::String("You are a focused channel assistant.".into()),
    );
    params
}

#[test]
fn recipe_compiles_to_attachment_or_job_spec() {
    let recipe = builtin_recipes()
        .into_iter()
        .find(|recipe| recipe.id == "dedicated-channel-agent")
        .expect("builtin recipe");

    let spec = compile_recipe_to_spec(&recipe, &sample_params()).expect("compile spec");

    assert!(matches!(spec.execution.kind.as_str(), "attachment" | "job"));
    assert!(!spec.actions.is_empty());
    assert_eq!(
        spec.source.get("recipeId").and_then(Value::as_str),
        Some(recipe.id.as_str())
    );
    assert_eq!(
        spec.source.get("recipeCompiler").and_then(Value::as_str),
        Some("structuredTemplate")
    );
    assert!(spec.source.get("legacyRecipeId").is_none());
}

#[test]
fn config_patch_only_recipe_compiles_to_attachment_spec() {
    let recipe = builtin_recipes()
        .into_iter()
        .find(|recipe| recipe.id == "discord-channel-persona")
        .expect("builtin recipe");

    let spec = compile_recipe_to_spec(&recipe, &sample_params()).expect("compile spec");

    assert_eq!(spec.execution.kind, "attachment");
    assert_eq!(spec.actions.len(), 1);
    assert_eq!(
        spec.outputs[0].get("kind").and_then(Value::as_str),
        Some("recipe-summary")
    );
    let patch = spec.actions[0]
        .args
        .get("patch")
        .and_then(Value::as_object)
        .expect("rendered patch");
    assert!(patch.get("channels").is_some());
    let rendered_patch = serde_json::to_string(&spec.actions[0].args).expect("patch json");
    assert!(rendered_patch.contains("\"guild-1\""));
    assert!(rendered_patch.contains("\"channel-1\""));
    assert!(!rendered_patch.contains("{{guild_id}}"));
}

#[test]
fn structured_recipe_template_skips_optional_actions_with_empty_params() {
    let recipe = builtin_recipes()
        .into_iter()
        .find(|recipe| recipe.id == "dedicated-channel-agent")
        .expect("builtin recipe");
    let mut params = sample_params();
    params.insert("name".into(), Value::String(String::new()));
    params.insert("emoji".into(), Value::String(String::new()));
    params.insert("persona".into(), Value::String(String::new()));

    let spec = compile_recipe_to_spec(&recipe, &params).expect("compile spec");

    assert_eq!(spec.actions.len(), 2);
    assert_eq!(spec.actions[0].kind.as_deref(), Some("create_agent"));
    assert_eq!(spec.actions[1].kind.as_deref(), Some("bind_channel"));
}

#[test]
fn export_recipe_source_normalizes_step_only_recipe_to_structured_document() {
    let recipe = Recipe {
        id: "legacy-channel-persona".into(),
        name: "Legacy Channel Persona".into(),
        description: "Set channel persona with steps only".into(),
        version: "1.0.0".into(),
        tags: vec!["discord".into(), "persona".into()],
        difficulty: "easy".into(),
        params: vec![
            RecipeParam {
                id: "guild_id".into(),
                label: "Guild".into(),
                kind: "discord_guild".into(),
                required: true,
                pattern: None,
                min_length: None,
                max_length: None,
                placeholder: None,
                depends_on: None,
                default_value: None,
            },
            RecipeParam {
                id: "channel_id".into(),
                label: "Channel".into(),
                kind: "discord_channel".into(),
                required: true,
                pattern: None,
                min_length: None,
                max_length: None,
                placeholder: None,
                depends_on: None,
                default_value: None,
            },
        ],
        steps: vec![RecipeStep {
            action: "config_patch".into(),
            label: "Set channel persona".into(),
            args: serde_json::from_value(serde_json::json!({
                "patchTemplate": "{\"channels\":{\"discord\":{\"guilds\":{\"{{guild_id}}\":{\"channels\":{\"{{channel_id}}\":{\"systemPrompt\":\"hello\"}}}}}}}"
            }))
            .expect("step args"),
        }],
        bundle: None,
        execution_spec_template: None,
    };

    let exported = export_recipe_source(&recipe).expect("export source");

    assert!(exported.contains("\"bundle\""));
    assert!(exported.contains("\"executionSpecTemplate\""));
    assert!(exported.contains("\"supportedKinds\": [\n        \"attachment\""));
    assert!(exported.contains("\"{{guild_id}}\""));
}

#[test]
fn exported_recipe_source_validates_as_structured_document() {
    let recipe = builtin_recipes()
        .into_iter()
        .find(|recipe| recipe.id == "discord-channel-persona")
        .expect("builtin recipe");
    let source = export_recipe_source(&recipe).expect("export source");

    let diagnostics = validate_recipe_source(&source).expect("validate source");

    assert!(diagnostics.errors.is_empty());
}

#[test]
fn validate_recipe_source_flags_parse_errors() {
    let diagnostics = validate_recipe_source("{ broken").expect("validate source");

    assert_eq!(diagnostics.errors.len(), 1);
    assert_eq!(diagnostics.errors[0].category, "parse");
}

#[test]
fn validate_recipe_source_flags_bundle_consistency_errors() {
    let diagnostics = validate_recipe_source(
        r#"{
          "recipes": [{
            "id": "bundle-mismatch",
            "name": "Bundle Mismatch",
            "description": "Invalid bundle/spec pairing",
            "version": "1.0.0",
            "tags": [],
            "difficulty": "easy",
            "params": [],
            "steps": [],
            "bundle": {
              "apiVersion": "strategy.platform/v1",
              "kind": "StrategyBundle",
              "metadata": {},
              "compatibility": {},
              "inputs": [],
              "capabilities": { "allowed": [] },
              "resources": { "supportedKinds": [] },
              "execution": { "supportedKinds": ["attachment"] },
              "runner": {},
              "outputs": []
            },
            "executionSpecTemplate": {
              "apiVersion": "strategy.platform/v1",
              "kind": "ExecutionSpec",
              "metadata": {},
              "source": {},
              "target": {},
              "execution": { "kind": "job" },
              "capabilities": { "usedCapabilities": [] },
              "resources": { "claims": [] },
              "secrets": { "bindings": [] },
              "desiredState": {},
              "actions": [],
              "outputs": []
            }
          }]
        }"#,
    )
    .expect("validate source");

    assert_eq!(diagnostics.errors.len(), 1);
    assert_eq!(diagnostics.errors[0].category, "bundle");
}

#[test]
fn validate_recipe_source_flags_step_alignment_errors() {
    let diagnostics = validate_recipe_source(
        r#"{
          "recipes": [{
            "id": "step-mismatch",
            "name": "Step Mismatch",
            "description": "Invalid step/action alignment",
            "version": "1.0.0",
            "tags": [],
            "difficulty": "easy",
            "params": [],
            "steps": [
              { "action": "config_patch", "label": "First", "args": {} },
              { "action": "config_patch", "label": "Second", "args": {} }
            ],
            "bundle": {
              "apiVersion": "strategy.platform/v1",
              "kind": "StrategyBundle",
              "metadata": {},
              "compatibility": {},
              "inputs": [],
              "capabilities": { "allowed": [] },
              "resources": { "supportedKinds": [] },
              "execution": { "supportedKinds": ["attachment"] },
              "runner": {},
              "outputs": []
            },
            "executionSpecTemplate": {
              "apiVersion": "strategy.platform/v1",
              "kind": "ExecutionSpec",
              "metadata": {},
              "source": {},
              "target": {},
              "execution": { "kind": "attachment" },
              "capabilities": { "usedCapabilities": [] },
              "resources": { "claims": [] },
              "secrets": { "bindings": [] },
              "desiredState": {},
              "actions": [
                { "kind": "config_patch", "name": "Only action", "args": {} }
              ],
              "outputs": []
            }
          }]
        }"#,
    )
    .expect("validate source");

    assert_eq!(diagnostics.errors.len(), 1);
    assert_eq!(diagnostics.errors[0].category, "alignment");
}

#[test]
fn validate_recipe_source_flags_hidden_actions_without_ui_steps() {
    let diagnostics = validate_recipe_source(
        r#"{
          "recipes": [{
            "id": "hidden-actions",
            "name": "Hidden Actions",
            "description": "Execution actions without UI steps",
            "version": "1.0.0",
            "tags": [],
            "difficulty": "easy",
            "params": [],
            "steps": [],
            "bundle": {
              "apiVersion": "strategy.platform/v1",
              "kind": "StrategyBundle",
              "metadata": {},
              "compatibility": {},
              "inputs": [],
              "capabilities": { "allowed": [] },
              "resources": { "supportedKinds": [] },
              "execution": { "supportedKinds": ["attachment"] },
              "runner": {},
              "outputs": []
            },
            "executionSpecTemplate": {
              "apiVersion": "strategy.platform/v1",
              "kind": "ExecutionSpec",
              "metadata": {},
              "source": {},
              "target": {},
              "execution": { "kind": "attachment" },
              "capabilities": { "usedCapabilities": [] },
              "resources": { "claims": [] },
              "secrets": { "bindings": [] },
              "desiredState": {},
              "actions": [
                { "kind": "config_patch", "name": "Only action", "args": {} }
              ],
              "outputs": []
            }
          }]
        }"#,
    )
    .expect("validate source");

    assert_eq!(diagnostics.errors.len(), 1);
    assert_eq!(diagnostics.errors[0].category, "alignment");
}
