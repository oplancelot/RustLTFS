use super::TapeOperations;
use super::hash::CheckSumBlockwiseCalculator;
use super::utils::format_ltfs_timestamp;
use crate::error::{Result, RustLtfsError};
use std::io::BufRead;
use std::path::Path;
use tokio::fs::File;
use tokio::io::{AsyncReadExt, BufReader};
use tracing::{debug, error, info};

/// Partition write state (corresponds to VB.NET partition management)
pub struct PartitionWriteState {
    pub current_partition: u8,
    pub current_block: u64,
}

/// TapeOperations写入操作实现
impl TapeOperations {
    /// Locate to write position precisely (corresponds to VB.NET LocateToWritePosition)
    pub async fn locate_to_write_position(&mut self) -> Result<PartitionWriteState> {
        info!(
            "Locating to write position with ExtraPartitionCount = {}",
            self.get_extra_partition_count()
        );

        // Read current position
        let current_pos = self.scsi.read_position()?;
        info!(
            "Current tape position: partition={}, block={}",
            current_pos.partition, current_pos.block_number
        );

        // 使用ExtraPartitionCount进行分区映射 (对应LTFSCopyGUI的Math.Min逻辑)
        let logical_data_partition = 1u8; // Partition B (data partition)
        let data_partition = self.get_target_partition(logical_data_partition);
        let mut target_block = current_pos.block_number;

        info!(
            "Partition mapping: logical={} -> physical={} (ExtraPartitionCount={})",
            logical_data_partition,
            data_partition,
            self.get_extra_partition_count()
        );

        // If not in target data partition, move to end of data partition
        if current_pos.partition != data_partition {
            info!(
                "Moving to end of data partition {} using locate_to_eod",
                data_partition
            );

            // Use single locate_to_eod command instead of two-step locate_block+space
            // This ensures proper partition switching and EOD positioning atomically
            self.scsi.locate_to_eod(data_partition)?;

            let eod_pos = self.scsi.read_position()?;

            // Verify we're actually in the correct partition
            if eod_pos.partition != data_partition {
                return Err(RustLtfsError::tape_device(format!(
                    "Failed to switch to data partition {}: ended up at partition {} after locate_to_eod",
                    data_partition, eod_pos.partition
                )));
            }

            target_block = eod_pos.block_number;
            info!(
                "End of data position: partition={}, block={}",
                eod_pos.partition, eod_pos.block_number
            );
        } else {
            // In data partition, check if we need to move to end
            if self.write_options.goto_eod_on_write {
                self.scsi.space(crate::scsi::SpaceType::EndOfData, 0)?;
                let eod_pos = self.scsi.read_position()?;
                target_block = eod_pos.block_number;
                info!(
                    "Moved to end of data: partition={}, block={}",
                    eod_pos.partition, eod_pos.block_number
                );
            }
        }

        // Validate position is reasonable (对应LTFSCopyGUI的分区验证逻辑)
        if let Some(ref schema) = &self.schema {
            let schema_partition = if schema.location.partition == "b" {
                1
            } else {
                0
            };
            let target_schema_partition = self.get_target_partition(schema_partition);

            if target_schema_partition == data_partition
                && target_block <= schema.location.startblock
            {
                return Err(RustLtfsError::tape_device(format!(
                    "Current position p{}b{} not allowed for index write, index is at p{}b{}",
                    data_partition,
                    target_block,
                    target_schema_partition,
                    schema.location.startblock
                )));
            }
        }

        let write_state = PartitionWriteState {
            current_partition: data_partition,
            current_block: target_block,
        };

        info!(
            "Positioning complete, write position: partition={}, block={}",
            write_state.current_partition, write_state.current_block
        );

        Ok(write_state)
    }

