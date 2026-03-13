use std::collections::BTreeMap;
use std::fs;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::config_io::write_text;
use crate::models::resolve_paths;
use crate::recipe::load_recipes_from_source_text;
use crate::recipe_library::RecipeLibraryImportResult;

const WORKSPACE_FILE_SUFFIX: &str = ".recipe.json";
const WORKSPACE_INDEX_FILE: &str = ".bundled-seed-index.json";

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum RecipeWorkspaceSourceKind {
    Bundled,
    LocalImport,
    RemoteUrl,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum BundledRecipeState {
    Missing,
    UpToDate,
    UpdateAvailable,
    LocalModified,
    ConflictedUpdate,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum RecipeTrustLevel {
    Trusted,
    Caution,
    Untrusted,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum RecipeRiskLevel {
    Low,
    Medium,
    High,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct RecipeWorkspaceEntry {
    pub slug: String,
    pub path: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub recipe_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_kind: Option<RecipeWorkspaceSourceKind>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bundled_version: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bundled_state: Option<BundledRecipeState>,
    pub trust_level: RecipeTrustLevel,
    pub risk_level: RecipeRiskLevel,
    pub approval_required: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct RecipeSourceSaveResult {
    pub slug: String,
    pub path: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
struct RecipeWorkspaceIndexEntry {
    pub recipe_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_kind: Option<RecipeWorkspaceSourceKind>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub seeded_digest: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bundled_version: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub approval_digest: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
#[serde(rename_all = "camelCase", default)]
struct RecipeWorkspaceIndex {
    #[serde(default)]
    pub entries: BTreeMap<String, RecipeWorkspaceIndexEntry>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct BundledRecipeDescriptor {
    pub recipe_id: String,
    pub version: String,
    pub digest: String,
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
                recipe_id: None,
                version: None,
                source_kind: None,
                bundled_version: None,
                bundled_state: None,
                trust_level: RecipeTrustLevel::Caution,
                risk_level: RecipeRiskLevel::Medium,
                approval_required: false,
            });
        }

        entries.sort_by(|left, right| left.slug.cmp(&right.slug));
        Ok(entries)
    }

    pub(crate) fn describe_entries(
        &self,
        bundled_descriptors: &BTreeMap<String, BundledRecipeDescriptor>,
    ) -> Result<Vec<RecipeWorkspaceEntry>, String> {
        let index = self.read_workspace_index()?;
        let mut entries = self.list_entries()?;

        for entry in &mut entries {
            let source_text = fs::read_to_string(&entry.path).map_err(|error| {
                format!("failed to read recipe source '{}': {}", entry.slug, error)
            })?;
            let recipe = load_recipes_from_source_text(&source_text)?
                .into_iter()
                .next()
                .ok_or_else(|| format!("workspace recipe '{}' is empty", entry.slug))?;
            let source_digest = Self::source_digest(&source_text);
            let index_entry = index.entries.get(&entry.slug);
            let source_kind = index_entry
                .and_then(|value| value.source_kind)
                .unwrap_or(RecipeWorkspaceSourceKind::LocalImport);
            let bundled_state = if source_kind == RecipeWorkspaceSourceKind::Bundled {
                bundled_descriptors
                    .get(&entry.slug)
                    .map(|descriptor| {
                        self.bundled_recipe_state_with_seeded_digest(
                            &entry.slug,
                            &source_digest,
                            descriptor.digest.as_str(),
                            index_entry.and_then(|value| value.seeded_digest.as_deref()),
                        )
                    })
                    .transpose()?
            } else {
                None
            };
            let risk_level = risk_level_for_recipe_source(&source_text)?;
            let approval_required = approval_required_for(source_kind, risk_level)
                && index_entry.and_then(|value| value.approval_digest.as_deref())
                    != Some(source_digest.as_str());

            entry.recipe_id = Some(recipe.id);
            entry.version = Some(recipe.version);
            entry.source_kind = Some(source_kind);
            entry.bundled_version = index_entry.and_then(|value| value.bundled_version.clone());
            entry.bundled_state = bundled_state;
            entry.trust_level = trust_level_for_source_kind(source_kind);
            entry.risk_level = risk_level;
            entry.approval_required = approval_required;
        }

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
        let (recipe_id, _) = parse_recipe_header(source)?;
        let saved = self.write_recipe_source(&slug, source)?;
        let mut index = self.read_workspace_index()?;
        let existing = index.entries.get(&slug).cloned();
        index.entries.insert(
            slug.clone(),
            RecipeWorkspaceIndexEntry {
                recipe_id,
                source_kind: existing
                    .as_ref()
                    .and_then(|value| value.source_kind)
                    .or(Some(RecipeWorkspaceSourceKind::LocalImport)),
                seeded_digest: existing
                    .as_ref()
                    .and_then(|value| value.seeded_digest.clone()),
                bundled_version: existing
                    .as_ref()
                    .and_then(|value| value.bundled_version.clone()),
                approval_digest: None,
            },
        );
        self.write_workspace_index(&index)?;
        Ok(saved)
    }

    pub fn save_imported_recipe_source(
        &self,
        raw_slug: &str,
        source: &str,
        source_kind: RecipeWorkspaceSourceKind,
    ) -> Result<RecipeSourceSaveResult, String> {
        let slug = normalize_recipe_slug(raw_slug)?;
        let (recipe_id, _) = parse_recipe_header(source)?;
        let saved = self.write_recipe_source(&slug, source)?;
        let mut index = self.read_workspace_index()?;
        index.entries.insert(
            slug.clone(),
            RecipeWorkspaceIndexEntry {
                recipe_id,
                source_kind: Some(source_kind),
                seeded_digest: None,
                bundled_version: None,
                approval_digest: None,
            },
        );
        self.write_workspace_index(&index)?;
        Ok(saved)
    }

    pub fn save_bundled_recipe_source(
        &self,
        raw_slug: &str,
        source: &str,
        recipe_id: &str,
        bundled_version: &str,
    ) -> Result<RecipeSourceSaveResult, String> {
        let slug = normalize_recipe_slug(raw_slug)?;
        let saved = self.write_recipe_source(&slug, source)?;
        let mut index = self.read_workspace_index()?;
        index.entries.insert(
            slug.clone(),
            RecipeWorkspaceIndexEntry {
                recipe_id: recipe_id.trim().to_string(),
                source_kind: Some(RecipeWorkspaceSourceKind::Bundled),
                seeded_digest: Some(Self::source_digest(source)),
                bundled_version: Some(bundled_version.trim().to_string()),
                approval_digest: None,
            },
        );
        self.write_workspace_index(&index)?;
        Ok(saved)
    }

    pub fn delete_recipe_source(&self, raw_slug: &str) -> Result<(), String> {
        let slug = normalize_recipe_slug(raw_slug)?;
        let path = self.path_for_slug(&slug)?;
        if path.exists() {
            fs::remove_file(path).map_err(|error| error.to_string())?;
        }
        self.clear_workspace_index_entry(&slug)?;
        Ok(())
    }

    pub fn import_recipe_library(
        &self,
        root: &PathBuf,
    ) -> Result<RecipeLibraryImportResult, String> {
        crate::recipe_library::import_recipe_library(root, self)
    }

    pub(crate) fn bundled_recipe_state(
        &self,
        raw_slug: &str,
        current_bundled_source: &str,
    ) -> Result<BundledRecipeState, String> {
        let slug = normalize_recipe_slug(raw_slug)?;
        let path = self.path_for_slug(&slug)?;
        if !path.exists() {
            return Ok(BundledRecipeState::Missing);
        }

        let current = fs::read_to_string(&path)
            .map_err(|error| format!("failed to read recipe source '{}': {}", slug, error))?;
        let current_digest = Self::source_digest(&current);
        let bundled_digest = Self::source_digest(current_bundled_source);
        let index = self.read_workspace_index()?;
        let seeded_digest = index
            .entries
            .get(&slug)
            .and_then(|entry| entry.seeded_digest.as_deref());

        self.bundled_recipe_state_with_seeded_digest(
            &slug,
            &current_digest,
            &bundled_digest,
            seeded_digest,
        )
    }

    pub fn approve_recipe(&self, raw_slug: &str, digest: &str) -> Result<(), String> {
        let slug = normalize_recipe_slug(raw_slug)?;
        let mut index = self.read_workspace_index()?;
        let entry = index
            .entries
            .get_mut(&slug)
            .ok_or_else(|| format!("workspace recipe '{}' is not tracked", slug))?;
        entry.approval_digest = Some(digest.trim().to_string());
        self.write_workspace_index(&index)
    }

    pub fn is_recipe_approved(&self, raw_slug: &str, digest: &str) -> Result<bool, String> {
        let slug = normalize_recipe_slug(raw_slug)?;
        let index = self.read_workspace_index()?;
        Ok(index
            .entries
            .get(&slug)
            .and_then(|entry| entry.approval_digest.as_deref())
            == Some(digest.trim()))
    }

    pub fn source_digest(source: &str) -> String {
        recipe_source_digest(source)
    }

    pub(crate) fn workspace_source_kind(
        &self,
        raw_slug: &str,
    ) -> Result<Option<RecipeWorkspaceSourceKind>, String> {
        let slug = normalize_recipe_slug(raw_slug)?;
        let index = self.read_workspace_index()?;
        Ok(index.entries.get(&slug).and_then(|entry| entry.source_kind))
    }

    pub(crate) fn workspace_risk_level(&self, raw_slug: &str) -> Result<RecipeRiskLevel, String> {
        let slug = normalize_recipe_slug(raw_slug)?;
        let source = self.read_recipe_source(&slug)?;
        risk_level_for_recipe_source(&source)
    }

    fn path_for_slug(&self, raw_slug: &str) -> Result<PathBuf, String> {
        let slug = normalize_recipe_slug(raw_slug)?;
        Ok(self.root.join(format!("{}{}", slug, WORKSPACE_FILE_SUFFIX)))
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

    fn workspace_index_path(&self) -> PathBuf {
        self.root.join(WORKSPACE_INDEX_FILE)
    }

    fn read_workspace_index(&self) -> Result<RecipeWorkspaceIndex, String> {
        let path = self.workspace_index_path();
        if !path.exists() {
            return Ok(RecipeWorkspaceIndex::default());
        }

        let text = fs::read_to_string(&path)
            .map_err(|error| format!("failed to read recipe workspace index: {}", error))?;
        json5::from_str::<RecipeWorkspaceIndex>(&text)
            .map_err(|error| format!("failed to parse recipe workspace index: {}", error))
    }

    fn write_workspace_index(&self, index: &RecipeWorkspaceIndex) -> Result<(), String> {
        let path = self.workspace_index_path();
        if index.entries.is_empty() {
            if path.exists() {
                fs::remove_file(path).map_err(|error| error.to_string())?;
            }
            return Ok(());
        }

        let text = serde_json::to_string_pretty(index).map_err(|error| error.to_string())?;
        write_text(&path, &text)
    }

    fn clear_workspace_index_entry(&self, slug: &str) -> Result<(), String> {
        let mut index = self.read_workspace_index()?;
        if index.entries.remove(slug).is_some() {
            self.write_workspace_index(&index)?;
        }
        Ok(())
    }

    fn bundled_recipe_state_with_seeded_digest(
        &self,
        slug: &str,
        current_workspace_digest: &str,
        current_bundled_digest: &str,
        seeded_digest: Option<&str>,
    ) -> Result<BundledRecipeState, String> {
        let seeded_digest = seeded_digest.ok_or_else(|| {
            format!(
                "workspace recipe '{}' is missing bundled seed metadata",
                slug
            )
        })?;

        if current_workspace_digest == seeded_digest {
            if current_bundled_digest == seeded_digest {
                Ok(BundledRecipeState::UpToDate)
            } else {
                Ok(BundledRecipeState::UpdateAvailable)
            }
        } else if current_bundled_digest == seeded_digest {
            Ok(BundledRecipeState::LocalModified)
        } else {
            Ok(BundledRecipeState::ConflictedUpdate)
        }
    }
}

fn recipe_source_digest(source: &str) -> String {
    Uuid::new_v5(&Uuid::NAMESPACE_URL, source.as_bytes()).to_string()
}

fn parse_recipe_header(source: &str) -> Result<(String, String), String> {
    let recipe = load_recipes_from_source_text(source)?
        .into_iter()
        .next()
        .ok_or_else(|| "recipe source does not contain any recipes".to_string())?;
    Ok((
        recipe.id.trim().to_string(),
        recipe.version.trim().to_string(),
    ))
}

fn risk_level_for_recipe_source(source: &str) -> Result<RecipeRiskLevel, String> {
    let recipe = load_recipes_from_source_text(source)?
        .into_iter()
        .next()
        .ok_or_else(|| "recipe source does not contain any recipes".to_string())?;

    let action_kinds = if let Some(spec) = recipe.execution_spec_template.as_ref() {
        spec.actions
            .iter()
            .filter_map(|action| action.kind.as_ref())
            .map(|kind| kind.trim().to_string())
            .collect::<Vec<_>>()
    } else {
        recipe
            .steps
            .iter()
            .map(|step| step.action.trim().to_string())
            .collect::<Vec<_>>()
    };

    Ok(risk_level_for_action_kinds(&action_kinds))
}

fn risk_level_for_action_kinds(action_kinds: &[String]) -> RecipeRiskLevel {
    if action_kinds.is_empty() {
        return RecipeRiskLevel::Low;
    }

    let catalog = crate::recipe_action_catalog::list_recipe_actions();
    let all_read_only = action_kinds.iter().all(|kind| {
        catalog
            .iter()
            .find(|entry| entry.kind == *kind)
            .map(|entry| entry.read_only)
            .unwrap_or(false)
    });
    if all_read_only {
        return RecipeRiskLevel::Low;
    }

    if action_kinds.iter().any(|kind| {
        matches!(
            kind.as_str(),
            "delete_agent"
                | "unbind_agent"
                | "delete_model_profile"
                | "delete_provider_auth"
                | "delete_markdown_document"
                | "ensure_model_profile"
                | "ensure_provider_auth"
                | "set_config_value"
                | "unset_config_value"
                | "config_patch"
                | "apply_secrets_plan"
        )
    }) {
        return RecipeRiskLevel::High;
    }

    RecipeRiskLevel::Medium
}

pub(crate) fn trust_level_for_source_kind(
    source_kind: RecipeWorkspaceSourceKind,
) -> RecipeTrustLevel {
    match source_kind {
        RecipeWorkspaceSourceKind::Bundled => RecipeTrustLevel::Trusted,
        RecipeWorkspaceSourceKind::LocalImport => RecipeTrustLevel::Caution,
        RecipeWorkspaceSourceKind::RemoteUrl => RecipeTrustLevel::Untrusted,
    }
}

pub(crate) fn approval_required_for(
    source_kind: RecipeWorkspaceSourceKind,
    risk_level: RecipeRiskLevel,
) -> bool {
    match source_kind {
        RecipeWorkspaceSourceKind::Bundled => risk_level == RecipeRiskLevel::High,
        RecipeWorkspaceSourceKind::LocalImport | RecipeWorkspaceSourceKind::RemoteUrl => {
            risk_level != RecipeRiskLevel::Low
        }
    }
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
