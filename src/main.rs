mod cli;
mod error;
mod logger;
mod scsi;
mod tape;
mod ltfs;
mod ltfs_index;
mod file_ops;
mod display;

#[cfg(test)]
mod tests;

use crate::cli::{Cli, Commands};
use crate::error::Result;
use tracing::{info, error};

#[tokio::main]
async fn main() -> Result<()> {
    let args = Cli::parse_args();
    
    // Initialize logging system
    logger::init(args.verbose)?;
    
    info!("RustLTFS CLI starting");
    
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
        Commands::Copy { 
            source, 
            device, 
            destination, 
            force, 
            verify, 
            progress 
        } => {
            info!("Starting copy operation: {:?} -> {}:{:?}", source, device, destination);
            file_ops::copy_to_tape(source, device, destination, force, verify, progress).await
        }
        
        Commands::List { detailed } => {
            info!("Listing tape devices");
            tape::list_devices(detailed).await
        }
        
        Commands::Info { device } => {
            info!("Getting device information: {}", device);
            tape::get_device_info(device).await
        }
        
        Commands::Read { 
            device, 
            source, 
            destination, 
            verify,
            lines
        } => {
            info!("Smart reading from tape: {}:{:?}", device, source);
            
            // Create LTFS direct access instance
            let mut ltfs = ltfs::create_ltfs_access(device).await?;
            
            // Check path type to determine operation mode
            match ltfs.check_path_type(&source.to_string_lossy())? {
                ltfs_index::PathType::Directory(_) => {
                    // Directory listing mode (ignore destination parameter)
                    info!("Listing directory contents: {}", source.display());
                    let entries = ltfs.list_directory(&source.to_string_lossy())?;
                    display::display_directory_listing(entries);
                },
                ltfs_index::PathType::File(file) => {
                    match destination {
                        None => {
                            // Display file content mode
                            info!("Displaying file content: {}", file.name);
                            display::display_file_content(&ltfs, &file, lines).await?;
                        },
                        Some(dest_path) => {
                            // Copy file to local mode
                            info!("Copying file to local: {} -> {:?}", file.name, dest_path);
                            ltfs.read_file_to_local(&file, &dest_path, verify).await?;
                            println!("File copied successfully: {} -> {}", file.name, dest_path.display());
                        }
                    }
                },
                ltfs_index::PathType::NotFound => {
                    return Err(crate::error::RustLtfsError::file_operation(
                        format!("Path not found on tape: {}", source.display())
                    ));
                }
            }
            
            Ok(())
        }
        
        Commands::Status { device } => {
            info!("Checking device status: {}", device);
            tape::get_device_status(device).await
        }
    }
}