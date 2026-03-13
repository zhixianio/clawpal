use crate::recipe_bundle::parse_recipe_bundle;

#[test]
fn recipe_bundle_rejects_unknown_execution_kind() {
    let raw = r#"apiVersion: strategy.platform/v1
kind: StrategyBundle
execution: { supportedKinds: [workflow] }"#;

    assert!(parse_recipe_bundle(raw).is_err());
}
