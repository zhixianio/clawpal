use clap::{Parser, Subcommand};
use clawpal_core::connect::{connect_docker, connect_ssh};
use clawpal_core::health::{check_instance, HealthStatus};
use clawpal_core::install;
use clawpal_core::instance::{Instance, InstanceRegistry, InstanceType};
use clawpal_core::openclaw::OpenclawCli;
use clawpal_core::profile::{
    delete_profile, list_profiles, test_profile, upsert_profile, ModelProfile,
};
use serde_json::json;

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
        #[arg(long)]
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
        #[arg(long)]
        api_key: Option<String>,
    },
    Remove {
        id: String,
    },
    Test {
        id: String,
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
        command => Ok(json!({
            "status": "not yet implemented",
            "command": format!("{command:?}"),
        })),
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
        InstallCommands::Docker { home, command } => {
            let options = install::DockerInstallOptions {
                home,
                label: None,
                dry_run: false,
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
