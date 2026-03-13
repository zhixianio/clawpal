use serde_json::{Map, Value};

use crate::recipe::{load_recipes_from_source_text, Recipe};
use crate::recipe_adapter::export_recipe_source;
use crate::recipe_planner::{build_recipe_plan, build_recipe_plan_from_source_text};

const TEST_RECIPES_SOURCE: &str = r#"{
  "recipes": [
    {
      "id": "dedicated-channel-agent",
      "name": "Create dedicated Agent for Channel",
      "description": "Create an agent and bind it to a Discord channel",
      "version": "1.0.0",
      "tags": ["discord", "agent", "persona"],
      "difficulty": "easy",
      "params": [
        { "id": "agent_id", "label": "Agent ID", "type": "string", "required": true, "placeholder": "e.g. my-bot" },
        { "id": "model", "label": "Model", "type": "model_profile", "required": true, "defaultValue": "__default__" },
        { "id": "guild_id", "label": "Guild", "type": "discord_guild", "required": true },
        { "id": "channel_id", "label": "Channel", "type": "discord_channel", "required": true },
        { "id": "independent", "label": "Create independent agent", "type": "boolean", "required": false },
        { "id": "name", "label": "Display Name", "type": "string", "required": false, "dependsOn": "independent" },
        { "id": "emoji", "label": "Emoji", "type": "string", "required": false, "dependsOn": "independent" },
        { "id": "persona", "label": "Persona", "type": "textarea", "required": false, "dependsOn": "independent" }
      ],
      "bundle": {
        "apiVersion": "strategy.platform/v1",
        "kind": "StrategyBundle",
        "metadata": {
          "name": "dedicated-channel-agent",
          "version": "1.0.0",
          "description": "Create an agent and bind it to a Discord channel"
        },
        "compatibility": {},
        "inputs": [],
        "capabilities": {
          "allowed": ["agent.manage", "agent.identity.write", "binding.manage", "config.write"]
        },
        "resources": {
          "supportedKinds": ["agent", "channel", "file"]
        },
        "execution": {
          "supportedKinds": ["job"]
        },
        "runner": {},
        "outputs": [{ "kind": "recipe-summary", "recipeId": "dedicated-channel-agent" }]
      },
      "executionSpecTemplate": {
        "apiVersion": "strategy.platform/v1",
        "kind": "ExecutionSpec",
        "metadata": {
          "name": "dedicated-channel-agent"
        },
        "source": {},
        "target": {},
        "execution": {
          "kind": "job"
        },
        "capabilities": {
          "usedCapabilities": []
        },
        "resources": {
          "claims": []
        },
        "secrets": {
          "bindings": []
        },
        "desiredState": {
          "actionCount": 4
        },
        "actions": [
          {
            "kind": "create_agent",
            "name": "Create agent",
            "args": {
              "agentId": "{{agent_id}}",
              "modelProfileId": "{{model}}",
              "independent": "{{independent}}"
            }
          },
          {
            "kind": "setup_identity",
            "name": "Set agent identity",
            "args": {
              "agentId": "{{agent_id}}",
              "name": "{{name}}",
              "emoji": "{{emoji}}"
            }
          },
          {
            "kind": "bind_channel",
            "name": "Bind channel to agent",
            "args": {
              "channelType": "discord",
              "peerId": "{{channel_id}}",
              "agentId": "{{agent_id}}"
            }
          },
          {
            "kind": "config_patch",
            "name": "Set channel persona",
            "args": {
              "patch": {
                "channels": {
                  "discord": {
                    "guilds": {
                      "{{guild_id}}": {
                        "channels": {
                          "{{channel_id}}": {
                            "systemPrompt": "{{persona}}"
                          }
                        }
                      }
                    }
                  }
                }
              }
            }
          }
        ],
        "outputs": [{ "kind": "recipe-summary", "recipeId": "dedicated-channel-agent" }]
      },
      "steps": [
        { "action": "create_agent", "label": "Create agent", "args": { "agentId": "{{agent_id}}", "modelProfileId": "{{model}}", "independent": "{{independent}}" } },
        { "action": "setup_identity", "label": "Set agent identity", "args": { "agentId": "{{agent_id}}", "name": "{{name}}", "emoji": "{{emoji}}" } },
        { "action": "bind_channel", "label": "Bind channel to agent", "args": { "channelType": "discord", "peerId": "{{channel_id}}", "agentId": "{{agent_id}}" } },
        { "action": "config_patch", "label": "Set channel persona", "args": { "patchTemplate": "{\"channels\":{\"discord\":{\"guilds\":{\"{{guild_id}}\":{\"channels\":{\"{{channel_id}}\":{\"systemPrompt\":\"{{persona}}\"}}}}}}}" } }
      ]
    },
    {
      "id": "discord-channel-persona",
      "name": "Channel Persona",
      "description": "Set a custom persona for a Discord channel",
      "version": "1.0.0",
      "tags": ["discord", "persona", "beginner"],
      "difficulty": "easy",
      "params": [
        { "id": "guild_id", "label": "Guild", "type": "discord_guild", "required": true },
        { "id": "channel_id", "label": "Channel", "type": "discord_channel", "required": true },
        { "id": "persona", "label": "Persona", "type": "textarea", "required": true, "placeholder": "You are..." }
      ],
      "bundle": {
        "apiVersion": "strategy.platform/v1",
        "kind": "StrategyBundle",
        "metadata": {
          "name": "discord-channel-persona",
          "version": "1.0.0",
          "description": "Set a custom persona for a Discord channel"
        },
        "compatibility": {},
        "inputs": [],
        "capabilities": {
          "allowed": ["config.write"]
        },
        "resources": {
          "supportedKinds": ["file"]
        },
        "execution": {
          "supportedKinds": ["attachment"]
        },
        "runner": {},
        "outputs": [{ "kind": "recipe-summary", "recipeId": "discord-channel-persona" }]
      },
      "executionSpecTemplate": {
        "apiVersion": "strategy.platform/v1",
        "kind": "ExecutionSpec",
        "metadata": {
          "name": "discord-channel-persona"
        },
        "source": {},
        "target": {},
        "execution": {
          "kind": "attachment"
        },
        "capabilities": {
          "usedCapabilities": []
        },
        "resources": {
          "claims": []
        },
        "secrets": {
          "bindings": []
        },
        "desiredState": {
          "actionCount": 1
        },
        "actions": [
          {
            "kind": "config_patch",
            "name": "Set channel persona",
            "args": {
              "patch": {
                "channels": {
                  "discord": {
                    "guilds": {
                      "{{guild_id}}": {
                        "channels": {
                          "{{channel_id}}": {
                            "systemPrompt": "{{persona}}"
                          }
                        }
                      }
                    }
                  }
                }
              }
            }
          }
        ],
        "outputs": [{ "kind": "recipe-summary", "recipeId": "discord-channel-persona" }]
      },
      "steps": [
        { "action": "config_patch", "label": "Set channel persona", "args": { "patchTemplate": "{\"channels\":{\"discord\":{\"guilds\":{\"{{guild_id}}\":{\"channels\":{\"{{channel_id}}\":{\"systemPrompt\":\"{{persona}}\"}}}}}}}" } }
      ]
    }
  ]
}"#;

