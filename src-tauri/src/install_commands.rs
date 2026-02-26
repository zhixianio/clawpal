use tauri::AppHandle;

use crate::doctor_commands::register_runtime_invoke;
use crate::doctor_runtime_bridge::emit_runtime_event;
use crate::runtime::types::{RuntimeAdapter, RuntimeDomain, RuntimeEvent, RuntimeSessionKey};
use crate::runtime::zeroclaw::install_adapter::ZeroclawInstallAdapter;

#[tauri::command]
pub async fn install_start_session(
    app: AppHandle,
    context: String,
    session_key: String,
    agent_id: String,
    instance_id: Option<String>,
) -> Result<(), String> {
    let instance = instance_id.unwrap_or_else(|| "local".to_string());
    let key = RuntimeSessionKey::new(
        "zeroclaw",
        RuntimeDomain::Install,
        instance,
        agent_id.clone(),
        session_key.clone(),
    );
    let adapter = ZeroclawInstallAdapter;
    match adapter.start(&key, &context) {
        Ok(events) => {
            for ev in events {
                register_runtime_invoke(&ev);
                emit_runtime_event(&app, ev);
            }
            Ok(())
        }
        Err(e) => {
            let code = e.code.as_str();
            emit_runtime_event(&app, RuntimeEvent::Error { error: e });
            Err(format!("zeroclaw install start failed [{code}]"))
        }
    }
}

#[tauri::command]
pub async fn install_send_message(
    app: AppHandle,
    message: String,
    session_key: String,
    agent_id: String,
    instance_id: Option<String>,
) -> Result<(), String> {
    let instance = instance_id.unwrap_or_else(|| "local".to_string());
    let key = RuntimeSessionKey::new(
        "zeroclaw",
        RuntimeDomain::Install,
        instance,
        agent_id.clone(),
        session_key.clone(),
    );
    let adapter = ZeroclawInstallAdapter;
    match adapter.send(&key, &message) {
        Ok(events) => {
            for ev in events {
                register_runtime_invoke(&ev);
                emit_runtime_event(&app, ev);
            }
            Ok(())
        }
        Err(e) => {
            let code = e.code.as_str();
            emit_runtime_event(&app, RuntimeEvent::Error { error: e });
            Err(format!("zeroclaw install send failed [{code}]"))
        }
    }
}
