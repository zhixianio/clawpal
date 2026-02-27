use clap::{Parser, Subcommand};
use clawpal_core::connect::{connect_docker, connect_ssh};
use clawpal_core::health::{check_instance, HealthStatus};
use clawpal_core::install;
use clawpal_core::instance::{Instance, InstanceRegistry, InstanceType};
use clawpal_core::openclaw::OpenclawCli;
use clawpal_core::profile::{
    delete_profile, list_profiles, test_profile, upsert_profile, ModelProfile,
};
use clawpal_core::ssh::SshSession;
use serde_json::json;
use std::process::Command;

#[derive(Parser, Debug)]
#[command(name = "clawpal")]
#[command(about = "ClawPal CLI")]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand, Debug)]
enum Commands {
    Instance {
        #[command(subcommand)]
        command: InstanceCommands,
    },
    Install {
        #[command(subcommand)]
        command: InstallCommands,
    },
    Connect {
        #[command(subcommand)]
        command: ConnectCommands,
    },
    Health {
        #[command(subcommand)]
        command: HealthCommands,
    },
    Ssh {
        #[command(subcommand)]
        command: SshCommands,
    },
    Profile {
        #[command(subcommand)]
        command: ProfileCommands,
    },
    Doctor {
        #[command(subcommand)]
        command: DoctorCommands,
    },
}

#[derive(Subcommand, Debug)]
enum InstanceCommands {
    List,
    Remove { id: String },
}

#[derive(Subcommand, Debug)]
enum InstallCommands {
    Docker {
        #[arg(long)]
        home: Option<String>,
        #[arg(long)]
        label: Option<String>,
        #[arg(long = "dry-run", alias = "dry_run")]
        dry_run: bool,
        #[command(subcommand)]
        command: Option<InstallDockerSubcommands>,
    },
    Local,
}

#[derive(Subcommand, Debug)]
enum InstallDockerSubcommands {
    Pull,
    Configure,
    Up,
}

#[derive(Subcommand, Debug)]
enum ConnectCommands {
    Docker {
        #[arg(long)]
        home: String,
        #[arg(long)]
        label: Option<String>,
    },
    Ssh {
        #[arg(long)]
        host: String,
        #[arg(long, default_value_t = 22)]
        port: u16,
        #[arg(long)]
        user: Option<String>,
        #[arg(long)]
        id: Option<String>,
        #[arg(long)]
        label: Option<String>,
        #[arg(long = "key-path", alias = "key_path")]
        key_path: Option<String>,
    },
}

#[derive(Subcommand, Debug)]
enum HealthCommands {
    Check {
        id: Option<String>,
        #[arg(long)]
        all: bool,
    },
}

#[derive(Subcommand, Debug)]
enum SshCommands {
    Connect { host_id: String },
    Disconnect { host_id: String },
    List,
}

#[derive(Subcommand, Debug)]
enum ProfileCommands {
    List,
    Add {
        #[arg(long)]
        provider: String,
        #[arg(long)]
        model: String,
        #[arg(long)]
        name: Option<String>,
        #[arg(long = "api-key", alias = "api_key")]
        api_key: Option<String>,
    },
    Remove {
        id: String,
    },
    Test {
        id: String,
    },
}

#[derive(Subcommand, Debug)]
enum DoctorCommands {
    ProbeOpenclaw {
        #[arg(long)]
        instance: Option<String>,
    },
    FixOpenclawPath {
        #[arg(long)]
        instance: String,
    },
    ConfigDelete {
        path: String,
        #[arg(long)]
        instance: Option<String>,
    },
    ConfigRead {
        path: Option<String>,
        #[arg(long)]
        instance: Option<String>,
    },
    ConfigUpsert {
        path: String,
        value: String,
        #[arg(long)]
        instance: Option<String>,
    },
    SessionsRead {
        path: Option<String>,
        #[arg(long)]
        instance: Option<String>,
    },
    SessionsUpsert {
        path: String,
        value: String,
        #[arg(long)]
        instance: Option<String>,
    },
    SessionsDelete {
        path: String,
        #[arg(long)]
        instance: Option<String>,
    },
    File {
        #[command(subcommand)]
        command: DoctorFileCommands,
    },
}

#[derive(Subcommand, Debug)]
enum DoctorFileCommands {
    Read {
        #[arg(long)]
        domain: String,
        #[arg(long)]
        path: Option<String>,
        #[arg(long)]
        instance: Option<String>,
    },
    Write {
        #[arg(long)]
        domain: String,
        #[arg(long)]
        path: Option<String>,
        #[arg(long)]
        content: String,
        #[arg(long)]
        backup: bool,
        #[arg(long)]
        instance: Option<String>,
    },
}

fn main() {
    let cli = Cli::parse();
    let result = match cli.command {
        Commands::Instance { command } => run_instance_command(command),
        Commands::Health { command } => run_health_command(command),
        Commands::Install { command } => run_install_command(command),
        Commands::Connect { command } => run_connect_command(command),
        Commands::Profile { command } => run_profile_command(command),
        Commands::Ssh { command } => run_ssh_command(command),
        Commands::Doctor { command } => run_doctor_command(command),
    };

    match result {
        Ok(value) => println!("{value}"),
        Err(message) => {
            println!("{}", json!({ "error": message }));
            std::process::exit(1);
        }
    }
}

