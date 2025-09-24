use crate::error::Result;
use crate::tape_ops;
use tracing::info;

pub async fn handle_update_index_command(
    device: String,
    _force: bool,
    _detailed: bool
) -> Result<()> {
    info!("Updating LTFS index on tape: {}", device);
    
    // Create tape operations instance
    let mut ops = tape_ops::TapeOperations::new(&device, false);
    
    // Initialize to load current index
    ops.initialize().await?;
    
    // Execute manual index update operation
    match ops.update_index_on_tape_manual_new() {
        Ok(()) => {
            println!("✅ LTFS index updated on tape successfully");
            Ok(())
        }
        Err(e) => {
            tracing::error!("Failed to update LTFS index on tape: {}", e);
            Err(e)
        }
    }
}