use serde::Serialize;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RecipeActionCatalogEntry {
    pub kind: String,
    pub title: String,
    pub group: String,
    pub category: String,
    pub backend: String,
    pub description: String,
    pub read_only: bool,
    pub interactive: bool,
    pub runner_supported: bool,
    pub recommended: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cli_command: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub legacy_alias_of: Option<String>,
    #[serde(default)]
    pub capabilities: Vec<String>,
    #[serde(default)]
    pub resource_kinds: Vec<String>,
}

impl RecipeActionCatalogEntry {
    fn new(
        kind: &str,
        title: &str,
        group: &str,
        category: &str,
        backend: &str,
        description: &str,
    ) -> Self {
        Self {
            kind: kind.into(),
            title: title.into(),
            group: group.into(),
            category: category.into(),
            backend: backend.into(),
            description: description.into(),
            read_only: false,
            interactive: false,
            runner_supported: true,
            recommended: false,
            cli_command: None,
            legacy_alias_of: None,
            capabilities: Vec::new(),
            resource_kinds: Vec::new(),
        }
    }

    fn read_only(mut self) -> Self {
        self.read_only = true;
        self
    }

    fn interactive(mut self) -> Self {
        self.interactive = true;
        self.runner_supported = false;
        self
    }

    fn unsupported(mut self) -> Self {
        self.runner_supported = false;
        self
    }

    fn recommended(mut self) -> Self {
        self.recommended = true;
        self
    }

    fn cli(mut self, cli_command: &str) -> Self {
        self.cli_command = Some(cli_command.into());
        self
    }

    fn alias_of(mut self, kind: &str) -> Self {
        self.legacy_alias_of = Some(kind.into());
        self
    }

    fn capabilities(mut self, capabilities: &[&str]) -> Self {
        self.capabilities = capabilities.iter().map(|item| item.to_string()).collect();
        self
    }

    fn resource_kinds(mut self, kinds: &[&str]) -> Self {
        self.resource_kinds = kinds.iter().map(|item| item.to_string()).collect();
        self
    }
}