fn run_health_command(command: HealthCommands) -> Result<serde_json::Value, String> {
    match command {
        HealthCommands::Check { id, all } => {
            let registry = InstanceRegistry::load().map_err(|e| e.to_string())?;
            if all {
                let statuses: Result<Vec<_>, String> = registry
                    .list()
                    .into_iter()
                    .map(|instance| {
                        let status = check_instance(&instance).map_err(|e| e.to_string())?;
                        Ok(json!({
                            "id": instance.id,
                            "status": status,
                        }))
                    })
                    .collect();
                return statuses.map(serde_json::Value::Array);
            }

            let instance = if let Some(id) = id {
                if id == "local" {
                    default_local_instance()
                } else {
                    registry
                        .get(&id)
                        .cloned()
                        .ok_or_else(|| format!("instance '{id}' not found"))?
                }
            } else {
                default_local_instance()
            };
            let status: HealthStatus = check_instance(&instance).map_err(|e| e.to_string())?;
            Ok(json!({
                "id": instance.id,
                "status": status,
            }))
        }
    }
}

fn default_local_instance() -> Instance {
    Instance {
        id: "local".to_string(),
        instance_type: InstanceType::Local,
        label: "Local".to_string(),
        openclaw_home: None,
        clawpal_data_dir: None,
        ssh_host_config: None,
    }
}

fn run_profile_command(command: ProfileCommands) -> Result<serde_json::Value, String> {
    let openclaw = OpenclawCli::new();
    match command {
        ProfileCommands::List => {
            let profiles = list_profiles(&openclaw).map_err(|e| e.to_string())?;
            Ok(json!(profiles))
        }
        ProfileCommands::Add {
            provider,
            model,
            name,
            api_key,
        } => {
            let profile = ModelProfile {
                id: String::new(),
                name: name.unwrap_or_default(),
                provider,
                model,
                auth_ref: String::new(),
                api_key,
                base_url: None,
                description: None,
                enabled: true,
            };
            let saved = upsert_profile(&openclaw, profile).map_err(|e| e.to_string())?;
            Ok(json!(saved))
        }
        ProfileCommands::Remove { id } => {
            let removed = delete_profile(&openclaw, &id).map_err(|e| e.to_string())?;
            Ok(json!({ "removed": removed, "id": id }))
        }
        ProfileCommands::Test { id } => {
            let result = test_profile(&openclaw, &id).map_err(|e| e.to_string())?;
            Ok(json!(result))
        }
    }
}

fn run_install_command(command: InstallCommands) -> Result<serde_json::Value, String> {
    match command {
        InstallCommands::Docker {
            home,
            label,
            dry_run,
            command,
        } => {
            let options = install::DockerInstallOptions {
                home,
                label,
                dry_run,
            };
            let value = match command {
                Some(InstallDockerSubcommands::Pull) => install::docker::pull(&options)
                    .map_err(|e| e.to_string())
                    .map(|r| json!(r))?,
                Some(InstallDockerSubcommands::Configure) => install::docker::configure(&options)
                    .map_err(|e| e.to_string())
                    .map(|r| json!(r))?,
                Some(InstallDockerSubcommands::Up) => install::docker::up(&options)
                    .map_err(|e| e.to_string())
                    .map(|r| json!(r))?,
                None => install::install_docker(options)
                    .map_err(|e| e.to_string())
                    .map(|r| json!(r))?,
            };
            Ok(value)
        }
        InstallCommands::Local => install::install_local(install::LocalInstallOptions::default())
            .map_err(|e| e.to_string())
            .map(|r| json!(r)),
    }
}

fn run_connect_command(command: ConnectCommands) -> Result<serde_json::Value, String> {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|e| e.to_string())?;
    match command {
        ConnectCommands::Docker { home, label } => runtime
            .block_on(connect_docker(&home, label.as_deref()))
            .map_err(|e| e.to_string())
            .map(|instance| json!(instance)),
        ConnectCommands::Ssh {
            host,
            port,
            user,
            id,
            label,
            key_path,
        } => {
            let host_id = id.unwrap_or_else(|| format!("ssh:{host}"));
            let config = clawpal_core::instance::SshHostConfig {
                id: host_id,
                label: label.unwrap_or_else(|| host.clone()),
                host,
                port,
                username: user.unwrap_or_else(|| "root".to_string()),
                auth_method: "key".to_string(),
                key_path,
                password: None,
            };
            runtime
                .block_on(connect_ssh(config))
                .map_err(|e| e.to_string())
                .map(|instance| json!(instance))
        }
    }
}

fn run_instance_command(command: InstanceCommands) -> Result<serde_json::Value, String> {
    match command {
        InstanceCommands::List => {
            let registry = InstanceRegistry::load().map_err(|e| e.to_string())?;
            Ok(json!(registry.list()))
        }
        InstanceCommands::Remove { id } => {
            let mut registry = InstanceRegistry::load().map_err(|e| e.to_string())?;
            let removed = registry.remove(&id).is_some();
            registry.save().map_err(|e| e.to_string())?;
            Ok(json!({ "removed": removed, "id": id }))
        }
    }
}

fn run_ssh_command(command: SshCommands) -> Result<serde_json::Value, String> {
    match command {
        SshCommands::List => {
            let hosts = clawpal_core::ssh::registry::list_ssh_hosts().map_err(|e| e.to_string())?;
            Ok(json!(hosts))
        }
        SshCommands::Connect { host_id } => {
            let registry = InstanceRegistry::load().map_err(|e| e.to_string())?;
            let instance = registry
                .get(&host_id)
                .cloned()
                .ok_or_else(|| format!("instance '{host_id}' not found"))?;
            let host = instance
                .ssh_host_config
                .ok_or_else(|| format!("instance '{host_id}' is not an SSH instance"))?;
            let runtime = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .map_err(|e| e.to_string())?;
            let result = runtime.block_on(async {
                let session = SshSession::connect(&host)
                    .await
                    .map_err(|e| e.to_string())?;
                let output = session
                    .exec("echo connected")
                    .await
                    .map_err(|e| e.to_string())?;
                Ok::<_, String>(output)
            })?;
            Ok(json!({
                "hostId": host_id,
                "connected": result.exit_code == 0,
                "stdout": result.stdout,
                "stderr": result.stderr,
                "exitCode": result.exit_code,
            }))
        }
        SshCommands::Disconnect { host_id } => Ok(json!({
            "hostId": host_id,
            "disconnected": true,
            "note": "stateless ssh mode has no persistent session",
        })),
    }
}

