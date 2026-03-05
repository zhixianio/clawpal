use clawpal::doctor_commands::collect_doctor_context;
use serde_json::Value;

#[tokio::test]
async fn collect_doctor_context_includes_doc_guidance_shape() {
    let raw = collect_doctor_context()
        .await
        .expect("collect_doctor_context should return JSON payload");
    let parsed: Value = serde_json::from_str(&raw).expect("doctor context must be valid JSON");

    let doc_guidance = parsed
        .get("docGuidance")
        .and_then(Value::as_object)
        .expect("docGuidance must exist as object");

    let status = doc_guidance
        .get("status")
        .and_then(Value::as_str)
        .expect("docGuidance.status must be string");
    assert!(
        status == "ok" || status == "unavailable",
        "unexpected docGuidance.status={status}"
    );

    assert!(
        doc_guidance
            .get("sourceStrategy")
            .and_then(Value::as_str)
            .is_some(),
        "docGuidance.sourceStrategy must be present"
    );
    assert!(
        doc_guidance
            .get("rootCauseHypotheses")
            .and_then(Value::as_array)
            .is_some(),
        "docGuidance.rootCauseHypotheses must be array"
    );
    assert!(
        doc_guidance
            .get("fixSteps")
            .and_then(Value::as_array)
            .is_some(),
        "docGuidance.fixSteps must be array"
    );
    assert!(
        doc_guidance
            .get("citations")
            .and_then(Value::as_array)
            .is_some(),
        "docGuidance.citations must be array"
    );
    assert!(
        doc_guidance
            .get("confidence")
            .and_then(Value::as_f64)
            .is_some(),
        "docGuidance.confidence must be numeric"
    );
    assert!(
        doc_guidance
            .get("resolverMeta")
            .and_then(Value::as_object)
            .is_some(),
        "docGuidance.resolverMeta must exist"
    );
}
