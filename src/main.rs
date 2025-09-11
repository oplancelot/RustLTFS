mod cli;
mod error;
mod logger;
mod scsi;
mod tape;
mod ltfs;
mod ltfs_index;
mod file_ops;
mod display;
mod tape_ops;


use crate::cli::{Cli, Commands};
use crate::error::Result;
use tracing::{info, error, warn};

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
        Commands::Write { 
            source,
            device, 
            destination,
            skip_index,
            index_file, 
            force,
            verify,
            progress
        } => {
            info!("Starting write operation: {:?} -> {}:{:?}", source, device, destination);
            
            // Create tape operations instance
            let mut ops = tape_ops::TapeOperations::new(&device, skip_index);
            
            // Configure write options (对应LTFSCopyGUI的各种设置)
            let mut write_options = tape_ops::WriteOptions::default();
            write_options.overwrite = force;  // Use force flag as overwrite option
            write_options.verify = verify;
            write_options.hash_on_write = verify; // Calculate hash when verify is enabled
            
            ops.set_write_options(write_options);
            
            // Display progress if requested
            if progress {
                println!("🔧 Initializing tape device: {}", device);
            }
            
            // Initialize tape device with proper error handling
            let device_initialized = match ops.initialize().await {
                Ok(_) => {
                    if progress {
                        println!("✅ Device initialized successfully");
                    }
                    info!("Device initialized successfully for write operation");
                    true
                }
                Err(e) => {
                    error!("Device initialization failed: {}", e);
                    
                    // Provide helpful error messages for write operations
                    if e.to_string().contains("No tape loaded") {
                        println!("❌ No tape cartridge detected in drive: {}", device);
                        println!("💡 Insert a tape cartridge and try again");
                        return Err(e);
                    } else if e.to_string().contains("Write protected") {
                        println!("❌ Tape is write-protected");
                        println!("💡 Remove write protection or use a different tape");
                        return Err(e);
                    } else {
                        println!("❌ Device initialization failed: {}", e);
                        if !skip_index && index_file.is_some() {
                            println!("💡 Trying offline mode with provided index file...");
                        } else {
                            return Err(e);
                        }
                    }
                    false
                }
            };
            
            // Load index from file if specified, or read from tape
            if let Some(ref index_path) = index_file {
                if progress {
                    println!("📂 Loading index from file: {:?}", index_path);
                }
                ops.load_index_from_file(index_path).await?;
                if progress {
                    println!("✅ Index loaded from file");
                }
            } else if device_initialized && !skip_index {
                if progress {
                    println!("📼 Reading index from tape...");
                }
                // Index was already loaded during initialization
                if progress {
                    println!("✅ Index read from tape");
                }
            }
            
            // Display write operation details
            println!("\n🚀 Starting Write Operation");
            println!("  Source: {:?}", source);
            println!("  Device: {}", device);
            println!("  Target: {:?}", destination);
            if force {
                println!("  Mode: Overwrite existing files");
            } else {
                println!("  Mode: Skip existing files");
            }
            if verify {
                println!("  Verification: Enabled (with hash checking)");
            }
            
            // Check if source exists
            if !source.exists() {
                return Err(error::RustLtfsError::file_operation(
                    format!("Source path does not exist: {:?}", source)
                ));
            }
            
            // Display current write progress if requested
            if progress {
                let write_progress = ops.get_write_progress();
                println!("\n📊 Initial Status:");
                println!("  Files processed: {}", write_progress.current_files_processed);
                println!("  Bytes processed: {}", rust_ltfs::utils::format_bytes(write_progress.current_bytes_processed));
            }
            
            // Execute write operation
            let write_start = std::time::Instant::now();
            
            if source.is_dir() {
                if progress {
                    println!("\n📁 Writing directory to tape...");
                }
                ops.write_directory_to_tape(&source, &destination.to_string_lossy()).await?;
            } else {
                if progress {
                    println!("\n📄 Writing file to tape...");
                }
                ops.write_file_to_tape(&source, &destination.to_string_lossy()).await?;
            }
            
            let write_duration = write_start.elapsed();
            
            // Show final progress
            if progress {
                let final_progress = ops.get_write_progress();
                println!("\n✅ Write Operation Completed");
                println!("  Files written: {}", final_progress.current_files_processed);
                println!("  Bytes written: {}", rust_ltfs::utils::format_bytes(final_progress.current_bytes_processed));
                println!("  Duration: {:.2}s", write_duration.as_secs_f64());
                
                if final_progress.current_bytes_processed > 0 && write_duration.as_secs() > 0 {
                    let speed = final_progress.current_bytes_processed as f64 / write_duration.as_secs_f64();
                    println!("  Average Speed: {}/s", rust_ltfs::utils::format_bytes(speed as u64));
                }
            }
            
            info!("Write operation completed in {:.2}s", write_duration.as_secs_f64());
            
            // Auto update LTFS index (对应LTFSCopyGUI的自动索引更新)
            if device_initialized && !skip_index {
                if progress {
                    println!("\n🔄 Updating LTFS index...");
                }
                info!("Auto updating LTFS index...");
                
                match ops.update_index_on_tape().await {
                    Ok(_) => {
                        if progress {
                            println!("✅ Index updated successfully");
                        }
                        info!("Index update completed");
                    }
                    Err(e) => {
                        warn!("Index update failed: {}", e);
                        println!("⚠️  Index update failed: {}", e);
                        println!("💡 Manual index update may be required");
                    }
                }
            }
            
            // Save index to local file for backup
            if device_initialized && !skip_index {
                let timestamp = chrono::Utc::now().format("%Y%m%d_%H%M%S").to_string();
                let index_filename = format!("LTFSIndex_Write_{}.schema", timestamp);
                
                if progress {
                    println!("\n💾 Saving index backup: {}", index_filename);
                }
                
                match ops.save_index_to_file(&std::path::PathBuf::from(&index_filename)).await {
                    Ok(_) => {
                        if progress {
                            println!("✅ Index backup saved");
                        }
                        info!("Index backup saved: {}", index_filename);
                    }
                    Err(e) => {
                        warn!("Index backup failed: {}", e);
                        println!("⚠️  Index backup failed: {}", e);
                    }
                }
            }
            
            println!("\n🎉 Write operation completed successfully!");
            Ok(())
        }
        
        Commands::Read { 
            device,
            source,
            destination,
            skip_index,
            index_file,
            verify,
            lines,
            detailed 
        } => {
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
                        error!("Device not found: {}", device);
                        info!("🔍 Suggestions:");
                        info!("  1. Check if a tape drive is connected to your system");
                        info!("  2. Try different device paths: \\\\.\\TAPE0, \\\\.\\TAPE1, etc.");
                        info!("  3. Run diagnostics: rustltfs.exe diagnose --tape {} --detailed --test-read", device);
                        info!("  4. Use --skip-index option for offline mode: rustltfs.exe read --tape {} --skip-index", device);
                    } else if e.to_string().contains("No tape loaded") {
                        error!("No tape cartridge detected in drive: {}", device);
                        info!("🔍 Suggestions:");
                        info!("  1. Insert a tape cartridge into the drive");
                        info!("  2. Wait for the drive to recognize the tape");
                        info!("  3. Run diagnostics: rustltfs.exe diagnose --tape {} --detailed", device);
                    } else if e.to_string().contains("Direct block read operation failed") {
                        error!("Failed to read LTFS index from tape: {}", device);
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
                    // List root directory content
                    info!("Listing tape root directory content");
                    if let Some(stats) = ops.get_index_statistics() {
                        println!("\n📊 Tape Index Information:");
                        println!("  • Volume UUID: {}", stats.volume_uuid);
                        println!("  • Generation Number: {}", stats.generation_number);
                        println!("  • Update Time: {}", stats.update_time);
                        println!("  • Total Files: {}", stats.total_files);
                    }
                }
                (Some(src_path), None) => {
                    // Display file or directory content
                    info!("Displaying tape content: {:?}", src_path);
                    
                    // Parse tape path and display content
                    if let Some(content) = ops.list_path_content(&src_path.to_string_lossy()).await? {
                        match content {
                            tape_ops::PathContent::Directory(entries) => {
                                println!("\n📁 Directory Content: {}", src_path.display());
                                for entry in entries {
                                    let type_icon = if entry.is_directory { "📁" } else { "📄" };
                                    let size_info = if entry.is_directory {
                                        format!("({} items)", entry.file_count.unwrap_or(0))
                                    } else {
                                        format!("({} bytes)", entry.size.unwrap_or(0))
                                    };
                                    println!("  {} {} {}", type_icon, entry.name, size_info);
                                    
                                    if detailed {
                                        println!("    Created: {}", entry.created_time.as_deref().unwrap_or("Unknown"));
                                        println!("    Modified: {}", entry.modified_time.as_deref().unwrap_or("Unknown"));
                                        if let Some(uid) = entry.file_uid {
                                            println!("    File UID: {}", uid);
                                        }
                                    }
                                }
                            }
                            tape_ops::PathContent::File(file_info) => {
                                println!("\n📄 File Information: {}", src_path.display());
                                println!("  Size: {} bytes", file_info.size);
                                println!("  Created: {}", file_info.created_time.as_deref().unwrap_or("Unknown"));
                                println!("  Modified: {}", file_info.modified_time.as_deref().unwrap_or("Unknown"));
                                println!("  File UID: {}", file_info.file_uid);
                                
                                // Display file content preview
                                if file_info.size <= 1024 * 1024 && lines > 0 { // Preview files under 1MB only
                                    println!("\n📖 File Content Preview (first {} lines):", lines);
                                    if let Ok(preview) = ops.preview_file_content(file_info.file_uid, lines).await {
                                        println!("{}", preview);
                                    }
                                }
                            }
                        }
                    } else {
                        println!("❌ Path does not exist or is not accessible: {}", src_path.display());
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
        
        Commands::ViewIndex { 
            index_file, 
            detailed, 
            export_format, 
            output 
        } => {
            info!("Viewing LTFS index file: {:?}", index_file);
            tape_ops::IndexViewer::handle_view_index_command(
                &index_file,
                detailed,
                export_format,
                output.as_deref(),
            ).await
        }
        
        Commands::List { detailed } => {
            info!("Listing tape devices");
            tape::list_devices(detailed).await
        }
        
        Commands::Info { device } => {
            info!("Getting device information: {}", device);
            tape::get_device_info(device).await
        }
        
        Commands::Status { device } => {
            info!("Checking device status: {}", device);
            tape::get_device_status(device).await
        }
        
        Commands::Diagnose { device, detailed, test_read } => {
            info!("Starting tape diagnosis: {}", device);
            // Note: diagnose_tape_status method is not needed anymore
            println!("⚠️  Diagnose functionality is not implemented in the new LTFS commands");
            println!("💡 Try using other commands like 'info' or 'status' for device information");
            Ok(())
        }

        Commands::Space { device, skip_index, detailed } => {
            info!("Getting tape space information: {}", device);
            
            // Create tape operations instance
            let mut ops = tape_ops::TapeOperations::new(&device, skip_index);
            
            // Get space information
            ops.get_tape_space_info(detailed).await
        }

        Commands::ReadIndex { device, output, .. } => {
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
                    error!("Failed to read LTFS index from tape: {}", e);
                    Err(e)
                }
            }
        }

        Commands::ReadDataIndex { device, output, .. } => {
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
                    error!("Failed to read data partition index from tape: {}", e);
                    Err(e)
                }
            }
        }

        Commands::UpdateIndex { device, .. } => {
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
                    error!("Failed to update LTFS index on tape: {}", e);
                    Err(e)
                }
            }
        }

        Commands::Mkltfs { 
            device, barcode, volume_label, partition_count, block_size, 
            capacity, p0_size, p1_size, immediate, force, progress 
        } => {
            info!("Starting MKLTFS operation on device: {}", device);
            
            // Create MKLTFS parameters
            let mut params = tape_ops::MkltfsParams::default();
            
            if let Some(ref bc) = barcode {
                params.set_barcode(bc);
            }
            
            if let Some(ref label) = volume_label {
                params.volume_label = label.clone();
            }
            
            params.extra_partition_count = partition_count;
            params.block_length = block_size;
            params.capacity = capacity;
            params.p0_size = p0_size;
            params.p1_size = p1_size;
            params.immediate_mode = immediate;
            
            // Display configuration information
            info!("MKLTFS Configuration:");
            info!("  Device: {}", device);
            if let Some(ref bc) = barcode {
                info!("  Barcode: {}", bc);
            }
            if let Some(ref label) = volume_label {
                info!("  Volume Label: {}", label);
            }
            info!("  Partition Count: {}", partition_count);
            info!("  Block Size: {} bytes", block_size);
            info!("  P0 Size: {}GB", p0_size);
            info!("  P1 Size: {}GB", p1_size);
            
            // Safety confirmation (unless using --force)
            if !force {
                println!("⚠️  WARNING: This operation will completely format the tape and ALL existing data will be lost!");
                println!("📋 MKLTFS Configuration:");
                println!("   Device: {}", device);
                if let Some(ref bc) = barcode {
                    println!("   Barcode: {}", bc);
                }
                if let Some(ref label) = volume_label {
                    println!("   Volume Label: {}", label);
                }
                println!("   Partition Config: {} ({})", 
                    partition_count, 
                    if partition_count > 0 { "Dual Partition" } else { "Single Partition" }
                );
                println!("   P0 Partition: {}GB", p0_size);
                println!("   P1 Partition: {}GB", p1_size);
                println!();
                println!("❓ Confirm to continue? (Type 'yes' to confirm)");
                
                let mut input = String::new();
                std::io::stdin().read_line(&mut input)?;
                
                if input.trim().to_lowercase() != "yes" {
                    info!("User cancelled MKLTFS operation");
                    println!("⛔ Operation cancelled");
                    return Ok(());
                }
            }
            
            // Create tape operations instance
            let mut ops = tape_ops::TapeOperations::new(&device, false);
            
            // Set progress callback (if enabled)
            let progress_callback: Option<tape_ops::MkltfsProgressCallback> = if progress {
                Some(std::sync::Arc::new(|msg: &str| {
                    println!("📈 {}", msg);
                }))
            } else {
                None
            };
            
            let finish_callback: Option<tape_ops::MkltfsFinishCallback> = Some(std::sync::Arc::new(|msg: &str| {
                println!("✅ {}", msg);
            }));
            
            let error_callback: Option<tape_ops::MkltfsErrorCallback> = Some(std::sync::Arc::new(|msg: &str| {
                eprintln!("❌ {}", msg);
            }));
            
            // Execute MKLTFS operation
            match ops.mkltfs(params, progress_callback, finish_callback, error_callback).await {
                Ok(true) => {
                    println!("🎉 MKLTFS operation completed successfully! Tape has been formatted as LTFS");
                    Ok(())
                }
                Ok(false) => {
                    warn!("MKLTFS operation was not completed (possibly offline mode)");
                    Ok(())
                }
                Err(e) => {
                    error!("MKLTFS operation failed: {}", e);
                    Err(e)
                }
            }
        }
    }
}