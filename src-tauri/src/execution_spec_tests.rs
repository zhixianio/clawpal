use crate::execution_spec::parse_execution_spec;
use crate::recipe_bundle::{parse_recipe_bundle, validate_execution_spec_against_bundle};

#[test]
fn execution_spec_rejects_inline_secret_value() {
    let raw = r#"apiVersion: strategy.platform/v1
kind: ExecutionSpec
execution: { kind: job }
secrets: { bindings: [{ id: "k", source: "plain://abc" }] }"#;

    assert!(parse_execution_spec(raw).is_err());
}

#[test]
fn execution_spec_rejects_capabilities_outside_bundle_budget() {
    let bundle_raw = r#"apiVersion: strategy.platform/v1
kind: StrategyBundle
capabilities: { allowed: ["service.manage"] }
resources: { supportedKinds: ["path"] }
execution: { supportedKinds: ["job"] }"#;
    let spec_raw = r#"apiVersion: strategy.platform/v1
kind: ExecutionSpec
execution: { kind: "job" }
capabilities: { usedCapabilities: ["service.manage", "secret.read"] }
resources: { claims: [{ kind: "path", path: "/tmp/openclaw" }] }"#;

    let bundle = parse_recipe_bundle(bundle_raw).expect("parse bundle");
    let spec = parse_execution_spec(spec_raw).expect("parse spec");

    assert!(validate_execution_spec_against_bundle(&bundle, &spec).is_err());
}

#[test]
fn execution_spec_rejects_unknown_resource_claim_kind() {
    let bundle_raw = r#"apiVersion: strategy.platform/v1
kind: StrategyBundle
capabilities: { allowed: ["service.manage"] }
resources: { supportedKinds: ["path"] }
execution: { supportedKinds: ["job"] }"#;
    let spec_raw = r#"apiVersion: strategy.platform/v1
kind: ExecutionSpec
execution: { kind: "job" }
capabilities: { usedCapabilities: ["service.manage"] }
resources: { claims: [{ kind: "file", path: "/tmp/app.sock" }] }"#;

    let bundle = parse_recipe_bundle(bundle_raw).expect("parse bundle");
    let spec = parse_execution_spec(spec_raw).expect("parse spec");

    assert!(validate_execution_spec_against_bundle(&bundle, &spec).is_err());
}

#[test]
fn execution_spec_rejects_unknown_resource_kind() {
    let raw = r#"apiVersion: strategy.platform/v1
kind: ExecutionSpec
execution:
  kind: job
resources:
  claims:
    - id: workspace
      kind: workflow"#;

    assert!(parse_execution_spec(raw).is_err());
}

#[test]
fn execution_spec_accepts_recipe_runner_resource_claim_kinds() {
    let raw = r#"apiVersion: strategy.platform/v1
kind: ExecutionSpec
execution:
  kind: job
resources:
  claims:
    - kind: document
      path: ~/.openclaw/agents/main/agent/IDENTITY.md
    - kind: modelProfile
      id: remote-openai
    - kind: authProfile
      id: openai:default"#;

    assert!(parse_execution_spec(raw).is_ok());
}
