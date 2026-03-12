use std::fs;
use std::path::{Path, PathBuf};

use serde_json::{Map, Value};
use uuid::Uuid;

use crate::recipe::load_recipes_from_source_text;
use crate::recipe_adapter::compile_recipe_to_spec;
use crate::recipe_library::import_recipe_library;
use crate::recipe_workspace::RecipeWorkspace;

struct TempDir(PathBuf);

impl TempDir {
    fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn temp_dir(prefix: &str) -> TempDir {
    let path = std::env::temp_dir().join(format!("clawpal-{}-{}", prefix, Uuid::new_v4()));
    fs::create_dir_all(&path).expect("create temp dir");
    TempDir(path)
}

fn write_recipe(dir: &Path, name: &str, source: &str) {
    let recipe_dir = dir.join(name);
    fs::create_dir_all(&recipe_dir).expect("create recipe dir");
    fs::write(recipe_dir.join("recipe.json"), source).expect("write recipe");
}

#[test]
fn import_recipe_library_compiles_preset_assets_into_workspace_recipe() {
    let library_root = temp_dir("recipe-library");
    let workspace_root = temp_dir("recipe-workspace");
    let workspace = RecipeWorkspace::new(workspace_root.path().to_path_buf());

    write_recipe(
        library_root.path(),
        "dedicated-channel-agent",
        r#"{
          "id": "dedicated-channel-agent",
          "name": "Dedicated Channel Agent",
          "description": "Create a dedicated agent and bind it to a channel",
          "version": "1.0.0",
          "tags": ["discord", "agent"],
          "difficulty": "easy",
          "params": [
            { "id": "agent_id", "label": "Agent ID", "type": "string", "required": true },
            { "id": "guild_id", "label": "Guild", "type": "discord_guild", "required": true },
            { "id": "channel_id", "label": "Channel", "type": "discord_channel", "required": true }
          ],
          "steps": [
            { "action": "create_agent", "label": "Create agent", "args": { "agentId": "{{agent_id}}", "independent": true } },
            { "action": "bind_channel", "label": "Bind channel", "args": { "channelType": "discord", "peerId": "{{channel_id}}", "agentId": "{{agent_id}}" } }
          ],
          "bundle": {
            "apiVersion": "strategy.platform/v1",
            "kind": "StrategyBundle",
            "metadata": {},
            "compatibility": {},
            "inputs": [],
            "capabilities": { "allowed": ["agent.manage", "binding.manage"] },
            "resources": { "supportedKinds": ["agent", "channel"] },
            "execution": { "supportedKinds": ["job"] },
            "runner": {},
            "outputs": []
          },
          "executionSpecTemplate": {
            "apiVersion": "strategy.platform/v1",
            "kind": "ExecutionSpec",
            "metadata": { "name": "dedicated-channel-agent" },
            "source": {},
            "target": {},
            "execution": { "kind": "job" },
            "capabilities": { "usedCapabilities": ["agent.manage", "binding.manage"] },
            "resources": { "claims": [] },
            "secrets": { "bindings": [] },
            "desiredState": {},
            "actions": [
              { "kind": "create_agent", "name": "Create agent", "args": { "agentId": "{{agent_id}}", "independent": true } },
              { "kind": "bind_channel", "name": "Bind channel", "args": { "channelType": "discord", "peerId": "{{channel_id}}", "agentId": "{{agent_id}}" } }
            ],
            "outputs": []
          }
        }"#,
    );

    let persona_dir = library_root
        .path()
        .join("agent-persona-pack")
        .join("assets")
        .join("personas");
    fs::create_dir_all(&persona_dir).expect("create persona asset dir");
    fs::write(
        persona_dir.join("friendly.md"),
        "You are warm, concise, and practical.\n",
    )
    .expect("write asset");

