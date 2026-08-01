use std::{
    fs,
    path::{Path, PathBuf},
};

use anyhow::{Context, Result};
use clap::{Parser, Subcommand, ValueEnum};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, Default, ValueEnum, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum LayoutPreference {
    #[default]
    Auto,
    Compact,
    Wide,
}

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum AiMode {
    #[default]
    Local,
    Agent,
    Api,
}

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum AgentProvider {
    #[default]
    Codex,
    Claude,
    Opencode,
    Custom,
}

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum ApiProvider {
    #[default]
    Openai,
    Anthropic,
    Gemini,
    Openrouter,
    Compatible,
}

#[derive(Debug, Parser)]
#[command(name = "gitside", version, about)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Option<CliCommand>,

    /// Repository paths. The current directory is used when omitted.
    #[arg(value_name = "PATH")]
    pub paths: Vec<PathBuf>,

    /// Add a repository path (repeatable).
    #[arg(long = "repo", value_name = "PATH")]
    pub repos: Vec<PathBuf>,

    /// Load configuration from this TOML file.
    #[arg(long)]
    pub config: Option<PathBuf>,

    /// Disable terminal mouse capture.
    #[arg(long)]
    pub no_mouse: bool,

    /// Override the external editor command.
    #[arg(long)]
    pub editor: Option<String>,

    /// Override responsive layout selection.
    #[arg(long, value_enum)]
    pub layout: Option<LayoutPreference>,
}

#[derive(Debug, Clone, Subcommand)]
pub enum CliCommand {
    /// Initialize a Git repository and open it.
    Init {
        #[arg(default_value = ".")]
        path: PathBuf,
    },
    /// Clone a repository and open the new checkout.
    Clone {
        url: String,
        destination: Option<PathBuf>,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Settings {
    pub mouse: bool,
    pub confirm_destructive: bool,
    pub graph_page_size: usize,
    pub refresh_ms: u64,
    pub layout: LayoutPreference,
    pub editor: EditorSettings,
    pub ai: AiSettings,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct EditorSettings {
    pub command: Option<String>,
    pub args: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct AiSettings {
    pub enabled: bool,
    pub mode: AiMode,
    pub emoji: bool,
    pub instructions: String,
    pub max_diff_bytes: usize,
    pub max_files: usize,
    pub agent: AgentSettings,
    pub api: ApiSettings,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct AgentSettings {
    pub provider: AgentProvider,
    pub command: Option<String>,
    pub args: Vec<String>,
    pub model: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct ApiSettings {
    pub provider: ApiProvider,
    pub model: Option<String>,
    pub api_key_env: Option<String>,
    pub endpoint: Option<String>,
}

impl Default for AiSettings {
    fn default() -> Self {
        Self {
            enabled: false,
            mode: AiMode::Local,
            emoji: false,
            instructions: String::new(),
            max_diff_bytes: 32_000,
            max_files: 3,
            agent: AgentSettings::default(),
            api: ApiSettings::default(),
        }
    }
}

impl Default for AgentSettings {
    fn default() -> Self {
        Self {
            provider: AgentProvider::Codex,
            command: None,
            args: Vec::new(),
            model: None,
        }
    }
}

impl Default for ApiSettings {
    fn default() -> Self {
        Self {
            provider: ApiProvider::Openai,
            model: None,
            api_key_env: None,
            endpoint: None,
        }
    }
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            mouse: true,
            confirm_destructive: true,
            graph_page_size: 200,
            refresh_ms: 1500,
            layout: LayoutPreference::Auto,
            editor: EditorSettings::default(),
            ai: AiSettings::default(),
        }
    }
}

impl Settings {
    pub fn load(explicit: Option<&Path>) -> Result<Self> {
        let path = explicit.map(PathBuf::from).or_else(default_config_path);
        let Some(path) = path else {
            return Ok(Self::default());
        };
        if !path.exists() {
            return Ok(Self::default());
        }
        let source = fs::read_to_string(&path)
            .with_context(|| format!("failed to read configuration {}", path.display()))?;
        toml::from_str(&source).with_context(|| format!("invalid configuration {}", path.display()))
    }

    pub fn merge_cli(mut self, cli: &Cli) -> Self {
        if cli.no_mouse {
            self.mouse = false;
        }
        if let Some(layout) = cli.layout {
            self.layout = layout;
        }
        if let Some(editor) = &cli.editor {
            self.editor.command = Some(editor.clone());
            self.editor.args.clear();
        }
        self
    }
}

fn default_config_path() -> Option<PathBuf> {
    dirs::config_dir().map(|path| path.join("gitside").join("config.toml"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn removed_log_level_option_is_rejected() {
        assert!(Cli::try_parse_from(["gitside", "--log-level", "debug"]).is_err());
    }

    #[test]
    fn removed_theme_configuration_is_rejected() {
        let source = "[theme]\naccent = \"blue\"\n";
        assert!(toml::from_str::<Settings>(source).is_err());
    }

    #[test]
    fn documented_configuration_still_parses() {
        let source = r#"
mouse = false
confirm_destructive = true
graph_page_size = 100
refresh_ms = 2000
layout = "compact"

[editor]
command = "code"
args = ["--reuse-window", "--goto", "{path}"]

[ai]
enabled = true
mode = "agent"
emoji = true
instructions = "Use conventional commits."
max_diff_bytes = 24000
max_files = 4

[ai.agent]
provider = "claude"
model = "sonnet"

[ai.api]
provider = "openrouter"
model = "anthropic/claude-sonnet-4"
api_key_env = "OPENROUTER_API_KEY"
"#;
        let settings = toml::from_str::<Settings>(source).unwrap();
        assert!(!settings.mouse);
        assert_eq!(settings.graph_page_size, 100);
        assert_eq!(settings.layout, LayoutPreference::Compact);
        assert_eq!(settings.editor.command.as_deref(), Some("code"));
        assert!(settings.ai.enabled);
        assert_eq!(settings.ai.mode, AiMode::Agent);
        assert!(settings.ai.emoji);
        assert_eq!(settings.ai.agent.provider, AgentProvider::Claude);
        assert_eq!(settings.ai.api.provider, ApiProvider::Openrouter);
    }
}
