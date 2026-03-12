use serde_json::{Map, Value};

use crate::recipe::builtin_recipes;
use crate::recipe_planner::build_recipe_plan;

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