pub fn list_recipe_actions() -> Vec<RecipeActionCatalogEntry> {
    vec![
        RecipeActionCatalogEntry::new(
            "create_agent",
            "Create agent",
            "business",
            "agents",
            "openclaw_cli",
            "Create a new OpenClaw agent.",
        )
        .cli("openclaw agents add")
        .recommended()
        .capabilities(&["agent.manage"])
        .resource_kinds(&["agent"]),
        RecipeActionCatalogEntry::new(
            "delete_agent",
            "Delete agent",
            "business",
            "agents",
            "openclaw_cli",
            "Delete an OpenClaw agent after binding safety checks.",
        )
        .cli("openclaw agents delete")
        .recommended()
        .capabilities(&["agent.manage"])
        .resource_kinds(&["agent", "channel"]),
        RecipeActionCatalogEntry::new(
            "bind_agent",
            "Bind agent",
            "business",
            "agents",
            "openclaw_cli",
            "Bind a channel routing target to an agent using OpenClaw binding syntax.",
        )
        .cli("openclaw agents bind")
        .recommended()
        .capabilities(&["binding.manage"])
        .resource_kinds(&["agent", "channel"]),
        RecipeActionCatalogEntry::new(
            "unbind_agent",
            "Unbind agent",
            "business",
            "agents",
            "openclaw_cli",
            "Remove one or all routing bindings from an agent.",
        )
        .cli("openclaw agents unbind")
        .recommended()
        .capabilities(&["binding.manage"])
        .resource_kinds(&["agent", "channel"]),
        RecipeActionCatalogEntry::new(
            "set_agent_identity",
            "Set agent identity",
            "business",
            "agents",
            "openclaw_cli",
            "Update an agent identity using OpenClaw identity fields.",
        )
        .cli("openclaw agents set-identity")
        .recommended()
        .capabilities(&["agent.identity.write"])
        .resource_kinds(&["agent"]),
        RecipeActionCatalogEntry::new(
            "set_agent_model",
            "Set agent model",
            "business",
            "models",
            "orchestrated",
            "Set an agent model after ensuring the target model profile exists.",
        )
        .recommended()
        .capabilities(&["model.manage", "secret.sync"])
        .resource_kinds(&["agent", "modelProfile"]),
        RecipeActionCatalogEntry::new(
            "set_agent_persona",
            "Set agent persona",
            "business",
            "agents",
            "clawpal_fallback",
            "Update the persona section in an agent markdown document.",
        )
        .recommended()
        .capabilities(&["agent.identity.write"])
        .resource_kinds(&["agent"]),
        RecipeActionCatalogEntry::new(
            "clear_agent_persona",
            "Clear agent persona",
            "business",
            "agents",
            "clawpal_fallback",
            "Remove the persona section from an agent markdown document.",
        )
        .recommended()
        .capabilities(&["agent.identity.write"])
        .resource_kinds(&["agent"]),
        RecipeActionCatalogEntry::new(
            "set_channel_persona",
            "Set channel persona",
            "business",
            "channels",
            "openclaw_cli",
            "Set the systemPrompt for a channel through OpenClaw config.",
        )
        .recommended()
        .capabilities(&["config.write"])
        .resource_kinds(&["channel"]),
        RecipeActionCatalogEntry::new(
            "clear_channel_persona",
            "Clear channel persona",
            "business",
            "channels",
            "openclaw_cli",
            "Clear the systemPrompt for a channel through OpenClaw config.",
        )
        .recommended()
        .capabilities(&["config.write"])
        .resource_kinds(&["channel"]),
        RecipeActionCatalogEntry::new(
            "upsert_markdown_document",
            "Upsert markdown document",
            "document",
            "documents",
            "clawpal_fallback",
            "Write or update a text/markdown document using a controlled document target.",
        )
        .capabilities(&["document.write"])
        .resource_kinds(&["document"]),
        RecipeActionCatalogEntry::new(
            "delete_markdown_document",
            "Delete markdown document",
            "document",
            "documents",
            "clawpal_fallback",
            "Delete a text/markdown document using a controlled document target.",
        )
        .capabilities(&["document.delete"])
        .resource_kinds(&["document"]),
        RecipeActionCatalogEntry::new(
            "ensure_model_profile",
            "Ensure model profile",
            "environment",
            "models",
            "orchestrated",
            "Ensure a model profile and its dependent auth are available in the target environment.",
        )
        .recommended()
        .capabilities(&["model.manage", "secret.sync"])
        .resource_kinds(&["modelProfile", "authProfile"]),
        RecipeActionCatalogEntry::new(
            "delete_model_profile",
            "Delete model profile",
            "environment",
            "models",
            "orchestrated",
            "Delete a model profile after checking for active bindings.",
        )
        .recommended()
        .capabilities(&["model.manage"])
        .resource_kinds(&["modelProfile", "authProfile"]),
        RecipeActionCatalogEntry::new(
            "ensure_provider_auth",
            "Ensure provider auth",
            "environment",
            "models",
            "orchestrated",
            "Ensure a provider auth profile exists in the target environment.",
        )
        .recommended()
        .capabilities(&["auth.manage", "secret.sync"])
        .resource_kinds(&["authProfile"]),
        RecipeActionCatalogEntry::new(
            "delete_provider_auth",
            "Delete provider auth",
            "environment",
            "models",
            "orchestrated",
            "Delete a provider auth profile after checking for dependent model bindings.",
        )
        .recommended()
        .capabilities(&["auth.manage"])
        .resource_kinds(&["authProfile"]),
        RecipeActionCatalogEntry::new(
            "setup_identity",
            "Setup identity",
            "legacy",
            "agents",
            "clawpal_fallback",
            "Legacy compatibility action for identity and persona updates.",
        )
        .alias_of("set_agent_identity")
        .capabilities(&["agent.identity.write"])
        .resource_kinds(&["agent"]),
        RecipeActionCatalogEntry::new(
            "bind_channel",
            "Bind channel",
            "legacy",
            "agents",
            "openclaw_cli",
            "Legacy compatibility action for channel binding based on peer/channel fields.",
        )
        .alias_of("bind_agent")
        .capabilities(&["binding.manage"])
        .resource_kinds(&["agent", "channel"]),
        RecipeActionCatalogEntry::new(
            "unbind_channel",
            "Unbind channel",
            "legacy",
            "agents",
            "openclaw_cli",
            "Legacy compatibility action for channel unbinding based on peer/channel fields.",
        )
        .alias_of("unbind_agent")
        .capabilities(&["binding.manage"])
        .resource_kinds(&["channel"]),
        RecipeActionCatalogEntry::new(
            "config_patch",
            "Config patch",
            "legacy",
            "config",
            "openclaw_cli",
            "Low-level escape hatch for direct config set operations.",
        )
        .capabilities(&["config.write"])
        .resource_kinds(&["file"]),
        RecipeActionCatalogEntry::new(
            "list_agents",
            "List agents",
            "cli",
            "agents",
            "openclaw_cli",
            "Run `openclaw agents list` as a read-only inspection action.",
        )
        .cli("openclaw agents list")
        .read_only(),
        RecipeActionCatalogEntry::new(
            "list_agent_bindings",
            "List agent bindings",
            "cli",
            "agents",
            "openclaw_cli",
            "Run `openclaw agents bindings` as a read-only inspection action.",
        )
        .cli("openclaw agents bindings")
        .read_only(),
        RecipeActionCatalogEntry::new(
            "show_config_file",
            "Show config file",
            "cli",
            "config",
            "openclaw_cli",
            "Print the active OpenClaw config file path.",
        )
        .cli("openclaw config file")
        .read_only(),
        RecipeActionCatalogEntry::new(
            "get_config_value",
            "Get config value",
            "cli",
            "config",
            "openclaw_cli",
            "Read a config value through `openclaw config get`.",
        )
        .cli("openclaw config get")
        .read_only(),
        RecipeActionCatalogEntry::new(
            "set_config_value",
            "Set config value",
            "cli",
            "config",
            "openclaw_cli",
            "Set a config value through `openclaw config set`.",
        )
        .cli("openclaw config set")
        .capabilities(&["config.write"])
        .resource_kinds(&["file"]),
        RecipeActionCatalogEntry::new(
            "unset_config_value",
            "Unset config value",
            "cli",
            "config",
            "openclaw_cli",
            "Unset a config value through `openclaw config unset`.",
        )
        .cli("openclaw config unset")
        .capabilities(&["config.write"])
        .resource_kinds(&["file"]),
        RecipeActionCatalogEntry::new(
            "validate_config",
            "Validate config",
            "cli",
            "config",
            "openclaw_cli",
            "Validate the active config without starting the gateway.",
        )
        .cli("openclaw config validate")
        .read_only(),
        RecipeActionCatalogEntry::new(
            "models_status",
            "Models status",
            "cli",
            "models",
            "openclaw_cli",
            "Inspect resolved default models, fallbacks, and auth state.",
        )
        .cli("openclaw models status")
        .read_only(),
        RecipeActionCatalogEntry::new(
            "list_models",
            "List models",
            "cli",
            "models",
            "openclaw_cli",
            "List known models through `openclaw models list`.",
        )
        .cli("openclaw models list")
        .read_only(),
        RecipeActionCatalogEntry::new(
            "set_default_model",
            "Set default model",
            "cli",
            "models",
            "openclaw_cli",
            "Set the default OpenClaw model or alias.",
        )
        .cli("openclaw models set")
        .capabilities(&["model.manage"])
        .resource_kinds(&["modelProfile"]),
        RecipeActionCatalogEntry::new(
            "scan_models",
            "Scan models",
            "cli",
            "models",
            "openclaw_cli",
            "Probe model/provider availability through `openclaw models scan`.",
        )
        .cli("openclaw models scan")
        .read_only(),
        RecipeActionCatalogEntry::new(
            "list_model_aliases",
            "List model aliases",
            "cli",
            "models",
            "openclaw_cli",
            "List configured model aliases.",
        )
        .cli("openclaw models aliases list")
        .read_only(),
        RecipeActionCatalogEntry::new(
            "list_model_fallbacks",
            "List model fallbacks",
            "cli",
            "models",
            "openclaw_cli",
            "List configured model fallbacks.",
        )
        .cli("openclaw models fallbacks list")
        .read_only(),
        RecipeActionCatalogEntry::new(
            "add_model_auth_profile",
            "Add model auth profile",
            "cli",
            "models",
            "openclaw_cli",
            "Create a provider auth profile with provider-specific inputs.",
        )
        .cli("openclaw models auth add")
        .unsupported(),
        RecipeActionCatalogEntry::new(
            "login_model_auth",
            "Login model auth",
            "cli",
            "models",
            "openclaw_cli",
            "Run a provider login flow for model auth.",
        )
        .cli("openclaw models auth login")
        .interactive(),
        RecipeActionCatalogEntry::new(
            "setup_model_auth_token",
            "Setup model auth token",
            "cli",
            "models",
            "openclaw_cli",
            "Prompt for a setup token for provider auth.",
        )
        .cli("openclaw models auth setup-token")
        .interactive(),
        RecipeActionCatalogEntry::new(
            "paste_model_auth_token",
            "Paste model auth token",
            "cli",
            "models",
            "openclaw_cli",
            "Paste a token for model auth. Not suitable for Recipe source because it carries secret material.",
        )
        .cli("openclaw models auth paste-token")
        .unsupported(),
        RecipeActionCatalogEntry::new(
            "list_channels",
            "List channels",
            "cli",
            "channels",
            "openclaw_cli",
            "List configured channel accounts.",
        )
        .cli("openclaw channels list")
        .read_only(),
        RecipeActionCatalogEntry::new(
            "channels_status",
            "Channels status",
            "cli",
            "channels",
            "openclaw_cli",
            "Inspect live channel health and config-only fallbacks.",
        )
        .cli("openclaw channels status")
        .read_only(),
        RecipeActionCatalogEntry::new(
            "read_channel_logs",
            "Read channel logs",
            "cli",
            "channels",
            "openclaw_cli",
            "Read recent channel logs.",
        )
        .cli("openclaw channels logs")
        .read_only()
        .unsupported(),
        RecipeActionCatalogEntry::new(
            "add_channel_account",
            "Add channel account",
            "cli",
            "channels",
            "openclaw_cli",
            "Add a channel account with provider-specific flags.",
        )
        .cli("openclaw channels add")
        .unsupported(),
        RecipeActionCatalogEntry::new(
            "remove_channel_account",
            "Remove channel account",
            "cli",
            "channels",
            "openclaw_cli",
            "Remove a configured channel account.",
        )
        .cli("openclaw channels remove")
        .unsupported(),
        RecipeActionCatalogEntry::new(
            "login_channel_account",
            "Login channel account",
            "cli",
            "channels",
            "openclaw_cli",
            "Run an interactive login flow for a channel account.",
        )
        .cli("openclaw channels login")
        .interactive(),
        RecipeActionCatalogEntry::new(
            "logout_channel_account",
            "Logout channel account",
            "cli",
            "channels",
            "openclaw_cli",
            "Run an interactive logout flow for a channel account.",
        )
        .cli("openclaw channels logout")
        .interactive(),
        RecipeActionCatalogEntry::new(
            "inspect_channel_capabilities",
            "Inspect channel capabilities",
            "cli",
            "channels",
            "openclaw_cli",
            "Probe channel capabilities and target reachability.",
        )
        .cli("openclaw channels capabilities")
        .read_only(),
        RecipeActionCatalogEntry::new(
            "resolve_channel_targets",
            "Resolve channel targets",
            "cli",
            "channels",
            "openclaw_cli",
            "Resolve names to channel/user ids through provider directories.",
        )
        .cli("openclaw channels resolve")
        .read_only(),
        RecipeActionCatalogEntry::new(
            "reload_secrets",
            "Reload secrets",
            "cli",
            "secrets",
            "openclaw_cli",
            "Reload the active runtime secret snapshot.",
        )
        .cli("openclaw secrets reload")
        .read_only(),
        RecipeActionCatalogEntry::new(
            "audit_secrets",
            "Audit secrets",
            "cli",
            "secrets",
            "openclaw_cli",
            "Audit unresolved SecretRefs and plaintext residues.",
        )
        .cli("openclaw secrets audit")
        .read_only(),
        RecipeActionCatalogEntry::new(
            "configure_secrets",
            "Configure secrets",
            "cli",
            "secrets",
            "openclaw_cli",
            "Run the interactive SecretRef configuration helper.",
        )
        .cli("openclaw secrets configure")
        .interactive(),
        RecipeActionCatalogEntry::new(
            "apply_secrets_plan",
            "Apply secrets plan",
            "cli",
            "secrets",
            "openclaw_cli",
            "Apply a saved secrets migration plan.",
        )
        .cli("openclaw secrets apply")
        .capabilities(&["auth.manage", "secret.sync"])
        .resource_kinds(&["authProfile", "file"]),
    ]
}

pub fn find_recipe_action(kind: &str) -> Option<RecipeActionCatalogEntry> {
    list_recipe_actions()
        .into_iter()
        .find(|entry| entry.kind == kind)
}
