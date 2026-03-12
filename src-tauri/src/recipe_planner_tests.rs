use serde_json::{Map, Value};

use crate::recipe::{builtin_recipes, load_recipes_from_source_text};
use crate::recipe_adapter::export_recipe_source;
use crate::recipe_planner::{build_recipe_plan, build_recipe_plan_from_source_text};

fn sample_inputs() -> Map<String, Value> {
    let mut params = Map::new();
    params.insert("guild_id".into(), Value::String("guild-1".into()));
    params.insert("channel_id".into(), Value::String("channel-1".into()));
    params.insert(
        "persona".into(),
        Value::String("Keep answers concise".into()),
    );
    params
}

#[test]
fn plan_recipe_returns_capabilities_claims_and_digest() {
    let recipe = builtin_recipes()
        .into_iter()
        .find(|recipe| recipe.id == "discord-channel-persona")
        .expect("builtin recipe");

    let plan = build_recipe_plan(&recipe, &sample_inputs()).expect("build plan");

    assert!(!plan.used_capabilities.is_empty());
    assert!(!plan.concrete_claims.is_empty());
    assert!(!plan.execution_spec_digest.is_empty());
}

#[test]
fn plan_recipe_includes_execution_spec_for_executor_bridge() {
    let recipe = builtin_recipes()
        .into_iter()
        .find(|recipe| recipe.id == "discord-channel-persona")
        .expect("builtin recipe");

    let plan = build_recipe_plan(&recipe, &sample_inputs()).expect("build plan");

    assert_eq!(plan.execution_spec.kind, "ExecutionSpec");
    assert!(!plan.execution_spec.actions.is_empty());
}

#[test]
fn plan_recipe_does_not_emit_legacy_bridge_warning() {
    let recipe = builtin_recipes()
        .into_iter()
        .find(|recipe| recipe.id == "discord-channel-persona")
        .expect("builtin recipe");

    let plan = build_recipe_plan(&recipe, &sample_inputs()).expect("build plan");

    assert!(plan
        .warnings
        .iter()
        .all(|warning| !warning.to_ascii_lowercase().contains("legacy")));
}

#[test]
fn plan_recipe_skips_optional_steps_from_structured_template() {
    let recipe = builtin_recipes()
        .into_iter()
        .find(|recipe| recipe.id == "dedicated-channel-agent")
        .expect("builtin recipe");
    let mut params = sample_inputs();
    params.insert("agent_id".into(), Value::String("bot-alpha".into()));
    params.insert("model".into(), Value::String("__default__".into()));
    params.insert("independent".into(), Value::String("true".into()));
    params.insert("name".into(), Value::String(String::new()));
    params.insert("emoji".into(), Value::String(String::new()));
    params.insert("persona".into(), Value::String(String::new()));

    let plan = build_recipe_plan(&recipe, &params).expect("build plan");

    assert_eq!(plan.summary.skipped_step_count, 2);
    assert_eq!(plan.summary.action_count, 2);
    assert_eq!(plan.execution_spec.actions.len(), 2);
}

#[test]
fn plan_recipe_source_uses_unsaved_draft_text() {
    let recipe = builtin_recipes()
        .into_iter()
        .find(|recipe| recipe.id == "discord-channel-persona")
        .expect("builtin recipe");
    let source = export_recipe_source(&recipe).expect("export source");
    let recipes = load_recipes_from_source_text(&source).expect("parse source");

    let plan =
        build_recipe_plan_from_source_text("discord-channel-persona", &sample_inputs(), &source)
            .expect("build plan from source");

    assert_eq!(recipes.len(), 1);
    assert_eq!(plan.summary.recipe_id, "discord-channel-persona");
    assert_eq!(plan.execution_spec.kind, "ExecutionSpec");
}
