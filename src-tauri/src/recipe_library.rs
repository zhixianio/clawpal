use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use tauri::Manager;

use crate::recipe::validate_recipe_source;
use crate::recipe_workspace::RecipeWorkspace;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ImportedRecipe {
    pub slug: String,
    pub recipe_id: String,
    pub path: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct SkippedRecipeImport {
    pub recipe_dir: String,
    pub reason: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct RecipeLibraryImportResult {
    #[serde(default)]
    pub imported: Vec<ImportedRecipe>,
    #[serde(default)]
    pub skipped: Vec<SkippedRecipeImport>,
    #[serde(default)]
    pub warnings: Vec<String>,
}

pub fn import_recipe_library(
    root: &Path,
    workspace: &RecipeWorkspace,
) -> Result<RecipeLibraryImportResult, String> {
    let recipe_dirs = collect_recipe_dirs(root)?;
    let mut result = RecipeLibraryImportResult::default();
    let mut seen_recipe_ids = std::collections::BTreeSet::new();
    let mut seen_slugs = workspace
        .list_entries()?
        .into_iter()
        .map(|entry| entry.slug)
        .collect::<std::collections::BTreeSet<_>>();
    for recipe_dir in recipe_dirs {
        match import_recipe_dir(
            &recipe_dir,
            workspace,
            &mut seen_recipe_ids,
            &mut seen_slugs,
        ) {
            Ok(imported) => result.imported.push(imported),
            Err(error) => result.skipped.push(SkippedRecipeImport {
                recipe_dir: recipe_dir.to_string_lossy().to_string(),
                reason: error,
            }),
        }
    }

    Ok(result)
}

pub fn seed_recipe_library(
    root: &Path,
    workspace: &RecipeWorkspace,
) -> Result<RecipeLibraryImportResult, String> {
    let recipe_dirs = collect_recipe_dirs(root)?;
    let mut seen_slugs = workspace
        .list_entries()?
        .into_iter()
        .map(|entry| entry.slug)
        .collect::<std::collections::BTreeSet<_>>();
    let mut seen_recipe_ids = std::collections::BTreeSet::new();
    let mut result = RecipeLibraryImportResult::default();

    for recipe_dir in recipe_dirs {
        let recipe_path = recipe_dir.join("recipe.json");
        if !recipe_path.exists() {
            result.skipped.push(SkippedRecipeImport {
                recipe_dir: recipe_dir.to_string_lossy().to_string(),
                reason: "recipe.json not found".into(),
            });
            continue;
        }

        let source = match fs::read_to_string(&recipe_path) {
            Ok(source) => source,
            Err(error) => {
                result.skipped.push(SkippedRecipeImport {
                    recipe_dir: recipe_dir.to_string_lossy().to_string(),
                    reason: format!(
                        "failed to read recipe source '{}': {}",
                        recipe_path.to_string_lossy(),
                        error
                    ),
                });
                continue;
            }
        };
        let (recipe_id, compiled_source) = match compile_recipe_source(&recipe_dir, &source) {
            Ok(compiled) => compiled,
            Err(error) => {
                result.skipped.push(SkippedRecipeImport {
                    recipe_dir: recipe_dir.to_string_lossy().to_string(),
                    reason: error,
                });
                continue;
            }
        };
        let slug = match crate::recipe_workspace::normalize_recipe_slug(&recipe_id) {
            Ok(slug) => slug,
            Err(error) => {
                result.skipped.push(SkippedRecipeImport {
                    recipe_dir: recipe_dir.to_string_lossy().to_string(),
                    reason: error,
                });
                continue;
            }
        };

        if !seen_recipe_ids.insert(recipe_id.clone()) {
            result.skipped.push(SkippedRecipeImport {
                recipe_dir: recipe_dir.to_string_lossy().to_string(),
                reason: format!("duplicate recipe id '{}'", recipe_id),
            });
            continue;
        }

        if !seen_slugs.insert(slug.clone()) {
            result.warnings.push(format!(
                "Skipped bundled recipe '{}' because workspace recipe '{}' already exists.",
                recipe_id, slug
            ));
            continue;
        }

        let diagnostics = validate_recipe_source(&compiled_source)?;
        if !diagnostics.errors.is_empty() {
            result.skipped.push(SkippedRecipeImport {
                recipe_dir: recipe_dir.to_string_lossy().to_string(),
                reason: diagnostics
                    .errors
                    .iter()
                    .map(|diagnostic| diagnostic.message.clone())
                    .collect::<Vec<_>>()
                    .join("; "),
            });
            continue;
        }

        let saved = workspace.save_recipe_source(&slug, &compiled_source)?;
        result.imported.push(ImportedRecipe {
            slug: saved.slug,
            recipe_id,
            path: saved.path,
        });
    }

    Ok(result)
}

pub fn seed_bundled_recipe_library(
    app_handle: &tauri::AppHandle,
) -> Result<RecipeLibraryImportResult, String> {
    let root = resolve_bundled_recipe_library_root(app_handle)?;
    let workspace = RecipeWorkspace::from_resolved_paths();
    seed_recipe_library(&root, &workspace)
}

fn resolve_bundled_recipe_library_root(app_handle: &tauri::AppHandle) -> Result<PathBuf, String> {
    let candidates = bundled_recipe_library_candidates(app_handle);
    select_recipe_library_root(candidates)
}

pub(crate) fn bundled_recipe_library_candidates(app_handle: &tauri::AppHandle) -> Vec<PathBuf> {
    let mut candidates = Vec::new();

    if let Ok(resource_root) = app_handle
        .path()
        .resolve("recipe-library", tauri::path::BaseDirectory::Resource)
    {
        candidates.push(resource_root);
    }

    if let Ok(resource_root) = app_handle.path().resolve(
        "examples/recipe-library",
        tauri::path::BaseDirectory::Resource,
    ) {
        candidates.push(resource_root);
    }

    candidates.push(dev_recipe_library_root());
    dedupe_paths(candidates)
}

pub(crate) fn dev_recipe_library_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("examples")
        .join("recipe-library")
}

pub(crate) fn select_recipe_library_root(candidates: Vec<PathBuf>) -> Result<PathBuf, String> {
    candidates
        .iter()
        .find(|path| looks_like_recipe_library_root(path))
        .cloned()
        .ok_or_else(|| {
            let joined = candidates
                .iter()
                .map(|path| path.to_string_lossy().to_string())
                .collect::<Vec<_>>()
                .join(", ");
            format!(
                "bundled recipe library resource not found; checked: {}",
                joined
            )
        })
}

fn dedupe_paths(paths: Vec<PathBuf>) -> Vec<PathBuf> {
    let mut seen = std::collections::BTreeSet::new();
    let mut deduped = Vec::new();
    for path in paths {
        let key = path.to_string_lossy().to_string();
        if seen.insert(key) {
            deduped.push(path);
        }
    }
    deduped
}

pub(crate) fn looks_like_recipe_library_root(path: &Path) -> bool {
    if !path.is_dir() {
        return false;
    }

    let entries = match fs::read_dir(path) {
        Ok(entries) => entries,
        Err(_) => return false,
    };

    entries.flatten().any(|entry| {
        let recipe_dir = entry.path();
        recipe_dir.is_dir() && recipe_dir.join("recipe.json").is_file()
    })
}

fn collect_recipe_dirs(root: &Path) -> Result<Vec<std::path::PathBuf>, String> {
    if !root.exists() {
        return Err(format!(
            "recipe library root does not exist: {}",
            root.to_string_lossy()
        ));
    }
    if !root.is_dir() {
        return Err(format!(
            "recipe library root is not a directory: {}",
            root.to_string_lossy()
        ));
    }

    let mut recipe_dirs = Vec::new();
    for entry in fs::read_dir(root).map_err(|error| error.to_string())? {
        let entry = entry.map_err(|error| error.to_string())?;
        let path = entry.path();
        if path.is_dir() {
            recipe_dirs.push(path);
        }
    }
    recipe_dirs.sort();
    Ok(recipe_dirs)
}

fn import_recipe_dir(
    recipe_dir: &Path,
    workspace: &RecipeWorkspace,
    seen_recipe_ids: &mut std::collections::BTreeSet<String>,
    seen_slugs: &mut std::collections::BTreeSet<String>,
) -> Result<ImportedRecipe, String> {
    let recipe_path = recipe_dir.join("recipe.json");
    if !recipe_path.exists() {
        return Err("recipe.json not found".into());
    }

    let source = fs::read_to_string(&recipe_path).map_err(|error| {
        format!(
            "failed to read recipe source '{}': {}",
            recipe_path.to_string_lossy(),
            error
        )
    })?;

    let (recipe_id, compiled_source) = compile_recipe_source(recipe_dir, &source)?;
    let slug = crate::recipe_workspace::normalize_recipe_slug(&recipe_id)?;
    if !seen_recipe_ids.insert(recipe_id.clone()) {
        return Err(format!("duplicate recipe id '{}'", recipe_id));
    }
    if !seen_slugs.insert(slug.clone()) {
        return Err(format!("duplicate recipe slug '{}'", slug));
    }
    let diagnostics = validate_recipe_source(&compiled_source)?;
    if !diagnostics.errors.is_empty() {
        return Err(diagnostics
            .errors
            .iter()
            .map(|diagnostic| diagnostic.message.clone())
            .collect::<Vec<_>>()
            .join("; "));
    }

    let saved = workspace.save_recipe_source(&slug, &compiled_source)?;
    Ok(ImportedRecipe {
        slug: saved.slug,
        recipe_id,
        path: saved.path,
    })
}

fn compile_recipe_source(recipe_dir: &Path, source: &str) -> Result<(String, String), String> {
    let mut document: Value = json5::from_str(source).map_err(|error| error.to_string())?;
    let recipe = document
        .as_object_mut()
        .ok_or_else(|| "recipe.json must contain a single recipe object".to_string())?;

    let preset_specs = compile_preset_specs(recipe_dir, recipe.get("clawpalImport"))?;
    if !preset_specs.is_empty() {
        inject_param_options(recipe, &preset_specs)?;
        inject_preset_maps(recipe, &preset_specs);
    } else {
        recipe.remove("clawpalImport");
    }
    let recipe = document
        .as_object_mut()
        .ok_or_else(|| "compiled recipe document must stay as an object".to_string())?;
    recipe.remove("clawpalImport");

    let recipe_id = document
        .get("id")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| "recipe.id is required".to_string())?
        .to_string();

    let compiled = serde_json::to_string_pretty(&document).map_err(|error| error.to_string())?;
    Ok((recipe_id, compiled))
}

