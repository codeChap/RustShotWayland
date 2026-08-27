use anyhow::Result;
use clap::Parser;

mod canvas;
mod capture;
mod cli;
mod config;
mod client;
mod daemon;
mod dbus;
mod error;
mod export;
mod logging;
mod tray;
mod ui;

fn main() -> Result<()> {
    logging::init();
    let cli = cli::Cli::parse();
    match cli.command {
        None => daemon::run(),
        Some(cmd) => client::run(cmd),
    }
}
