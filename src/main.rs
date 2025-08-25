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
            
            // Initialize tape device
            ops.initialize().await?;
            
            // Load index from file if specified
            if let Some(ref index_path) = index_file {
                ops.load_index_from_file(index_path).await?;
            }
            
            // Execute write operation
            if source.is_dir() {
                ops.write_directory_to_tape(&source, &destination.to_string_lossy()).await?;
            } else {
                ops.write_file_to_tape(&source, &destination.to_string_lossy()).await?;
            }
            
            info!("Write operation completed");
            
            // Auto update LTFS index
            info!("Auto updating LTFS index...");
            ops.update_index_on_tape().await?;
            info!("Index update completed");
            
            // Save index to local file
            if !skip_index {
                let timestamp = chrono::Utc::now().format("%Y%m%d_%H%M%S").to_string();
                let index_filename = format!("LTFSIndex_Load_{}.schema", timestamp);
                info!("Saving index to local file: {}", index_filename);
                ops.save_index_to_file(&std::path::PathBuf::from(&index_filename)).await?;
                info!("Index file saved successfully: {}", index_filename);
            }
            
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
                    info!("自动保存索引文件到当前目录: {}", index_filename);
                    
                    match ops.save_index_to_file(&std::path::PathBuf::from(&index_filename)).await {
                        Ok(_) => {
                            println!("✅ 索引文件已自动保存: {}", index_filename);
                            info!("Index file saved successfully: {}", index_filename);
                        }
                        Err(e) => {
                            warn!("Failed to save index file: {}", e);
                            println!("⚠️  索引文件保存失败: {}", e);
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
                info!("自动保存从磁带读取的索引文件到当前目录: {}", index_filename);
                
                match ops.save_index_to_file(&std::path::PathBuf::from(&index_filename)).await {
                    Ok(_) => {
                        println!("✅ 索引文件已自动保存: {}", index_filename);
                        info!("Index file saved successfully: {}", index_filename);
                    }
                    Err(e) => {
                        warn!("Failed to save index file: {}", e);
                        println!("⚠️  索引文件保存失败: {}", e);
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
    }
}