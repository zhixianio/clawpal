use std::collections::BTreeMap;
use std::fs;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::config_io::write_text;
use crate::models::resolve_paths;
use crate::recipe_library::RecipeLibraryImportResult;

const WORKSPACE_FILE_SUFFIX: &str = ".recipe.json";
const BUNDLED_SEED_INDEX_FILE: &str = ".bundled-seed-index.json";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct RecipeWorkspaceEntry {
    pub slug: String,
    pub path: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct RecipeSourceSaveResult {
    pub slug: String,
    pub path: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
struct BundledSeedIndexEntry {
    pub recipe_id: String,
    pub seeded_digest: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
struct BundledSeedIndex {
    #[serde(default)]
    pub entries: BTreeMap<String, BundledSeedIndexEntry>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum BundledSeedStatus {
    Missing,
    Unchanged,
    ModifiedSinceSeed,
    UntrackedExisting,
}

#[derive(Debug, Clone)]
pub struct RecipeWorkspace {
    root: PathBuf,
}

impl RecipeWorkspace {
    pub fn new(root: PathBuf) -> Self {
        Self { root }
    }

    pub fn from_resolved_paths() -> Self {
        let root = resolve_paths()
            .clawpal_dir
            .join("recipes")
            .join("workspace");
        Self::new(root)
    }

    pub fn list_entries(&self) -> Result<Vec<RecipeWorkspaceEntry>, String> {
        if !self.root.exists() {
            return Ok(Vec::new());
        }

        let mut entries = Vec::new();
        for entry in fs::read_dir(&self.root).map_err(|error| error.to_string())? {
            let entry = entry.map_err(|error| error.to_string())?;
            let path = entry.path();
            if !path.is_file() {
                continue;
            }

            let Some(file_name) = path.file_name().and_then(|value| value.to_str()) else {
                continue;
            };
            let Some(slug) = file_name.strip_suffix(WORKSPACE_FILE_SUFFIX) else {
                continue;
            };

            entries.push(RecipeWorkspaceEntry {
                slug: slug.to_string(),
                path: path.to_string_lossy().to_string(),
            });
        }

        entries.sort_by(|left, right| left.slug.cmp(&right.slug));
        Ok(entries)
    }

    pub fn read_recipe_source(&self, slug: &str) -> Result<String, String> {
        let path = self.path_for_slug(slug)?;
        fs::read_to_string(&path)
            .map_err(|error| format!("failed to read recipe source '{}': {}", slug, error))
    }

    pub fn resolve_recipe_source_path(&self, raw_slug: &str) -> Result<String, String> {
        self.path_for_slug(raw_slug)
            .map(|path| path.to_string_lossy().to_string())
    }

    pub fn save_recipe_source(
        &self,
        raw_slug: &str,
        source: &str,
    ) -> Result<RecipeSourceSaveResult, String> {
        let slug = normalize_recipe_slug(raw_slug)?;
        let saved = self.write_recipe_source(&slug, source)?;
        self.clear_bundled_seed_entry(&slug)?;
        Ok(saved)
    }

    pub fn save_bundled_recipe_source(
        &self,
        raw_slug: &str,
        source: &str,
        recipe_id: &str,
    ) -> Result<RecipeSourceSaveResult, String> {
        let slug = normalize_recipe_slug(raw_slug)?;
        let saved = self.write_recipe_source(&slug, source)?;
        let mut index = self.read_bundled_seed_index()?;
        index.entries.insert(
            slug.clone(),
            BundledSeedIndexEntry {
                recipe_id: recipe_id.trim().to_string(),
                seeded_digest: recipe_source_digest(source),
            },
        );
        self.write_bundled_seed_index(&index)?;
        Ok(saved)
    }

    pub fn delete_recipe_source(&self, raw_slug: &str) -> Result<(), String> {
        let slug = normalize_recipe_slug(raw_slug)?;
        let path = self.path_for_slug(&slug)?;
        if path.exists() {
            fs::remove_file(path).map_err(|error| error.to_string())?;
        }
        self.clear_bundled_seed_entry(&slug)?;
        Ok(())
    }

    pub fn import_recipe_library(
        &self,
        root: &PathBuf,
    ) -> Result<RecipeLibraryImportResult, String> {
        crate::recipe_library::import_recipe_library(root, self)
    }

    fn path_for_slug(&self, raw_slug: &str) -> Result<PathBuf, String> {
        let slug = normalize_recipe_slug(raw_slug)?;
        Ok(self.root.join(format!("{}{}", slug, WORKSPACE_FILE_SUFFIX)))
    }

    pub(crate) fn bundled_seed_status(&self, raw_slug: &str) -> Result<BundledSeedStatus, String> {
        let slug = normalize_recipe_slug(raw_slug)?;
        let path = self.path_for_slug(&slug)?;
        if !path.exists() {
            return Ok(BundledSeedStatus::Missing);
        }

        let index = self.read_bundled_seed_index()?;
        let Some(entry) = index.entries.get(&slug) else {
            return Ok(BundledSeedStatus::UntrackedExisting);
        };

        let current = fs::read_to_string(&path)
            .map_err(|error| format!("failed to read recipe source '{}': {}", slug, error))?;
        if recipe_source_digest(&current) == entry.seeded_digest {
            Ok(BundledSeedStatus::Unchanged)
        } else {
            Ok(BundledSeedStatus::ModifiedSinceSeed)
        }
    }

    fn write_recipe_source(
        &self,
        slug: &str,
        source: &str,
    ) -> Result<RecipeSourceSaveResult, String> {
        let path = self.root.join(format!("{}{}", slug, WORKSPACE_FILE_SUFFIX));
        write_text(&path, source)?;
        Ok(RecipeSourceSaveResult {
            slug: slug.to_string(),
            path: path.to_string_lossy().to_string(),
        })
    }

    fn bundled_seed_index_path(&self) -> PathBuf {
        self.root.join(BUNDLED_SEED_INDEX_FILE)
    }

    fn read_bundled_seed_index(&self) -> Result<BundledSeedIndex, String> {
        let path = self.bundled_seed_index_path();
        if !path.exists() {
            return Ok(BundledSeedIndex::default());
        }

        let text = fs::read_to_string(&path)
            .map_err(|error| format!("failed to read bundled seed index: {}", error))?;
        json5::from_str::<BundledSeedIndex>(&text)
            .map_err(|error| format!("failed to parse bundled seed index: {}", error))
    }

    fn write_bundled_seed_index(&self, index: &BundledSeedIndex) -> Result<(), String> {
        let path = self.bundled_seed_index_path();
        if index.entries.is_empty() {
            if path.exists() {
                fs::remove_file(path).map_err(|error| error.to_string())?;
            }
            return Ok(());
        }

        let text = serde_json::to_string_pretty(index).map_err(|error| error.to_string())?;
        write_text(&path, &text)
    }

    fn clear_bundled_seed_entry(&self, slug: &str) -> Result<(), String> {
        let mut index = self.read_bundled_seed_index()?;
        if index.entries.remove(slug).is_some() {
            self.write_bundled_seed_index(&index)?;
        }
        Ok(())
    }
}

fn recipe_source_digest(source: &str) -> String {
    Uuid::new_v5(&Uuid::NAMESPACE_URL, source.as_bytes()).to_string()
}

pub(crate) fn normalize_recipe_slug(raw_slug: &str) -> Result<String, String> {
    let trimmed = raw_slug.trim();
    if trimmed.is_empty() {
        return Err("recipe slug cannot be empty".into());
    }
    if trimmed.contains('/') || trimmed.contains('\\') || trimmed.contains("..") {
        return Err("recipe slug contains a disallowed path segment".into());
    }

    let mut slug = String::new();
    let mut last_was_dash = false;
    for ch in trimmed.chars() {
        if ch.is_ascii_alphanumeric() {
            slug.push(ch.to_ascii_lowercase());
            last_was_dash = false;
            continue;
        }

        if matches!(ch, '-' | '_' | ' ') {
            if !slug.is_empty() && !last_was_dash {
                slug.push('-');
                last_was_dash = true;
            }
            continue;
        }

        return Err(format!(
            "recipe slug contains unsupported character '{}'",
            ch
        ));
    }

    while slug.ends_with('-') {
        slug.pop();
    }

    if slug.is_empty() {
        return Err("recipe slug must contain at least one alphanumeric character".into());
    }

    Ok(slug)
}
