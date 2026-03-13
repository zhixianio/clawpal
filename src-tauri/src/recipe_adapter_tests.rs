use serde_json::{Map, Value};

use crate::recipe::{
    load_recipes_from_source_text, validate_recipe_source, Recipe, RecipeParam, RecipePresentation,
    RecipeStep,
};
use crate::recipe_adapter::{compile_recipe_to_spec, export_recipe_source};

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
    let recipe = test_recipe("dedicated-channel-agent");

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
    let recipe = test_recipe("discord-channel-persona");

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
    let recipe = test_recipe("dedicated-channel-agent");
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
        presentation: Some(RecipePresentation {
            result_summary: Some("Updated persona for {{channel_id}}".into()),
        }),
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
                options: None,
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
                options: None,
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
        clawpal_preset_maps: None,
        bundle: None,
        execution_spec_template: None,
    };

    let exported = export_recipe_source(&recipe).expect("export source");

    assert!(exported.contains("\"bundle\""));
    assert!(exported.contains("\"executionSpecTemplate\""));
    assert!(exported.contains("\"presentation\""));
    assert!(exported.contains("Updated persona for {{channel_id}}"));
    assert!(exported.contains("\"supportedKinds\": [\n        \"attachment\""));
    assert!(exported.contains("\"{{guild_id}}\""));
}

#[test]
fn structured_recipe_compilation_renders_result_summary_into_spec_source() {
    let recipe = Recipe {
        id: "persona-pack".into(),
        name: "Persona Pack".into(),
        description: "Apply a persona pack".into(),
        version: "1.0.0".into(),
        tags: vec!["agent".into(), "persona".into()],
        difficulty: "easy".into(),
        presentation: Some(RecipePresentation {
            result_summary: Some("Updated persona for {{agent_id}}".into()),
        }),
        params: vec![RecipeParam {
            id: "agent_id".into(),
            label: "Agent".into(),
            kind: "agent".into(),
            required: true,
            pattern: None,
            min_length: None,
            max_length: None,
            placeholder: None,
            depends_on: None,
            default_value: None,
            options: None,
        }],
        steps: vec![RecipeStep {
            action: "setup_identity".into(),
            label: "Apply persona".into(),
            args: serde_json::from_value(serde_json::json!({
                "agentId": "{{agent_id}}",
                "persona": "You are calm and direct."
            }))
            .expect("step args"),
        }],
        clawpal_preset_maps: None,
        bundle: None,
        execution_spec_template: Some(
            serde_json::from_value(serde_json::json!({
                "apiVersion": "strategy.platform/v1",
                "kind": "ExecutionSpec",
                "metadata": { "name": "persona-pack" },
                "source": {},
                "target": {},
                "execution": { "kind": "job" },
                "capabilities": { "usedCapabilities": ["agent.identity.write"] },
                "resources": { "claims": [{ "kind": "agent", "id": "{{agent_id}}" }] },
                "secrets": { "bindings": [] },
                "desiredState": { "actionCount": 1 },
                "actions": [
                  {
                    "kind": "setup_identity",
                    "name": "Apply persona",
                    "args": {
                      "agentId": "{{agent_id}}",
                      "persona": "You are calm and direct."
                    }
                  }
                ],
                "outputs": []
            }))
            .expect("template"),
        ),
    };
    let mut params = Map::new();
    params.insert("agent_id".into(), Value::String("main".into()));

    let spec = compile_recipe_to_spec(&recipe, &params).expect("compile spec");

    assert_eq!(
        spec.source
            .get("recipePresentation")
            .and_then(|value| value.get("resultSummary"))
            .and_then(Value::as_str),
        Some("Updated persona for main")
    );
}

