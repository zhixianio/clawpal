use clawpal::doctor::{classify_engine_error, parse_engine, DoctorEngine};

#[test]
fn parse_engine_defaults_to_zeroclaw() {
    assert_eq!(parse_engine(None).unwrap(), DoctorEngine::ZeroClaw);
}

#[test]
fn parse_engine_accepts_zeroclaw() {
    assert_eq!(
        parse_engine(Some("zeroclaw".to_string())).unwrap(),
        DoctorEngine::ZeroClaw
    );
}

#[test]
fn parse_engine_rejects_openclaw_and_unknown() {
    let err_openclaw = parse_engine(Some("openclaw".to_string())).unwrap_err();
    assert!(err_openclaw.contains("Unsupported doctor engine"));
    let err = parse_engine(Some("unknown".to_string())).unwrap_err();
    assert!(err.contains("Unsupported doctor engine"));
}

#[test]
fn classify_model_not_found_error() {
    let msg = r#"Anthropic API error (404 Not Found): {"type":"error","error":{"type":"not_found_error","message":"model: claude-sonnet-4-5"}}"#;
    assert_eq!(classify_engine_error(msg), "MODEL_UNAVAILABLE");
}

#[test]
fn classify_missing_key_error() {
    assert_eq!(
        classify_engine_error("OpenRouter API key not set"),
        "CONFIG_MISSING"
    );
}
