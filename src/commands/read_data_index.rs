use crate::error::Result;
use crate::tape_ops;
use std::path::PathBuf;
use tracing::info;

pub async fn handle_read_data_index_command(
    device: String,
    output: Option<PathBuf>,
    _detailed: bool
) -> Result<()> {
    info!("Reading data partition index from tape: {}", device);
    
    // Create tape operations instance  
    let mut ops = tape_ops::TapeOperations::new(&device, false);
    
    // Execute read data index operation
    match ops.read_data_index_from_tape_new(output.map(|p| p.to_string_lossy().to_string())) {
        Ok(_) => {
            println!("✅ Data partition index read from tape successfully");
            Ok(())
        }
        Err(e) => {
            tracing::error!("Failed to read data partition index from tape: {}", e);
            Err(e)
        }
    }
}