//! Write Command Handler
//!
//! Handles the `write` subcommand for writing files/directories to tape.

use crate::error::{Result, RustLtfsError};
use crate::tape_ops;
use crate::utils;
use std::io::{self, BufRead, BufReader};
use std::path::PathBuf;
use tracing::{error, info, warn};

pub async fn execute(
    source: Option<PathBuf>,
    device: String,
    destination: PathBuf,
    verify: bool,
    progress: bool,
) -> Result<()> {
    info!(
        "Starting write operation: {:?} -> {}:{:?}",
        source
            .as_ref()
            .map(|s| s.as_path())
            .unwrap_or_else(|| std::path::Path::new("<stdin>")),
        device,
        destination
    );

    // Create tape operations instance
    let mut ops = tape_ops::TapeOperations::new(&device);

    // Configure advanced write options
    let mut write_options = tape_ops::WriteOptions::default();
    write_options.verify = verify;

    ops.set_write_options(write_options);

    // Display progress if requested
    let show_progress = progress;
    if show_progress {
        println!("🔧 Initializing tape device: {}", device);
    }

    // Initialize tape device with proper error handling
    let device_initialized =
        match ops
            .initialize(Some(tape_ops::core::OperationType::Write))
            .await
        {
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
                    println!("❌ No tape cartridge detected in drive: {}", device);
                    println!("💡 Insert a tape cartridge and try again");
                    return Err(e);
                } else if e.to_string().contains("Write protected") {
                    println!("❌ Tape is write-protected");
                    println!("💡 Remove write protection or use a different tape");
                    return Err(e);
                } else {
                    println!("❌ Device initialization failed: {}", e);
                    return Err(e);
                }
            }
        };

    let (estimated_size, operation_mode): (Option<u64>, &str) = match &source {
        Some(source_path) => {
            // File or directory input
            if !source_path.exists() {
                return Err(RustLtfsError::file_operation(format!(
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
                    RustLtfsError::file_operation(format!(
                        "Cannot get file metadata {:?}: {}",
                        source_path, e
                    ))
                })?;
                let file_size = metadata.len();

                (Some(file_size), "file")
            }
        }
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
    println!("\n🚀 Starting Write Operation");
    println!("  Source: {}", source_display);
    println!("  Device: {}", device);
    println!("  Target: {:?}", destination);

    let mut options = Vec::new();
    if verify {
        options.push("Hash verification enabled".to_string())
    };

    if !options.is_empty() {
        let options_str: Vec<&str> = options.iter().map(|s| s.as_str()).collect();
        println!("  Options: {}", options_str.join(", "));
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
            utils::format_bytes(write_progress.current_bytes_processed)
        );
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
        }
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
        }
        "stdin" => {
            // Stdin mode - stream from stdin to tape (IMPORTANT: Don't read_to_end - streams are huge!)
            if show_progress {
                println!("\n📄 Writing from stdin to tape...");
            }

            // Create a buffered reader directly from stdin for true streaming
            // This avoids loading the entire tar stream (potentially 200GB+) into memory
            let stdin = io::stdin();
            let reader: Box<dyn BufRead + Send> = Box::new(BufReader::with_capacity(
                8 * 1024 * 1024, // 8MB buffer - good balance for tape write performance
                stdin,
            ));

            ops.write_reader_to_tape(reader, &destination.to_string_lossy(), estimated_size)
                .await
                .map(|_| ())?;
        }
        _ => {
            return Err(RustLtfsError::parameter_validation(
                "Invalid operation mode".to_string(),
            ));
        }
    }

    let write_duration = write_start.elapsed();

    // Show final progress
    let final_progress = ops.get_write_progress();
    println!("\n✅ Write Operation Completed");
    println!(
        "  Files written: {}",
        final_progress.current_files_processed
    );
    println!(
        "  Bytes written: {}",
        utils::format_bytes(final_progress.current_bytes_processed)
    );
    println!(
        "  Duration: {}",
        utils::format_duration(write_duration.as_secs_f64())
    );

    if final_progress.current_bytes_processed > 0 && write_duration.as_secs() > 0 {
        println!(
            "  Average Speed: {}/s",
            utils::format_speed(
                final_progress.current_bytes_processed,
                write_duration.as_secs_f64()
            )
        );
    }

    info!(
        "Write operation completed in {:.2}s",
        write_duration.as_secs_f64()
    );

    // Auto update LTFS index
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

    // Save index to local file for backup
    if device_initialized {
        let timestamp = chrono::Utc::now().format("%Y%m%d_%H%M%S").to_string();
        
        // Create schema directory if it doesn't exist
        let schema_dir = std::path::PathBuf::from("schema");
        if !schema_dir.exists() {
            if let Err(e) = std::fs::create_dir_all(&schema_dir) {
                warn!("Failed to create schema directory: {}", e);
            }
        }
        
        let index_filename = schema_dir.join(format!("LTFSIndex_Write_{}.schema", timestamp));

        if progress {
            println!("\n💾 Saving index backup: {}", index_filename.display());
        }

        match ops
            .save_index_to_file(&index_filename)
            .await
        {
            Ok(_) => {
                if progress {
                    println!("✅ Index backup saved");
                }
                info!("Index backup saved: {}", index_filename.display());
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
