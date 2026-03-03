use serde::{Deserialize, Serialize};

/// Model pricing: (prompt_price_per_1k_tokens, completion_price_per_1k_tokens)
fn model_pricing(model: &str) -> Option<(f64, f64)> {
    let lower = model.trim().to_ascii_lowercase();
    // Match by substring to handle provider prefixes like "openrouter/anthropic/claude-sonnet-4-5"
    // Also match legacy IDs for backward compat with existing user configs.
    if lower.contains("claude-sonnet-4-5")
        || lower.contains("claude-3.7-sonnet")
        || lower.contains("claude-3-7-sonnet")
    {
        Some((0.003, 0.015))
    } else if lower.contains("claude-haiku-4-5")
        || lower.contains("claude-3.5-haiku")
        || lower.contains("claude-3-5-haiku")
    {
        Some((0.001, 0.005))
    } else if lower.contains("gpt-4o-mini") {
        Some((0.00015, 0.0006))
    } else if lower.contains("gpt-4o") {
        Some((0.0025, 0.01))
    } else if lower.contains("gpt-4.1") {
        Some((0.002, 0.008))
    } else if lower.contains("gemini-2.0-flash") {
        Some((0.0001, 0.0004))
    } else if lower.contains("kimi-k2") {
        Some((0.0006, 0.002))
    } else {
        None
    }
}

/// Estimate cost in USD for a given model and token counts.
pub fn estimate_cost(model: &str, prompt_tokens: u64, completion_tokens: u64) -> Option<f64> {
    let (prompt_price, completion_price) = model_pricing(model)?;
    let cost = (prompt_tokens as f64 / 1000.0) * prompt_price
        + (completion_tokens as f64 / 1000.0) * completion_price;
    Some(cost)
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct CostEstimate {
    pub model: String,
    pub prompt_tokens: u64,
    pub completion_tokens: u64,
    pub estimated_cost_usd: Option<f64>,
}

#[tauri::command]
pub fn estimate_query_cost(
    model: String,
    prompt_tokens: u64,
    completion_tokens: u64,
) -> Result<CostEstimate, String> {
    let cost = estimate_cost(&model, prompt_tokens, completion_tokens);
    Ok(CostEstimate {
        model,
        prompt_tokens,
        completion_tokens,
        estimated_cost_usd: cost,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn estimate_cost_for_known_model() {
        let cost = estimate_cost("gpt-4o", 1000, 1000).unwrap();
        assert!((cost - 0.0125).abs() < 0.0001);
    }

    #[test]
    fn estimate_cost_for_unknown_model() {
        assert!(estimate_cost("unknown-model", 1000, 1000).is_none());
    }

    #[test]
    fn estimate_cost_with_provider_prefix() {
        let cost = estimate_cost("openrouter/anthropic/claude-sonnet-4-5", 1000, 500).unwrap();
        assert!(cost > 0.0);
    }

    #[test]
    fn estimate_cost_legacy_model_still_matches() {
        // Old IDs from user configs should still resolve
        assert!(estimate_cost("claude-3-7-sonnet-latest", 100, 100).is_some());
        assert!(estimate_cost("claude-3-5-haiku-20241022", 100, 100).is_some());
    }

    #[test]
    fn estimate_cost_new_haiku_model() {
        let cost = estimate_cost("claude-haiku-4-5", 1000, 1000).unwrap();
        assert!((cost - 0.006).abs() < 0.0001);
    }
}
