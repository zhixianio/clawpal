use clap::{Parser, Subcommand};
use clawpal_core::connect::{connect_docker, connect_ssh};
use clawpal_core::health::{check_instance, HealthStatus};
use clawpal_core::install;
use clawpal_core::instance::{Instance, InstanceRegistry, InstanceType};
use clawpal_core::openclaw::OpenclawCli;
use clawpal_core::profile::{
    delete_profile, list_profiles, test_profile, upsert_profile, ModelProfile,
};
use clawpal_core::shell::{shell_quote, wrap_login_shell_eval};
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
            .block_on(connect_docker(&home, label.as_deref(), None))
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
                passphrase: None,
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
    Remote {
        id: String,
        host: Box<clawpal_core::instance::SshHostConfig>,
    },
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
        host: Box::new(host),
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
        },
    }
}

async fn doctor_probe_openclaw(target: DoctorTarget) -> Result<serde_json::Value, String> {
    match target {
        DoctorTarget::Local => {
            let version_out = Command::new(clawpal_core::openclaw::resolve_openclaw_bin())
                .arg("--version")
                .output()
                .map_err(|e| format!("failed to run openclaw --version: {e}"))?;
            let version = String::from_utf8_lossy(&version_out.stdout)
                .trim()
                .to_string();
            let probe_cmd =
                wrap_login_shell_eval(clawpal_core::doctor::openclaw_which_probe_script());
            let path_out = Command::new("sh")
                .args(["-c", &probe_cmd])
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
            let session = SshSession::connect(&host)
                .await
                .map_err(|e| e.to_string())?;
            let path = session
                .exec(&wrap_login_shell_eval(
                    clawpal_core::doctor::openclaw_which_probe_script(),
                ))
                .await
                .map_err(|e| e.to_string())?
                .stdout
                .trim()
                .to_string();
            let version = session
                .exec(&wrap_login_shell_eval(
                    clawpal_core::doctor::remote_openclaw_version_probe_script(),
                ))
                .await
                .map_err(|e| e.to_string())?
                .stdout
                .trim()
                .to_string();
            let env_path = session
                .exec(&wrap_login_shell_eval(
                    clawpal_core::doctor::shell_path_probe_script(),
                ))
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
        DoctorTarget::Local => {
            Err("doctor fix-openclaw-path currently supports remote target only".to_string())
        }
        DoctorTarget::Remote { id, host } => {
            let session = SshSession::connect(&host)
                .await
                .map_err(|e| e.to_string())?;
            let find_cmd =
                wrap_login_shell_eval(clawpal_core::doctor::remote_openclaw_fix_find_dir_script());
            let find = session.exec(&find_cmd).await.map_err(|e| e.to_string())?;
            let dir = find.stdout.trim().to_string();
            if dir.is_empty() {
                return Err("cannot locate openclaw binary in known directories".to_string());
            }
            let patch = clawpal_core::doctor::remote_openclaw_fix_patch_script(&dir);
            let patch_cmd = wrap_login_shell_eval(&patch);
            let result = session.exec(&patch_cmd).await.map_err(|e| e.to_string())?;
            let located = result.stdout.trim().to_string();
            Ok(json!({
                "target": id,
                "remote": true,
                "updatedPathDir": dir,
                "openclawPathAfterFix": if located.is_empty() { serde_json::Value::Null } else { json!(located) },
                "ok": !located.is_empty(),
                "stdout": result.stdout,
                "stderr": result.stderr,
                "exitCode": result.exit_code,
            }))
        }
    }
}

async fn resolve_remote_config_path(session: &SshSession) -> Result<String, String> {
    let cmd =
        wrap_login_shell_eval(clawpal_core::doctor::remote_openclaw_config_path_probe_script());
    let out = session.exec(&cmd).await.map_err(|e| e.to_string())?;
    Ok(out.stdout.trim().to_string())
}

async fn doctor_config_delete(
    target: DoctorTarget,
    dotted_path: &str,
) -> Result<serde_json::Value, String> {
    if dotted_path.trim().is_empty() {
        return Err("doctor config-delete requires <json.path>".to_string());
    }
    match target {
        DoctorTarget::Local => {
            let config_path = clawpal_core::doctor::local_openclaw_config_path_from_env();
            let raw = std::fs::read_to_string(&config_path)
                .map_err(|e| format!("failed to read local config: {e}"))?;
            let (rendered, deleted) = clawpal_core::doctor::delete_json_path_in_str(
                &raw,
                dotted_path,
                "local config",
                "config",
            )?;
            if deleted {
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
            let session = SshSession::connect(&host)
                .await
                .map_err(|e| e.to_string())?;
            let config_path = resolve_remote_config_path(&session).await?;
            let raw = session
                .sftp_read(&config_path)
                .await
                .map_err(|e| e.to_string())?;
            let raw = String::from_utf8_lossy(&raw).to_string();
            let (rendered, deleted) = clawpal_core::doctor::delete_json_path_in_str(
                &raw,
                dotted_path,
                "remote config",
                "config",
            )?;
            if deleted {
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
            let config_path = clawpal_core::doctor::local_openclaw_config_path_from_env();
            let raw = std::fs::read_to_string(&config_path)
                .map_err(|e| format!("failed to read local config: {e}"))?;
            let value = clawpal_core::doctor::select_json_value_from_str(
                &raw,
                dotted_path,
                "local config",
            )?;
            Ok(json!({
                "target": "local",
                "remote": false,
                "configPath": config_path.to_string_lossy(),
                "path": dotted_path.unwrap_or(""),
                "value": value,
            }))
        }
        DoctorTarget::Remote { id, host } => {
            let session = SshSession::connect(&host)
                .await
                .map_err(|e| e.to_string())?;
            let config_path = resolve_remote_config_path(&session).await?;
            let raw = session
                .sftp_read(&config_path)
                .await
                .map_err(|e| e.to_string())?;
            let raw = String::from_utf8_lossy(&raw).to_string();
            let value = clawpal_core::doctor::select_json_value_from_str(
                &raw,
                dotted_path,
                "remote config",
            )?;
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
    let parsed = clawpal_core::doctor::parse_json_value_arg(value_json, "doctor config-upsert")?;
    match target {
        DoctorTarget::Local => {
            let config_path = clawpal_core::doctor::local_openclaw_config_path_from_env();
            let raw = std::fs::read_to_string(&config_path)
                .map_err(|e| format!("failed to read local config: {e}"))?;
            let rendered = clawpal_core::doctor::upsert_json_path_in_str(
                &raw,
                dotted_path,
                parsed,
                "local config",
                "config",
            )?;
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
            let session = SshSession::connect(&host)
                .await
                .map_err(|e| e.to_string())?;
            let config_path = resolve_remote_config_path(&session).await?;
            let raw = session
                .sftp_read(&config_path)
                .await
                .map_err(|e| e.to_string())?;
            let raw = String::from_utf8_lossy(&raw).to_string();
            let rendered = clawpal_core::doctor::upsert_json_path_in_str(
                &raw,
                dotted_path,
                parsed,
                "remote config",
                "config",
            )?;
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

async fn resolve_remote_sessions_path(session: &SshSession) -> Result<String, String> {
    let cmd = wrap_login_shell_eval(clawpal_core::doctor::remote_sessions_discovery_script());
    let out = session.exec(&cmd).await.map_err(|e| e.to_string())?;
    Ok(out.stdout.trim().to_string())
}

async fn doctor_sessions_read(
    target: DoctorTarget,
    dotted_path: Option<&str>,
) -> Result<serde_json::Value, String> {
    match target {
        DoctorTarget::Local => {
            let sessions_path = clawpal_core::doctor::resolve_local_sessions_path(
                &clawpal_core::doctor::local_openclaw_root_from_env(),
            );
            let raw = std::fs::read_to_string(&sessions_path)
                .map_err(|e| format!("failed to read local sessions: {e}"))?;
            let value = clawpal_core::doctor::select_json_value_from_str(
                &raw,
                dotted_path,
                "local sessions",
            )?;
            Ok(json!({
                "target": "local",
                "remote": false,
                "sessionsPath": sessions_path.to_string_lossy(),
                "path": dotted_path.unwrap_or(""),
                "value": value,
            }))
        }
        DoctorTarget::Remote { id, host } => {
            let session = SshSession::connect(&host)
                .await
                .map_err(|e| e.to_string())?;
            let sessions_path = resolve_remote_sessions_path(&session).await?;
            let raw = session
                .sftp_read(&sessions_path)
                .await
                .map_err(|e| e.to_string())?;
            let raw = String::from_utf8_lossy(&raw).to_string();
            let value = clawpal_core::doctor::select_json_value_from_str(
                &raw,
                dotted_path,
                "remote sessions",
            )?;
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
    let parsed = clawpal_core::doctor::parse_json_value_arg(value_json, "doctor sessions-upsert")?;
    match target {
        DoctorTarget::Local => {
            let sessions_path = clawpal_core::doctor::resolve_local_sessions_path(
                &clawpal_core::doctor::local_openclaw_root_from_env(),
            );
            let raw = std::fs::read_to_string(&sessions_path)
                .map_err(|e| format!("failed to read local sessions: {e}"))?;
            let rendered = clawpal_core::doctor::upsert_json_path_in_str(
                &raw,
                dotted_path,
                parsed,
                "local sessions",
                "sessions",
            )?;
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
            let session = SshSession::connect(&host)
                .await
                .map_err(|e| e.to_string())?;
            let sessions_path = resolve_remote_sessions_path(&session).await?;
            let raw = session
                .sftp_read(&sessions_path)
                .await
                .map_err(|e| e.to_string())?;
            let raw = String::from_utf8_lossy(&raw).to_string();
            let rendered = clawpal_core::doctor::upsert_json_path_in_str(
                &raw,
                dotted_path,
                parsed,
                "remote sessions",
                "sessions",
            )?;
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
            let sessions_path = clawpal_core::doctor::resolve_local_sessions_path(
                &clawpal_core::doctor::local_openclaw_root_from_env(),
            );
            let raw = std::fs::read_to_string(&sessions_path)
                .map_err(|e| format!("failed to read local sessions: {e}"))?;
            let (rendered, deleted) = clawpal_core::doctor::delete_json_path_in_str(
                &raw,
                dotted_path,
                "local sessions",
                "sessions",
            )?;
            if deleted {
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
            let session = SshSession::connect(&host)
                .await
                .map_err(|e| e.to_string())?;
            let sessions_path = resolve_remote_sessions_path(&session).await?;
            let raw = session
                .sftp_read(&sessions_path)
                .await
                .map_err(|e| e.to_string())?;
            let raw = String::from_utf8_lossy(&raw).to_string();
            let (rendered, deleted) = clawpal_core::doctor::delete_json_path_in_str(
                &raw,
                dotted_path,
                "remote sessions",
                "sessions",
            )?;
            if deleted {
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

async fn doctor_domain_remote_root(session: &SshSession, domain: &str) -> Result<String, String> {
    let cmd = wrap_login_shell_eval(clawpal_core::doctor::remote_openclaw_root_probe_script());
    let out = session.exec(&cmd).await.map_err(|e| e.to_string())?;
    let base = out.stdout.trim().to_string();
    clawpal_core::doctor::doctor_domain_remote_root(&base, domain)
}

async fn doctor_file_read(
    target: DoctorTarget,
    domain: &str,
    path: Option<&str>,
) -> Result<serde_json::Value, String> {
    match target {
        DoctorTarget::Local => {
            let openclaw_root = clawpal_core::doctor::local_openclaw_root_from_env();
            let root = clawpal_core::doctor::doctor_domain_local_root(&openclaw_root, domain)?;
            let rel = match path {
                Some(p) => p.to_string(),
                None => match domain {
                    "sessions" => {
                        let abs = clawpal_core::doctor::resolve_local_sessions_path(&openclaw_root);
                        clawpal_core::doctor::relpath_from_local_abs(&root, &abs).ok_or_else(
                            || {
                                format!(
                                    "failed to resolve sessions path under domain root: {}",
                                    root.display()
                                )
                            },
                        )?
                    }
                    _ => clawpal_core::doctor::doctor_domain_default_relpath(domain)
                        .ok_or_else(|| {
                            "doctor file read requires --path for this domain".to_string()
                        })?
                        .to_string(),
                },
            };
            clawpal_core::doctor::validate_doctor_relative_path(&rel)?;
            let full = root.join(&rel);
            let content =
                std::fs::read_to_string(&full).map_err(|e| format!("failed to read file: {e}"))?;
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
            let session = SshSession::connect(&host)
                .await
                .map_err(|e| e.to_string())?;
            let root = doctor_domain_remote_root(&session, domain).await?;
            let rel = match path {
                Some(p) => p.to_string(),
                None => match domain {
                    "sessions" => {
                        let abs = resolve_remote_sessions_path(&session).await?;
                        clawpal_core::doctor::relpath_from_remote_abs(&root, &abs).ok_or_else(
                            || format!("failed to resolve sessions path under domain root: {root}"),
                        )?
                    }
                    _ => clawpal_core::doctor::doctor_domain_default_relpath(domain)
                        .ok_or_else(|| {
                            "doctor file read requires --path for this domain".to_string()
                        })?
                        .to_string(),
                },
            };
            clawpal_core::doctor::validate_doctor_relative_path(&rel)?;
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
            let openclaw_root = clawpal_core::doctor::local_openclaw_root_from_env();
            let root = clawpal_core::doctor::doctor_domain_local_root(&openclaw_root, domain)?;
            let rel = match path {
                Some(p) => p.to_string(),
                None => match domain {
                    "sessions" => {
                        let abs = clawpal_core::doctor::resolve_local_sessions_path(&openclaw_root);
                        clawpal_core::doctor::relpath_from_local_abs(&root, &abs).ok_or_else(
                            || {
                                format!(
                                    "failed to resolve sessions path under domain root: {}",
                                    root.display()
                                )
                            },
                        )?
                    }
                    _ => clawpal_core::doctor::doctor_domain_default_relpath(domain)
                        .ok_or_else(|| {
                            "doctor file write requires --path for this domain".to_string()
                        })?
                        .to_string(),
                },
            };
            clawpal_core::doctor::validate_doctor_relative_path(&rel)?;
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
            let session = SshSession::connect(&host)
                .await
                .map_err(|e| e.to_string())?;
            let root = doctor_domain_remote_root(&session, domain).await?;
            let rel = match path {
                Some(p) => p.to_string(),
                None => match domain {
                    "sessions" => {
                        let abs = resolve_remote_sessions_path(&session).await?;
                        clawpal_core::doctor::relpath_from_remote_abs(&root, &abs).ok_or_else(
                            || format!("failed to resolve sessions path under domain root: {root}"),
                        )?
                    }
                    _ => clawpal_core::doctor::doctor_domain_default_relpath(domain)
                        .ok_or_else(|| {
                            "doctor file write requires --path for this domain".to_string()
                        })?
                        .to_string(),
                },
            };
            clawpal_core::doctor::validate_doctor_relative_path(&rel)?;
            let full = format!("{}/{}", root.trim_end_matches('/'), rel);
            let mkdir_raw = format!("mkdir -p \"$(dirname {})\"", shell_quote(&full));
            let mkdir_cmd = wrap_login_shell_eval(&mkdir_raw);
            let mkdir_out = session.exec(&mkdir_cmd).await.map_err(|e| e.to_string())?;
            if mkdir_out.exit_code != 0 {
                return Err(format!(
                    "doctor file write mkdir failed (exit {}): {}",
                    mkdir_out.exit_code, mkdir_out.stderr
                ));
            }
            if backup {
                let backup_raw = format!(
                    "if [ -f {f} ]; then cp {f} {f}.bak.$(date +%Y%m%d%H%M%S); fi",
                    f = shell_quote(&full)
                );
                let backup_cmd = wrap_login_shell_eval(&backup_raw);
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
