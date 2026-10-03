//! Private command-line adapters around the typed library API.

mod args;
mod execution;
mod io;

use anyhow::Result;
use clap::Parser;

use args::Config;

pub(crate) async fn run() -> Result<()> {
    execution::run(Config::parse().command).await
}