    /// Stream write file to tape (refactored version, solves large file memory issues)
    /// Corresponds to VB.NET block read/write logic
    pub async fn write_file_to_tape_streaming(
        &mut self,
        source_path: &Path,
        target_path: &str,
    ) -> Result<()> {
        info!(
            "Streaming file write to tape: {:?} -> {}",
            source_path, target_path
        );

        // Check stop flag




        // Get file metadata
        let metadata = tokio::fs::metadata(source_path).await.map_err(|e| {
            RustLtfsError::file_operation(format!("Unable to get file information: {}", e))
        })?;

        let file_size = metadata.len();
        info!("File size: {} bytes", file_size);

        // Skip .xattr files
        if let Some(ext) = source_path.extension() {
            if ext.to_string_lossy().to_lowercase() == "xattr" {
                info!("Skipping .xattr file: {:?}", source_path);
                return Ok(());
            }
        }

        // Skip symlinks if configured
        if self.write_options.skip_symlinks && metadata.file_type().is_symlink() {
            info!("Skipping symlink: {:?}", source_path);
            return Ok(());
        }

        // Check available tape space
        if let Err(e) = self.check_available_space(file_size) {
            return Err(RustLtfsError::tape_device(format!(
                "Insufficient tape space: {}",
                e
            )));
        }

        // Locate to write position
        let _write_state = self.locate_to_write_position().await?;

        // Explicitly set block size (and Buffered Mode) before writing
        // This corresponds to LTFSCopyGUI: TapeUtils.SetBlockSize(driveHandle, plabel.blocksize)
        info!("Setting drive block size to {} (Buffered Mode enabled)", self.block_size);
        self.scsi.set_block_size(self.block_size)?;

        // Get write start position
        let write_start_position = self.scsi.read_position()?;

        // Open file and create buffered reader
        let file = File::open(source_path)
            .await
            .map_err(|e| RustLtfsError::file_operation(format!("Unable to open file: {}", e)))?;

        let mut buf_reader = BufReader::with_capacity(
            self.block_size as usize * 32, // 32-block buffer
            file,
        );

        // Initialize hash calculator (if enabled) based on configuration
        let mut hash_calculator = if self.write_options.hash_on_write {
            Some(CheckSumBlockwiseCalculator::new_with_options(
                &self.write_options,
            ))
        } else {
            None
        };


        let mut total_blocks_written = 0u32;
        let mut total_bytes_written = 0u64;
        let write_start_time = std::time::Instant::now();
        let mut last_progress_bytes = 0u64;
        let mut last_progress_time = std::time::Instant::now();

        // Choose processing strategy based on file size
        if file_size <= self.block_size as u64 {
            // Small file: read and write in one go
            info!("Processing small file ({} bytes)", file_size);

            let mut buffer = vec![0u8; self.block_size as usize];
            let bytes_read = buf_reader
                .read(&mut buffer[..file_size as usize])
                .await
                .map_err(|e| {
                    RustLtfsError::file_operation(format!("Failed to read file: {}", e))
                })?;

            if bytes_read != file_size as usize {
                return Err(RustLtfsError::file_operation(format!(
                    "Bytes read mismatch: expected {} bytes, actually read {} bytes",
                    file_size, bytes_read
                )));
            }

            // Calculate hash
            if let Some(ref mut calc) = hash_calculator {
                calc.propagate(&buffer[..bytes_read]);
                calc.process_final_block();
            }



            // Write to tape (variable-length for last/short block)
            let blocks_written = self.scsi.write_blocks(1, &buffer[..bytes_read])?;

            if blocks_written != 1 {
                return Err(RustLtfsError::scsi(format!(
                    "Expected to write 1 block, but actually wrote {} blocks",
                    blocks_written
                )));
            }

            total_blocks_written = blocks_written;
            total_bytes_written = bytes_read as u64;

            // Update write progress counters for small-file write
            self.write_progress.current_bytes_processed += bytes_read as u64;
            self.write_progress.current_files_processed += 1;
            self.write_progress.files_written += 1;
            self.write_progress.bytes_written += bytes_read as u64;
        } else {
            // Large file: block-wise streaming (same as LTFSCopyGUI)
            // Windows SCSI pass-through doesn't support multi-block batch writes
            info!(
                "Processing large file ({} bytes), using block-wise streaming",
                file_size
            );

            let mut buffer = vec![0u8; self.block_size as usize];
            let mut remaining_bytes = file_size;
            
            info!("Starting write loop (Block size: {})", self.block_size);

            while remaining_bytes > 0 {
                // Calculate bytes to read for current block
                let bytes_to_read = std::cmp::min(remaining_bytes, self.block_size as u64) as usize;

                // Read data from file
                let bytes_read = buf_reader
                    .read(&mut buffer[..bytes_to_read])
                    .await
                    .map_err(|e| {
                        RustLtfsError::file_operation(format!("Failed to read file: {}", e))
                    })?;

                if bytes_read == 0 {
                    break; // End of file
                }

                // Calculate hash
                if let Some(ref mut calc) = hash_calculator {
                    calc.propagate(&buffer[..bytes_read]);
                }

                // Write single block to tape (like LTFSCopyGUI)
                let blocks_written = self.scsi.write_blocks(1, &buffer[..bytes_read])?;

                if blocks_written != 1 {
                    return Err(RustLtfsError::scsi(format!(
                        "Expected to write 1 block, but actually wrote {} blocks",
                        blocks_written
                    )));
                }

                total_blocks_written += blocks_written;
                total_bytes_written += bytes_read as u64;
                remaining_bytes -= bytes_read as u64;

                // Update progress
                self.write_progress.current_bytes_processed += bytes_read as u64;

                // Log progress every 100MB
                let bytes_since_last_log = total_bytes_written - last_progress_bytes;
                if bytes_since_last_log >= 100 * 1024 * 1024 {
                    let elapsed = write_start_time.elapsed();
                    let elapsed_secs = elapsed.as_secs_f64();
                    
                    let overall_speed_mbps = if elapsed_secs > 0.0 {
                        (total_bytes_written as f64 / (1024.0 * 1024.0)) / elapsed_secs
                    } else {
                        0.0
                    };
                    
                    let recent_elapsed = last_progress_time.elapsed().as_secs_f64();
                    let recent_speed_mbps = if recent_elapsed > 0.0 {
                        (bytes_since_last_log as f64 / (1024.0 * 1024.0)) / recent_elapsed
                    } else {
                        0.0
                    };
                    
                    let gb_written = total_bytes_written as f64 / (1024.0 * 1024.0 * 1024.0);
                    
                    info!(
                        "📊 Write progress: {:.2} GB written | Speed: {:.2} MB/s (avg: {:.2} MB/s) | Blocks: {}",
                        gb_written,
                        recent_speed_mbps,
                        overall_speed_mbps,
                        total_blocks_written
                    );
                    
                    last_progress_bytes = total_bytes_written;
                    last_progress_time = std::time::Instant::now();
                }
            }

            // Complete hash calculation
            if let Some(ref mut calc) = hash_calculator {
                calc.process_final_block();
            }
        }

        // Write file mark to separate files
        self.scsi.write_filemarks(1)?;

        let write_duration = write_start_time.elapsed();
        let speed_mbps = if write_duration.as_millis() > 0 {
            (total_bytes_written as f64 / (1024.0 * 1024.0))
                / (write_duration.as_millis() as f64 / 1000.0)
        } else {
            0.0
        };

        info!(
            "File write complete: {} blocks ({} bytes), took {:?}, speed {:.2} MiB/s",
            total_blocks_written, total_bytes_written, write_duration, speed_mbps
        );

        // Update LTFS index with computed hashes
        if let Some(hash_calc) = &hash_calculator {
            let hashes = hash_calc.get_enabled_hashes(&self.write_options);
            self.update_index_for_file_write_enhanced(
                source_path,
                target_path,
                file_size,
                &write_start_position,
                Some(hashes),
            )?;
        } else {
            self.update_index_for_file_write(
                source_path,
                target_path,
                file_size,
                &write_start_position,
            )?;
        }

        // Update progress counters
        self.write_progress.current_files_processed += 1;
        self.write_progress.total_bytes_unindexed += file_size;


        // For testing and small files, we automatically force index write when total unindexed data is small
        let should_force_index = self.write_options.force_index
            || (self.write_progress.total_bytes_unindexed < 100 * 1024 * 1024 && // Less than 100MB
                                  self.write_progress.current_files_processed <= 10); // And few files

        if self.write_progress.total_bytes_unindexed >= self.write_options.index_write_interval
            || should_force_index
        {
            info!("Index write triggered: interval_reached={}, should_force={}, total_unindexed={}, files_processed={}",
                  self.write_progress.total_bytes_unindexed >= self.write_options.index_write_interval,
                  should_force_index && !self.write_options.force_index,
                  self.write_progress.total_bytes_unindexed,
                  self.write_progress.current_files_processed);
            self.update_index_on_tape_with_options_dual_partition(should_force_index)
                .await?;
        } else {
            info!(
                "Index write skipped: total_unindexed={}, interval={}, files_processed={}",
                self.write_progress.total_bytes_unindexed,
                self.write_options.index_write_interval,
                self.write_progress.current_files_processed
            );
        }

        Ok(())
    }