#[derive(Debug)]
enum DoctorTarget {
    Local,
    Remote { id: String, host: clawpal_core::instance::SshHostConfig },
}

fn resolve_doctor_target(instance: Option<String>) -> Result<DoctorTarget, String> {
    let Some(instance_id) = instance else {
        return Ok(DoctorTarget::Local);
    };
    if instance_id == "local" {
        return Ok(DoctorTarget::Local);
    }
    let registry = InstanceRegistry::load().map_err(|e| e.to_string())?;
    let instance = registry
        .get(&instance_id)
        .cloned()
        .ok_or_else(|| format!("instance '{instance_id}' not found"))?;
    let host = instance
        .ssh_host_config
        .ok_or_else(|| format!("instance '{instance_id}' is not an SSH instance"))?;
    Ok(DoctorTarget::Remote {
        id: instance_id,
        host,
    })
}

fn run_doctor_command(command: DoctorCommands) -> Result<serde_json::Value, String> {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|e| e.to_string())?;
    match command {
        DoctorCommands::ProbeOpenclaw { instance } => {
            let target = resolve_doctor_target(instance)?;
            runtime
                .block_on(async { doctor_probe_openclaw(target).await })
                .map(|v| json!(v))
        }
        DoctorCommands::FixOpenclawPath { instance } => {
            let target = resolve_doctor_target(Some(instance))?;
            runtime
                .block_on(async { doctor_fix_openclaw_path(target).await })
                .map(|v| json!(v))
        }
        DoctorCommands::ConfigDelete { path, instance } => {
            let target = resolve_doctor_target(instance)?;
            runtime
                .block_on(async { doctor_config_delete(target, &path).await })
                .map(|v| json!(v))
        }
        DoctorCommands::ConfigRead { path, instance } => {
            let target = resolve_doctor_target(instance)?;
            runtime
                .block_on(async { doctor_config_read(target, path.as_deref()).await })
                .map(|v| json!(v))
        }
        DoctorCommands::ConfigUpsert {
            path,
            value,
            instance,
        } => {
            let target = resolve_doctor_target(instance)?;
            runtime
                .block_on(async { doctor_config_upsert(target, &path, &value).await })
                .map(|v| json!(v))
        }
        DoctorCommands::SessionsRead { path, instance } => {
            let target = resolve_doctor_target(instance)?;
            runtime
                .block_on(async { doctor_sessions_read(target, path.as_deref()).await })
                .map(|v| json!(v))
        }
        DoctorCommands::SessionsUpsert {
            path,
            value,
            instance,
        } => {
            let target = resolve_doctor_target(instance)?;
            runtime
                .block_on(async { doctor_sessions_upsert(target, &path, &value).await })
                .map(|v| json!(v))
        }
        DoctorCommands::SessionsDelete { path, instance } => {
            let target = resolve_doctor_target(instance)?;
            runtime
                .block_on(async { doctor_sessions_delete(target, &path).await })
                .map(|v| json!(v))
        }
        DoctorCommands::File { command } => match command {
            DoctorFileCommands::Read {
                domain,
                path,
                instance,
            } => {
                let target = resolve_doctor_target(instance)?;
                runtime
                    .block_on(async { doctor_file_read(target, &domain, path.as_deref()).await })
                    .map(|v| json!(v))
            }
            DoctorFileCommands::Write {
                domain,
                path,
                content,
                backup,
                instance,
            } => {
                let target = resolve_doctor_target(instance)?;
                runtime
                    .block_on(async {
                        doctor_file_write(target, &domain, path.as_deref(), &content, backup).await
                    })
                    .map(|v| json!(v))
            }
        }
    }
}

async fn doctor_probe_openclaw(target: DoctorTarget) -> Result<serde_json::Value, String> {
    match target {
        DoctorTarget::Local => {
            let version_out = Command::new(clawpal_core::openclaw::resolve_openclaw_bin())
                .arg("--version")
                .output()
                .map_err(|e| format!("failed to run openclaw --version: {e}"))?;
            let version = String::from_utf8_lossy(&version_out.stdout).trim().to_string();
            let path_out = Command::new("bash")
                .args(["-lc", "command -v openclaw 2>/dev/null || true"])
                .output()
                .map_err(|e| format!("failed to probe openclaw path: {e}"))?;
            let openclaw_path = String::from_utf8_lossy(&path_out.stdout).trim().to_string();
            Ok(json!({
                "target": "local",
                "remote": false,
                "version": if version.is_empty() { serde_json::Value::Null } else { json!(version) },
                "openclawPath": if openclaw_path.is_empty() { serde_json::Value::Null } else { json!(openclaw_path) },
                "path": std::env::var("PATH").unwrap_or_default(),
            }))
        }
        DoctorTarget::Remote { id, host } => {
            let session = SshSession::connect(&host).await.map_err(|e| e.to_string())?;
            let path = session
                .exec("sh -lc 'command -v openclaw 2>/dev/null || true'")
                .await
                .map_err(|e| e.to_string())?
                .stdout
                .trim()
                .to_string();
            let version = session
                .exec("sh -lc 'openclaw --version 2>/dev/null || true'")
                .await
                .map_err(|e| e.to_string())?
                .stdout
                .trim()
                .to_string();
            let env_path = session
                .exec("sh -lc 'printf %s \"$PATH\"'")
                .await
                .map_err(|e| e.to_string())?
                .stdout;
            Ok(json!({
                "target": id,
                "remote": true,
                "version": if version.is_empty() { serde_json::Value::Null } else { json!(version) },
                "openclawPath": if path.is_empty() { serde_json::Value::Null } else { json!(path) },
                "path": env_path.trim(),
            }))
        }
    }
}

