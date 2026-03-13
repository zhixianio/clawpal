use std::fs;
use std::path::PathBuf;

use uuid::Uuid;

use crate::recipe_workspace::{BundledSeedStatus, RecipeWorkspace};

const SAMPLE_SOURCE: &str = r#"{
  "id": "channel-persona",
  "name": "Channel Persona",
  "description": "Set a custom persona for a channel",
  "version": "1.0.0",
  "tags": ["discord", "persona"],
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
    "actions": [],
    "outputs": []
  }
}"#;

struct TempWorkspaceRoot(PathBuf);

impl TempWorkspaceRoot {
    fn path(&self) -> &PathBuf {
        &self.0
    }
}

impl Drop for TempWorkspaceRoot {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn temp_workspace_root() -> TempWorkspaceRoot {
    let root = std::env::temp_dir().join(format!("clawpal-recipe-workspace-{}", Uuid::new_v4()));
    fs::create_dir_all(&root).expect("create temp workspace root");
    TempWorkspaceRoot(root)
}

#[test]
fn workspace_recipe_save_writes_under_clawpal_recipe_workspace() {
    let root = temp_workspace_root();
    let store = RecipeWorkspace::new(root.path().clone());

    let result = store
        .save_recipe_source("channel-persona", SAMPLE_SOURCE)
        .expect("save recipe source");

    assert_eq!(result.slug, "channel-persona");
    assert_eq!(
        result.path,
        root.path()
            .join("channel-persona.recipe.json")
            .to_string_lossy()
    );
    assert!(root.path().join("channel-persona.recipe.json").exists());
}

#[test]
fn workspace_recipe_save_rejects_parent_traversal() {
    let root = temp_workspace_root();
    let store = RecipeWorkspace::new(root.path().clone());

    assert!(store
        .save_recipe_source("../escape", SAMPLE_SOURCE)
        .is_err());
}

#[test]
fn delete_workspace_recipe_removes_saved_file() {
    let root = temp_workspace_root();
    let store = RecipeWorkspace::new(root.path().clone());
    let saved = store
        .save_recipe_source("persona", SAMPLE_SOURCE)
        .expect("save recipe source");

    store
        .delete_recipe_source(saved.slug.as_str())
        .expect("delete recipe source");

    assert!(!root.path().join("persona.recipe.json").exists());
}

#[test]
fn list_workspace_entries_returns_saved_recipes() {
    let root = temp_workspace_root();
    let store = RecipeWorkspace::new(root.path().clone());
    store
        .save_recipe_source("zeta", SAMPLE_SOURCE)
        .expect("save zeta");
    store
        .save_recipe_source("alpha", SAMPLE_SOURCE)
        .expect("save alpha");

    let entries = store.list_entries().expect("list entries");

    assert_eq!(entries.len(), 2);
    assert_eq!(entries[0].slug, "alpha");
    assert_eq!(entries[1].slug, "zeta");
}

#[test]
fn bundled_seeded_recipe_is_tracked_until_user_saves_a_workspace_copy() {
    let root = temp_workspace_root();
    let store = RecipeWorkspace::new(root.path().clone());

    store
        .save_bundled_recipe_source("channel-persona", SAMPLE_SOURCE, "channel-persona")
        .expect("save bundled recipe");

    assert_eq!(
        store
            .bundled_seed_status("channel-persona")
            .expect("bundled seed status"),
        BundledSeedStatus::Unchanged
    );

    store
        .save_recipe_source(
            "channel-persona",
            SAMPLE_SOURCE.replace("easy", "normal").as_str(),
        )
        .expect("save user recipe");

    assert_eq!(
        store
            .bundled_seed_status("channel-persona")
            .expect("bundled seed status after manual save"),
        BundledSeedStatus::UntrackedExisting
    );
}
