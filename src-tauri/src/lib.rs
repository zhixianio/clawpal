use crate::agent_fallback::explain_operation_error;
use crate::bug_report::{capture_frontend_error, get_bug_report_stats, test_bug_report_connection};
use crate::cli_runner::{
    apply_queued_commands, discard_queued_commands, list_queued_commands, preview_queued_commands,
    queue_command, queued_commands_count, remote_apply_queued_commands,
    remote_discard_queued_commands, remote_list_queued_commands, remote_preview_queued_commands,
    remote_queue_command, remote_queued_commands_count, remote_remove_queued_command,
    remove_queued_command, CliCache, CommandQueue, RemoteCommandQueues,
};
use crate::commands::{
    analyze_sessions, apply_config_patch, backup_before_upgrade, chat_via_openclaw,
    check_openclaw_update, clear_all_sessions, clear_session_model_override,
    connect_docker_instance, connect_local_instance, connect_ssh_instance, create_agent,
    delete_agent, delete_backup, delete_cron_job, delete_local_instance_home, delete_model_profile,
    delete_recipe_workspace_source, delete_registered_instance, delete_sessions_by_ids,
    delete_ssh_host, deploy_watchdog, diagnose_doctor_assistant, diagnose_primary_via_rescue,
    diagnose_ssh, discover_local_instances, ensure_access_profile, execute_recipe,
    export_recipe_source, extract_model_profiles_from_config, fix_issues, get_app_preferences,
    get_bug_report_settings, get_cached_model_catalog, get_channels_config_snapshot,
    get_channels_runtime_snapshot, get_cron_config_snapshot, get_cron_runs,
    get_cron_runtime_snapshot, get_instance_config_snapshot, get_instance_runtime_snapshot,
    get_rescue_bot_status, get_session_model_override, get_ssh_transfer_stats, get_status_extra,
    get_status_light, get_system_status, get_watchdog_status, list_agents_overview, list_backups,
    list_bindings, list_channels_minimal, list_cron_jobs, list_discord_guild_channels,
    list_history, list_model_profiles, list_recipe_instances, list_recipe_runs,
    list_recipe_workspace_entries, list_recipes, list_recipes_from_source_text,
    list_registered_instances, list_session_files, list_ssh_config_hosts, list_ssh_hosts,
    local_openclaw_cli_available, local_openclaw_config_exists, log_app_event, manage_rescue_bot,
    migrate_legacy_instances, open_url, plan_recipe, plan_recipe_source, precheck_auth,
    precheck_instance, precheck_registry, precheck_transport, preview_rollback, preview_session,
    probe_ssh_connection_profile, push_model_profiles_to_local_openclaw,
    push_model_profiles_to_remote_openclaw, push_related_secrets_to_remote, read_app_log,
    read_error_log, read_gateway_error_log, read_gateway_log, read_helper_log, read_raw_config,
    read_recipe_workspace_source, record_install_experience, refresh_discord_guild_channels,
    refresh_model_catalog, remote_analyze_sessions, remote_apply_config_patch,
    remote_backup_before_upgrade, remote_chat_via_openclaw, remote_check_openclaw_update,
    remote_clear_all_sessions, remote_delete_backup, remote_delete_cron_job,
    remote_delete_model_profile, remote_delete_sessions_by_ids, remote_deploy_watchdog,
    remote_diagnose_doctor_assistant, remote_diagnose_primary_via_rescue,
    remote_extract_model_profiles_from_config, remote_fix_issues,
    remote_get_channels_config_snapshot, remote_get_channels_runtime_snapshot,
    remote_get_cron_config_snapshot, remote_get_cron_runs, remote_get_cron_runtime_snapshot,
    remote_get_instance_config_snapshot, remote_get_instance_runtime_snapshot,
    remote_get_rescue_bot_status, remote_get_ssh_connection_profile, remote_get_status_extra,
    remote_get_system_status, remote_get_watchdog_status, remote_list_agents_overview,
    remote_list_backups, remote_list_bindings, remote_list_channels_minimal, remote_list_cron_jobs,
    remote_list_discord_guild_channels, remote_list_history, remote_list_model_profiles,
    remote_list_session_files, remote_manage_rescue_bot, remote_preview_rollback,
    remote_preview_session, remote_read_app_log, remote_read_error_log,
    remote_read_gateway_error_log, remote_read_gateway_log, remote_read_helper_log,
    remote_read_raw_config, remote_refresh_model_catalog, remote_repair_doctor_assistant,
    remote_repair_primary_via_rescue, remote_resolve_api_keys, remote_restart_gateway,
    remote_restore_from_backup, remote_rollback, remote_run_doctor, remote_run_openclaw_upgrade,
    remote_setup_agent_identity, remote_start_watchdog, remote_stop_watchdog,
    remote_sync_profiles_to_local_auth, remote_test_model_profile, remote_trigger_cron_job,
    remote_uninstall_watchdog, remote_upsert_model_profile, remote_write_raw_config,
    repair_doctor_assistant, repair_primary_via_rescue, resolve_api_keys, resolve_provider_auth,
    restart_gateway, restore_from_backup, rollback, run_doctor_command, run_openclaw_upgrade,
    save_recipe_workspace_source, set_active_clawpal_data_dir, set_active_openclaw_home,
    set_agent_model, set_bug_report_settings, set_global_model, set_session_model_override,
    set_ssh_transfer_speed_ui_preference, setup_agent_identity, sftp_list_dir, sftp_read_file,
    sftp_remove_file, sftp_write_file, ssh_connect, ssh_connect_with_passphrase, ssh_disconnect,
    ssh_exec, ssh_status, start_watchdog, stop_watchdog, test_model_profile, trigger_cron_job,
    uninstall_watchdog, upsert_model_profile, upsert_ssh_host, validate_recipe_source_text,
};
use crate::install::commands::{
    install_create_session, install_decide_target, install_get_session, install_list_methods,
    install_orchestrator_next, install_run_step,
};
use crate::install::session_store::InstallSessionStore;
use crate::node_client::NodeClient;
use crate::ssh::SshConnectionPool;