async fn doctor_fix_openclaw_path(target: DoctorTarget) -> Result<serde_json::Value, String> {
    match target {
        DoctorTarget::Local => Err("doctor fix-openclaw-path currently supports remote target only".to_string()),
        DoctorTarget::Remote { id, host } => {
            let session = SshSession::connect(&host).await.map_err(|e| e.to_string())?;
            let result = session
                .exec("sh -lc 'if command -v openclaw >/dev/null 2>&1; then command -v openclaw; elif [ -x /usr/local/bin/openclaw ]; then ln -sf /usr/local/bin/openclaw ~/.local/bin/openclaw 2>/dev/null || true; command -v openclaw || true; else echo missing; fi'")
                .await
                .map_err(|e| e.to_string())?;
            Ok(json!({
                "target": id,
                "remote": true,
                "fixed": result.exit_code == 0,
                "stdout": result.stdout,
                "stderr": result.stderr,
                "exitCode": result.exit_code,
            }))
        }
    }
}

async fn doctor_config_delete(target: DoctorTarget, dotted_path: &str) -> Result<serde_json::Value, String> {
    if dotted_path.trim().is_empty() {
        return Err("doctor config-delete requires <json.path>".to_string());
    }
    match target {
        DoctorTarget::Local => {
            let openclaw_dir =
                std::env::var("OPENCLAW_HOME").unwrap_or_else(|_| format!("{}/.openclaw", dirs::home_dir().map(|p| p.display().to_string()).unwrap_or_default()));
            let config_path = std::path::PathBuf::from(openclaw_dir).join("openclaw.json");
            let raw = std::fs::read_to_string(&config_path)
                .map_err(|e| format!("failed to read local config: {e}"))?;
            let mut json_doc: serde_json::Value =
                serde_json::from_str(&raw).map_err(|e| format!("invalid local config json: {e}"))?;
            let deleted = delete_json_path(&mut json_doc, dotted_path);
            if deleted {
                let rendered = serde_json::to_string_pretty(&json_doc)
                    .map_err(|e| format!("serialize config: {e}"))?;
                std::fs::write(&config_path, rendered)
                    .map_err(|e| format!("failed to write local config: {e}"))?;
            }
            Ok(json!({
                "target": "local",
                "remote": false,
                "configPath": config_path.to_string_lossy(),
                "path": dotted_path,
                "deleted": deleted,
            }))
        }
        DoctorTarget::Remote { id, host } => {
            let session = SshSession::connect(&host).await.map_err(|e| e.to_string())?;
            let config_path = session
                .exec("sh -lc 'echo \"${OPENCLAW_STATE_DIR:-${OPENCLAW_HOME:-$HOME/.openclaw}}/openclaw.json\"'")
                .await
                .map_err(|e| e.to_string())?
                .stdout
                .trim()
                .to_string();
            let raw = session
                .sftp_read(&config_path)
                .await
                .map_err(|e| e.to_string())?;
            let mut json_doc: serde_json::Value =
                serde_json::from_slice(&raw).map_err(|e| format!("invalid remote config json: {e}"))?;
            let deleted = delete_json_path(&mut json_doc, dotted_path);
            if deleted {
                let rendered = serde_json::to_string_pretty(&json_doc)
                    .map_err(|e| format!("serialize config: {e}"))?;
                session
                    .sftp_write(&config_path, rendered.as_bytes())
                    .await
                    .map_err(|e| e.to_string())?;
            }
            Ok(json!({
                "target": id,
                "remote": true,
                "configPath": config_path,
                "path": dotted_path,
                "deleted": deleted,
            }))
        }
    }
}

async fn doctor_config_read(
    target: DoctorTarget,
    dotted_path: Option<&str>,
) -> Result<serde_json::Value, String> {
    match target {
        DoctorTarget::Local => {
            let openclaw_dir = std::env::var("OPENCLAW_HOME")
                .unwrap_or_else(|_| format!("{}/.openclaw", dirs::home_dir().map(|p| p.display().to_string()).unwrap_or_default()));
            let config_path = std::path::PathBuf::from(openclaw_dir).join("openclaw.json");
            let raw = std::fs::read_to_string(&config_path)
                .map_err(|e| format!("failed to read local config: {e}"))?;
            let json_doc: serde_json::Value =
                serde_json::from_str(&raw).map_err(|e| format!("invalid local config json: {e}"))?;
            let value = dotted_path
                .and_then(|p| json_path_get(&json_doc, p).cloned())
                .unwrap_or(json_doc.clone());
            Ok(json!({
                "target": "local",
                "remote": false,
                "configPath": config_path.to_string_lossy(),
                "path": dotted_path.unwrap_or(""),
                "value": value,
            }))
        }
        DoctorTarget::Remote { id, host } => {
            let session = SshSession::connect(&host).await.map_err(|e| e.to_string())?;
            let config_path = session
                .exec("sh -lc 'echo \"${OPENCLAW_STATE_DIR:-${OPENCLAW_HOME:-$HOME/.openclaw}}/openclaw.json\"'")
                .await
                .map_err(|e| e.to_string())?
                .stdout
                .trim()
                .to_string();
            let raw = session
                .sftp_read(&config_path)
                .await
                .map_err(|e| e.to_string())?;
            let json_doc: serde_json::Value =
                serde_json::from_slice(&raw).map_err(|e| format!("invalid remote config json: {e}"))?;
            let value = dotted_path
                .and_then(|p| json_path_get(&json_doc, p).cloned())
                .unwrap_or(json_doc.clone());
            Ok(json!({
                "target": id,
                "remote": true,
                "configPath": config_path,
                "path": dotted_path.unwrap_or(""),
                "value": value,
            }))
        }
    }
}

