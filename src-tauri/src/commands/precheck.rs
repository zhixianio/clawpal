use clawpal_core::precheck::{self, PrecheckIssue};

#[tauri::command]
pub async fn precheck_registry() -> Result<Vec<PrecheckIssue>, String> {
    let registry_path = clawpal_core::instance::registry_path();
    Ok(precheck::precheck_registry(&registry_path))
}

#[tauri::command]
pub async fn precheck_instance(instance_id: String) -> Result<Vec<PrecheckIssue>, String> {
    let registry = clawpal_core::instance::InstanceRegistry::load().map_err(|e| e.to_string())?;
    let instance = registry
        .get(&instance_id)
        .ok_or_else(|| format!("Instance not found: {instance_id}"))?;
    Ok(precheck::precheck_instance_state(instance))
}