#[derive(Debug, Clone)]
struct PresetSpec {
    options: Vec<Value>,
    values: Map<String, Value>,
}

fn compile_preset_specs(
    recipe_dir: &Path,
    clawpal_import: Option<&Value>,
) -> Result<BTreeMap<String, PresetSpec>, String> {
    let mut result = BTreeMap::new();
    let Some(import_object) = clawpal_import.and_then(Value::as_object) else {
        return Ok(result);
    };
    let Some(preset_params) = import_object.get("presetParams").and_then(Value::as_object) else {
        return Ok(result);
    };

    for (param_id, entries) in preset_params {
        let entries = entries
            .as_array()
            .ok_or_else(|| format!("clawpalImport.presetParams.{} must be an array", param_id))?;
        let mut options = Vec::new();
        let mut values = Map::new();

        for entry in entries {
            let entry = entry.as_object().ok_or_else(|| {
                format!(
                    "clawpalImport.presetParams.{} entries must be objects",
                    param_id
                )
            })?;
            let value = required_string(entry, "value", param_id)?;
            let label = required_string(entry, "label", param_id)?;
            let asset = required_string(entry, "asset", param_id)?;
            let asset_path = recipe_dir.join(&asset);
            if !asset_path.exists() {
                return Err(format!(
                    "missing asset '{}' for preset param '{}'",
                    asset, param_id
                ));
            }
            let text = fs::read_to_string(&asset_path).map_err(|error| {
                format!(
                    "failed to read asset '{}' for preset param '{}': {}",
                    asset, param_id, error
                )
            })?;

            options.push(serde_json::json!({
                "value": value,
                "label": label,
            }));
            values.insert(value, Value::String(text));
        }

        result.insert(param_id.clone(), PresetSpec { options, values });
    }

    Ok(result)
}