async fn doctor_config_upsert(
    target: DoctorTarget,
    dotted_path: &str,
    value_json: &str,
) -> Result<serde_json::Value, String> {
    if dotted_path.trim().is_empty() {
        return Err("doctor config-upsert requires <json.path>".to_string());
    }
    let parsed: serde_json::Value = serde_json::from_str(value_json)
        .map_err(|e| format!("doctor config-upsert requires valid JSON value: {e}"))?;
    match target {
        DoctorTarget::Local => {
            let openclaw_dir = std::env::var("OPENCLAW_HOME")
                .unwrap_or_else(|_| format!("{}/.openclaw", dirs::home_dir().map(|p| p.display().to_string()).unwrap_or_default()));
            let config_path = std::path::PathBuf::from(openclaw_dir).join("openclaw.json");
            let raw = std::fs::read_to_string(&config_path)
                .map_err(|e| format!("failed to read local config: {e}"))?;
            let mut json_doc: serde_json::Value =
                serde_json::from_str(&raw).map_err(|e| format!("invalid local config json: {e}"))?;
            upsert_json_path(&mut json_doc, dotted_path, parsed)?;
            let rendered = serde_json::to_string_pretty(&json_doc)
                .map_err(|e| format!("serialize config: {e}"))?;
            std::fs::write(&config_path, rendered)
                .map_err(|e| format!("failed to write local config: {e}"))?;
            Ok(json!({
                "target": "local",
                "remote": false,
                "configPath": config_path.to_string_lossy(),
                "path": dotted_path,
                "upserted": true,
            }))
        }
        DoctorTarget::Remote { id, host } => {
            let session = SshSession::connect(&host).await.map_err(|e| e.to_string())?;
            let config_path = session
                .exec("sh -lc 'echo \"${OPENCLAW_STATE_DIR:-${OPENCLAW_HOME:-$HOME/.openclaw}}/openclaw.json\"'")
                .await
                .map_err(|e| e.to_string())?
                .stdout
                .trim()
                .to_string();
            let raw = session
                .sftp_read(&config_path)
                .await
                .map_err(|e| e.to_string())?;
            let mut json_doc: serde_json::Value =
                serde_json::from_slice(&raw).map_err(|e| format!("invalid remote config json: {e}"))?;
            upsert_json_path(&mut json_doc, dotted_path, parsed)?;
            let rendered = serde_json::to_string_pretty(&json_doc)
                .map_err(|e| format!("serialize config: {e}"))?;
            session
                .sftp_write(&config_path, rendered.as_bytes())
                .await
                .map_err(|e| e.to_string())?;
            Ok(json!({
                "target": id,
                "remote": true,
                "configPath": config_path,
                "path": dotted_path,
                "upserted": true,
            }))
        }
    }
}

fn resolve_local_sessions_path() -> std::path::PathBuf {
    let openclaw_dir = std::env::var("OPENCLAW_HOME").unwrap_or_else(|_| {
        format!(
            "{}/.openclaw",
            dirs::home_dir()
                .map(|p| p.display().to_string())
                .unwrap_or_default()
        )
    });
    let agents_dir = std::path::PathBuf::from(&openclaw_dir).join("agents");
    if let Ok(agent_entries) = std::fs::read_dir(&agents_dir) {
        for agent_entry in agent_entries.flatten() {
            let candidate = agent_entry.path().join("sessions").join("sessions.json");
            if candidate.exists() {
                return candidate;
            }
        }
    }
    std::path::PathBuf::from(openclaw_dir)
        .join("agents")
        .join("test")
        .join("sessions")
        .join("sessions.json")
}

async fn resolve_remote_sessions_path(session: &SshSession) -> Result<String, String> {
    let out = session
        .exec("sh -lc 'root=\"${OPENCLAW_STATE_DIR:-${OPENCLAW_HOME:-$HOME/.openclaw}}\"; first=\"$(find \"$root/agents\" -type f -path \"*/sessions/sessions.json\" 2>/dev/null | head -n 1)\"; if [ -n \"$first\" ]; then printf \"%s\" \"$first\"; else printf \"%s\" \"$root/agents/test/sessions/sessions.json\"; fi'")
        .await
        .map_err(|e| e.to_string())?;
    Ok(out.stdout.trim().to_string())
}

async fn doctor_sessions_read(
    target: DoctorTarget,
    dotted_path: Option<&str>,
) -> Result<serde_json::Value, String> {
    match target {
        DoctorTarget::Local => {
            let sessions_path = resolve_local_sessions_path();
            let raw = std::fs::read_to_string(&sessions_path)
                .map_err(|e| format!("failed to read local sessions: {e}"))?;
            let json_doc: serde_json::Value = serde_json::from_str(&raw)
                .map_err(|e| format!("invalid local sessions json: {e}"))?;
            let value = dotted_path
                .and_then(|p| json_path_get(&json_doc, p).cloned())
                .unwrap_or(json_doc.clone());
            Ok(json!({
                "target": "local",
                "remote": false,
                "sessionsPath": sessions_path.to_string_lossy(),
                "path": dotted_path.unwrap_or(""),
                "value": value,
            }))
        }
        DoctorTarget::Remote { id, host } => {
            let session = SshSession::connect(&host).await.map_err(|e| e.to_string())?;
            let sessions_path = resolve_remote_sessions_path(&session).await?;
            let raw = session
                .sftp_read(&sessions_path)
                .await
                .map_err(|e| e.to_string())?;
            let json_doc: serde_json::Value = serde_json::from_slice(&raw)
                .map_err(|e| format!("invalid remote sessions json: {e}"))?;
            let value = dotted_path
                .and_then(|p| json_path_get(&json_doc, p).cloned())
                .unwrap_or(json_doc.clone());
            Ok(json!({
                "target": id,
                "remote": true,
                "sessionsPath": sessions_path,
                "path": dotted_path.unwrap_or(""),
                "value": value,
            }))
        }
    }
}