    write_recipe(
        library_root.path(),
        "agent-persona-pack",
        r#"{
          "id": "agent-persona-pack",
          "name": "Agent Persona Pack",
          "description": "Import persona presets into an existing agent",
          "version": "1.0.0",
          "tags": ["agent", "persona"],
          "difficulty": "easy",
          "params": [
            { "id": "agent_id", "label": "Agent", "type": "agent", "required": true },
            { "id": "persona_preset", "label": "Persona preset", "type": "string", "required": true }
          ],
          "steps": [
            {
              "action": "setup_identity",
              "label": "Apply persona preset",
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
                "name": "Apply persona preset",
                "args": {
                  "agentId": "{{agent_id}}",
                  "persona": "{{presetMap:persona_preset}}"
                }
              }
            ],
            "outputs": []
          },
          "clawpalImport": {
            "presetParams": {
              "persona_preset": [
                { "value": "friendly", "label": "Friendly", "asset": "assets/personas/friendly.md" }
              ]
            }
          }
        }"#,
    );

    let result =
        import_recipe_library(library_root.path(), &workspace).expect("import recipe library");

    assert_eq!(result.imported.len(), 2);
    assert!(result.skipped.is_empty());

    let imported = workspace
        .read_recipe_source("agent-persona-pack")
        .expect("read imported recipe");
    let imported_json: Value = serde_json::from_str(&imported).expect("parse imported recipe");

    let params = imported_json
        .get("params")
        .and_then(Value::as_array)
        .expect("params");
    let persona_param = params
        .iter()
        .find(|param| param.get("id").and_then(Value::as_str) == Some("persona_preset"))
        .expect("persona_preset param");
    let options = persona_param
        .get("options")
        .and_then(Value::as_array)
        .expect("persona options");
    assert_eq!(options.len(), 1);
    assert_eq!(
        options[0].get("value").and_then(Value::as_str),
        Some("friendly")
    );
    assert_eq!(
        options[0].get("label").and_then(Value::as_str),
        Some("Friendly")
    );

    let persona_map = imported_json
        .pointer("/clawpalPresetMaps/persona_preset")
        .and_then(Value::as_object)
        .expect("persona preset map");
    assert_eq!(
        persona_map.get("friendly").and_then(Value::as_str),
        Some("You are warm, concise, and practical.\n")
    );
    assert!(imported_json.get("clawpalImport").is_none());

    let imported_recipe = load_recipes_from_source_text(&imported)
        .expect("load imported recipe")
        .into_iter()
        .next()
        .expect("first recipe");
    let mut params = Map::new();
    params.insert("agent_id".into(), Value::String("lobster".into()));
    params.insert("persona_preset".into(), Value::String("friendly".into()));
    let spec = compile_recipe_to_spec(&imported_recipe, &params).expect("compile imported recipe");

    assert_eq!(
        spec.actions[0].args.get("persona").and_then(Value::as_str),
        Some("You are warm, concise, and practical.\n")
    );
}

