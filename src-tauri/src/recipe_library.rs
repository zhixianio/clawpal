use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use tauri::Manager;

use crate::recipe::{
    load_recipes_from_source, load_recipes_from_source_text, validate_recipe_source,
};
use crate::recipe_adapter::export_recipe_source as export_recipe_source_document;
use crate::recipe_workspace::{
    BundledRecipeDescriptor, BundledRecipeState, RecipeWorkspace, RecipeWorkspaceSourceKind,
};

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

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct RecipeImportConflict {
    pub slug: String,
    pub recipe_id: String,
    pub path: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct SkippedRecipeSourceImport {
    pub source: String,
    pub reason: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum RecipeImportSourceKind {
    LocalFile,
    LocalRecipeDirectory,
    LocalRecipeLibrary,
    RemoteUrl,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct RecipeSourceImportResult {
    pub source_kind: Option<RecipeImportSourceKind>,
    #[serde(default)]
    pub imported: Vec<ImportedRecipe>,
    #[serde(default)]
    pub skipped: Vec<SkippedRecipeSourceImport>,
    #[serde(default)]
    pub warnings: Vec<String>,
    #[serde(default)]
    pub conflicts: Vec<RecipeImportConflict>,
}

#[derive(Debug, Clone)]
struct PreparedRecipeImport {
    slug: String,
    recipe_id: String,
    source_text: String,
}

#[derive(Debug, Clone)]
pub(crate) struct BundledRecipeSource {
    pub recipe_id: String,
    pub version: String,
    pub source_text: String,
    pub digest: String,
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
    let mut seen_slugs = std::collections::BTreeSet::new();
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
            result.skipped.push(SkippedRecipeImport {
                recipe_dir: recipe_dir.to_string_lossy().to_string(),
                reason: format!("duplicate recipe slug '{}'", slug),
            });
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

        match workspace.bundled_recipe_state(&slug, &compiled_source) {
            Ok(BundledRecipeState::UpToDate | BundledRecipeState::UpdateAvailable) => continue,
            Ok(BundledRecipeState::LocalModified | BundledRecipeState::ConflictedUpdate) => {
                result.warnings.push(format!(
                    "Skipped bundled recipe '{}' because workspace recipe '{}' was modified locally.",
                    recipe_id, slug
                ));
                continue;
            }
            Ok(BundledRecipeState::Missing) | Err(_) => {
                if workspace
                    .resolve_recipe_source_path(&slug)
                    .ok()
                    .is_some_and(|path| Path::new(&path).exists())
                {
                    result.warnings.push(format!(
                        "Skipped bundled recipe '{}' because workspace recipe '{}' already exists.",
                        recipe_id, slug
                    ));
                    continue;
                }
            }
        }

        let version = load_recipes_from_source_text(&compiled_source)?
            .into_iter()
            .next()
            .map(|recipe| recipe.version)
            .unwrap_or_else(|| "0.0.0".into());
        let saved =
            workspace.save_bundled_recipe_source(&slug, &compiled_source, &recipe_id, &version)?;
        result.imported.push(ImportedRecipe {
            slug: saved.slug,
            recipe_id,
            path: saved.path,
        });
    }

    Ok(result)
}

pub fn import_recipe_source(
    source: &str,
    workspace: &RecipeWorkspace,
    overwrite_existing: bool,
) -> Result<RecipeSourceImportResult, String> {
    let trimmed = source.trim();
    if trimmed.is_empty() {
        return Err("recipe import source cannot be empty".into());
    }

    let prepared = prepare_recipe_imports(trimmed)?;
    let import_source_kind = workspace_source_kind_for_import(prepared.source_kind.clone());
    let mut result = RecipeSourceImportResult {
        source_kind: Some(prepared.source_kind.clone()),
        skipped: prepared.skipped,
        warnings: prepared.warnings,
        ..RecipeSourceImportResult::default()
    };

    let existing = workspace
        .list_entries()?
        .into_iter()
        .map(|entry| (entry.slug, entry.path))
        .collect::<std::collections::BTreeMap<_, _>>();

    if !overwrite_existing {
        result.conflicts = prepared
            .items
            .iter()
            .filter_map(|item| {
                existing.get(&item.slug).map(|path| RecipeImportConflict {
                    slug: item.slug.clone(),
                    recipe_id: item.recipe_id.clone(),
                    path: path.clone(),
                })
            })
            .collect();
        if !result.conflicts.is_empty() {
            return Ok(result);
        }
    }

    for item in prepared.items {
        let saved = workspace.save_imported_recipe_source(
            &item.slug,
            &item.source_text,
            import_source_kind.clone(),
        )?;
        result.imported.push(ImportedRecipe {
            slug: saved.slug,
            recipe_id: item.recipe_id,
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

pub fn upgrade_bundled_recipe(
    app_handle: &tauri::AppHandle,
    workspace: &RecipeWorkspace,
    slug: &str,
) -> Result<crate::recipe_workspace::RecipeSourceSaveResult, String> {
    let sources = load_bundled_recipe_sources(app_handle)?;
    let bundled = sources
        .get(slug)
        .ok_or_else(|| format!("bundled recipe '{}' not found", slug))?;
    match workspace.bundled_recipe_state(slug, &bundled.source_text)? {
        BundledRecipeState::UpdateAvailable | BundledRecipeState::Missing => {}
        BundledRecipeState::UpToDate => {
            return Err(format!("bundled recipe '{}' is already up to date", slug));
        }
        BundledRecipeState::LocalModified => {
            return Err(format!(
                "bundled recipe '{}' has local changes and must be reviewed before replacing",
                slug
            ));
        }
        BundledRecipeState::ConflictedUpdate => {
            return Err(format!(
                "bundled recipe '{}' has local changes and a newer bundled version",
                slug
            ));
        }
    }
    workspace.save_bundled_recipe_source(
        slug,
        &bundled.source_text,
        &bundled.recipe_id,
        &bundled.version,
    )
}

pub(crate) fn load_bundled_recipe_descriptors(
    app_handle: &tauri::AppHandle,
) -> Result<BTreeMap<String, BundledRecipeDescriptor>, String> {
    Ok(load_bundled_recipe_sources(app_handle)?
        .into_iter()
        .map(|(slug, source)| {
            (
                slug,
                BundledRecipeDescriptor {
                    recipe_id: source.recipe_id,
                    version: source.version,
                    digest: source.digest,
                },
            )
        })
        .collect())
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

    if let Ok(resource_root) = app_handle
        .path()
        .resolve("_up_/recipe-library", tauri::path::BaseDirectory::Resource)
    {
        candidates.push(resource_root);
    }

    if let Ok(resource_root) = app_handle.path().resolve(
        "_up_/examples/recipe-library",
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
    let (recipe_id, compiled_source) = compile_recipe_directory_source(recipe_dir)?;
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

    let saved = workspace.save_imported_recipe_source(
        &slug,
        &compiled_source,
        RecipeWorkspaceSourceKind::LocalImport,
    )?;
    Ok(ImportedRecipe {
        slug: saved.slug,
        recipe_id,
        path: saved.path,
    })
}

fn load_bundled_recipe_sources(
    app_handle: &tauri::AppHandle,
) -> Result<BTreeMap<String, BundledRecipeSource>, String> {
    let root = resolve_bundled_recipe_library_root(app_handle)?;
    load_bundled_recipe_sources_from_root(&root)
}

fn load_bundled_recipe_sources_from_root(
    root: &Path,
) -> Result<BTreeMap<String, BundledRecipeSource>, String> {
    let mut sources = BTreeMap::new();
    for recipe_dir in collect_recipe_dirs(root)? {
        let (recipe_id, compiled_source) = compile_recipe_directory_source(&recipe_dir)?;
        let slug = crate::recipe_workspace::normalize_recipe_slug(&recipe_id)?;
        let version = load_recipes_from_source_text(&compiled_source)?
            .into_iter()
            .next()
            .map(|recipe| recipe.version)
            .unwrap_or_else(|| "0.0.0".into());
        sources.insert(
            slug.clone(),
            BundledRecipeSource {
                recipe_id,
                version,
                digest: RecipeWorkspace::source_digest(&compiled_source),
                source_text: compiled_source,
            },
        );
    }
    Ok(sources)
}

fn workspace_source_kind_for_import(
    source_kind: RecipeImportSourceKind,
) -> RecipeWorkspaceSourceKind {
    match source_kind {
        RecipeImportSourceKind::RemoteUrl => RecipeWorkspaceSourceKind::RemoteUrl,
        RecipeImportSourceKind::LocalFile
        | RecipeImportSourceKind::LocalRecipeDirectory
        | RecipeImportSourceKind::LocalRecipeLibrary => RecipeWorkspaceSourceKind::LocalImport,
    }
}

pub(crate) fn compile_recipe_directory_source(
    recipe_dir: &Path,
) -> Result<(String, String), String> {
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

    compile_recipe_source(recipe_dir, &source)
}

fn prepare_recipe_imports(source: &str) -> Result<PreparedRecipeImports, String> {
    if looks_like_http_source(source) {
        return prepare_imports_from_loaded_recipes(
            RecipeImportSourceKind::RemoteUrl,
            source,
            source,
        );
    }

    let path = PathBuf::from(shellexpand::tilde(source).to_string());
    if path.is_dir() {
        if looks_like_recipe_library_root(&path) {
            return prepare_imports_from_recipe_library(&path);
        }
        if path.join("recipe.json").is_file() {
            return prepare_imports_from_loaded_recipes(
                RecipeImportSourceKind::LocalRecipeDirectory,
                source,
                &path.to_string_lossy(),
            );
        }
        return Err(format!(
            "recipe source directory is neither a recipe folder nor a recipe library root: {}",
            path.to_string_lossy()
        ));
    }

    prepare_imports_from_loaded_recipes(
        RecipeImportSourceKind::LocalFile,
        source,
        &path.to_string_lossy(),
    )
}

struct PreparedRecipeImports {
    source_kind: RecipeImportSourceKind,
    items: Vec<PreparedRecipeImport>,
    skipped: Vec<SkippedRecipeSourceImport>,
    warnings: Vec<String>,
}

fn prepare_imports_from_loaded_recipes(
    source_kind: RecipeImportSourceKind,
    raw_source: &str,
    source_ref: &str,
) -> Result<PreparedRecipeImports, String> {
    let recipes = load_recipes_from_source(raw_source)?;
    let mut seen_recipe_ids = std::collections::BTreeSet::new();
    let mut seen_slugs = std::collections::BTreeSet::new();
    let mut items = Vec::new();
    let mut skipped = Vec::new();

    for recipe in recipes {
        let recipe_id = recipe.id.trim().to_string();
        let slug = crate::recipe_workspace::normalize_recipe_slug(&recipe_id)?;
        if !seen_recipe_ids.insert(recipe_id.clone()) {
            skipped.push(SkippedRecipeSourceImport {
                source: source_ref.to_string(),
                reason: format!("duplicate recipe id '{}'", recipe_id),
            });
            continue;
        }
        if !seen_slugs.insert(slug.clone()) {
            skipped.push(SkippedRecipeSourceImport {
                source: source_ref.to_string(),
                reason: format!("duplicate recipe slug '{}'", slug),
            });
            continue;
        }
        let source_text = export_recipe_source_document(&recipe)?;
        items.push(PreparedRecipeImport {
            slug,
            recipe_id,
            source_text,
        });
    }

    Ok(PreparedRecipeImports {
        source_kind,
        items,
        skipped,
        warnings: Vec::new(),
    })
}

fn prepare_imports_from_recipe_library(root: &Path) -> Result<PreparedRecipeImports, String> {
    let recipe_dirs = collect_recipe_dirs(root)?;
    let mut seen_recipe_ids = std::collections::BTreeSet::new();
    let mut seen_slugs = std::collections::BTreeSet::new();
    let mut items = Vec::new();
    let mut skipped = Vec::new();

    for recipe_dir in recipe_dirs {
        match compile_recipe_directory_source(&recipe_dir) {
            Ok((recipe_id, compiled_source)) => {
                let slug = crate::recipe_workspace::normalize_recipe_slug(&recipe_id)?;
                if !seen_recipe_ids.insert(recipe_id.clone()) {
                    skipped.push(SkippedRecipeSourceImport {
                        source: recipe_dir.to_string_lossy().to_string(),
                        reason: format!("duplicate recipe id '{}'", recipe_id),
                    });
                    continue;
                }
                if !seen_slugs.insert(slug.clone()) {
                    skipped.push(SkippedRecipeSourceImport {
                        source: recipe_dir.to_string_lossy().to_string(),
                        reason: format!("duplicate recipe slug '{}'", slug),
                    });
                    continue;
                }
                let diagnostics = validate_recipe_source(&compiled_source)?;
                if !diagnostics.errors.is_empty() {
                    skipped.push(SkippedRecipeSourceImport {
                        source: recipe_dir.to_string_lossy().to_string(),
                        reason: diagnostics
                            .errors
                            .iter()
                            .map(|diagnostic| diagnostic.message.clone())
                            .collect::<Vec<_>>()
                            .join("; "),
                    });
                    continue;
                }
                items.push(PreparedRecipeImport {
                    slug,
                    recipe_id,
                    source_text: compiled_source,
                });
            }
            Err(error) => skipped.push(SkippedRecipeSourceImport {
                source: recipe_dir.to_string_lossy().to_string(),
                reason: error,
            }),
        }
    }

    Ok(PreparedRecipeImports {
        source_kind: RecipeImportSourceKind::LocalRecipeLibrary,
        items,
        skipped,
        warnings: Vec::new(),
    })
}

fn looks_like_http_source(source: &str) -> bool {
    let trimmed = source.trim();
    trimmed.starts_with("http://") || trimmed.starts_with("https://")
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
