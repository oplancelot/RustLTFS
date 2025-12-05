//! Space Command Handler
//!
//! Handles the `space` subcommand for querying tape capacity information.

use crate::error::Result;
use crate::tape_ops;
use tracing::info;

pub async fn execute(device: String, detailed: bool) -> Result<()> {
    info!("Getting tape space information: {}", device);

    // Create tape operations instance (never offline for space command)
    let mut ops = tape_ops::TapeOperations::new(&device);

    // Initialize for space operation
    ops.initialize(Some(tape_ops::core::OperationType::Space))
        .await?;

    // Get space information
    let space_info = ops.get_tape_capacity_info().await?;

    println!("📦 Tape Space Information:");
    println!(
        "  Total Capacity: {} GB",
        space_info.total_capacity / (1024 * 1024 * 1024)
    );
    println!(
        "  Used Space: {} GB",
        space_info.used_space / (1024 * 1024 * 1024)
    );
    println!(
        "  Available Space: {} GB",
        space_info.available_space / (1024 * 1024 * 1024)
    );

    if detailed {
        println!("  Detailed information would be shown here");
    }

    Ok(())
}
