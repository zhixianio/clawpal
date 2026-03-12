use regex::Regex;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use std::{
    env, fs,
    path::{Path, PathBuf},
};

use crate::execution_spec::ExecutionSpec;
use crate::recipe_bundle::RecipeBundle;
use crate::{
    execution_spec::validate_execution_spec,
    recipe_adapter::{build_recipe_spec_template, canonical_recipe_bundle},
    recipe_bundle::validate_execution_spec_against_bundle,
};

const BUILTIN_RECIPES_JSON: &str = include_str!("../recipes.json");

#[derive(Debug, Serialize, Deserialize)]
#[serde(untagged)]
enum RecipeDocument {
    Single(Recipe),
    List(Vec<Recipe>),
    Wrapped { recipes: Vec<Recipe> },
}

#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct RecipeParam {
    pub id: String,
    pub label: String,
    #[serde(rename = "type")]
    pub kind: String,
    pub required: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pattern: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub min_length: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_length: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub placeholder: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub depends_on: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub default_value: Option<String>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct RecipeStep {
    pub action: String,
    pub label: String,
    pub args: Map<String, Value>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct Recipe {
    pub id: String,
    pub name: String,
    pub description: String,
    pub version: String,
    pub tags: Vec<String>,
    pub difficulty: String,
    pub params: Vec<RecipeParam>,
    pub steps: Vec<RecipeStep>,
    #[serde(skip_serializing, default)]
    pub bundle: Option<RecipeBundle>,
    #[serde(skip_serializing, default)]
    pub execution_spec_template: Option<ExecutionSpec>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct ChangeItem {
    pub path: String,
    pub op: String,
    pub risk: String,
    pub reason: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PreviewResult {
    pub recipe_id: String,
    pub diff: String,
    pub config_before: String,
    pub config_after: String,
    pub changes: Vec<ChangeItem>,
    pub overwrites_existing: bool,
    pub can_rollback: bool,
    pub impact_level: String,
    pub warnings: Vec<String>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ApplyResult {
    pub ok: bool,
    pub snapshot_id: Option<String>,
    pub config_path: String,
    pub backup_path: Option<String>,
    pub warnings: Vec<String>,
    pub errors: Vec<String>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct RecipeSourceDiagnostic {
    pub category: String,
    pub severity: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub recipe_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    pub message: String,
}

#[derive(Debug, Serialize, Deserialize, Clone, Default)]
#[serde(rename_all = "camelCase")]
pub struct RecipeSourceDiagnostics {
    #[serde(default)]
    pub errors: Vec<RecipeSourceDiagnostic>,
    #[serde(default)]
    pub warnings: Vec<RecipeSourceDiagnostic>,
}

pub fn builtin_recipes() -> Vec<Recipe> {
    parse_recipes_document(BUILTIN_RECIPES_JSON).unwrap_or_else(|_| Vec::new())
}

fn is_http_url(candidate: &str) -> bool {
    candidate.starts_with("http://") || candidate.starts_with("https://")
}

fn expand_user_path(candidate: &str) -> PathBuf {
    if let Some(rest) = candidate.strip_prefix("~/") {
        if let Some(home) = dirs::home_dir() {
            return home.join(rest);
        }
    }
    PathBuf::from(candidate)
}

fn parse_recipes_document(text: &str) -> Result<Vec<Recipe>, String> {
    let document: RecipeDocument = json5::from_str(text).map_err(|e| e.to_string())?;
    match document {
        RecipeDocument::Single(recipe) => Ok(vec![recipe]),
        RecipeDocument::List(recipes) => Ok(recipes),
        RecipeDocument::Wrapped { recipes } => Ok(recipes),
    }
}

pub fn load_recipes_from_source_text(text: &str) -> Result<Vec<Recipe>, String> {
    if text.trim().is_empty() {
        return Err("empty recipe source".into());
    }
    parse_recipes_document(text)
}

pub fn load_recipes_from_source(source: &str) -> Result<Vec<Recipe>, String> {
    if source.trim().is_empty() {
        return Err("empty recipe source".into());
    }

    if is_http_url(source) {
        let response = reqwest::blocking::get(source).map_err(|e| e.to_string())?;
        if !response.status().is_success() {
            return Err(format!("request failed: {}", response.status()));
        }
        let text = response.text().map_err(|e| e.to_string())?;
        load_recipes_from_source_text(&text)
    } else {
        let path = expand_user_path(source);
        let path = Path::new(&path);
        if !path.exists() {
            return Err(format!("recipe file not found: {}", path.to_string_lossy()));
        }
        let text = fs::read_to_string(path).map_err(|e| e.to_string())?;
        load_recipes_from_source_text(&text)
    }
}

pub fn load_recipes_with_fallback(
    explicit_source: Option<String>,
    default_path: &Path,
) -> Vec<Recipe> {
    let builtin = builtin_recipes();

    let candidates = [
        explicit_source,
        env::var("CLAWPAL_RECIPES_SOURCE").ok(),
        Some(default_path.to_string_lossy().to_string()),
    ];

    for candidate in candidates.iter().flatten() {
        if candidate.trim().is_empty() {
            continue;
        }
        if let Ok(recipes) = load_recipes_from_source(candidate) {
            if !recipes.is_empty() {
                return recipes;
            }
        }
    }

    builtin
}

pub fn find_recipe(id: &str) -> Option<Recipe> {
    find_recipe_with_source(id, None)
}

pub fn find_recipe_with_source(id: &str, source: Option<String>) -> Option<Recipe> {
    let paths = crate::models::resolve_paths();
    let default_path = paths.clawpal_dir.join("recipes").join("recipes.json");
    load_recipes_with_fallback(source, &default_path)
        .into_iter()
        .find(|r| r.id == id)
}

pub fn validate_recipe_source(text: &str) -> Result<RecipeSourceDiagnostics, String> {
    let mut diagnostics = RecipeSourceDiagnostics::default();
    let recipes = match load_recipes_from_source_text(text) {
        Ok(recipes) => recipes,
        Err(error) => {
            diagnostics.errors.push(RecipeSourceDiagnostic {
                category: "parse".into(),
                severity: "error".into(),
                recipe_id: None,
                path: None,
                message: error,
            });
            return Ok(diagnostics);
        }
    };

    for recipe in &recipes {
        validate_recipe_definition(recipe, &mut diagnostics);
    }

    Ok(diagnostics)
}

fn validate_recipe_definition(recipe: &Recipe, diagnostics: &mut RecipeSourceDiagnostics) {
    if let Some(template) = &recipe.execution_spec_template {
        if template.actions.len() != recipe.steps.len() {
            diagnostics.errors.push(RecipeSourceDiagnostic {
                category: "alignment".into(),
                severity: "error".into(),
                recipe_id: Some(recipe.id.clone()),
                path: Some("steps".into()),
                message: format!(
                    "recipe '{}' declares {} UI step(s) but {} execution action(s)",
                    recipe.id,
                    recipe.steps.len(),
                    template.actions.len()
                ),
            });
        }
    }

    let spec = match build_recipe_spec_template(recipe) {
        Ok(spec) => spec,
        Err(error) => {
            diagnostics.errors.push(RecipeSourceDiagnostic {
                category: "schema".into(),
                severity: "error".into(),
                recipe_id: Some(recipe.id.clone()),
                path: Some("executionSpecTemplate".into()),
                message: error,
            });
            return;
        }
    };

    if let Err(error) = validate_execution_spec(&spec) {
        diagnostics.errors.push(RecipeSourceDiagnostic {
            category: "schema".into(),
            severity: "error".into(),
            recipe_id: Some(recipe.id.clone()),
            path: Some("executionSpecTemplate".into()),
            message: error,
        });
        return;
    }

    let bundle = canonical_recipe_bundle(recipe, &spec);
    if let Err(error) = validate_execution_spec_against_bundle(&bundle, &spec) {
        diagnostics.errors.push(RecipeSourceDiagnostic {
            category: "bundle".into(),
            severity: "error".into(),
            recipe_id: Some(recipe.id.clone()),
            path: Some("bundle".into()),
            message: error,
        });
    }
}

pub fn validate(recipe: &Recipe, params: &Map<String, Value>) -> Vec<String> {
    let mut errors = Vec::new();
    for p in &recipe.params {
        if p.required && !params.contains_key(&p.id) {
            errors.push(format!("missing required param: {}", p.id));
            continue;
        }

        if let Some(v) = params.get(&p.id) {
            let s = match v {
                Value::String(s) => s.clone(),
                _ => {
                    errors.push(format!("param {} must be string", p.id));
                    continue;
                }
            };
            if let Some(min) = p.min_length {
                if s.len() < min {
                    errors.push(format!("param {} too short", p.id));
                }
            }
            if let Some(max) = p.max_length {
                if s.len() > max {
                    errors.push(format!("param {} too long", p.id));
                }
            }
            if let Some(pattern) = &p.pattern {
                let re = Regex::new(pattern).map_err(|e| e.to_string()).ok();
                if let Some(re) = re {
                    if !re.is_match(&s) {
                        errors.push(format!("param {} not match pattern", p.id));
                    }
                } else {
                    errors.push("invalid validation pattern".into());
                }
            }
        }
    }
    errors
}

fn param_value_to_string(value: &Value) -> String {
    match value {
        Value::String(text) => text.clone(),
        _ => value.to_string(),
    }
}

fn extract_placeholders(text: &str) -> Vec<String> {
    Regex::new(r"\{\{(\w+)\}\}")
        .ok()
        .map(|regex| {
            regex
                .captures_iter(text)
                .filter_map(|capture| capture.get(1).map(|value| value.as_str().to_string()))
                .collect()
        })
        .unwrap_or_default()
}

pub fn render_template_string(template: &str, params: &Map<String, Value>) -> String {
    let mut text = template.to_string();
    for (k, v) in params {
        let placeholder = format!("{{{{{}}}}}", k);
        let replacement = param_value_to_string(v);
        text = text.replace(&placeholder, &replacement);
    }
    text
}

pub fn render_template_value(value: &Value, params: &Map<String, Value>) -> Value {
    match value {
        Value::String(text) => {
            if let Some(param_id) = text
                .strip_prefix("{{")
                .and_then(|rest| rest.strip_suffix("}}"))
            {
                if param_id
                    .chars()
                    .all(|ch| ch.is_ascii_alphanumeric() || ch == '_')
                {
                    return params
                        .get(param_id)
                        .cloned()
                        .unwrap_or_else(|| Value::String(String::new()));
                }
            }
            Value::String(render_template_string(text, params))
        }
        Value::Array(items) => Value::Array(
            items
                .iter()
                .map(|item| render_template_value(item, params))
                .collect(),
        ),
        Value::Object(map) => Value::Object(
            map.iter()
                .map(|(key, value)| {
                    (
                        render_template_string(key, params),
                        render_template_value(value, params),
                    )
                })
                .collect(),
        ),
        _ => value.clone(),
    }
}

pub fn render_step_args(
    args: &Map<String, Value>,
    params: &Map<String, Value>,
) -> Map<String, Value> {
    args.iter()
        .map(|(key, value)| (key.clone(), render_template_value(value, params)))
        .collect()
}

pub fn step_references_empty_param(step: &RecipeStep, params: &Map<String, Value>) -> bool {
    fn value_references_empty_param(value: &Value, params: &Map<String, Value>) -> bool {
        match value {
            Value::String(text) => extract_placeholders(text).into_iter().any(|param_id| {
                params
                    .get(&param_id)
                    .and_then(Value::as_str)
                    .map(|value| value.trim().is_empty())
                    .unwrap_or(false)
            }),
            Value::Array(items) => items
                .iter()
                .any(|item| value_references_empty_param(item, params)),
            Value::Object(map) => map
                .values()
                .any(|item| value_references_empty_param(item, params)),
            _ => false,
        }
    }

    step.args
        .values()
        .any(|value| value_references_empty_param(value, params))
}

pub fn build_candidate_config_from_template(
    current: &Value,
    template: &str,
    params: &Map<String, Value>,
) -> Result<(Value, Vec<ChangeItem>), String> {
    let rendered = render_template_string(template, params);
    let patch: Value = json5::from_str(&rendered).map_err(|e| e.to_string())?;
    let mut merged = current.clone();
    let mut changes = Vec::new();
    apply_merge_patch(&mut merged, &patch, "", &mut changes);
    Ok((merged, changes))
}

fn apply_merge_patch(
    target: &mut Value,
    patch: &Value,
    prefix: &str,
    changes: &mut Vec<ChangeItem>,
) {
    if patch.is_object() && target.is_object() {
        let t = target.as_object_mut().unwrap();
        for (k, pv) in patch.as_object().unwrap() {
            let path = if prefix.is_empty() {
                k.clone()
            } else {
                format!("{}.{}", prefix, k)
            };
            match pv {
                Value::Null => {
                    if t.remove(k).is_some() {
                        changes.push(ChangeItem {
                            path: path.clone(),
                            op: "remove".into(),
                            risk: "medium".into(),
                            reason: None,
                        });
                    }
                }
                _ => {
                    if let Some(tv) = t.get_mut(k) {
                        if tv.is_object() && pv.is_object() {
                            apply_merge_patch(tv, pv, &path, changes);
                        } else {
                            *tv = pv.clone();
                            changes.push(ChangeItem {
                                path,
                                op: "replace".into(),
                                risk: "low".into(),
                                reason: None,
                            });
                        }
                    } else {
                        t.insert(k.clone(), pv.clone());
                        changes.push(ChangeItem {
                            path,
                            op: "add".into(),
                            risk: "low".into(),
                            reason: None,
                        });
                    }
                }
            }
        }
    } else {
        *target = patch.clone();
        changes.push(ChangeItem {
            path: prefix.to_string(),
            op: "replace".into(),
            risk: "medium".into(),
            reason: None,
        });
    }
}

pub fn collect_change_paths(current: &Value, patched: &Value) -> Vec<ChangeItem> {
    if current == patched {
        Vec::new()
    } else {
        vec![ChangeItem {
            path: "root".to_string(),
            op: "replace".to_string(),
            risk: "medium".to_string(),
            reason: None,
        }]
    }
}

pub fn format_diff(before: &Value, after: &Value) -> String {
    let before_text = serde_json::to_string_pretty(before).unwrap_or_else(|_| "{}".into());
    let after_text = serde_json::to_string_pretty(after).unwrap_or_else(|_| "{}".into());
    format!("before:\n{}\n\nafter:\n{}", before_text, after_text)
}