#[test]
fn import_recipe_library_skips_recipe_when_asset_is_missing() {
    let library_root = temp_dir("recipe-library-missing-asset");
    let workspace_root = temp_dir("recipe-workspace-missing-asset");
    let workspace = RecipeWorkspace::new(workspace_root.path().to_path_buf());

    write_recipe(
        library_root.path(),
        "channel-persona-pack",
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
            { "id": "persona_preset", "label": "Persona preset", "type": "string", "required": true }
          ],
          "steps": [
            {
              "action": "config_patch",
              "label": "Apply persona preset",
              "args": {
                "patchTemplate": "{\"channels\":{\"discord\":{\"guilds\":{\"{{guild_id}}\":{\"channels\":{\"{{channel_id}}\":{\"systemPrompt\":\"{{persona}}\"}}}}}}"
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
          "clawpalImport": {
            "presetParams": {
              "persona_preset": [
                { "value": "ops", "label": "Ops", "asset": "assets/personas/ops.md" }
              ]
            }
          }
        }"#,
    );

    let result =
        import_recipe_library(library_root.path(), &workspace).expect("import recipe library");

    assert!(result.imported.is_empty());
    assert_eq!(result.skipped.len(), 1);
    assert!(result.skipped[0].reason.contains("assets/personas/ops.md"));
    assert!(workspace
        .list_entries()
        .expect("workspace entries")
        .is_empty());
}

#[test]
fn import_recipe_library_accepts_repo_example_library() {
    let workspace_root = temp_dir("recipe-workspace-examples");
    let workspace = RecipeWorkspace::new(workspace_root.path().to_path_buf());
    let example_root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("examples")
        .join("recipe-library");

    let result = import_recipe_library(&example_root, &workspace).expect("import recipe library");

    assert_eq!(result.imported.len(), 3);
    assert!(result.skipped.is_empty());
    let imported_ids = result
        .imported
        .iter()
        .map(|recipe| recipe.recipe_id.as_str())
        .collect::<std::collections::BTreeSet<_>>();
    assert_eq!(
        imported_ids,
        std::collections::BTreeSet::from([
            "agent-persona-pack",
            "channel-persona-pack",
            "dedicated-agent",
        ])
    );
    let entries = workspace.list_entries().expect("workspace entries");
    assert_eq!(entries.len(), 3);

    let dedicated_source = workspace
        .read_recipe_source("dedicated-agent")
        .expect("read dedicated agent recipe");
    let dedicated_json: Value =
        serde_json::from_str(&dedicated_source).expect("parse dedicated agent recipe");
    let params = dedicated_json
        .get("params")
        .and_then(Value::as_array)
        .expect("dedicated params");
    assert!(params
        .iter()
        .all(|param| param.get("id").and_then(Value::as_str) != Some("guild_id")));
    assert!(params
        .iter()
        .all(|param| param.get("id").and_then(Value::as_str) != Some("channel_id")));
    let actions = dedicated_json
        .pointer("/executionSpecTemplate/actions")
        .and_then(Value::as_array)
        .expect("dedicated actions");
    assert!(actions
        .iter()
        .all(|action| action.get("kind").and_then(Value::as_str) != Some("bind_channel")));
}

#[test]
fn import_recipe_library_skips_duplicate_slug_against_existing_workspace_recipe() {
    let library_root = temp_dir("recipe-library-duplicate-slug");
    let workspace_root = temp_dir("recipe-workspace-duplicate-slug");
    let workspace = RecipeWorkspace::new(workspace_root.path().to_path_buf());

    workspace
        .save_recipe_source(
            "agent-persona-pack",
            r#"{
              "id": "agent-persona-pack",
              "name": "Existing Agent Persona Pack",
              "description": "Existing workspace recipe",
              "version": "1.0.0",
              "tags": ["agent"],
              "difficulty": "easy",
              "params": [],
              "steps": []
            }"#,
        )
        .expect("seed workspace recipe");

    let persona_dir = library_root
        .path()
        .join("agent-persona-pack")
        .join("assets")
        .join("personas");
    fs::create_dir_all(&persona_dir).expect("create persona dir");
    fs::write(
        persona_dir.join("coach.md"),
        "You coach incidents calmly.\n",
    )
    .expect("write asset");

    write_recipe(
        library_root.path(),
        "agent-persona-pack",
        r#"{
          "id": "agent-persona-pack",
          "name": "Agent Persona Pack",
          "description": "Import persona presets into an existing agent",
          "version": "1.0.0",
          "tags": ["agent", "persona"],
          "difficulty": "easy",
          "params": [
            { "id": "agent_id", "label": "Agent", "type": "agent", "required": true },
            { "id": "persona_preset", "label": "Persona preset", "type": "string", "required": true }
          ],
          "steps": [
            {
              "action": "setup_identity",
              "label": "Apply persona preset",
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
                "name": "Apply persona preset",
                "args": {
                  "agentId": "{{agent_id}}",
                  "persona": "{{presetMap:persona_preset}}"
                }
              }
            ],
            "outputs": []
          },
          "clawpalImport": {
            "presetParams": {
              "persona_preset": [
                { "value": "coach", "label": "Coach", "asset": "assets/personas/coach.md" }
              ]
            }
          }
        }"#,
    );

    let result =
        import_recipe_library(library_root.path(), &workspace).expect("import recipe library");

    assert!(result.imported.is_empty());
    assert_eq!(result.skipped.len(), 1);
    assert!(result.skipped[0]
        .reason
        .contains("duplicate recipe slug 'agent-persona-pack'"));
}
