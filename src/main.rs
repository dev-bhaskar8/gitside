mod app;
mod config;
mod git;
mod github;
mod model;
mod terminal;
mod ui;

use anyhow::Result;
use clap::Parser;

use crate::{app::App, config::Cli};

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();
    let settings = config::Settings::load(cli.config.as_deref())?.merge_cli(&cli);
    let mut app = App::new(cli, settings).await?;
    terminal::run(&mut app).await
}
