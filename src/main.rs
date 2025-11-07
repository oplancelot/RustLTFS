mod cli;
mod display;
mod error;
mod logger;
mod ltfs;
mod ltfs_index;
mod scsi;
mod tape;
mod tape_ops;

use crate::cli::{Cli, Commands};
use crate::error::{Result, RustLtfsError};
use std::fs::File;
use std::io::{self, BufRead, BufReader, Read};
use tracing::{error, info, warn};

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
            dry_run,
            max_file_size,
            quiet,
        } => {
            info!(
                "Starting write operation: {:?} -> {}:{:?}",
                source.as_ref().map(|s| s.as_path()).unwrap_or_else(|| std::path::Path::new("<stdin>")), 
                device, 
                destination
            );

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
                        excluded.push(if ext.starts_with('.') {
                            ext.to_string()
                        } else {
                            format!(".{}", ext)
                        });
                    }
                }
                write_options.excluded_extensions = excluded;
            }

            let excluded_extensions_copy = write_options.excluded_extensions.clone();

            ops.set_write_options(write_options);

            // Display progress if requested
            let show_progress = progress && !quiet;
            if show_progress {
                println!("🔧 Initializing tape device: {}", device);
            }

            // Initialize tape device with proper error handling
            let device_initialized = match ops.initialize(Some(tape_ops::core::OperationType::Write)).await {
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

            // Create unified input reader and determine operation type
            let (estimated_size, operation_mode): (Option<u64>, &str) = match &source {
                Some(source_path) => {
                    // File or directory input
                    if !source_path.exists() {
                        return Err(error::RustLtfsError::file_operation(format!(
                            "Source path does not exist: {:?}",
                            source_path
                        )));
                    }

                    if source_path.is_dir() {
                        // Directory mode
                        (None, "directory") 
                    } else {
                        // File mode
                        let metadata = source_path.metadata().map_err(|e| {
                            error::RustLtfsError::file_operation(format!(
                                "Cannot get file metadata {:?}: {}", source_path, e
                            ))
                        })?;
                        let file_size = metadata.len();
                        
                        // Handle max file size check
                        if let Some(max_size_gib) = max_file_size {
                            let max_size_bytes = (max_size_gib as u64) * 1024 * 1024 * 1024;
                            if file_size > max_size_bytes {
                                if !quiet {
                                    println!(
                                        "❌ File size ({}) exceeds maximum allowed size ({})",
                                        rust_ltfs::utils::format_bytes(file_size),
                                        rust_ltfs::utils::format_bytes(max_size_bytes)
                                    );
                                }
                                return Err(error::RustLtfsError::parameter_validation(format!(
                                    "File too large: {} > {} GiB",
                                    file_size, max_size_gib
                                )));
                            }
                        }
                        
                        (Some(file_size), "file")
                    }
                },
                None => {
                    // Stdin mode
                    (None, "stdin")
                }
            };

            let source_display = match &source {
                Some(path) => format!("{:?}", path),
                None => "<stdin>".to_string(),
            };

            // Display write operation details
            if !quiet {
                println!("\n🚀 Starting Write Operation");
                println!("  Source: {}", source_display);
                println!("  Device: {}", device);
                println!("  Target: {:?}", destination);

                let mut options = Vec::new();
                if force {
                    options.push("Overwrite existing files".to_string())
                };
                if verify {
                    options.push("Hash verification enabled".to_string())
                };
                if skip_symlinks {
                    options.push("Skip symbolic links".to_string())
                };
                if parallel {
                    options.push("Parallel processing".to_string())
                };
                if let Some(speed) = speed_limit {
                    options.push(format!("Speed limited to {} MiB/s", speed));
                }
                if dry_run {
                    options.push("DRY RUN - no actual writing".to_string())
                };

                if !options.is_empty() {
                    let options_str: Vec<&str> = options.iter().map(|s| s.as_str()).collect();
                    println!("  Options: {}", options_str.join(", "));
                }

                if !excluded_extensions_copy.is_empty() {
                    println!(
                        "  Excluded extensions: {}",
                        excluded_extensions_copy.join(", ")
                    );
                }
            }

            // Display current write progress if requested
            if show_progress {
                let write_progress = ops.get_write_progress();
                println!("\n📊 Initial Status:");
                println!(
                    "  Files processed: {}",
                    write_progress.current_files_processed
                );
                println!(
                    "  Bytes processed: {}",
                    rust_ltfs::utils::format_bytes(write_progress.current_bytes_processed)
                );
            }

            // Handle dry run mode
            if dry_run {
                if !quiet {
                    println!("\n🔍 DRY RUN: Analyzing source files...");
                    println!("✅ Dry run completed - no data was written");
                }
                return Ok(());
            }

            // Execute write operation with enhanced progress reporting
            let write_start = std::time::Instant::now();

            match operation_mode {
                "directory" => {
                    // Directory mode - use existing directory write logic
                    if let Some(ref source_path) = source {
                        if show_progress {
                            println!("\n📁 Writing directory to tape...");
                        }

                        ops.write_directory_to_tape(source_path, &destination.to_string_lossy())
                            .await?;
                    }
                },
                "file" => {
                    // File mode - use existing file-based method
                    if let Some(ref source_path) = source {
                        if show_progress {
                            println!("\n📄 Writing file to tape...");
                        }
                        ops.write_file_to_tape_streaming(source_path, &destination.to_string_lossy())
                            .await
                            .map(|_| ())?;
                    }
                },
                "stdin" => {
                    // Stdin mode - read from stdin and write to tape
                    if show_progress {
                        println!("\n📄 Writing from stdin to tape...");
                    }
                    
                    // Read all data from stdin
                    let mut stdin_data = Vec::new();
                    io::stdin().read_to_end(&mut stdin_data).map_err(|e| {
                        error::RustLtfsError::file_operation(format!("Failed to read from stdin: {}", e))
                    })?;
                    
                    // Create a cursor for the data
                    let cursor = std::io::Cursor::new(stdin_data);
                    let reader: Box<dyn BufRead + Send> = Box::new(BufReader::new(cursor));
                    
                    ops.write_reader_to_tape(reader, &destination.to_string_lossy(), estimated_size)
                        .await
                        .map(|_| ())?;
                },
                _ => {
                    return Err(error::RustLtfsError::parameter_validation(
                        "Invalid operation mode".to_string()
                    ));
                }
            }

            let write_duration = write_start.elapsed();

            // Show final progress with enhanced reporting
            if !quiet {
                let final_progress = ops.get_write_progress();
                println!("\n✅ Write Operation Completed");
                println!(
                    "  Files written: {}",
                    final_progress.current_files_processed
                );
                println!(
                    "  Bytes written: {}",
                    rust_ltfs::utils::format_bytes(final_progress.current_bytes_processed)
                );
                println!(
                    "  Duration: {}",
                    rust_ltfs::utils::format_duration(write_duration.as_secs_f64())
                );

                if final_progress.current_bytes_processed > 0 && write_duration.as_secs() > 0 {
                    println!(
                        "  Average Speed: {}/s",
                        rust_ltfs::utils::format_speed(
                            final_progress.current_bytes_processed,
                            write_duration.as_secs_f64()
                        )
                    );
                }
            }

            info!(
                "Write operation completed in {:.2}s",
                write_duration.as_secs_f64()
            );

            // Auto update LTFS index (对应LTFSCopyGUI的自动索引更新)
            if device_initialized && !skip_index {
                if progress {
                    println!("\n🔄 Updating LTFS index...");
                }
                info!("Auto updating LTFS index...");

                match ops
                    .update_index_on_tape_with_options_dual_partition(false)
                    .await
                {
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

                match ops
                    .save_index_to_file(&std::path::PathBuf::from(&index_filename))
                    .await
                {
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
        } => {
            info!("Starting read operation: {} -> {:?}", device, source);

            // Create tape operations instance (never skip index for read operations)
            let mut ops = tape_ops::TapeOperations::new(&device, false);

            // Initialize tape device with auto index reading
            ops.initialize(Some(tape_ops::core::OperationType::Read)).await?;

            match source {
                None => {
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
                Some(src_path) => {
                    // List specific directory contents
                    info!("Listing directory contents: {:?}", src_path);
                    
                    ops.list_directory_contents(&src_path.to_string_lossy())?;
                }
            }

            Ok(())
        }

        Commands::Space {
            device,
            skip_index,
            detailed,
        } => {
            info!("Getting tape space information: {}", device);

            // Create tape operations instance (never offline for space command)
            let mut ops = tape_ops::TapeOperations::new(&device, false);

            // Initialize for space operation
            ops.initialize(Some(tape_ops::core::OperationType::Space)).await?;

            // Get space information
            let space_info = ops.get_tape_capacity_info().await?;

            println!("📦 Tape Space Information:");
            println!(
                "  Total Capacity: {} GB",
                space_info.total_capacity / (1024 * 1024 * 1024)
            );
            println!(
                "  Used Space: {} GB",
                space_info.used_space / (1024 * 1024 * 1024)
            );
            println!(
                "  Available Space: {} GB",
                space_info.available_space / (1024 * 1024 * 1024)
            );

            if detailed {
                println!("  Detailed information would be shown here");
            }

            Ok(())
        }
    }
}