async fn doctor_sessions_upsert(
    target: DoctorTarget,
    dotted_path: &str,
    value_json: &str,
) -> Result<serde_json::Value, String> {
    if dotted_path.trim().is_empty() {
        return Err("doctor sessions-upsert requires <json.path>".to_string());
    }
    let parsed: serde_json::Value = serde_json::from_str(value_json)
        .map_err(|e| format!("doctor sessions-upsert requires valid JSON value: {e}"))?;
    match target {
        DoctorTarget::Local => {
            let sessions_path = resolve_local_sessions_path();
            let raw = std::fs::read_to_string(&sessions_path)
                .map_err(|e| format!("failed to read local sessions: {e}"))?;
            let mut json_doc: serde_json::Value = serde_json::from_str(&raw)
                .map_err(|e| format!("invalid local sessions json: {e}"))?;
            upsert_json_path(&mut json_doc, dotted_path, parsed)?;
            let rendered = serde_json::to_string_pretty(&json_doc)
                .map_err(|e| format!("serialize sessions: {e}"))?;
            std::fs::write(&sessions_path, rendered)
                .map_err(|e| format!("failed to write local sessions: {e}"))?;
            Ok(json!({
                "target": "local",
                "remote": false,
                "sessionsPath": sessions_path.to_string_lossy(),
                "path": dotted_path,
                "upserted": true,
            }))
        }
        DoctorTarget::Remote { id, host } => {
            let session = SshSession::connect(&host).await.map_err(|e| e.to_string())?;
            let sessions_path = resolve_remote_sessions_path(&session).await?;
            let raw = session
                .sftp_read(&sessions_path)
                .await
                .map_err(|e| e.to_string())?;
            let mut json_doc: serde_json::Value = serde_json::from_slice(&raw)
                .map_err(|e| format!("invalid remote sessions json: {e}"))?;
            upsert_json_path(&mut json_doc, dotted_path, parsed)?;
            let rendered = serde_json::to_string_pretty(&json_doc)
                .map_err(|e| format!("serialize sessions: {e}"))?;
            session
                .sftp_write(&sessions_path, rendered.as_bytes())
                .await
                .map_err(|e| e.to_string())?;
            Ok(json!({
                "target": id,
                "remote": true,
                "sessionsPath": sessions_path,
                "path": dotted_path,
                "upserted": true,
            }))
        }
    }
}

async fn doctor_sessions_delete(
    target: DoctorTarget,
    dotted_path: &str,
) -> Result<serde_json::Value, String> {
    if dotted_path.trim().is_empty() {
        return Err("doctor sessions-delete requires <json.path>".to_string());
    }
    match target {
        DoctorTarget::Local => {
            let sessions_path = resolve_local_sessions_path();
            let raw = std::fs::read_to_string(&sessions_path)
                .map_err(|e| format!("failed to read local sessions: {e}"))?;
            let mut json_doc: serde_json::Value = serde_json::from_str(&raw)
                .map_err(|e| format!("invalid local sessions json: {e}"))?;
            let deleted = delete_json_path(&mut json_doc, dotted_path);
            if deleted {
                let rendered = serde_json::to_string_pretty(&json_doc)
                    .map_err(|e| format!("serialize sessions: {e}"))?;
                std::fs::write(&sessions_path, rendered)
                    .map_err(|e| format!("failed to write local sessions: {e}"))?;
            }
            Ok(json!({
                "target": "local",
                "remote": false,
                "sessionsPath": sessions_path.to_string_lossy(),
                "path": dotted_path,
                "deleted": deleted,
            }))
        }
        DoctorTarget::Remote { id, host } => {
            let session = SshSession::connect(&host).await.map_err(|e| e.to_string())?;
            let sessions_path = resolve_remote_sessions_path(&session).await?;
            let raw = session
                .sftp_read(&sessions_path)
                .await
                .map_err(|e| e.to_string())?;
            let mut json_doc: serde_json::Value = serde_json::from_slice(&raw)
                .map_err(|e| format!("invalid remote sessions json: {e}"))?;
            let deleted = delete_json_path(&mut json_doc, dotted_path);
            if deleted {
                let rendered = serde_json::to_string_pretty(&json_doc)
                    .map_err(|e| format!("serialize sessions: {e}"))?;
                session
                    .sftp_write(&sessions_path, rendered.as_bytes())
                    .await
                    .map_err(|e| e.to_string())?;
            }
            Ok(json!({
                "target": id,
                "remote": true,
                "sessionsPath": sessions_path,
                "path": dotted_path,
                "deleted": deleted,
            }))
        }
    }
}

fn validate_doctor_relative_path(path: &str) -> Result<(), String> {
    let trimmed = path.trim();
    if trimmed.is_empty() {
        return Err("doctor file path cannot be empty".to_string());
    }
    if trimmed.starts_with('/') || trimmed.starts_with('~') {
        return Err("doctor file path must be relative to domain root".to_string());
    }
    if trimmed
        .split('/')
        .any(|seg| seg == ".." || seg.contains('\0') || seg.is_empty() && trimmed.contains("//"))
    {
        return Err("doctor file path contains forbidden traversal segment".to_string());
    }
    Ok(())
}

