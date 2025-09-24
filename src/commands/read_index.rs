use crate::error::Result;
use crate::tape_ops;
use std::path::PathBuf;
use tracing::info;

pub async fn handle_read_index_command(
    device: String,
    output: Option<PathBuf>,
    _detailed: bool
) -> Result<()> {
    info!("Reading LTFS index from tape: {}", device);
    
    // Create tape operations instance
    let mut ops = tape_ops::TapeOperations::new(&device, false);
    
    // Initialize and read index using async version
    ops.initialize().await?;
    match ops.read_index_from_tape().await {
        Ok(()) => {
            // Save index to file if requested
            if let Some(output_path) = output {
                let save_path = output_path.to_string_lossy().to_string();
                ops.save_index_to_file(&std::path::Path::new(&save_path)).await?;
                info!("LTFS index saved to: {}", save_path);
            }
            println!("✅ LTFS index read from tape successfully");
            Ok(())
        }
        Err(e) => {
            tracing::error!("Failed to read LTFS index from tape: {}", e);
            Err(e)
        }
    }
}