fn required_string(
    entry: &Map<String, Value>,
    field: &str,
    param_id: &str,
) -> Result<String, String> {
    entry
        .get(field)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
        .ok_or_else(|| {
            format!(
                "clawpalImport.presetParams.{} entry is missing '{}'",
                param_id, field
            )
        })
}

fn inject_param_options(
    recipe: &mut Map<String, Value>,
    preset_specs: &BTreeMap<String, PresetSpec>,
) -> Result<(), String> {
    let params = recipe
        .get_mut("params")
        .and_then(Value::as_array_mut)
        .ok_or_else(|| "recipe.params must be an array".to_string())?;

    for (param_id, spec) in preset_specs {
        let Some(param) = params
            .iter_mut()
            .find(|param| param.get("id").and_then(Value::as_str) == Some(param_id.as_str()))
        else {
            return Err(format!(
                "clawpalImport.presetParams references unknown param '{}'",
                param_id
            ));
        };
        let param_object = param
            .as_object_mut()
            .ok_or_else(|| format!("param '{}' must be an object", param_id))?;
        param_object.insert("options".into(), Value::Array(spec.options.clone()));
    }

    Ok(())
}

fn inject_preset_maps(
    recipe: &mut Map<String, Value>,
    preset_specs: &BTreeMap<String, PresetSpec>,
) {
    let preset_maps = preset_specs
        .iter()
        .map(|(param_id, spec)| (param_id.clone(), Value::Object(spec.values.clone())))
        .collect();
    recipe.insert("clawpalPresetMaps".into(), Value::Object(preset_maps));
}