pub mod access_discovery;
pub mod agent_fallback;
pub mod agent_identity;
pub mod bridge_client;
pub mod bug_report;
pub mod cli_runner;
pub mod commands;
pub mod config_io;
pub mod doctor;
pub mod execution_spec;
pub mod history;
pub mod install;
pub mod json_util;
pub mod logging;
pub mod models;
pub mod node_client;
pub mod openclaw_doc_resolver;
pub mod path_fix;
pub mod prompt_templates;
pub mod recipe;
pub mod recipe_adapter;
pub mod recipe_bundle;
pub mod recipe_executor;
pub mod recipe_planner;
pub mod recipe_runtime;
pub mod recipe_store;
pub mod recipe_workspace;
pub mod ssh;

#[cfg(test)]
mod execution_spec_tests;
#[cfg(test)]
mod recipe_adapter_tests;
#[cfg(test)]
mod recipe_bundle_tests;
#[cfg(test)]
mod recipe_executor_tests;
#[cfg(test)]
mod recipe_planner_tests;
#[cfg(test)]
mod recipe_store_tests;
#[cfg(test)]
mod recipe_workspace_tests;

pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_updater::Builder::new().build())
        .plugin(tauri_plugin_process::init())
        .manage(SshConnectionPool::new())
        .manage(NodeClient::new())
        .manage(CommandQueue::new())
        .manage(RemoteCommandQueues::new())
        .manage(CliCache::new())
        .manage(InstallSessionStore::new())
        .invoke_handler(tauri::generate_handler![
            install_create_session,
            install_decide_target,
            install_get_session,
            install_list_methods,
            install_orchestrator_next,
            install_run_step,
            set_active_openclaw_home,
            set_active_clawpal_data_dir,
            explain_operation_error,
            local_openclaw_config_exists,
            local_openclaw_cli_available,
            delete_local_instance_home,
            list_registered_instances,
            delete_registered_instance,
            connect_docker_instance,
            connect_local_instance,
            connect_ssh_instance,
            discover_local_instances,
            migrate_legacy_instances,
            ensure_access_profile,
            record_install_experience,
            get_system_status,
            get_status_light,
            get_status_extra,
            get_app_preferences,
            get_bug_report_settings,
            set_bug_report_settings,
            get_bug_report_stats,
            test_bug_report_connection,
            capture_frontend_error,
            set_session_model_override,
            get_session_model_override,
            clear_session_model_override,
            list_recipes,
            list_recipes_from_source_text,
            validate_recipe_source_text,
            list_recipe_workspace_entries,
            read_recipe_workspace_source,
            save_recipe_workspace_source,
            delete_recipe_workspace_source,
            export_recipe_source,
            execute_recipe,
            plan_recipe,
            plan_recipe_source,
            list_recipe_instances,
            list_recipe_runs,
            list_model_profiles,
            get_cached_model_catalog,
            refresh_model_catalog,
            upsert_model_profile,
            delete_model_profile,
            test_model_profile,
            resolve_provider_auth,
            list_agents_overview,
            create_agent,
            delete_agent,
            setup_agent_identity,
            list_session_files,
            clear_all_sessions,
            analyze_sessions,
            delete_sessions_by_ids,
            preview_session,
            check_openclaw_update,
            extract_model_profiles_from_config,
            apply_config_patch,
            list_history,
            preview_rollback,
            rollback,
            run_doctor_command,
            fix_issues,
            resolve_api_keys,
            read_raw_config,
            get_instance_config_snapshot,
            get_instance_runtime_snapshot,
            open_url,
            chat_via_openclaw,
            backup_before_upgrade,
            list_backups,
            restore_from_backup,
            delete_backup,
            list_channels_minimal,
            get_channels_config_snapshot,
            get_channels_runtime_snapshot,
            list_discord_guild_channels,
            refresh_discord_guild_channels,
            restart_gateway,
            diagnose_doctor_assistant,
            repair_doctor_assistant,
            get_rescue_bot_status,
            manage_rescue_bot,
            diagnose_primary_via_rescue,
            repair_primary_via_rescue,
            set_global_model,
            set_agent_model,
            set_ssh_transfer_speed_ui_preference,
            list_bindings,
            list_ssh_hosts,
            list_ssh_config_hosts,
            upsert_ssh_host,
            delete_ssh_host,
            ssh_connect,
            ssh_connect_with_passphrase,
            ssh_disconnect,
            ssh_status,
            diagnose_ssh,
            get_ssh_transfer_stats,
            probe_ssh_connection_profile,
            ssh_exec,
            sftp_read_file,
            sftp_write_file,
            sftp_list_dir,
            sftp_remove_file,
            remote_read_raw_config,
            remote_get_instance_config_snapshot,
            remote_get_instance_runtime_snapshot,
            remote_get_system_status,
            remote_get_ssh_connection_profile,
            remote_get_status_extra,
            remote_list_agents_overview,
            remote_get_channels_config_snapshot,
            remote_get_channels_runtime_snapshot,
            remote_list_channels_minimal,
            remote_list_bindings,
            remote_restart_gateway,
            remote_diagnose_doctor_assistant,
            remote_repair_doctor_assistant,
            remote_get_rescue_bot_status,
            remote_manage_rescue_bot,
            remote_diagnose_primary_via_rescue,
            remote_repair_primary_via_rescue,
            remote_apply_config_patch,
            remote_setup_agent_identity,
            remote_run_doctor,
            remote_fix_issues,
            remote_list_history,
            remote_preview_rollback,
            remote_rollback,
            remote_list_discord_guild_channels,
            remote_write_raw_config,
            remote_analyze_sessions,
            remote_delete_sessions_by_ids,
            remote_list_session_files,
            remote_clear_all_sessions,
            remote_preview_session,
            remote_list_model_profiles,
            remote_upsert_model_profile,
            remote_delete_model_profile,
            remote_resolve_api_keys,
            remote_test_model_profile,
            remote_extract_model_profiles_from_config,
            remote_sync_profiles_to_local_auth,
            push_model_profiles_to_local_openclaw,
            push_model_profiles_to_remote_openclaw,
            push_related_secrets_to_remote,
            remote_refresh_model_catalog,
            remote_chat_via_openclaw,
            remote_check_openclaw_update,
            run_openclaw_upgrade,
            remote_run_openclaw_upgrade,
            remote_backup_before_upgrade,
            remote_list_backups,
            remote_restore_from_backup,
            remote_delete_backup,
            list_cron_jobs,
            get_cron_config_snapshot,
            get_cron_runs,
            get_cron_runtime_snapshot,
            trigger_cron_job,
            delete_cron_job,
            remote_list_cron_jobs,
            remote_get_cron_config_snapshot,
            remote_get_cron_runs,
            remote_get_cron_runtime_snapshot,
            remote_trigger_cron_job,
            remote_delete_cron_job,
            get_watchdog_status,
            deploy_watchdog,
            start_watchdog,
            stop_watchdog,
            uninstall_watchdog,
            remote_get_watchdog_status,
            remote_deploy_watchdog,
            remote_start_watchdog,
            remote_stop_watchdog,
            remote_uninstall_watchdog,
            read_app_log,
            read_error_log,
            read_helper_log,
            read_gateway_log,
            read_gateway_error_log,
            log_app_event,
            remote_read_app_log,
            remote_read_error_log,
            remote_read_helper_log,
            remote_read_gateway_log,
            remote_read_gateway_error_log,
            queue_command,
            remove_queued_command,
            list_queued_commands,
            discard_queued_commands,
            queued_commands_count,
            preview_queued_commands,
            apply_queued_commands,
            remote_queue_command,
            remote_remove_queued_command,
            remote_list_queued_commands,
            remote_discard_queued_commands,
            remote_queued_commands_count,
            remote_preview_queued_commands,
            remote_apply_queued_commands,
            precheck_registry,
            precheck_instance,
            precheck_transport,
            precheck_auth,
        ])
        .setup(|_app| {
            crate::bug_report::install_panic_hook();
            let settings = crate::commands::preferences::load_bug_report_settings_from_paths(
                &crate::models::resolve_paths(),
            );
            if let Err(err) = crate::bug_report::queue::cleanup_old_logs() {
                eprintln!("[bug-report] cleanup failed: {err}");
            }
            if let Err(err) = crate::bug_report::queue::flush(&settings) {
                eprintln!("[bug-report] startup flush failed: {err}");
            }
            // Run PATH fix in background so it doesn't block window creation.
            // openclaw commands won't fire until user interaction, giving this
            // plenty of time to complete.
            std::thread::spawn(|| {
                crate::path_fix::ensure_tool_paths();
            });
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("failed to run app");
}
