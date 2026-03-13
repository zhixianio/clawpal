use std::fs;
use std::path::{Path, PathBuf};

use uuid::Uuid;

use crate::recipe::{find_recipe_with_source, load_recipes_from_source};

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

fn write_recipe_dir(path: &Path, source: &str) {
    fs::create_dir_all(path).expect("create recipe dir");
    fs::write(path.join("recipe.json"), source).expect("write recipe");
}

#[test]
fn load_recipes_from_source_supports_single_recipe_directory() {
    let recipe_dir = temp_dir("recipe-source-directory");
    let asset_dir = recipe_dir.path().join("assets").join("personas");
    fs::create_dir_all(&asset_dir).expect("create asset dir");
    fs::write(
        asset_dir.join("friendly.md"),
        "You are warm, concise, and practical.\n",
    )
    .expect("write asset");

    write_recipe_dir(
        recipe_dir.path(),
        r#"{
          "id": "agent-persona-pack",
          "name": "Agent Persona Pack",
          "description": "Apply a persona preset",
          "version": "1.0.0",
          "tags": ["agent", "persona"],
          "difficulty": "easy",
          "params": [
            { "id": "persona_preset", "label": "Persona", "type": "string", "required": true }
          ],
          "steps": [],
          "clawpalImport": {
            "presetParams": {
              "persona_preset": [
                { "value": "friendly", "label": "Friendly", "asset": "assets/personas/friendly.md" }
              ]
            }
          }
        }"#,
    );

    let recipes = load_recipes_from_source(recipe_dir.path().to_string_lossy().as_ref())
        .expect("load recipe directory");

    assert_eq!(recipes.len(), 1);
    assert_eq!(recipes[0].id, "agent-persona-pack");
    assert_eq!(
        recipes[0]
            .params
            .first()
            .and_then(|param| param.options.as_ref())
            .and_then(|options| options.first())
            .map(|option| option.value.as_str()),
        Some("friendly")
    );
    assert_eq!(
        recipes[0]
            .clawpal_preset_maps
            .as_ref()
            .and_then(|maps| maps.get("persona_preset"))
            .and_then(|value| value.get("friendly"))
            .and_then(|value| value.as_str()),
        Some("You are warm, concise, and practical.\n")
    );
}

#[test]
fn find_recipe_with_source_supports_single_recipe_directory() {
    let recipe_dir = temp_dir("recipe-find-directory");
    write_recipe_dir(
        recipe_dir.path(),
        r#"{
          "id": "directory-only-recipe",
          "name": "Directory Only Recipe",
          "description": "Loaded from a recipe directory",
          "version": "1.0.0",
          "tags": ["directory"],
          "difficulty": "easy",
          "params": [],
          "steps": []
        }"#,
    );

    let recipe = find_recipe_with_source(
        "directory-only-recipe",
        Some(recipe_dir.path().to_string_lossy().to_string()),
    )
    .expect("find recipe from directory source");

    assert_eq!(recipe.name, "Directory Only Recipe");
}

#[test]
fn load_recipes_from_source_rejects_recipe_directory_without_recipe_json() {
    let recipe_dir = temp_dir("recipe-source-missing-json");

    let error = load_recipes_from_source(recipe_dir.path().to_string_lossy().as_ref())
        .expect_err("directory without recipe.json should fail");

    assert!(
        error.contains("recipe.json not found"),
        "unexpected error: {error}"
    );
}
