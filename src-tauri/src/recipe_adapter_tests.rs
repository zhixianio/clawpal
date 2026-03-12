use serde_json::{Map, Value};

use crate::recipe::builtin_recipes;
use crate::recipe_adapter::compile_recipe_to_spec;

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
}
