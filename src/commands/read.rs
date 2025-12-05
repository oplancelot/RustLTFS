//! Read Command Handler
//!
//! Handles the `read` subcommand for reading tape index and listing contents.

use crate::error::Result;
use crate::tape_ops;
use std::path::PathBuf;
use tracing::info;

pub async fn execute(device: String, source: Option<PathBuf>) -> Result<()> {
    info!("Starting read operation: {} -> {:?}", device, source);

    // Create tape operations instance (never skip index for read operations)
    let mut ops = tape_ops::TapeOperations::new(&device);

    // Initialize tape device with auto index reading
    ops.initialize(Some(tape_ops::core::OperationType::Read))
        .await?;

    match source {
        None => {
            // Display complete directory tree structure
            info!("Displaying tape directory tree structure");

            // Show index statistics first
            if let Some(stats) = ops.get_index_statistics() {
                println!("\n📊 Tape Index Information:");
                println!("  • Volume UUID: {}", stats.volume_uuid);
                println!("  • Generation Number: {}", stats.generation_number);
                println!("  • Update Time: {}", stats.update_time);
                println!("  • Total Files: {}", stats.total_files);
            }

            // Display complete directory tree
            ops.print_directory_tree();
        }
        Some(src_path) => {
            // List specific directory contents
            info!("Listing directory contents: {:?}", src_path);

            ops.list_directory_contents(&src_path.to_string_lossy())?;
        }
    }

    Ok(())
}
