mod cli;
mod commands;
mod error;
mod logger;
mod ltfs_index;
mod scsi;
mod tape_ops;

use crate::cli::{Cli, Commands};
use crate::error::Result;
use tracing::{debug, error, info};

#[tokio::main]
async fn main() -> Result<()> {
    let args = Cli::parse_args();

    // Initialize logging system
    logger::init(args.verbose)?;

    debug!("RustLTFS CLI starting");

    match run(args).await {
        Ok(_) => {
            info!("Operation completed successfully");
            Ok(())
        }
        Err(e) => {
            error!("Operation failed: {}", e);
            std::process::exit(1);
        }
    }
}

async fn run(args: Cli) -> Result<()> {
    match args.command {
        Commands::Write {
            source,
            device,
            destination,
            verify,
            progress,
        } => commands::write::execute(source, device, destination, verify, progress).await,

        Commands::Read { device, source } => commands::read::execute(device, source).await,

        Commands::Space { device, detailed } => commands::space::execute(device, detailed).await,
    }
}
