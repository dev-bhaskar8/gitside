use std::{
    fs,
    io::Write,
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

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq, Hash)]
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
    #[serde(skip)]
    pub config_path: Option<PathBuf>,
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
    #[serde(rename = "emoji", skip_serializing)]
    pub(crate) legacy_emoji: Option<bool>,
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
            legacy_emoji: None,
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
            config_path: None,
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
            return Ok(Self {
                config_path: Some(path),
                ..Self::default()
            });
        }
        let source = fs::read_to_string(&path)
            .with_context(|| format!("failed to read configuration {}", path.display()))?;
        let mut settings: Self = toml::from_str(&source)
            .with_context(|| format!("invalid configuration {}", path.display()))?;
        settings.config_path = Some(path);
        Ok(settings)
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

    pub fn save_ai(&self) -> Result<()> {
        let path = self
            .config_path
            .as_ref()
            .context("the platform configuration directory is unavailable")?;
        let source = if path.exists() {
            fs::read_to_string(path)
                .with_context(|| format!("failed to read configuration {}", path.display()))?
        } else {
            String::new()
        };
        let mut document = if source.trim().is_empty() {
            toml_edit::DocumentMut::new()
        } else {
            source
                .parse::<toml_edit::DocumentMut>()
                .with_context(|| format!("invalid configuration {}", path.display()))?
        };
        let ai =
            toml_edit::ser::to_document(&self.ai).context("failed to serialize AI settings")?;
        document["ai"] = toml_edit::Item::Table(ai.as_table().clone());
        let parent = path.parent().context("configuration path has no parent")?;
        fs::create_dir_all(parent)
            .with_context(|| format!("failed to create {}", parent.display()))?;
        let mut temporary = tempfile::NamedTempFile::new_in(parent).with_context(|| {
            format!("failed to create a temporary file in {}", parent.display())
        })?;
        temporary
            .write_all(document.to_string().as_bytes())
            .context("failed to write AI configuration")?;
        temporary
            .as_file()
            .sync_all()
            .context("failed to sync AI configuration")?;
        temporary
            .persist(path)
            .map_err(|error| error.error)
            .with_context(|| format!("failed to replace configuration {}", path.display()))?;
        Ok(())
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
        assert_eq!(settings.ai.legacy_emoji, Some(true));
        assert_eq!(settings.ai.agent.provider, AgentProvider::Claude);
        assert_eq!(settings.ai.api.provider, ApiProvider::Openrouter);
    }

    #[test]
    fn saving_ai_preserves_other_configuration_and_never_writes_a_key() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("config.toml");
        fs::write(
            &path,
            "# keep this comment\nmouse = false\n\n[ai]\nenabled = false\nmode = \"local\"\nemoji = true\n",
        )
        .unwrap();
        let mut settings = Settings::load(Some(&path)).unwrap();
        settings.ai.enabled = true;
        settings.ai.mode = AiMode::Api;
        settings.ai.api.provider = ApiProvider::Openai;
        settings.ai.api.model = Some("test-model".into());

        settings.save_ai().unwrap();

        let saved = fs::read_to_string(&path).unwrap();
        assert!(saved.contains("# keep this comment"));
        assert!(saved.contains("mouse = false"));
        assert!(saved.contains("model = \"test-model\""));
        assert!(!saved.contains("emoji"));
        assert!(!saved.to_ascii_lowercase().contains("api_key ="));
        let loaded = Settings::load(Some(&path)).unwrap();
        assert!(loaded.ai.enabled);
        assert_eq!(loaded.ai.mode, AiMode::Api);
    }
}
