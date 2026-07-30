use std::{
    fs,
    path::{Path, PathBuf},
};

use anyhow::{Context, Result};
use clap::{Parser, ValueEnum};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, Default, ValueEnum, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum LayoutPreference {
    #[default]
    Auto,
    Compact,
    Wide,
}

#[derive(Debug, Parser)]
#[command(name = "sourcepane", version, about)]
pub struct Cli {
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

    /// Logging verbosity.
    #[arg(long, default_value = "warn")]
    pub log_level: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub mouse: bool,
    pub confirm_destructive: bool,
    pub graph_page_size: usize,
    pub refresh_ms: u64,
    pub layout: LayoutPreference,
    pub editor: EditorSettings,
    pub theme: ThemeSettings,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct EditorSettings {
    pub command: Option<String>,
    pub args: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct ThemeSettings {
    pub accent: String,
    pub added: String,
    pub deleted: String,
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
            theme: ThemeSettings::default(),
        }
    }
}

impl Default for ThemeSettings {
    fn default() -> Self {
        Self {
            accent: "blue".into(),
            added: "green".into(),
            deleted: "red".into(),
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
    dirs::config_dir().map(|path| path.join("sourcepane").join("config.toml"))
}
