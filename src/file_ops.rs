use crate::error::Result;
use crate::ltfs::LtfsDirectAccess;
use std::path::PathBuf;
use tracing::{info, debug, error};
use walkdir::WalkDir;
use tokio::fs;
use indicatif::{ProgressBar, ProgressStyle};

/// Copy files or folders to tape
pub async fn copy_to_tape(
    source: PathBuf,
    device: String,
    destination: PathBuf,
    force: bool,
    verify: bool,
    progress: bool,
) -> Result<()> {
    info!("Starting copy operation: {:?} -> {}:{:?}", source, device, destination);
    
    // Check if source path exists
    if !source.exists() {
        return Err(crate::error::RustLtfsError::file_operation(
            format!("Source path does not exist: {:?}", source)
        ));
    }
    
    // User confirmation (unless --force is used)
    if !force {
        if !confirm_operation(&source, &device, &destination)? {
            info!("Operation cancelled by user");
            return Ok(());
        }
    }
    
    // Execute different operations based on source path type
    if source.is_file() {
        copy_file_to_tape(source, device, destination, verify, progress).await
    } else if source.is_dir() {
        copy_directory_to_tape(source, device, destination, verify, progress).await
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

/// Copy a single file to tape
async fn copy_file_to_tape(
    source: PathBuf,
    device: String,
    destination: PathBuf,
    verify: bool,
    progress: bool,
) -> Result<()> {
    debug!("Copying file: {:?} -> {}:{:?}", source, device, destination);
    
    let file_size = fs::metadata(&source).await?.len();
    info!("File size: {} bytes", file_size);
    
    let progress_bar = if progress {
        Some(setup_progress_bar(file_size)?)
    } else {
        None
    };
    
    // Create LTFS direct access instance
    let mut ltfs = LtfsDirectAccess::new(device.clone());
    ltfs.initialize()?;
    
    // Use LTFS direct write
    match ltfs.write_file_direct(&source, &destination).await {
        Ok(_) => {
            if let Some(pb) = progress_bar {
                pb.finish_with_message("File write completed");
            }
            info!("File successfully written to tape");
        }
        Err(e) => {
            if let Some(pb) = progress_bar {
                pb.abandon_with_message("Write failed");
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

/// Copy directory to tape
async fn copy_directory_to_tape(
    source: PathBuf,
    device: String,
    destination: PathBuf,
    verify: bool,
    progress: bool,
) -> Result<()> {
    debug!("Copying directory: {:?} -> {}:{:?}", source, device, destination);
    
    // Traverse directory to get all files
    let mut files = Vec::new();
    let mut total_size = 0u64;
    
    for entry in WalkDir::new(&source) {
        let entry = entry.map_err(|e| crate::error::RustLtfsError::file_operation(
            format!("Directory traversal error: {}", e)
        ))?;
        
        if entry.file_type().is_file() {
            let metadata = fs::metadata(entry.path()).await?;
            total_size += metadata.len();
            
            // Calculate relative path for tape target path
            let relative_path = entry.path().strip_prefix(&source)
                .map_err(|e| crate::error::RustLtfsError::file_operation(
                    format!("Path calculation error: {}", e)
                ))?;
            let tape_file_path = destination.join(relative_path);
            
            files.push((entry.path().to_path_buf(), tape_file_path, metadata.len()));
        }
    }
    
    info!("Found {} files, total size: {} bytes", files.len(), total_size);
    
    let progress_bar = if progress {
        Some(setup_progress_bar(total_size)?)
    } else {
        None
    };
    
    // Create LTFS direct access instance
    let mut ltfs = LtfsDirectAccess::new(device.clone());
    ltfs.initialize()?;
    
    // Batch write files to tape
    let mut processed_size = 0u64;
    let files_count = files.len();
    for (file_path, tape_path, file_size) in files {
        debug!("Processing file: {:?} -> {:?} ({} bytes)", file_path, tape_path, file_size);
        
        match ltfs.write_file_direct(&file_path, &tape_path).await {
            Ok(_) => {
                processed_size += file_size;
                if let Some(pb) = &progress_bar {
                    pb.set_position(processed_size);
                }
                info!("File write successful: {:?}", file_path);
            }
            Err(e) => {
                error!("File write failed: {:?} - {}", file_path, e);
                if let Some(pb) = progress_bar {
                    pb.abandon_with_message("Batch write failed");
                }
                return Err(e);
            }
        }
        
        if verify {
            debug!("Verifying file: {:?}", file_path);
            verify_file_on_tape(&mut ltfs, &file_path, &tape_path).await?;
        }
    }
    
    if let Some(pb) = progress_bar {
        pb.finish_with_message("Directory write completed");
    }
    
    info!("Directory successfully written to tape, processed {} files", files_count);
    Ok(())
}

/// User operation confirmation
fn confirm_operation(source: &PathBuf, device: &str, destination: &PathBuf) -> Result<bool> {
    println!("About to perform the following operation:");
    println!("  Source path: {:?}", source);
    println!("  Tape device: {}", device);
    println!("  Target path: {:?}", destination);
    println!();
    print!("Confirm to continue? (y/N): ");
    
    use std::io::{self, Write};
    io::stdout().flush().map_err(|e| crate::error::RustLtfsError::Io(e))?;
    
    let mut input = String::new();
    io::stdin().read_line(&mut input).map_err(|e| crate::error::RustLtfsError::Io(e))?;
    
    let input = input.trim().to_lowercase();
    Ok(input == "y" || input == "yes")
}

/// Setup progress bar
fn setup_progress_bar(total_size: u64) -> Result<ProgressBar> {
    let pb = ProgressBar::new(total_size);
    pb.set_style(
        ProgressStyle::default_bar()
            .template("{spinner:.green} [{elapsed_precise}] [{wide_bar:.cyan/blue}] {bytes}/{total_bytes} ({eta})")
            .map_err(|e| crate::error::RustLtfsError::system(format!("Progress bar style error: {}", e)))?
            .progress_chars("#>-")
    );
    Ok(pb)
}