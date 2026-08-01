mod ai;
mod app;
mod config;
mod credentials;
mod git;
mod github;
mod model;
mod terminal;
mod ui;

use std::{path::PathBuf, process::Stdio};

use anyhow::{Context, Result, bail};
use clap::Parser;
use tokio::process::Command;

use crate::{
    app::App,
    config::{Cli, CliCommand},
};

#[tokio::main]
async fn main() -> Result<()> {
    let mut cli = Cli::parse();
    if let Some(command) = cli.command.take() {
        cli.paths = vec![run_setup_command(command).await?];
    }
    let settings = config::Settings::load(cli.config.as_deref())?.merge_cli(&cli);
    let mut app = App::new(cli, settings).await?;
    terminal::run(&mut app).await
}

async fn run_setup_command(command: CliCommand) -> Result<PathBuf> {
    match command {
        CliCommand::Init { path } => {
            let status = Command::new("git")
                .arg("init")
                .arg(&path)
                .stdin(Stdio::inherit())
                .stdout(Stdio::inherit())
                .stderr(Stdio::inherit())
                .status()
                .await
                .context("Git is required but was not found")?;
            if !status.success() {
                bail!("git init failed with {}", status.code().unwrap_or(-1));
            }
            Ok(path)
        }
        CliCommand::Clone { url, destination } => {
            let resolved = destination
                .clone()
                .unwrap_or_else(|| default_clone_destination(&url));
            let mut process = Command::new("git");
            process.arg("clone").arg("--").arg(&url);
            if let Some(destination) = &destination {
                process.arg(destination);
            }
            let status = process
                .stdin(Stdio::inherit())
                .stdout(Stdio::inherit())
                .stderr(Stdio::inherit())
                .status()
                .await
                .context("Git is required but was not found")?;
            if !status.success() {
                bail!("git clone failed with {}", status.code().unwrap_or(-1));
            }
            Ok(resolved)
        }
    }
}

fn default_clone_destination(url: &str) -> PathBuf {
    let trimmed = url.trim_end_matches('/');
    let name = trimmed
        .rsplit(['/', ':'])
        .next()
        .unwrap_or("repository")
        .strip_suffix(".git")
        .unwrap_or_else(|| trimmed.rsplit(['/', ':']).next().unwrap_or("repository"));
    PathBuf::from(name)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn derives_clone_directory_from_common_urls() {
        assert_eq!(
            default_clone_destination("https://github.com/acme/widget.git"),
            PathBuf::from("widget")
        );
        assert_eq!(
            default_clone_destination("git@github.com:acme/widget.git"),
            PathBuf::from("widget")
        );
    }
}