fn doctor_domain_local_root(domain: &str) -> Result<std::path::PathBuf, String> {
    let openclaw_dir = std::env::var("OPENCLAW_HOME").unwrap_or_else(|_| {
        format!(
            "{}/.openclaw",
            dirs::home_dir()
                .map(|p| p.display().to_string())
                .unwrap_or_default()
        )
    });
    let root = std::path::PathBuf::from(openclaw_dir);
    match domain {
        "config" => Ok(root),
        "sessions" => Ok(root.join("agents")),
        "logs" => Ok(root.join("logs")),
        "state" => Ok(root),
        _ => Err(format!("unsupported doctor file domain: {domain}")),
    }
}

fn doctor_domain_default_relpath(domain: &str) -> Option<&'static str> {
    match domain {
        "config" => Some("openclaw.json"),
        "logs" => Some("gateway.err.log"),
        _ => None,
    }
}

fn relpath_from_local_abs(root: &std::path::Path, abs: &std::path::Path) -> Option<String> {
    abs.strip_prefix(root)
        .ok()
        .map(|p| p.to_string_lossy().to_string())
}

fn relpath_from_remote_abs(root: &str, abs: &str) -> Option<String> {
    let root = root.trim_end_matches('/');
    let prefix = format!("{root}/");
    abs.strip_prefix(&prefix).map(str::to_string)
}

async fn doctor_domain_remote_root(session: &SshSession, domain: &str) -> Result<String, String> {
    let out = session
        .exec("sh -lc 'printf %s \"${OPENCLAW_STATE_DIR:-${OPENCLAW_HOME:-$HOME/.openclaw}}\"'")
        .await
        .map_err(|e| e.to_string())?;
    let base = out.stdout.trim().to_string();
    match domain {
        "config" => Ok(base),
        "sessions" => Ok(format!("{base}/agents")),
        "logs" => Ok(format!("{base}/logs")),
        "state" => Ok(base),
        _ => Err(format!("unsupported doctor file domain: {domain}")),
    }
}

async fn doctor_file_read(
    target: DoctorTarget,
    domain: &str,
    path: Option<&str>,
) -> Result<serde_json::Value, String> {
    match target {
        DoctorTarget::Local => {
            let root = doctor_domain_local_root(domain)?;
            let rel = match path {
                Some(p) => p.to_string(),
                None => match domain {
                    "sessions" => {
                        let abs = resolve_local_sessions_path();
                        relpath_from_local_abs(&root, &abs).ok_or_else(|| {
                            format!(
                                "failed to resolve sessions path under domain root: {}",
                                root.display()
                            )
                        })?
                    }
                    _ => doctor_domain_default_relpath(domain)
                        .ok_or_else(|| "doctor file read requires --path for this domain".to_string())?
                        .to_string(),
                },
            };
            validate_doctor_relative_path(&rel)?;
            let full = root.join(&rel);
            let content = std::fs::read_to_string(&full)
                .map_err(|e| format!("failed to read file: {e}"))?;
            Ok(json!({
                "target": "local",
                "remote": false,
                "domain": domain,
                "root": root.to_string_lossy(),
                "path": rel,
                "fullPath": full.to_string_lossy(),
                "content": content,
            }))
        }
        DoctorTarget::Remote { id, host } => {
            let session = SshSession::connect(&host).await.map_err(|e| e.to_string())?;
            let root = doctor_domain_remote_root(&session, domain).await?;
            let rel = match path {
                Some(p) => p.to_string(),
                None => match domain {
                    "sessions" => {
                        let abs = resolve_remote_sessions_path(&session).await?;
                        relpath_from_remote_abs(&root, &abs).ok_or_else(|| {
                            format!("failed to resolve sessions path under domain root: {root}")
                        })?
                    }
                    _ => doctor_domain_default_relpath(domain)
                        .ok_or_else(|| "doctor file read requires --path for this domain".to_string())?
                        .to_string(),
                },
            };
            validate_doctor_relative_path(&rel)?;
            let full = format!("{}/{}", root.trim_end_matches('/'), rel);
            let content = session.sftp_read(&full).await.map_err(|e| e.to_string())?;
            Ok(json!({
                "target": id,
                "remote": true,
                "domain": domain,
                "root": root,
                "path": rel,
                "fullPath": full,
                "content": String::from_utf8_lossy(&content).to_string(),
            }))
        }
    }
}

