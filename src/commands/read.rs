use crate::error::Result;
use crate::tape_ops;
use std::path::PathBuf;
use tracing::{info, warn};

pub async fn handle_read_command(
    device: String,
    source: Option<PathBuf>,
    destination: Option<PathBuf>,
    skip_index: bool,
    index_file: Option<PathBuf>,
    verify: bool,
    _lines: usize,
    _detailed: bool
) -> Result<()> {
    info!("Starting read operation: {} -> {:?}", device, source);
    
    // Create tape operations instance
    let mut ops = tape_ops::TapeOperations::new(&device, skip_index);
    
    // Initialize tape device with auto index reading (may fail for non-existent devices)
    let device_initialized = match ops.initialize().await {
        Ok(_) => {
            info!("Device initialized successfully");
            true
        }
        Err(e) => {
            warn!("Device initialization failed: {}", e);
            
            // Provide helpful suggestions for common errors
            if e.to_string().contains("Windows error code 0x00000002") {
                tracing::error!("Device not found: {}", device);
                info!("🔍 Suggestions:");
                info!("  1. Check if a tape drive is connected to your system");
                info!("  2. Try different device paths: \\\\.\\TAPE0, \\\\.\\TAPE1, etc.");
                info!("  3. Check device status: rustltfs.exe device {} --status --detailed", device);
                info!("  4. Use --skip-index option for offline mode: rustltfs.exe read --tape {} --skip-index", device);
            } else if e.to_string().contains("No tape loaded") {
                tracing::error!("No tape cartridge detected in drive: {}", device);
                info!("🔍 Suggestions:");
                info!("  1. Insert a tape cartridge into the drive");
                info!("  2. Wait for the drive to recognize the tape");
                info!("  3. Check device status: rustltfs.exe device {} --status --detailed", device);
            } else if e.to_string().contains("Direct block read operation failed") {
                tracing::error!("Failed to read LTFS index from tape: {}", device);
                info!("🔍 Possible causes: blank tape, incorrect position, hardware issue, SCSI problem");
                info!("🔧 Try: --skip-index option, full diagnostics, or --index-file <path>");
            }
            
            // Continue with offline operation if index file is provided
            if index_file.is_some() {
                info!("Continuing with offline operation using index file");
                false
            } else {
                return Err(e); // Fail if no index file provided
            }
        }
    };
    
    // Load index from file if specified
    if let Some(ref index_path) = index_file {
        ops.load_index_from_file(index_path).await?;
        
        // Auto save index to current directory after successful index loading
        if !skip_index {
            let timestamp = chrono::Utc::now().format("%Y%m%d_%H%M%S").to_string();
            let index_filename = format!("LTFSIndex_Load_{}.schema", timestamp);
            info!("Auto saving index file to current directory: {}", index_filename);
            
            match ops.save_index_to_file(&std::path::PathBuf::from(&index_filename)).await {
                Ok(_) => {
                    println!("✅ Index file automatically saved: {}", index_filename);
                    info!("Index file saved successfully: {}", index_filename);
                }
                Err(e) => {
                    warn!("Failed to save index file: {}", e);
                    println!("⚠️  Index file save failed: {}", e);
                }
            }
        }
    } else if !device_initialized {
        return Err(crate::error::RustLtfsError::cli_error(
            "Neither device initialization nor index file loading succeeded".to_string()
        ));
    }
    
    // Auto save index to current directory if loaded from tape
    if device_initialized && !skip_index && index_file.is_none() {
        let timestamp = chrono::Utc::now().format("%Y%m%d_%H%M%S").to_string();
        let index_filename = format!("LTFSIndex_Load_{}.schema", timestamp);
        info!("Auto saving index file loaded from tape to current directory: {}", index_filename);
        
        match ops.save_index_to_file(&std::path::PathBuf::from(&index_filename)).await {
            Ok(_) => {
                println!("✅ Index file automatically saved: {}", index_filename);
                info!("Index file saved successfully: {}", index_filename);
            }
            Err(e) => {
                warn!("Failed to save index file: {}", e);
                println!("⚠️  Index file save failed: {}", e);
            }
        }
    }
    
    // Execute different read operations based on parameters
    match (source, destination) {
        (None, None) => {
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
            ops.print_directory_tree()?;
        }
        (Some(src_path), None) => {
            // Download file or directory to current directory
            info!("Downloading from tape: {:?} -> current directory", src_path);
            
            let current_dir = std::env::current_dir().map_err(|e| 
                crate::error::RustLtfsError::cli_error(format!("Failed to get current directory: {}", e))
            )?;
            
            // Extract files to current directory
            let extract_result = ops.extract_from_tape(
                &src_path.to_string_lossy(),
                &current_dir,
                verify
            ).await?;
            
            println!("✅ Download Completed:");
            println!("  Files Downloaded: {}", extract_result.files_extracted);
            println!("  Directories Created: {}", extract_result.directories_created);
            println!("  Total Bytes: {} bytes", extract_result.total_bytes);
            println!("  Destination: {}", current_dir.display());
            
            if verify {
                println!("  Verification Status: {}", if extract_result.verification_passed {
                    "✅ Passed"
                } else {
                    "❌ Failed"
                });
            }
        }
        (Some(src_path), Some(dest_path)) => {
            // Extract files to local
            info!("Extracting files to local: {:?} -> {:?}", src_path, dest_path);
            
            // Parse source path, support file and directory extraction
            let extract_result = ops.extract_from_tape(
                &src_path.to_string_lossy(),
                &dest_path,
                verify
            ).await?;
            
            println!("✅ Extraction Completed:");
            println!("  Files Extracted: {}", extract_result.files_extracted);
            println!("  Directories Created: {}", extract_result.directories_created);
            println!("  Total Bytes: {} bytes", extract_result.total_bytes);
            
            if verify {
                println!("  Verification Status: {}", if extract_result.verification_passed {
                    "✅ Passed"
                } else {
                    "❌ Failed"
                });
            }
        }
        (None, Some(_)) => {
            return Err(crate::error::RustLtfsError::cli_error(
                "Source path must be specified to extract files".to_string()
            ));
        }
    }
    
    Ok(())
}