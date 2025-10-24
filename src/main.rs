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
use crate::error::{Result, RustLtfsError};
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
            progress,
            skip_symlinks,
            parallel,
            speed_limit,
            index_interval,
            exclude_extensions,
            resume,
            dry_run,
            compression_level,
            encrypt,
            checkpoint_interval,
            max_file_size,
            quiet
        } => {
            info!("Starting write operation: {:?} -> {}:{:?}", source, device, destination);
            
            // Handle conflicting options
            if quiet && progress {
                warn!("Both --quiet and --progress specified. Using progress mode.");
            }
            
            // Show dry run warning
            if dry_run && !quiet {
                println!("🔍 DRY RUN MODE - No actual data will be written");
            }
            
            // Create tape operations instance
            let mut ops = tape_ops::TapeOperations::new(&device, skip_index);
            
            // Configure advanced write options (对应LTFSCopyGUI的各种设置)
            let mut write_options = tape_ops::WriteOptions::default();
            write_options.overwrite = force;
            write_options.verify = verify;
            write_options.hash_on_write = verify;
            write_options.skip_symlinks = skip_symlinks;
            write_options.parallel_add = parallel;
            write_options.speed_limit = speed_limit;
            write_options.index_write_interval = (index_interval as u64) * 1024 * 1024 * 1024; // Convert GiB to bytes
            
            // Handle file exclusions
            if let Some(ref extensions) = exclude_extensions {
                let mut excluded = write_options.excluded_extensions.clone();
                for ext in extensions.split(',') {
                    let ext = ext.trim();
                    if !ext.is_empty() {
                        excluded.push(if ext.starts_with('.') { ext.to_string() } else { format!(".{}", ext) });
                    }
                }
                write_options.excluded_extensions = excluded;
            }
            
            let excluded_extensions_copy = write_options.excluded_extensions.clone();
            
            ops.set_write_options(write_options);
            
            // Handle encryption setup
            if encrypt {
                if !quiet {
                    println!("🔐 Encryption enabled - password will be prompted during operation");
                }
                // TODO: Implement encryption key handling
                warn!("Encryption feature is currently under development");
            }
            
            // Handle compression
            if let Some(level) = compression_level {
                if level > 3 {
                    return Err(error::RustLtfsError::parameter_validation("Compression level must be 0-3".to_string()));
                }
                if !quiet {
                    let level_name = match level {
                        0 => "None",
                        1 => "Low", 
                        2 => "Medium",
                        3 => "High",
                        _ => "Unknown"
                    };
                    println!("📦 Compression: {} (level {})", level_name, level);
                }
                // TODO: Implement compression level setting
            }
            
            // Display progress if requested
            let show_progress = progress && !quiet;
            if show_progress {
                println!("🔧 Initializing tape device: {}", device);
            }
            
            // Initialize tape device with proper error handling
            let device_initialized = match ops.initialize().await {
                Ok(_) => {
                    if show_progress {
                        println!("✅ Device initialized successfully");
                    }
                    info!("Device initialized successfully for write operation");
                    true
                }
                Err(e) => {
                    error!("Device initialization failed: {}", e);
                    
                    // Provide helpful error messages for write operations
                    if e.to_string().contains("No tape loaded") {
                        if !quiet {
                            println!("❌ No tape cartridge detected in drive: {}", device);
                            println!("💡 Insert a tape cartridge and try again");
                        }
                        return Err(e);
                    } else if e.to_string().contains("Write protected") {
                        if !quiet {
                            println!("❌ Tape is write-protected");
                            println!("💡 Remove write protection or use a different tape");
                        }
                        return Err(e);
                    } else {
                        if !quiet {
                            println!("❌ Device initialization failed: {}", e);
                            if index_file.is_some() {
                                println!("💡 Trying offline mode with provided index file...");
                            }
                        }
                        if !skip_index && index_file.is_none() {
                            return Err(e);
                        }
                    }
                    false
                }
            };
            
            // Load index from file if specified, or read from tape
            if let Some(ref index_path) = index_file {
                if show_progress {
                    println!("📂 Loading index from file: {:?}", index_path);
                }
                ops.load_index_from_file(index_path).await?;
                if show_progress {
                    println!("✅ Index loaded from file");
                }
            } else if device_initialized && !skip_index {
                if show_progress {
                    println!("📼 Reading index from tape...");
                }
                // Index was already loaded during initialization
                if show_progress {
                    println!("✅ Index read from tape");
                }
            }
            
            // Check if source exists and get size info
            if !source.exists() {
                return Err(error::RustLtfsError::file_operation(
                    format!("Source path does not exist: {:?}", source)
                ));
            }
            
            // Handle max file size check
            if let Some(max_size_gib) = max_file_size {
                let max_size_bytes = (max_size_gib as u64) * 1024 * 1024 * 1024;
                if source.is_file() {
                    let file_size = source.metadata()?.len();
                    if file_size > max_size_bytes {
                        if !quiet {
                            println!("❌ File size ({}) exceeds maximum allowed size ({})", 
                                   rust_ltfs::utils::format_bytes(file_size),
                                   rust_ltfs::utils::format_bytes(max_size_bytes));
                        }
                        return Err(error::RustLtfsError::parameter_validation(
                            format!("File too large: {} > {} GiB", file_size, max_size_gib)
                        ));
                    }
                }
            }
            
            // Handle resume functionality
            if resume {
                if !quiet {
                    println!("🔄 Resume mode enabled - checking for previous operations...");
                }
                // TODO: Implement resume functionality
                warn!("Resume feature is currently under development");
            }
            
            // Display write operation details
            if !quiet {
                println!("\n🚀 Starting Write Operation");
                println!("  Source: {:?}", source);
                println!("  Device: {}", device);
                println!("  Target: {:?}", destination);
                
                let mut options = Vec::new();
                if force { options.push("Overwrite existing files".to_string()); }
                if verify { options.push("Hash verification enabled".to_string()); }
                if skip_symlinks { options.push("Skip symbolic links".to_string()); }
                if parallel { options.push("Parallel processing".to_string()); }
                if let Some(speed) = speed_limit { 
                    options.push(format!("Speed limited to {} MiB/s", speed)); 
                }
                if dry_run { options.push("DRY RUN - no actual writing".to_string()); }
                
                if !options.is_empty() {
                    let options_str: Vec<&str> = options.iter().map(|s| s.as_str()).collect();
                    println!("  Options: {}", options_str.join(", "));
                }
                
                if !excluded_extensions_copy.is_empty() {
                    println!("  Excluded extensions: {}", excluded_extensions_copy.join(", "));
                }
            }
            
            // Display current write progress if requested
            if show_progress {
                let write_progress = ops.get_write_progress();
                println!("\n📊 Initial Status:");
                println!("  Files processed: {}", write_progress.current_files_processed);
                println!("  Bytes processed: {}", rust_ltfs::utils::format_bytes(write_progress.current_bytes_processed));
            }
            
            // Handle dry run mode
            if dry_run {
                if !quiet {
                    println!("\n🔍 DRY RUN: Analyzing source files...");
                    // TODO: Implement dry run analysis
                    println!("✅ Dry run completed - no data was written");
                }
                return Ok(());
            }
            
            // Execute write operation with enhanced progress reporting
            let write_start = std::time::Instant::now();
            let mut checkpoint_count = 0u32;
            
            if source.is_dir() {
                if show_progress {
                    println!("\n📁 Writing directory to tape...");
                }
                
                // Handle checkpoint intervals for large directory operations
                if let Some(interval) = checkpoint_interval {
                    if show_progress {
                        println!("🔖 Checkpoint every {} files", interval);
                    }
                    // TODO: Implement checkpoint logic
                }
                
                ops.write_directory_to_tape(&source, &destination.to_string_lossy()).await?;
            } else {
                if show_progress {
                    println!("\n📄 Writing file to tape...");
                }
                ops.write_file_to_tape_streaming(&source, &destination.to_string_lossy()).await.map(|_| ())?;
            }
            
            let write_duration = write_start.elapsed();
            
            // Show final progress with enhanced reporting
            if !quiet {
                let final_progress = ops.get_write_progress();
                println!("\n✅ Write Operation Completed");
                println!("  Files written: {}", final_progress.current_files_processed);
                println!("  Bytes written: {}", rust_ltfs::utils::format_bytes(final_progress.current_bytes_processed));
                println!("  Duration: {}", rust_ltfs::utils::format_duration(write_duration.as_secs_f64()));
                
                if final_progress.current_bytes_processed > 0 && write_duration.as_secs() > 0 {
                    println!("  Average Speed: {}/s", rust_ltfs::utils::format_speed(
                        final_progress.current_bytes_processed, write_duration.as_secs_f64()));
                }
                
                // Show checkpoint info if used
                if checkpoint_count > 0 {
                    println!("  Checkpoints created: {}", checkpoint_count);
                }
            }
            
            info!("Write operation completed in {:.2}s", write_duration.as_secs_f64());
            
            // Auto update LTFS index (对应LTFSCopyGUI的自动索引更新)
            if device_initialized && !skip_index {
                if progress {
                    println!("\n🔄 Updating LTFS index...");
                }
                info!("Auto updating LTFS index...");
                
                match ops.update_index_on_tape_with_options_dual_partition(false).await {
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
            lines: _,
            detailed: _ 
        } => {
            info!("Starting read operation: {} -> {:?}", device, source);
            
            // Create tape operations instance
            let mut ops = tape_ops::TapeOperations::new(&device, skip_index);
            
            // Initialize tape device with auto index reading (may fail for non-existent devices)
            let device_initialized = match ops.initialize().await {
                Ok(_) => {
                    info!("Device initialized successfully");
                    // Note: LTFS index was already loaded during initialization
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
                        info!("  3. Check device status: rustltfs.exe device {} --status --detailed", device);
                        info!("  4. Use --skip-index option for offline mode: rustltfs.exe read --tape {} --skip-index", device);
                    } else if e.to_string().contains("No tape loaded") {
                        error!("No tape cartridge detected in drive: {}", device);
                        info!("🔍 Suggestions:");
                        info!("  1. Insert a tape cartridge into the drive");
                        info!("  2. Wait for the drive to recognize the tape");
                        info!("  3. Check device status: rustltfs.exe device {} --status --detailed", device);
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
                    ops.print_directory_tree();
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
        
        Commands::ViewIndex { 
            index_file, 
            detailed, 
            export_format, 
            output 
        } => {
            info!("Viewing LTFS index file: {:?}", index_file);
            let format_str = export_format.as_ref().map(|f| match f {
                crate::cli::ExportFormat::Tsv => "list",
                crate::cli::ExportFormat::Json => "json",
                crate::cli::ExportFormat::Xml => "xml",
                crate::cli::ExportFormat::Batch => "batch",
            });
            let output_str = output.as_ref().map(|p| p.to_string_lossy().to_string());
            tape_ops::IndexViewer::handle_view_index_command(
                &index_file.to_string_lossy(),
                Some(detailed),
                format_str,
                output_str.as_deref(),
            )?;
            Ok(())
        }
        
        Commands::Device { action } => {
            use crate::cli::{DeviceAction, ReportType};
            use crate::tape_ops::device_management::{DeviceManager, DeviceReportGenerator};
            use std::fs;
            use std::time::Duration;

            match action {
                DeviceAction::Discover { detailed } => {
                    info!("发现磁带设备...");
                    let mut device_manager = DeviceManager::new();
                    
                    match device_manager.discover_devices().await {
                        Ok(devices) => {
                            if devices.is_empty() {
                                println!("🔍 未发现任何磁带设备");
                            } else {
                                println!("🔍 发现 {} 个磁带设备:", devices.len());
                                for device_path in devices {
                                    println!("  📱 {}", device_path);
                                    
                                    if detailed {
                                        if let Some(status) = device_manager.get_device_status(&device_path) {
                                            println!("    厂商: {}", status.device_info.vendor);
                                            println!("    型号: {}", status.device_info.model);
                                            println!("    健康状态: {:?}", status.health_status);
                                            println!("    在线状态: {}", if status.is_online { "在线" } else { "离线" });
                                        }
                                        println!();
                                    }
                                }
                            }
                        }
                        Err(e) => {
                            error!("设备发现失败: {}", e);
                            return Err(e);
                        }
                    }
                    Ok(())
                }

                DeviceAction::Status { device, monitor, interval } => {
                    info!("获取设备状态: {}", device);
                    let mut device_manager = DeviceManager::new();
                    
                    // 初始化设备
                    if device_manager.discover_devices().await.is_err() {
                        error!("无法发现设备");
                        return Ok(());
                    }

                    if monitor {
                        println!("📊 启动设备监控模式 (间隔: {}秒, 按Ctrl+C退出)", interval);
                        device_manager.enable_auto_monitoring(Duration::from_secs(interval));
                        
                        loop {
                            if let Err(e) = device_manager.update_device_status(&device).await {
                                warn!("更新设备状态失败: {}", e);
                            }
                            
                            if let Some(status) = device_manager.get_device_status(&device) {
                                println!("\n=== 设备状态更新 {} ===", status.last_updated);
                                println!("设备: {}", device);
                                println!("健康状态: {:?}", status.health_status);
                                println!("在线状态: {}", if status.is_online { "✅ 在线" } else { "❌ 离线" });
                                
                                if let Some(media) = &status.media_info {
                                    let usage_percent = (media.used_capacity as f64 / media.total_capacity as f64) * 100.0;
                                    println!("磁带使用率: {:.1}%", usage_percent);
                                }
                                
                                if !status.recent_issues.is_empty() {
                                    println!("⚠️  最近问题: {} 个", status.recent_issues.len());
                                }
                            }
                            
                            tokio::time::sleep(Duration::from_secs(interval)).await;
                        }
                    } else {
                        // 单次状态查询
                        if let Err(e) = device_manager.update_device_status(&device).await {
                            error!("更新设备状态失败: {}", e);
                        }
                        
                        if let Some(status) = device_manager.get_device_status(&device) {
                            println!("📱 设备: {}", device);
                            println!("厂商: {}", status.device_info.vendor);
                            println!("型号: {}", status.device_info.model);
                            println!("序列号: {}", status.device_info.serial);
                            println!("健康状态: {:?}", status.health_status);
                            println!("设备状态: {:?}", status.device_info.status);
                            println!("在线状态: {}", if status.is_online { "✅ 在线" } else { "❌ 离线" });
                            println!("最后更新: {}", status.last_updated);
                        } else {
                            error!("设备 {} 未找到", device);
                        }
                    }
                    Ok(())
                }

                DeviceAction::Report { report_type, device, output } => {
                    info!("生成设备报告...");
                    let mut device_manager = DeviceManager::new();
                    
                    // 发现设备
                    device_manager.discover_devices().await?;
                    
                    let report_generator = DeviceReportGenerator::new(device_manager);
                    
                    let report_content = match report_type {
                        ReportType::Summary => report_generator.generate_summary_report(),
                        ReportType::Detailed => {
                            if let Some(device_path) = device {
                                report_generator.generate_detailed_report(&device_path)?
                            } else {
                                return Err(RustLtfsError::parameter_validation("详细报告需要指定设备路径"));
                            }
                        }
                        ReportType::Inventory => report_generator.generate_device_inventory_csv(),
                        ReportType::Performance => {
                            // TODO: 实现性能报告
                            "性能报告功能开发中...".to_string()
                        }
                        ReportType::Health => {
                            // TODO: 实现健康报告
                            "健康报告功能开发中...".to_string()
                        }
                    };

                    if let Some(output_path) = output {
                        fs::write(&output_path, &report_content)?;
                        println!("📄 报告已保存到: {:?}", output_path);
                    } else {
                        println!("{}", report_content);
                    }
                    Ok(())
                }

                DeviceAction::HealthCheck { device, comprehensive } => {
                    info!("执行设备健康检查: {}", device);
                    let mut device_manager = DeviceManager::new();
                    
                    if device == "all" {
                        // 检查所有设备
                        device_manager.discover_devices().await?;
                        let devices: Vec<String> = device_manager.get_all_device_status().keys().cloned().collect();
                        
                        println!("🔍 对 {} 个设备执行健康检查...", devices.len());
                        
                        for device_path in devices {
                            println!("\n📱 检查设备: {}", device_path);
                            
                            if let Err(e) = device_manager.update_device_status(&device_path).await {
                                println!("❌ 健康检查失败: {}", e);
                                continue;
                            }
                            
                            if let Some(status) = device_manager.get_device_status(&device_path) {
                                match status.health_status {
                                    crate::tape_ops::device_management::DeviceHealth::Excellent => 
                                        println!("✅ 健康状态: 优秀"),
                                    crate::tape_ops::device_management::DeviceHealth::Good => 
                                        println!("✅ 健康状态: 良好"),
                                    crate::tape_ops::device_management::DeviceHealth::Fair => 
                                        println!("⚠️  健康状态: 一般"),
                                    crate::tape_ops::device_management::DeviceHealth::Poor => 
                                        println!("⚠️  健康状态: 不佳"),
                                    crate::tape_ops::device_management::DeviceHealth::Critical => 
                                        println!("❌ 健康状态: 严重"),
                                    crate::tape_ops::device_management::DeviceHealth::Unknown => 
                                        println!("❓ 健康状态: 未知"),
                                }
                                
                                if comprehensive && !status.recent_issues.is_empty() {
                                    println!("   问题详情:");
                                    for issue in &status.recent_issues {
                                        println!("   - [{:?}] {}", issue.severity, issue.description);
                                    }
                                }
                            }
                        }
                        Ok(())
                    } else {
                        // 检查单个设备
                        device_manager.discover_devices().await?;
                        
                        if let Err(e) = device_manager.update_device_status(&device).await {
                            error!("设备健康检查失败: {}", e);
                            return Err(e);
                        }
                        
                        if let Some(status) = device_manager.get_device_status(&device) {
                            println!("📱 设备: {}", device);
                            println!("健康状态: {:?}", status.health_status);
                            
                            if comprehensive {
                                println!("详细健康信息:");
                                let metrics = &status.performance_metrics;
                                println!("  错误率: {:.2} ppm", metrics.error_rate_ppm);
                                println!("  运行时间: {:.1} 小时", metrics.total_operation_hours);
                                println!("  装载次数: {}", metrics.total_loads);
                                
                                if !status.recent_issues.is_empty() {
                                    println!("  最近问题 ({} 个):", status.recent_issues.len());
                                    for issue in &status.recent_issues {
                                        println!("    - [{:?}] {}: {}", 
                                            issue.severity, issue.timestamp, issue.description);
                                        if let Some(action) = &issue.recommended_action {
                                            println!("      建议: {}", action);
                                        }
                                    }
                                } else {
                                    println!("  ✅ 无问题记录");
                                }
                            }
                            Ok(())
                        } else {
                            error!("设备 {} 未找到", device);
                            Ok(())
                        }
                    }
                }
            }
        }

        Commands::Space { device, skip_index, detailed } => {
            info!("Getting tape space information: {}", device);
            
            // Create tape operations instance
            let mut ops = tape_ops::TapeOperations::new(&device, skip_index);
            
            // Get space information
            let space_info = ops.get_tape_capacity_info().await?;
            
            println!("📦 Tape Space Information:");
            println!("  Total Capacity: {} GB", space_info.total_capacity / (1024 * 1024 * 1024));
            println!("  Used Space: {} GB", space_info.used_space / (1024 * 1024 * 1024));
            println!("  Available Space: {} GB", space_info.available_space / (1024 * 1024 * 1024));
            
            if detailed {
                println!("  Detailed information would be shown here");
            }
            
            Ok(())
        }

        Commands::ReadIndex { device, output, .. } => {
            info!("Reading LTFS index from tape: {}", device);
            
            // Create tape operations instance
            let mut ops = tape_ops::TapeOperations::new(&device, false);
            
            // Initialize and read index using async version
            ops.initialize().await?;
            // Note: LTFS index was already loaded during initialization
            
            // Save index to file if requested
            if let Some(output_path) = output {
                let save_path = output_path.to_string_lossy().to_string();
                ops.save_index_to_file(&std::path::Path::new(&save_path)).await?;
                info!("LTFS index saved to: {}", save_path);
            }
            println!("✅ LTFS index read from tape successfully");
            
            Ok(())
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

        Commands::DiagnoseBlock38 { device, quick, test_backtrack } => {
            info!("Diagnosing P1 Block38 positioning issue on device: {}", device);
            
            if test_backtrack {
                info!("Testing ReadFileMark backtrack logic");
                match tape_ops::block38_diagnostic::test_readfilemark_backtrack(&device) {
                    Ok(_) => {
                        info!("ReadFileMark backtrack test completed");
                        Ok(())
                    }
                    Err(e) => {
                        error!("ReadFileMark backtrack test failed: {}", e);
                        Err(e)
                    }
                }
            } else if quick {
                info!("Running quick P1 Block38 diagnostic");
                match tape_ops::block38_diagnostic::quick_check_block38(&device) {
                    Ok(_) => {
                        info!("Quick diagnostic completed");
                        Ok(())
                    }
                    Err(e) => {
                        error!("Quick diagnostic failed: {}", e);
                        Err(e)
                    }
                }
            } else {
                info!("Running full P1 Block38 diagnostic");
                match tape_ops::block38_diagnostic::diagnose_block38_issue(&device) {
                    Ok(_) => {
                        info!("Full diagnostic completed");
                        Ok(())
                    }
                    Err(e) => {
                        error!("Full diagnostic failed: {}", e);
                        Err(e)
                    }
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
            match ops.update_index_on_tape_manual_new().await {
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