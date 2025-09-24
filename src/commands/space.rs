use crate::error::Result;
use crate::tape_ops;
use tracing::info;

pub async fn handle_space_command(
    device: String,
    skip_index: bool,
    detailed: bool
) -> Result<()> {
    info!("Getting tape space information: {}", device);
    
    // Create tape operations instance
    let mut ops = tape_ops::TapeOperations::new(&device, skip_index);
    
    // Get space information
    ops.get_tape_space_info(detailed).await
}