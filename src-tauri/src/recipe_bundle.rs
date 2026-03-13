use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use serde_json::Value;

pub const SUPPORTED_EXECUTION_KINDS: &[&str] = &["job", "service", "schedule", "attachment"];

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase", default)]
pub struct BundleMetadata {
    pub name: Option<String>,
    pub version: Option<String>,
    pub description: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase", default)]
pub struct BundleCompatibility {
    pub min_runner_version: Option<String>,
    pub target_platforms: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase", default)]
pub struct BundleCapabilities {
    pub allowed: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase", default)]
pub struct BundleResources {
    pub supported_kinds: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase", default)]
pub struct BundleExecution {
    pub supported_kinds: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase", default)]
pub struct BundleRunner {
    pub name: Option<String>,
    pub version: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase", default)]
pub struct RecipeBundle {
    #[serde(rename = "apiVersion")]
    pub api_version: String,
    pub kind: String,
    pub metadata: BundleMetadata,
    pub compatibility: BundleCompatibility,
    pub inputs: Vec<Value>,
    pub capabilities: BundleCapabilities,
    pub resources: BundleResources,
    pub execution: BundleExecution,
    pub runner: BundleRunner,
    pub outputs: Vec<Value>,
}

pub fn parse_recipe_bundle(raw: &str) -> Result<RecipeBundle, String> {
    let bundle: RecipeBundle = parse_structured_document(raw)?;
    validate_recipe_bundle(&bundle)?;
    Ok(bundle)
}

pub fn validate_recipe_bundle(bundle: &RecipeBundle) -> Result<(), String> {
    if bundle.kind != "StrategyBundle" {
        return Err(format!("unsupported document kind: {}", bundle.kind));
    }

    for kind in &bundle.execution.supported_kinds {
        validate_execution_kind(kind)?;
    }
    Ok(())
}

pub fn validate_execution_spec_against_bundle(
    bundle: &RecipeBundle,
    spec: &crate::execution_spec::ExecutionSpec,
) -> Result<(), String> {
    crate::execution_spec::validate_execution_spec_against_bundle(spec, bundle)
}

pub(crate) fn parse_structured_document<T>(raw: &str) -> Result<T, String>
where
    T: DeserializeOwned,
{
    serde_json::from_str(raw)
        .or_else(|_| json5::from_str(raw))
        .or_else(|_| serde_yaml::from_str(raw))
        .map_err(|error| format!("failed to parse structured document: {error}"))
}

pub(crate) fn validate_execution_kind(kind: &str) -> Result<(), String> {
    if SUPPORTED_EXECUTION_KINDS.contains(&kind) {
        Ok(())
    } else {
        Err(format!("unsupported execution kind: {kind}"))
    }
}
