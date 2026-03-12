use std::fs::{self, File};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::models::resolve_paths;

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct ResourceClaim {
    pub kind: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub target: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct Artifact {
    pub id: String,
    pub kind: String,
    pub label: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct Run {
    pub id: String,
    pub instance_id: String,
    pub recipe_id: String,
    pub execution_kind: String,
    pub runner: String,
    pub status: String,
    pub summary: String,
    pub started_at: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub finished_at: Option<String>,
    #[serde(default)]
    pub artifacts: Vec<Artifact>,
    #[serde(default)]
    pub resource_claims: Vec<ResourceClaim>,
    #[serde(default)]
    pub warnings: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_origin: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_digest: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub workspace_path: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct RecipeInstance {
    pub id: String,
    pub recipe_id: String,
    pub execution_kind: String,
    pub runner: String,
    pub status: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_run_id: Option<String>,
    pub updated_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
struct RecipeRuntimeIndex {
    #[serde(default)]
    instances: Vec<RecipeInstance>,
    #[serde(default)]
    runs: Vec<Run>,
}

#[derive(Debug, Clone)]
pub struct RecipeStore {
    runtime_dir: PathBuf,
    index_path: PathBuf,
}

impl RecipeStore {
    pub fn new(runtime_dir: PathBuf) -> Self {
        Self {
            index_path: runtime_dir.join("index.json"),
            runtime_dir,
        }
    }

    pub fn from_resolved_paths() -> Self {
        Self::new(resolve_paths().recipe_runtime_dir)
    }

    pub fn for_test() -> Self {
        let root = std::env::temp_dir().join(format!("clawpal-recipe-store-{}", Uuid::new_v4()));
        Self::new(root)
    }

    pub fn record_run(&self, run: Run) -> Result<Run, String> {
        fs::create_dir_all(&self.runtime_dir).map_err(|error| error.to_string())?;

        let mut index = self.read_index()?;
        let updated_at = run
            .finished_at
            .clone()
            .unwrap_or_else(|| run.started_at.clone());

        index.runs.retain(|existing| existing.id != run.id);
        index.runs.push(run.clone());
        index.runs.sort_by(|left, right| {
            right
                .started_at
                .cmp(&left.started_at)
                .then_with(|| right.id.cmp(&left.id))
        });

        let next_instance = RecipeInstance {
            id: run.instance_id.clone(),
            recipe_id: run.recipe_id.clone(),
            execution_kind: run.execution_kind.clone(),
            runner: run.runner.clone(),
            status: run.status.clone(),
            last_run_id: Some(run.id.clone()),
            updated_at,
        };

        index
            .instances
            .retain(|instance| instance.id != next_instance.id);
        index.instances.push(next_instance);
        index.instances.sort_by(|left, right| {
            right
                .updated_at
                .cmp(&left.updated_at)
                .then_with(|| left.id.cmp(&right.id))
        });

        self.write_index(&index)?;
        Ok(run)
    }

    pub fn list_runs(&self, instance_id: &str) -> Result<Vec<Run>, String> {
        let index = self.read_index()?;
        Ok(index
            .runs
            .into_iter()
            .filter(|run| run.instance_id == instance_id)
            .collect())
    }

    pub fn list_all_runs(&self) -> Result<Vec<Run>, String> {
        Ok(self.read_index()?.runs)
    }

    pub fn list_instances(&self) -> Result<Vec<RecipeInstance>, String> {
        Ok(self.read_index()?.instances)
    }

    fn read_index(&self) -> Result<RecipeRuntimeIndex, String> {
        if !self.index_path.exists() {
            return Ok(RecipeRuntimeIndex::default());
        }

        let mut file = File::open(&self.index_path).map_err(|error| error.to_string())?;
        let mut text = String::new();
        file.read_to_string(&mut text)
            .map_err(|error| error.to_string())?;

        if text.trim().is_empty() {
            return Ok(RecipeRuntimeIndex::default());
        }

        serde_json::from_str(&text).map_err(|error| error.to_string())
    }

    fn write_index(&self, index: &RecipeRuntimeIndex) -> Result<(), String> {
        fs::create_dir_all(&self.runtime_dir).map_err(|error| error.to_string())?;
        let text = serde_json::to_string_pretty(index).map_err(|error| error.to_string())?;
        atomic_write(&self.index_path, &text)
    }
}

fn atomic_write(path: &Path, text: &str) -> Result<(), String> {
    let tmp_path = path.with_extension("tmp");
    {
        let mut file = File::create(&tmp_path).map_err(|error| error.to_string())?;
        file.write_all(text.as_bytes())
            .map_err(|error| error.to_string())?;
        file.sync_all().map_err(|error| error.to_string())?;
    }
    fs::rename(&tmp_path, path).map_err(|error| error.to_string())
}