#[test]
fn exported_recipe_source_validates_as_structured_document() {
    let recipe = test_recipe("discord-channel-persona");
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
fn structured_recipe_template_resolves_preset_map_placeholders_from_compiled_source() {
    let recipe = crate::recipe::load_recipes_from_source_text(
        r#"{
          "id": "channel-persona-pack",
          "name": "Channel Persona Pack",
          "description": "Apply a preset persona to a Discord channel",
          "version": "1.0.0",
          "tags": ["discord", "persona"],
          "difficulty": "easy",
          "params": [
            { "id": "guild_id", "label": "Guild", "type": "discord_guild", "required": true },
            { "id": "channel_id", "label": "Channel", "type": "discord_channel", "required": true },
            {
              "id": "persona_preset",
              "label": "Persona preset",
              "type": "string",
              "required": true,
              "options": [
                { "value": "ops", "label": "Ops" }
              ]
            }
          ],
          "steps": [
            {
              "action": "config_patch",
              "label": "Apply persona preset",
              "args": {
                "patchTemplate": "{\"channels\":{\"discord\":{\"guilds\":{\"{{guild_id}}\":{\"channels\":{\"{{channel_id}}\":{\"systemPrompt\":\"{{presetMap:persona_preset}}\"}}}}}}"
              }
            }
          ],
          "bundle": {
            "apiVersion": "strategy.platform/v1",
            "kind": "StrategyBundle",
            "metadata": {},
            "compatibility": {},
            "inputs": [],
            "capabilities": { "allowed": ["config.write"] },
            "resources": { "supportedKinds": ["file"] },
            "execution": { "supportedKinds": ["attachment"] },
            "runner": {},
            "outputs": []
          },
          "executionSpecTemplate": {
            "apiVersion": "strategy.platform/v1",
            "kind": "ExecutionSpec",
            "metadata": { "name": "channel-persona-pack" },
            "source": {},
            "target": {},
            "execution": { "kind": "attachment" },
            "capabilities": { "usedCapabilities": ["config.write"] },
            "resources": { "claims": [] },
            "secrets": { "bindings": [] },
            "desiredState": {},
            "actions": [
              {
                "kind": "config_patch",
                "name": "Apply persona preset",
                "args": {
                  "patch": {
                    "channels": {
                      "discord": {
                        "guilds": {
                          "{{guild_id}}": {
                            "channels": {
                              "{{channel_id}}": {
                                "systemPrompt": "{{presetMap:persona_preset}}"
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
            "outputs": []
          },
          "clawpalPresetMaps": {
            "persona_preset": {
              "ops": "You are an on-call operations coordinator."
            }
          }
        }"#,
    )
    .expect("load source")
    .into_iter()
    .next()
    .expect("recipe");

    let mut params = Map::new();
    params.insert("guild_id".into(), Value::String("guild-1".into()));
    params.insert("channel_id".into(), Value::String("channel-2".into()));
    params.insert("persona_preset".into(), Value::String("ops".into()));

    let spec = compile_recipe_to_spec(&recipe, &params).expect("compile spec");

    assert_eq!(
        spec.actions[0]
            .args
            .pointer("/patch/channels/discord/guilds/guild-1/channels/channel-2/systemPrompt")
            .and_then(Value::as_str),
        Some("You are an on-call operations coordinator.")
    );
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

#[test]
fn structured_recipe_template_resolves_agent_persona_preset_text() {
    let recipe = load_recipes_from_source_text(
        r#"{
          "id": "agent-persona-pack",
          "name": "Agent Persona Pack",
          "description": "Import persona presets into an existing agent",
          "version": "1.0.0",
          "tags": ["agent", "persona"],
          "difficulty": "easy",
          "params": [
            { "id": "agent_id", "label": "Agent", "type": "agent", "required": true },
            {
              "id": "persona_preset",
              "label": "Persona preset",
              "type": "string",
              "required": true,
              "options": [{ "value": "friendly", "label": "Friendly" }]
            }
          ],
          "steps": [
            {
              "action": "setup_identity",
              "label": "Apply preset",
              "args": {
                "agentId": "{{agent_id}}",
                "persona": "{{presetMap:persona_preset}}"
              }
            }
          ],
          "bundle": {
            "apiVersion": "strategy.platform/v1",
            "kind": "StrategyBundle",
            "metadata": {},
            "compatibility": {},
            "inputs": [],
            "capabilities": { "allowed": ["agent.identity.write"] },
            "resources": { "supportedKinds": ["agent"] },
            "execution": { "supportedKinds": ["job"] },
            "runner": {},
            "outputs": []
          },
          "executionSpecTemplate": {
            "apiVersion": "strategy.platform/v1",
            "kind": "ExecutionSpec",
            "metadata": { "name": "agent-persona-pack" },
            "source": {},
            "target": {},
            "execution": { "kind": "job" },
            "capabilities": { "usedCapabilities": ["agent.identity.write"] },
            "resources": { "claims": [] },
            "secrets": { "bindings": [] },
            "desiredState": {},
            "actions": [
              {
                "kind": "setup_identity",
                "name": "Apply preset",
                "args": {
                  "agentId": "{{agent_id}}",
                  "persona": "{{presetMap:persona_preset}}"
                }
              }
            ],
            "outputs": []
          },
          "clawpalPresetMaps": {
            "persona_preset": {
              "friendly": "You are warm, concise, and practical."
            }
          }
        }"#,
    )
    .expect("load recipe")
    .into_iter()
    .next()
    .expect("recipe");

    let mut params = Map::new();
    params.insert("agent_id".into(), Value::String("lobster".into()));
    params.insert("persona_preset".into(), Value::String("friendly".into()));

    let spec = compile_recipe_to_spec(&recipe, &params).expect("compile spec");

    assert_eq!(
        spec.actions[0].args.get("persona").and_then(Value::as_str),
        Some("You are warm, concise, and practical.")
    );
}

#[test]
fn structured_recipe_template_resolves_channel_persona_preset_into_patch() {
    let recipe = load_recipes_from_source_text(
        r#"{
          "id": "channel-persona-pack",
          "name": "Channel Persona Pack",
          "description": "Import persona presets into a Discord channel",
          "version": "1.0.0",
          "tags": ["discord", "persona"],
          "difficulty": "easy",
          "params": [
            { "id": "guild_id", "label": "Guild", "type": "discord_guild", "required": true },
            { "id": "channel_id", "label": "Channel", "type": "discord_channel", "required": true },
            {
              "id": "persona_preset",
              "label": "Persona preset",
              "type": "string",
              "required": true,
              "options": [{ "value": "ops", "label": "Ops" }]
            }
          ],
          "steps": [
            {
              "action": "config_patch",
              "label": "Apply preset",
              "args": {}
            }
          ],
          "bundle": {
            "apiVersion": "strategy.platform/v1",
            "kind": "StrategyBundle",
            "metadata": {},
            "compatibility": {},
            "inputs": [],
            "capabilities": { "allowed": ["config.write"] },
            "resources": { "supportedKinds": ["file"] },
            "execution": { "supportedKinds": ["attachment"] },
            "runner": {},
            "outputs": []
          },
          "executionSpecTemplate": {
            "apiVersion": "strategy.platform/v1",
            "kind": "ExecutionSpec",
            "metadata": { "name": "channel-persona-pack" },
            "source": {},
            "target": {},
            "execution": { "kind": "attachment" },
            "capabilities": { "usedCapabilities": ["config.write"] },
            "resources": { "claims": [] },
            "secrets": { "bindings": [] },
            "desiredState": {},
            "actions": [
              {
                "kind": "config_patch",
                "name": "Apply preset",
                "args": {
                  "patch": {
                    "channels": {
                      "discord": {
                        "guilds": {
                          "{{guild_id}}": {
                            "channels": {
                              "{{channel_id}}": {
                                "systemPrompt": "{{presetMap:persona_preset}}"
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
            "outputs": []
          },
          "clawpalPresetMaps": {
            "persona_preset": {
              "ops": "You are a crisp channel ops assistant."
            }
          }
        }"#,
    )
    .expect("load recipe")
    .into_iter()
    .next()
    .expect("recipe");

    let mut params = Map::new();
    params.insert("guild_id".into(), Value::String("guild-1".into()));
    params.insert("channel_id".into(), Value::String("channel-1".into()));
    params.insert("persona_preset".into(), Value::String("ops".into()));

    let spec = compile_recipe_to_spec(&recipe, &params).expect("compile spec");

    assert_eq!(
        spec.actions[0]
            .args
            .pointer("/patch/channels/discord/guilds/guild-1/channels/channel-1/systemPrompt")
            .and_then(Value::as_str),
        Some("You are a crisp channel ops assistant.")
    );
}

#[test]
fn structured_recipe_compilation_infers_capabilities_and_claims_for_new_actions() {
    let recipe = load_recipes_from_source_text(
        r##"{
          "id": "runner-action-suite",
          "name": "Runner Action Suite",
          "description": "Exercise the extended action surface",
          "version": "1.0.0",
          "tags": ["runner"],
          "difficulty": "easy",
          "params": [
            { "id": "agent_id", "label": "Agent", "type": "agent", "required": true },
            { "id": "channel_id", "label": "Channel", "type": "discord_channel", "required": true },
            { "id": "profile_id", "label": "Model profile", "type": "model_profile", "required": true }
          ],
          "steps": [
            {
              "action": "ensure_model_profile",
              "label": "Prepare model access",
              "args": { "profileId": "{{profile_id}}" }
            },
            {
              "action": "set_agent_persona",
              "label": "Set agent persona",
              "args": { "agentId": "{{agent_id}}", "persona": "You are direct." }
            },
            {
              "action": "set_channel_persona",
              "label": "Set channel persona",
              "args": { "channelType": "discord", "peerId": "{{channel_id}}", "persona": "Stay crisp." }
            },
            {
              "action": "upsert_markdown_document",
              "label": "Write agent notes",
              "args": {
                "target": { "scope": "agent", "agentId": "{{agent_id}}", "path": "PLAYBOOK.md" },
                "mode": "replace",
                "content": "# Playbook\n"
              }
            },
            {
              "action": "ensure_provider_auth",
              "label": "Ensure provider auth",
              "args": { "provider": "openai", "authRef": "openai:default" }
            }
          ],
          "bundle": {
            "apiVersion": "strategy.platform/v1",
            "kind": "StrategyBundle",
            "metadata": {},
            "compatibility": {},
            "inputs": [],
            "capabilities": {
              "allowed": [
                "model.manage",
                "agent.identity.write",
                "config.write",
                "document.write",
                "auth.manage",
                "secret.sync"
              ]
            },
            "resources": {
              "supportedKinds": ["agent", "channel", "document", "modelProfile", "authProfile"]
            },
            "execution": { "supportedKinds": ["job"] },
            "runner": {},
            "outputs": []
          },
          "executionSpecTemplate": {
            "apiVersion": "strategy.platform/v1",
            "kind": "ExecutionSpec",
            "metadata": { "name": "runner-action-suite" },
            "source": {},
            "target": {},
            "execution": { "kind": "job" },
            "capabilities": { "usedCapabilities": [] },
            "resources": { "claims": [] },
            "secrets": { "bindings": [] },
            "desiredState": {},
            "actions": [
              { "kind": "ensure_model_profile", "name": "Prepare model access", "args": { "profileId": "{{profile_id}}" } },
              { "kind": "set_agent_persona", "name": "Set agent persona", "args": { "agentId": "{{agent_id}}", "persona": "You are direct." } },
              { "kind": "set_channel_persona", "name": "Set channel persona", "args": { "channelType": "discord", "peerId": "{{channel_id}}", "persona": "Stay crisp." } },
              {
                "kind": "upsert_markdown_document",
                "name": "Write agent notes",
                "args": {
                  "target": { "scope": "agent", "agentId": "{{agent_id}}", "path": "PLAYBOOK.md" },
                  "mode": "replace",
                  "content": "# Playbook\n"
                }
              },
              { "kind": "ensure_provider_auth", "name": "Ensure provider auth", "args": { "provider": "openai", "authRef": "openai:default" } }
            ],
            "outputs": []
          }
        }"##,
    )
    .expect("load recipe")
    .into_iter()
    .next()
    .expect("recipe");

    let mut params = Map::new();
    params.insert("agent_id".into(), Value::String("main".into()));
    params.insert("channel_id".into(), Value::String("channel-1".into()));
    params.insert("profile_id".into(), Value::String("remote-openai".into()));

    let spec = compile_recipe_to_spec(&recipe, &params).expect("compile spec");

    assert!(spec
        .capabilities
        .used_capabilities
        .iter()
        .any(|value| value == "model.manage"));
    assert!(spec
        .capabilities
        .used_capabilities
        .iter()
        .any(|value| value == "agent.identity.write"));
    assert!(spec
        .capabilities
        .used_capabilities
        .iter()
        .any(|value| value == "config.write"));
    assert!(spec
        .capabilities
        .used_capabilities
        .iter()
        .any(|value| value == "document.write"));
    assert!(spec
        .capabilities
        .used_capabilities
        .iter()
        .any(|value| value == "auth.manage"));
    assert!(spec
        .capabilities
        .used_capabilities
        .iter()
        .any(|value| value == "secret.sync"));

    assert!(spec
        .resources
        .claims
        .iter()
        .any(|claim| { claim.kind == "agent" && claim.id.as_deref() == Some("main") }));
    assert!(spec
        .resources
        .claims
        .iter()
        .any(|claim| { claim.kind == "channel" && claim.id.as_deref() == Some("channel-1") }));
    assert!(spec.resources.claims.iter().any(|claim| {
        claim.kind == "document" && claim.path.as_deref() == Some("agent:main/PLAYBOOK.md")
    }));
    assert!(spec.resources.claims.iter().any(|claim| {
        claim.kind == "modelProfile" && claim.id.as_deref() == Some("remote-openai")
    }));
    assert!(spec.resources.claims.iter().any(|claim| {
        claim.kind == "authProfile" && claim.id.as_deref() == Some("openai:default")
    }));
}

#[test]
fn compile_recipe_rejects_documented_but_unsupported_actions() {
    let recipe = load_recipes_from_source_text(
        r##"{
          "id": "interactive-auth",
          "name": "Interactive auth",
          "description": "Should fail in compile",
          "version": "1.0.0",
          "tags": ["models"],
          "difficulty": "advanced",
          "params": [],
          "steps": [
            { "action": "login_model_auth", "label": "Login", "args": { "provider": "openai" } }
          ]
        }"##,
    )
    .expect("load recipe")
    .into_iter()
    .next()
    .expect("recipe");

    let error = compile_recipe_to_spec(&recipe, &Map::new()).expect_err("compile should fail");

    assert!(error.contains("not supported by the Recipe runner"));
}
