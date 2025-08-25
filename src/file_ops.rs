use crate::error::Result;
use crate::ltfs::LtfsDirectAccess;
use std::path::PathBuf;
use tracing::{info, debug, error};
use tokio::fs;

/// Write files or folders to tape
pub async fn write_to_tape(
    source: PathBuf,
    device: String,
    destination: PathBuf,
    _force: bool,
    verify: bool,
    progress: bool,
) -> Result<()> {
    info!("Starting write operation: {:?} -> {}:{:?}", source, device, destination);
    
    // Check if source path exists
    if !source.exists() {
        return Err(crate::error::RustLtfsError::file_operation(
            format!("Source path does not exist: {:?}", source)
        ));
    }
    
    // Execute different operations based on source path type
    if source.is_file() {
        write_file_to_tape(source, device, destination, verify, progress).await
    } else if source.is_dir() {
        Err(crate::error::RustLtfsError::file_operation(
            "Directory copying not yet implemented - use write command instead"
        ))
    } else {
        Err(crate::error::RustLtfsError::file_operation(
            "Unsupported source path type"
        ))
    }
}

/// Read files from tape to local storage
pub async fn read_from_tape(
    device: String,
    source: PathBuf,
    destination: PathBuf,
    verify: bool,
) -> Result<()> {
    info!("Reading from tape: {}:{:?} -> {:?}", device, source, destination);
    
    // Create LTFS direct access instance
    let mut ltfs = LtfsDirectAccess::new(device.clone());
    ltfs.initialize()?;
    
    // Read file from tape
    ltfs.read_file_direct(&source, &destination).await?;
    
    if verify {
        info!("Starting to verify read data...");
        // Verify the read file
        let local_data = fs::read(&destination).await?;
        
        // Create temporary file for secondary read verification
        let temp_dir = std::env::temp_dir();
        let temp_file = temp_dir.join(format!("rustltfs_read_verify_{}", 
            destination.file_name().unwrap().to_string_lossy()));
        
        ltfs.read_file_direct(&source, &temp_file).await?;
        let verify_data = fs::read(&temp_file).await?;
        
        if local_data == verify_data {
            info!("Read data verification passed");
        } else {
            return Err(crate::error::RustLtfsError::verification(
                "Read data verification failed"
            ));
        }
        
        // Clean up temporary file
        let _ = fs::remove_file(temp_file).await;
    }
    
    info!("File read completed: {:?}", destination);
    Ok(())
}

/// Write a single file to tape
async fn write_file_to_tape(
    source: PathBuf,
    device: String,
    destination: PathBuf,
    verify: bool,
    progress: bool,
) -> Result<()> {
    debug!("Writing file: {:?} -> {}:{:?}", source, device, destination);
    
    let file_size = fs::metadata(&source).await?.len();
    info!("File size: {} bytes", file_size);
    
    if progress {
        info!("Progress tracking enabled");
    }
    
    // Create LTFS direct access instance
    let mut ltfs = LtfsDirectAccess::new(device.clone());
    ltfs.initialize()?;
    
    // Use LTFS direct write
    match ltfs.write_file_direct(&source, &destination).await {
        Ok(_) => {
            if progress {
                info!("File write completed");
            }
            info!("File successfully written to tape");
        }
        Err(e) => {
            if progress {
                error!("Write failed: {}", e);
            }
            return Err(e);
        }
    }
    
    if verify {
        info!("Starting to verify written data...");
        verify_file_on_tape(&mut ltfs, &source, &destination).await?;
        info!("Data verification passed");
    }
    
    Ok(())
}

/// Verify files on tape
async fn verify_file_on_tape(
    ltfs: &mut LtfsDirectAccess, 
    original_path: &PathBuf, 
    tape_path: &PathBuf
) -> Result<()> {
    debug!("Verifying file: {:?} <-> {:?}", original_path, tape_path);
    
    // Read original file
    let original_data = fs::read(original_path).await?;
    
    // Create temporary directory for verification
    let temp_dir = std::env::temp_dir();
    let temp_file = temp_dir.join(format!("rustltfs_verify_{}", 
        original_path.file_name().unwrap().to_string_lossy()));
    
    // Read file from tape
    ltfs.read_file_direct(tape_path, &temp_file).await?;
    
    // Read verification file
    let tape_data = fs::read(&temp_file).await?;
    
    // Compare file contents
    if original_data == tape_data {
        debug!("File verification successful");
    } else {
        return Err(crate::error::RustLtfsError::verification(
            "File contents do not match"
        ));
    }
    
    // Clean up temporary file
    let _ = fs::remove_file(temp_file).await;
    
    Ok(())
}