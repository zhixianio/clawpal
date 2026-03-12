use std::fs;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::config_io::write_text;
use crate::models::resolve_paths;

const WORKSPACE_FILE_SUFFIX: &str = ".recipe.json";

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
        let slug = normalize_slug(raw_slug)?;
        let path = self.root.join(format!("{}{}", slug, WORKSPACE_FILE_SUFFIX));
        write_text(&path, source)?;
        Ok(RecipeSourceSaveResult {
            slug,
            path: path.to_string_lossy().to_string(),
        })
    }

    pub fn delete_recipe_source(&self, raw_slug: &str) -> Result<(), String> {
        let path = self.path_for_slug(raw_slug)?;
        if path.exists() {
            fs::remove_file(path).map_err(|error| error.to_string())?;
        }
        Ok(())
    }

    fn path_for_slug(&self, raw_slug: &str) -> Result<PathBuf, String> {
        let slug = normalize_slug(raw_slug)?;
        Ok(self.root.join(format!("{}{}", slug, WORKSPACE_FILE_SUFFIX)))
    }
}

fn normalize_slug(raw_slug: &str) -> Result<String, String> {
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
