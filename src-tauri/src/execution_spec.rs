use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeSet;

use crate::recipe_bundle::{parse_structured_document, validate_execution_kind, RecipeBundle};

const SUPPORTED_RESOURCE_CLAIM_KINDS: &[&str] = &[
    "path",
    "file",
    "service",
    "channel",
    "agent",
    "identity",
    "document",
    "modelProfile",
    "authProfile",
];

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase", default)]
pub struct ExecutionMetadata {
    pub name: Option<String>,
    pub digest: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase", default)]
pub struct ExecutionTarget {
    pub kind: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase", default)]
pub struct ExecutionCapabilities {
    pub used_capabilities: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase", default)]
pub struct ExecutionResourceClaim {
    pub kind: String,
    pub id: Option<String>,
    pub target: Option<String>,
    pub path: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase", default)]
pub struct ExecutionResources {
    pub claims: Vec<ExecutionResourceClaim>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase", default)]
pub struct ExecutionSecretBinding {
    pub id: String,
    pub source: String,
    pub mount: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase", default)]
pub struct ExecutionSecrets {
    pub bindings: Vec<ExecutionSecretBinding>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase", default)]
pub struct ExecutionAction {
    pub kind: Option<String>,
    pub name: Option<String>,
    pub args: Value,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase", default)]
pub struct ExecutionSpec {
    #[serde(rename = "apiVersion")]
    pub api_version: String,
    pub kind: String,
    pub metadata: ExecutionMetadata,
    pub source: Value,
    pub target: Value,
    pub execution: ExecutionTarget,
    pub capabilities: ExecutionCapabilities,
    pub resources: ExecutionResources,
    pub secrets: ExecutionSecrets,
    pub desired_state: Value,
    pub actions: Vec<ExecutionAction>,
    pub outputs: Vec<Value>,
}

pub fn parse_execution_spec(raw: &str) -> Result<ExecutionSpec, String> {
    let spec: ExecutionSpec = parse_structured_document(raw)?;
    validate_execution_spec(&spec)?;
    Ok(spec)
}

pub fn validate_execution_spec(spec: &ExecutionSpec) -> Result<(), String> {
    if spec.kind != "ExecutionSpec" {
        return Err(format!("unsupported document kind: {}", spec.kind));
    }

    validate_execution_kind(&spec.execution.kind)?;

    for claim in &spec.resources.claims {
        if !SUPPORTED_RESOURCE_CLAIM_KINDS.contains(&claim.kind.as_str()) {
            return Err(format!(
                "resource claim '{}' uses an unsupported kind",
                claim.kind
            ));
        }
    }

    for binding in &spec.secrets.bindings {
        if binding.source.trim().starts_with("plain://") {
            return Err(format!(
                "secret binding '{}' uses a disallowed plain source",
                binding.id
            ));
        }
    }

    Ok(())
}

pub fn validate_execution_spec_against_bundle(
    spec: &ExecutionSpec,
    bundle: &RecipeBundle,
) -> Result<(), String> {
    validate_execution_spec(spec)?;

    if !bundle.execution.supported_kinds.is_empty()
        && !bundle
            .execution
            .supported_kinds
            .iter()
            .any(|kind| kind == &spec.execution.kind)
    {
        return Err(format!(
            "execution kind '{}' is not supported by this bundle",
            spec.execution.kind
        ));
    }

    let allowed_capabilities: BTreeSet<&str> = bundle
        .capabilities
        .allowed
        .iter()
        .map(String::as_str)
        .collect();
    let unsupported_capabilities: Vec<&str> = spec
        .capabilities
        .used_capabilities
        .iter()
        .map(String::as_str)
        .filter(|capability| !allowed_capabilities.contains(capability))
        .collect();
    if !unsupported_capabilities.is_empty() {
        return Err(format!(
            "execution spec uses capabilities not granted by bundle: {}",
            unsupported_capabilities.join(", ")
        ));
    }

    let supported_resource_kinds: BTreeSet<&str> = bundle
        .resources
        .supported_kinds
        .iter()
        .map(String::as_str)
        .collect();
    let unsupported_claims: Vec<&str> = spec
        .resources
        .claims
        .iter()
        .map(|claim| claim.kind.as_str())
        .filter(|kind| !supported_resource_kinds.contains(kind))
        .collect();
    if !unsupported_claims.is_empty() {
        return Err(format!(
            "execution spec declares claims for unsupported resource kinds: {}",
            unsupported_claims.join(", ")
        ));
    }

    Ok(())
}