    /// Write data from a BufRead stream to tape (supports stdin and files)
    pub async fn write_reader_to_tape(
        &mut self,
        mut reader: Box<dyn BufRead + Send>,
        target_path: &str,
        _estimated_size: Option<u64>,
    ) -> Result<()> {
        info!("Writing from reader stream to tape: {}", target_path);

        // 🔒 CRITICAL SAFETY CHECK: Ensure index exists before writing
        // This prevents data loss by ensuring we always have the existing tape contents
        // in the index before adding new data
        if self.index.is_none() {
            // Check if this is a first-time write (empty tape) or a missing index error
            let current_pos = self.scsi.read_position()?;
            
            // If we're not at the very beginning (block 0), this means the tape has data
            // but we failed to load the index - this is a critical error
            if current_pos.block_number > 0 || current_pos.file_number > 0 {
                return Err(RustLtfsError::ltfs_index(format!(
                    "CRITICAL: Cannot write to tape - index is missing but tape has existing data at p{}b{}f{}. \
                     This would cause data loss. Please check tape status and retry reading the index.",
                    current_pos.partition, current_pos.block_number, current_pos.file_number
                )));
            }
            
            // First-time write to empty tape - create new index
            info!("First-time write detected (tape at beginning with no index), creating new LTFS index");
            self.index = Some(self.create_new_ltfs_index());
        }

        // Prepare for writing to tape
        self.scsi.locate_to_eod(1)?;

        // Explicitly set block size (and Buffered Mode) before writing
        let block_size_u32 = self.write_options.block_size;
        info!("Setting drive block size to {} (Buffered Mode enabled) for stream", block_size_u32);
        self.scsi.set_block_size(block_size_u32)?;

        let write_start_position = self.scsi.read_position()?;

        // Create file entry in index - now guaranteed to have index
        let current_time = chrono::Utc::now();
        let file_uid = self.index.as_ref()
            .and_then(|idx| idx.highestfileuid)
            .unwrap_or(0) + 1;

        // ⭐ STREAMING WRITE - Single block at a time (like LTFSCopyGUI)
        // Windows SCSI pass-through doesn't support multi-block batch writes
        let block_size = self.write_options.block_size as usize;
        
        let mut write_buffer = vec![0u8; block_size]; // Buffer for writing full blocks
        let mut read_buffer = vec![0u8; block_size];  // Buffer for reading from stream
        let mut buffer_fill = 0usize; // How many bytes are currently in write_buffer
        let mut total_bytes_written = 0u64;
        let mut total_blocks_written = 0u64;
        let write_start_time = std::time::Instant::now();
        let mut last_progress_bytes = 0u64;
        let mut last_progress_time = std::time::Instant::now();
        
        info!(
            "Starting streaming write (Block size: {} bytes, single-block mode)",
            block_size
        );

        loop {
            // Read data from the stream
            let bytes_read = reader.read(&mut read_buffer).map_err(|e| {
                RustLtfsError::file_operation(format!("Failed to read from input stream: {}", e))
            })?;
            
            if bytes_read == 0 {
                // EOF reached - write any remaining data in buffer as final block
                if buffer_fill > 0 {
                    info!("Writing final partial block: {} bytes", buffer_fill);
                    let blocks_written = self.scsi.write_blocks(1, &write_buffer[..buffer_fill])? as u64;
                    total_blocks_written += blocks_written;
                    total_bytes_written += buffer_fill as u64;
                    self.write_progress.current_bytes_processed += buffer_fill as u64;
                }
                break; // Stream completed
            }
            
            // Accumulate data into write_buffer
            let mut offset = 0;
            while offset < bytes_read {
                let space_left = block_size - buffer_fill;
                let bytes_to_copy = std::cmp::min(space_left, bytes_read - offset);
                
                // Copy data into write buffer
                write_buffer[buffer_fill..buffer_fill + bytes_to_copy]
                    .copy_from_slice(&read_buffer[offset..offset + bytes_to_copy]);
                
                buffer_fill += bytes_to_copy;
                offset += bytes_to_copy;
                
                // If buffer is full, write single block to tape
                if buffer_fill == block_size {
                    let blocks_written = self.scsi.write_blocks(1, &write_buffer)? as u64;
                    total_blocks_written += blocks_written;
                    total_bytes_written += block_size as u64;
                    self.write_progress.current_bytes_processed += block_size as u64;
                    buffer_fill = 0; // Reset buffer
                }
            }
            
            // Log progress every 100MB with detailed statistics
            let bytes_since_last_log = total_bytes_written - last_progress_bytes;
            if bytes_since_last_log >= 100 * 1024 * 1024 {
                let elapsed = write_start_time.elapsed();
                let elapsed_secs = elapsed.as_secs_f64();
                
                let overall_speed_mbps = if elapsed_secs > 0.0 {
                    (total_bytes_written as f64 / (1024.0 * 1024.0)) / elapsed_secs
                } else {
                    0.0
                };
                
                let recent_elapsed = last_progress_time.elapsed().as_secs_f64();
                let recent_speed_mbps = if recent_elapsed > 0.0 {
                    (bytes_since_last_log as f64 / (1024.0 * 1024.0)) / recent_elapsed
                } else {
                    0.0
                };
                
                let gb_written = total_bytes_written as f64 / (1024.0 * 1024.0 * 1024.0);
                
                info!(
                    "📊 Streaming progress: {:.2} GB written | Speed: {:.2} MB/s (avg: {:.2} MB/s) | Blocks: {}",
                    gb_written,
                    recent_speed_mbps,
                    overall_speed_mbps,
                    total_blocks_written
                );
                
                last_progress_bytes = total_bytes_written;
                last_progress_time = std::time::Instant::now();
            }
        }

        let total_elapsed = write_start_time.elapsed();
        let final_speed_mbps = if total_elapsed.as_secs_f64() > 0.0 {
            (total_bytes_written as f64 / (1024.0 * 1024.0)) / total_elapsed.as_secs_f64()
        } else {
            0.0
        };
        
        info!(
            "✅ Stream write completed: {:.2} GB ({} bytes) in {} blocks | Total time: {:?} | Average speed: {:.2} MB/s",
            total_bytes_written as f64 / (1024.0 * 1024.0 * 1024.0),
            total_bytes_written,
            total_blocks_written,
            total_elapsed,
            final_speed_mbps
        );

        // Write filemark to separate this file from next
        self.scsi.write_filemarks(1)?;

        // Add file to index
        if let Some(mut index) = self.index.take() {
            // Create file extent using actual bytes written
            let file_extent = crate::ltfs_index::FileExtent {
                file_offset: 0,
                start_block: write_start_position.block_number,
                byte_count: total_bytes_written,
                byte_offset: 0,
                partition: if write_start_position.partition == 0 {
                    "a".to_string()
                } else {
                    "b".to_string()
                },
            };

            let new_file = crate::ltfs_index::File {
                name: target_path.split('/').last().unwrap_or("unknown").to_string(),
                length: total_bytes_written,
                creation_time: format_ltfs_timestamp(current_time),
                change_time: format_ltfs_timestamp(current_time),
                access_time: format_ltfs_timestamp(current_time),
                backup_time: format_ltfs_timestamp(current_time),
                modify_time: format_ltfs_timestamp(current_time),
                read_only: false,
                uid: file_uid,
                extent_info: crate::ltfs_index::ExtentInfo {
                    extents: vec![file_extent],
                },
                openforwrite: false,
                symlink: None,
                extended_attributes: None,
            };

            // Update highest file uid
            index.highestfileuid = Some(file_uid);
            
            // Add to target directory using helper function
            self.add_file_to_target_directory(&mut index, new_file, target_path)?;

            // Put index back
            self.index = Some(index);
        }

        // Update progress counters
        self.write_progress.current_files_processed += 1;
        self.write_progress.total_bytes_unindexed += total_bytes_written;

        // Check if we should update the index
        let should_force_index = if self.write_progress.current_files_processed == 1 {
            true
        } else {
            self.write_progress.total_bytes_unindexed >= self.write_options.index_write_interval
        };

        if should_force_index {
            debug!(
                "Updating index: total_unindexed={} >= interval={}",
                self.write_progress.total_bytes_unindexed,
                self.write_options.index_write_interval
            );
            self.update_index_on_tape_with_options_dual_partition(should_force_index)
                .await?;
        }

        Ok(())
    }