async fn doctor_file_write(
    target: DoctorTarget,
    domain: &str,
    path: Option<&str>,
    content: &str,
    backup: bool,
) -> Result<serde_json::Value, String> {
    match target {
        DoctorTarget::Local => {
            let root = doctor_domain_local_root(domain)?;
            let rel = match path {
                Some(p) => p.to_string(),
                None => match domain {
                    "sessions" => {
                        let abs = resolve_local_sessions_path();
                        relpath_from_local_abs(&root, &abs).ok_or_else(|| {
                            format!(
                                "failed to resolve sessions path under domain root: {}",
                                root.display()
                            )
                        })?
                    }
                    _ => doctor_domain_default_relpath(domain)
                        .ok_or_else(|| "doctor file write requires --path for this domain".to_string())?
                        .to_string(),
                },
            };
            validate_doctor_relative_path(&rel)?;
            let full = root.join(&rel);
            if let Some(parent) = full.parent() {
                std::fs::create_dir_all(parent)
                    .map_err(|e| format!("failed to create parent dir: {e}"))?;
            }
            if backup && full.exists() {
                let ts = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map(|d| d.as_secs())
                    .unwrap_or(0);
                let backup_path = full.with_extension(format!(
                    "{}bak.{ts}",
                    full.extension()
                        .map(|ext| format!("{}.", ext.to_string_lossy()))
                        .unwrap_or_default(),
                ));
                std::fs::copy(&full, backup_path)
                    .map_err(|e| format!("failed to create backup file: {e}"))?;
            }
            std::fs::write(&full, content).map_err(|e| format!("failed to write file: {e}"))?;
            let verify = std::fs::read_to_string(&full)
                .map_err(|e| format!("failed to verify written file: {e}"))?;
            if verify != content {
                return Err(
                    "doctor file write verification failed: local content mismatch".to_string(),
                );
            }
            Ok(json!({
                "target": "local",
                "remote": false,
                "domain": domain,
                "root": root.to_string_lossy(),
                "path": rel,
                "fullPath": full.to_string_lossy(),
                "written": true,
                "backup": backup,
            }))
        }
        DoctorTarget::Remote { id, host } => {
            let session = SshSession::connect(&host).await.map_err(|e| e.to_string())?;
            let root = doctor_domain_remote_root(&session, domain).await?;
            let rel = match path {
                Some(p) => p.to_string(),
                None => match domain {
                    "sessions" => {
                        let abs = resolve_remote_sessions_path(&session).await?;
                        relpath_from_remote_abs(&root, &abs).ok_or_else(|| {
                            format!("failed to resolve sessions path under domain root: {root}")
                        })?
                    }
                    _ => doctor_domain_default_relpath(domain)
                        .ok_or_else(|| "doctor file write requires --path for this domain".to_string())?
                        .to_string(),
                },
            };
            validate_doctor_relative_path(&rel)?;
            let full = format!("{}/{}", root.trim_end_matches('/'), rel);
            let mkdir_cmd = format!("sh -lc 'mkdir -p \"$(dirname {})\"'", sh_quote(&full));
            let mkdir_out = session.exec(&mkdir_cmd).await.map_err(|e| e.to_string())?;
            if mkdir_out.exit_code != 0 {
                return Err(format!(
                    "doctor file write mkdir failed (exit {}): {}",
                    mkdir_out.exit_code, mkdir_out.stderr
                ));
            }
            if backup {
                let backup_cmd = format!(
                    "sh -lc 'if [ -f {f} ]; then cp {f} {f}.bak.$(date +%Y%m%d%H%M%S); fi'",
                    f = sh_quote(&full)
                );
                let backup_out = session.exec(&backup_cmd).await.map_err(|e| e.to_string())?;
                if backup_out.exit_code != 0 {
                    return Err(format!(
                        "doctor file write backup failed (exit {}): {}",
                        backup_out.exit_code, backup_out.stderr
                    ));
                }
            }
            session
                .sftp_write(&full, content.as_bytes())
                .await
                .map_err(|e| e.to_string())?;
            let verify = session.sftp_read(&full).await.map_err(|e| e.to_string())?;
            if String::from_utf8_lossy(&verify) != content {
                return Err(
                    "doctor file write verification failed: remote content mismatch".to_string(),
                );
            }
            Ok(json!({
                "target": id,
                "remote": true,
                "domain": domain,
                "root": root,
                "path": rel,
                "fullPath": full,
                "written": true,
                "backup": backup,
            }))
        }
    }
}

fn sh_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\"'\"'"))
}

fn delete_json_path(value: &mut serde_json::Value, dotted_path: &str) -> bool {
    let parts: Vec<&str> = dotted_path
        .split('.')
        .map(str::trim)
        .filter(|p| !p.is_empty())
        .collect();
    if parts.is_empty() {
        return false;
    }
    let mut cursor = value;
    for part in &parts[..parts.len() - 1] {
        if let Some(next) = cursor.get_mut(*part) {
            cursor = next;
        } else {
            return false;
        }
    }
    if let Some(obj) = cursor.as_object_mut() {
        return obj.remove(parts[parts.len() - 1]).is_some();
    }
    false
}

fn upsert_json_path(
    value: &mut serde_json::Value,
    dotted_path: &str,
    next_value: serde_json::Value,
) -> Result<(), String> {
    let parts: Vec<&str> = dotted_path
        .split('.')
        .map(str::trim)
        .filter(|p| !p.is_empty())
        .collect();
    if parts.is_empty() {
        return Err("doctor config-upsert requires non-empty <json.path>".to_string());
    }
    let mut cursor = value;
    for part in &parts[..parts.len() - 1] {
        if cursor.get(*part).is_none() {
            if let Some(obj) = cursor.as_object_mut() {
                obj.insert((*part).to_string(), json!({}));
            } else {
                return Err(format!("path segment '{part}' is not an object"));
            }
        }
        cursor = cursor
            .get_mut(*part)
            .ok_or_else(|| format!("path segment '{part}' is missing"))?;
        if !cursor.is_object() {
            return Err(format!("path segment '{part}' is not an object"));
        }
    }
    let leaf = parts[parts.len() - 1];
    let obj = cursor
        .as_object_mut()
        .ok_or_else(|| "target parent is not an object".to_string())?;
    obj.insert(leaf.to_string(), next_value);
    Ok(())
}

fn json_path_get<'a>(value: &'a serde_json::Value, dotted_path: &str) -> Option<&'a serde_json::Value> {
    let parts: Vec<&str> = dotted_path
        .split('.')
        .map(str::trim)
        .filter(|p| !p.is_empty())
        .collect();
    if parts.is_empty() {
        return Some(value);
    }
    let mut cursor = value;
    for part in parts {
        cursor = cursor.get(part)?;
    }
    Some(cursor)
}