fn test_recipe(id: &str) -> Recipe {
    load_recipes_from_source_text(TEST_RECIPES_SOURCE)
        .expect("parse test recipe source")
        .into_iter()
        .find(|recipe| recipe.id == id)
        .expect("test recipe")
}

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
    let recipe = test_recipe("discord-channel-persona");

    let plan = build_recipe_plan(&recipe, &sample_inputs()).expect("build plan");

    assert!(!plan.used_capabilities.is_empty());
    assert!(!plan.concrete_claims.is_empty());
    assert!(!plan.execution_spec_digest.is_empty());
}

#[test]
fn plan_recipe_includes_execution_spec_for_executor_bridge() {
    let recipe = test_recipe("discord-channel-persona");

    let plan = build_recipe_plan(&recipe, &sample_inputs()).expect("build plan");

    assert_eq!(plan.execution_spec.kind, "ExecutionSpec");
    assert!(!plan.execution_spec.actions.is_empty());
}

#[test]
fn plan_recipe_does_not_emit_legacy_bridge_warning() {
    let recipe = test_recipe("discord-channel-persona");

    let plan = build_recipe_plan(&recipe, &sample_inputs()).expect("build plan");

    assert!(plan
        .warnings
        .iter()
        .all(|warning| !warning.to_ascii_lowercase().contains("legacy")));
}

#[test]
fn plan_recipe_skips_optional_steps_from_structured_template() {
    let recipe = test_recipe("dedicated-channel-agent");
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
    let recipe = test_recipe("discord-channel-persona");
    let source = export_recipe_source(&recipe).expect("export source");
    let recipes = load_recipes_from_source_text(&source).expect("parse source");

    let plan =
        build_recipe_plan_from_source_text("discord-channel-persona", &sample_inputs(), &source)
            .expect("build plan from source");

    assert_eq!(recipes.len(), 1);
    assert_eq!(plan.summary.recipe_id, "discord-channel-persona");
    assert_eq!(plan.execution_spec.kind, "ExecutionSpec");
}