    /// Write directory to tape (enhanced version based on LTFSCopyGUI AddDirectory)
    pub async fn write_directory_to_tape(
        &mut self,
        source_dir: &Path,
        target_path: &str,
    ) -> Result<()> {
        info!(
            "Writing directory to tape: {:?} -> {}",
            source_dir, target_path
        );



        // Skip symlinks if configured (对应LTFSCopyGUI的SkipSymlink)
        let metadata = tokio::fs::metadata(source_dir).await.map_err(|e| {
            RustLtfsError::file_operation(format!("Cannot get directory metadata: {}", e))
        })?;

        if self.write_options.skip_symlinks && metadata.file_type().is_symlink() {
            info!("Skipping symlink directory: {:?}", source_dir);
            return Ok(());
        }

        // Create or get directory in LTFS index
        let _dir_name = source_dir
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("unknown")
            .to_string();

        // Note: Directory structure is automatically created by ensure_directory_path_exists
        // when files are added, so we don't need to explicitly create directories here.
        // Explicit creation was causing directories to be added at root level incorrectly.

        // Get list of files and subdirectories
        let mut entries = tokio::fs::read_dir(source_dir)
            .await
            .map_err(|e| RustLtfsError::file_operation(format!("Cannot read directory: {}", e)))?;

        let mut files = Vec::new();
        let mut subdirs = Vec::new();

        while let Some(entry) = entries.next_entry().await.map_err(|e| {
            RustLtfsError::file_operation(format!("Cannot read directory entry: {}", e))
        })? {
            let entry_path = entry.path();
            let entry_metadata = entry.metadata().await.map_err(|e| {
                RustLtfsError::file_operation(format!("Cannot get entry metadata: {}", e))
            })?;

            if entry_metadata.is_file() {
                files.push(entry_path);
            } else if entry_metadata.is_dir() {
                subdirs.push(entry_path);
            }
        }

        // Sort files for consistent ordering (对应LTFSCopyGUI的排序逻辑)
        files.sort_by(|a, b| {
            a.file_name()
                .unwrap_or_default()
                .cmp(b.file_name().unwrap_or_default())
        });

        // Sequential file processing (对应LTFSCopyGUI的串行处理)
        info!("Processing {} files sequentially", files.len());

        for file_path in files {



                // Create target path for this file
                let file_name = file_path
                    .file_name()
                    .and_then(|n| n.to_str())
                    .unwrap_or("unknown");
                let file_target = format!("{}/{}", target_path, file_name);

                // Write individual file
                if let Err(e) = self
                    .write_file_to_tape_streaming(&file_path, &file_target)
                    .await
                {
                    error!("Failed to write file {:?}: {}", file_path, e);
                    // Continue with other files instead of failing entire directory
                }
            }

        // Recursively process subdirectories
        for subdir_path in subdirs {


            let subdir_name = subdir_path
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("unknown");
            let subdir_target = format!("{}/{}", target_path, subdir_name);

            // Recursively write subdirectory
            if let Err(e) =
                Box::pin(self.write_directory_to_tape(&subdir_path, &subdir_target)).await
            {
                error!("Failed to write subdirectory {:?}: {}", subdir_path, e);
                // Continue with other directories
            }
        }

        info!(
            "Directory write completed: {:?} -> {}",
            source_dir, target_path
        );
        Ok(())
    }

   

    /// Check available space on tape
    fn check_available_space(&self, required_size: u64) -> Result<()> {
        // For now, we assume there's enough space
        // In a full implementation, this would check MAM data or use other SCSI commands
        // to determine remaining capacity

        // Minimum safety check - require at least 1GB free space
        let min_required_space = required_size + 1024 * 1024 * 1024; // File size + 1GB buffer

        debug!(
            "Checking available space: required {} bytes (with buffer: {})",
            required_size, min_required_space
        );

        // This is a simplified check - in reality would query tape capacity
        if required_size > 8 * 1024 * 1024 * 1024 * 1024 {
            // 8TB limit for LTO-8
            return Err(RustLtfsError::tape_device(
                "File too large for tape capacity".to_string(),
            ));
        }

        Ok(())
    }


    // ================== 索引管理已移至 index/write.rs ==================



}
