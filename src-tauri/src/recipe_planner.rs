use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use uuid::Uuid;

use crate::execution_spec::{ExecutionResourceClaim, ExecutionSpec};
use crate::recipe::{load_recipes_from_source_text, step_references_empty_param, Recipe};
use crate::recipe_adapter::compile_recipe_to_spec;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RecipePlanSummary {
    pub recipe_id: String,
    pub recipe_name: String,
    pub execution_kind: String,
    pub action_count: usize,
    pub skipped_step_count: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RecipePlan {
    pub summary: RecipePlanSummary,
    pub used_capabilities: Vec<String>,
    pub concrete_claims: Vec<ExecutionResourceClaim>,
    pub execution_spec_digest: String,
    pub execution_spec: ExecutionSpec,
    pub warnings: Vec<String>,
}

pub fn build_recipe_plan(
    recipe: &Recipe,
    params: &Map<String, Value>,
) -> Result<RecipePlan, String> {
    let execution_spec = compile_recipe_to_spec(recipe, params)?;
    let skipped_step_count = recipe
        .steps
        .iter()
        .filter(|step| step_references_empty_param(step, params))
        .count();

    let mut warnings = Vec::new();
    if skipped_step_count > 0 {
        warnings.push(format!(
            "{} optional step(s) will be skipped because their parameters are empty.",
            skipped_step_count
        ));
    }
    let digest_source = serde_json::to_vec(&execution_spec).map_err(|error| error.to_string())?;
    let execution_spec_digest = Uuid::new_v5(&Uuid::NAMESPACE_OID, &digest_source).to_string();

    Ok(RecipePlan {
        summary: RecipePlanSummary {
            recipe_id: recipe.id.clone(),
            recipe_name: recipe.name.clone(),
            execution_kind: execution_spec.execution.kind.clone(),
            action_count: execution_spec.actions.len(),
            skipped_step_count,
        },
        used_capabilities: execution_spec.capabilities.used_capabilities.clone(),
        concrete_claims: execution_spec.resources.claims.clone(),
        execution_spec_digest,
        execution_spec,
        warnings,
    })
}

pub fn build_recipe_plan_from_source_text(
    recipe_id: &str,
    params: &Map<String, Value>,
    source_text: &str,
) -> Result<RecipePlan, String> {
    let recipe = load_recipes_from_source_text(source_text)?
        .into_iter()
        .find(|recipe| recipe.id == recipe_id)
        .ok_or_else(|| format!("recipe not found: {}", recipe_id))?;
    build_recipe_plan(&recipe, params)
}
