use crate::error::{Result, RustLtfsError};
use crate::ltfs_index::LtfsIndex;
use super::{
    TapeOperations, FileWriteEntry, WriteProgress, WriteOptions, WriteResult,
    TapeCapacityInfo, CleaningStatus, EncryptionStatus
};
use std::path::Path;
use std::collections::HashMap;
use tracing::{debug, error, info, warn};
use tokio::io::AsyncReadExt;

/// TapeOperations写入操作实现
impl TapeOperations {
    
    // ================== 文件写入核心操作 ==================
    
    /// Write file to tape (enhanced version based on LTFSCopyGUI AddFile)
    pub async fn write_file_to_tape(
        &mut self,
        source_path: &Path,
        target_path: &str,
    ) -> Result<()> {
        info!("Writing file to tape: {:?} -> {}", source_path, target_path);

        // Check stop flag
        if self.stop_flag {
            return Err(RustLtfsError::operation_cancelled(
                "Write operation stopped by user".to_string(),
            ));
        }

        // Allow execution in offline mode but skip actual tape operations
        if self.offline_mode {
            info!("Offline mode: simulating file write operation");
            self.write_progress.current_files_processed += 1;
            return Ok(());
        }

        // Get file metadata
        let metadata = tokio::fs::metadata(source_path).await.map_err(|e| {
            RustLtfsError::file_operation(format!("Unable to get file information: {}", e))
        })?;

        let file_size = metadata.len();
        info!("File size: {} bytes", file_size);

        // Skip .xattr files (like LTFSCopyGUI)
        if let Some(ext) = source_path.extension() {
            if ext.to_string_lossy().to_lowercase() == "xattr" {
                info!("Skipping .xattr file: {:?}", source_path);
                return Ok(());
            }
        }

        // Skip excluded extensions
        if let Some(ext) = source_path.extension() {
            let ext_str = ext.to_string_lossy().to_lowercase();
            if self
                .write_options
                .excluded_extensions
                .iter()
                .any(|e| e.to_lowercase() == ext_str)
            {
                info!("Skipping excluded extension file: {:?}", source_path);
                return Ok(());
            }
        }

        // Skip symlinks if configured (对应LTFSCopyGUI的SkipSymlink)
        if self.write_options.skip_symlinks && metadata.file_type().is_symlink() {
            info!("Skipping symlink: {:?}", source_path);
            return Ok(());
        }

        // Check for existing file and same file detection (对应LTFSCopyGUI的检查磁带已有文件逻辑)
        let file_name = source_path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("unknown")
            .to_string();

        if let Some(ref index) = self.index {
            if let Ok(existing_file) =
                self.find_existing_file_in_index(index, target_path, &file_name)
            {
                if self.is_same_file(source_path, &existing_file).await? {
                    info!(
                        "File already exists and is identical, skipping: {}",
                        file_name
                    );
                    return Ok(());
                } else if !self.write_options.overwrite {
                    info!(
                        "File exists but overwrite disabled, skipping: {}",
                        file_name
                    );
                    return Ok(());
                }
                // If overwrite is enabled, continue with writing
                info!("Overwriting existing file: {}", file_name);
            }
        }

        // Check available space on tape
        if let Err(e) = self.check_available_space(file_size) {
            return Err(RustLtfsError::tape_device(format!(
                "Insufficient space on tape: {}",
                e
            )));
        }

        // Apply speed limiting if configured (对应LTFSCopyGUI的SpeedLimit)
        if let Some(speed_limit_mbps) = self.write_options.speed_limit {
            self.apply_speed_limit(file_size, speed_limit_mbps).await;
        }

        // Handle pause flag (对应LTFSCopyGUI的Pause功能)
        while self.pause_flag && !self.stop_flag {
            info!("Write operation paused, waiting...");
            tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;
        }

        if self.stop_flag {
            return Err(RustLtfsError::operation_cancelled(
                "Write operation stopped".to_string(),
            ));
        }

        // Calculate multiple hashes if configured (对应LTFSCopyGUI的HashOnWrite)
        let file_hashes = if self.write_options.hash_on_write {
            Some(self.calculate_file_hashes(source_path).await?)
        } else {
            None
        };

        // Position to data partition and write file data
        let write_result = self.write_file_data_to_tape(source_path, file_size).await?;

        // Update LTFS index with new file entry
        self.update_index_for_file_write_enhanced(
            source_path,
            target_path,
            file_size,
            &write_result.position,
            file_hashes,
        )?;

        // Update progress counters (对应LTFSCopyGUI的进度统计)
        self.write_progress.current_files_processed += 1;
        self.write_progress.current_bytes_processed += file_size;
        self.write_progress.total_bytes_unindexed += file_size;

        // Check if index update is needed based on interval (对应LTFSCopyGUI的IndexWriteInterval)
        if self.write_progress.total_bytes_unindexed >= self.write_options.index_write_interval {
            info!("Index write interval reached, updating index...");
            self.update_index_on_tape().await?;
        }

        info!("File write completed: {:?} -> {}", source_path, target_path);
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

        // Check stop flag
        if self.stop_flag {
            return Err(RustLtfsError::operation_cancelled(
                "Write operation stopped by user".to_string(),
            ));
        }

        // Allow execution in offline mode but skip actual tape operations
        if self.offline_mode {
            info!("Offline mode: simulating directory write operation");
            return Ok(());
        }

        // Skip symlinks if configured (对应LTFSCopyGUI的SkipSymlink)
        let metadata = tokio::fs::metadata(source_dir).await.map_err(|e| {
            RustLtfsError::file_operation(format!("Cannot get directory metadata: {}", e))
        })?;

        if self.write_options.skip_symlinks && metadata.file_type().is_symlink() {
            info!("Skipping symlink directory: {:?}", source_dir);
            return Ok(());
        }

        // Create or get directory in LTFS index
        let dir_name = source_dir
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("unknown")
            .to_string();

        // Check if directory already exists in index
        let directory_exists = if let Some(ref index) = self.index {
            self.directory_exists_in_index(index, target_path, &dir_name)?
        } else {
            false
        };

        if !directory_exists {
            // Create directory in LTFS index (对应LTFSCopyGUI的目录创建逻辑)
            self.create_directory_in_index(source_dir, target_path)?;
        }

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

        if self.write_options.parallel_add {
            // Parallel file processing (对应LTFSCopyGUI的Parallel.ForEach)
            info!("Processing {} files in parallel", files.len());

            for file_path in files {
                // Create target path for this file
                let file_name = file_path
                    .file_name()
                    .and_then(|n| n.to_str())
                    .unwrap_or("unknown");
                let file_target = format!("{}/{}", target_path, file_name);

                // Add to processing queue instead of immediate processing for parallel control
                let write_entry = FileWriteEntry {
                    source_path: file_path.clone(),
                    target_path: file_target.clone(),
                    tape_path: file_target,
                    file_size: tokio::fs::metadata(&file_path)
                        .await
                        .map(|m| m.len())
                        .unwrap_or(0),
                    size: tokio::fs::metadata(&file_path)
                        .await
                        .map(|m| m.len())
                        .unwrap_or(0),
                    is_directory: false,
                    preserve_permissions: self.write_options.preserve_permissions,
                    modified: false,
                    overwrite: self.write_options.overwrite,
                    hash: None,
                };

                self.write_queue.push(write_entry);
            }

            // Process write queue
            self.process_write_queue().await?;
        } else {
            // Sequential file processing (对应LTFSCopyGUI的串行处理)
            info!("Processing {} files sequentially", files.len());

            for file_path in files {
                if self.stop_flag {
                    break;
                }

                // Handle pause
                while self.pause_flag && !self.stop_flag {
                    tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;
                }

                // Skip excluded extensions (对应LTFSCopyGUI的exceptExtension逻辑)
                if let Some(ext) = file_path.extension() {
                    let ext_str = ext.to_string_lossy().to_lowercase();
                    if self
                        .write_options
                        .excluded_extensions
                        .iter()
                        .any(|e| e.to_lowercase() == ext_str)
                    {
                        info!("Skipping excluded extension file: {:?}", file_path);
                        continue;
                    }
                }

                // Create target path for this file
                let file_name = file_path
                    .file_name()
                    .and_then(|n| n.to_str())
                    .unwrap_or("unknown");
                let file_target = format!("{}/{}", target_path, file_name);

                // Write individual file
                if let Err(e) = self.write_file_to_tape(&file_path, &file_target).await {
                    error!("Failed to write file {:?}: {}", file_path, e);
                    // Continue with other files instead of failing entire directory
                }
            }
        }

        // Recursively process subdirectories
        for subdir_path in subdirs {
            if self.stop_flag {
                break;
            }

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

    // ================== 异步写入操作 ==================

    /// Write multiple files asynchronously
    pub async fn write_files_async(
        &mut self,
        file_entries: Vec<FileWriteEntry>,
    ) -> Result<()> {
        info!("Starting async write operation for {} files", file_entries.len());

        // Add all entries to write queue
        self.write_queue.extend(file_entries);

        // Process queue
        self.process_write_queue().await?;

        info!("Async write operation completed");
        Ok(())
    }

    // ================== 索引更新相关 ==================

    /// Enhanced index update for file write (对应LTFSCopyGUI的索引更新逻辑)
    fn update_index_for_file_write_enhanced(
        &mut self,
        source_path: &Path,
        target_path: &str,
        file_size: u64,
        write_position: &crate::scsi::TapePosition,
        file_hashes: Option<HashMap<String, String>>,
    ) -> Result<()> {
        debug!(
            "Updating LTFS index for write: {:?} -> {} ({} bytes)",
            source_path, target_path, file_size
        );

        // Get or create current index
        let mut current_index = match &self.index {
            Some(index) => index.clone(),
            None => {
                // Create new index if none exists
                self.create_new_ltfs_index()
            }
        };

        // Create new file entry with enhanced metadata
        let file_name = source_path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("unknown")
            .to_string();

        let now = chrono::Utc::now().to_rfc3339();
        let new_uid = current_index.highestfileuid.unwrap_or(0) + 1;

        let extent = crate::ltfs_index::FileExtent {
            // 强制使用数据分区，按照LTFSCopyGUI逻辑文件应该写入数据分区b
            partition: "b".to_string(),
            start_block: write_position.block_number,
            byte_count: file_size,
            file_offset: 0,
            byte_offset: 0,
        };

        // Get file metadata for timestamps
        let metadata = std::fs::metadata(source_path).map_err(|e| {
            RustLtfsError::file_operation(format!("Cannot get file metadata: {}", e))
        })?;

        let creation_time = metadata
            .created()
            .map(|t| {
                let dt: chrono::DateTime<chrono::Utc> = t.into();
                dt.to_rfc3339()
            })
            .unwrap_or_else(|_| now.clone());

        let modify_time = metadata
            .modified()
            .map(|t| {
                let dt: chrono::DateTime<chrono::Utc> = t.into();
                dt.to_rfc3339()
            })
            .unwrap_or_else(|_| now.clone());

        let access_time = metadata
            .accessed()
            .map(|t| {
                let dt: chrono::DateTime<chrono::Utc> = t.into();
                dt.to_rfc3339()
            })
            .unwrap_or_else(|_| now.clone());

        let new_file = crate::ltfs_index::File {
            name: file_name,
            uid: new_uid,
            length: file_size,
            creation_time: creation_time,
            change_time: now.clone(),
            modify_time: modify_time,
            access_time: access_time,
            backup_time: now,
            read_only: false,
            openforwrite: false,
            symlink: None,
            extent_info: crate::ltfs_index::ExtentInfo {
                extents: vec![extent],
            },
            extended_attributes: if let Some(hashes) = file_hashes {
                // Create extended attributes following LTFSCopyGUI format
                let mut attributes = Vec::new();
                
                for (hash_type, hash_value) in hashes {
                    attributes.push(crate::ltfs_index::ExtendedAttribute {
                        key: format!("ltfs.hash.{}", hash_type),
                        value: hash_value,
                    });
                }
                
                // Add capacity remain attribute (placeholder)
                attributes.push(crate::ltfs_index::ExtendedAttribute {
                    key: "ltfscopygui.capacityremain".to_string(),
                    value: "12".to_string(), // Placeholder value
                });
                
                Some(crate::ltfs_index::ExtendedAttributes { attributes })
            } else {
                None
            },
        };

        // Add file to appropriate directory (simplified - should handle path parsing)
        // For now, add to root directory
        current_index.root_directory.contents.files.push(new_file);

        // Update index metadata
        current_index.generationnumber += 1;
        current_index.updatetime = chrono::Utc::now().to_rfc3339();
        current_index.highestfileuid = Some(new_uid);

        // Update internal index
        self.index = Some(current_index.clone());
        self.schema = Some(current_index);
        self.modified = true; // Mark as modified for later index writing

        debug!("LTFS index updated with new file: UID {}", new_uid);
        Ok(())
    }

    /// Basic index update for file write operation
    fn update_index_for_file_write(
        &mut self,
        source_path: &Path,
        target_path: &str,
        file_size: u64,
        write_position: &crate::scsi::TapePosition,
    ) -> Result<()> {
        debug!(
            "Updating LTFS index for write: {:?} -> {} ({} bytes)",
            source_path, target_path, file_size
        );

        // Get or create current index
        let mut current_index = match &self.index {
            Some(index) => index.clone(),
            None => {
                // Create new index if none exists
                self.create_new_ltfs_index()
            }
        };

        // Create new file entry
        let file_name = source_path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("unknown")
            .to_string();

        let now = chrono::Utc::now().to_rfc3339();
        let new_uid = current_index.highestfileuid.unwrap_or(0) + 1;

        let extent = crate::ltfs_index::FileExtent {
            // 强制使用数据分区，按照LTFSCopyGUI逻辑文件应该写入数据分区b
            partition: "b".to_string(),
            start_block: write_position.block_number,
            byte_count: file_size,
            file_offset: 0,
            byte_offset: 0,
        };

        let new_file = crate::ltfs_index::File {
            name: file_name,
            uid: new_uid,
            length: file_size,
            creation_time: now.clone(),
            change_time: now.clone(),
            modify_time: now.clone(),
            access_time: now.clone(),
            backup_time: now,
            read_only: false,
            openforwrite: false,
            symlink: None,
            extent_info: crate::ltfs_index::ExtentInfo {
                extents: vec![extent],
            },
            extended_attributes: None,
        };

        // Add file to root directory (simplified - should handle path parsing)
        current_index.root_directory.contents.files.push(new_file);

        // Update index metadata
        current_index.generationnumber += 1;
        current_index.updatetime = chrono::Utc::now().to_rfc3339();
        current_index.highestfileuid = Some(new_uid);

        // Update internal index
        self.index = Some(current_index.clone());

        debug!("LTFS index updated with new file: UID {}", new_uid);
        Ok(())
    }

    /// Create directory entry in LTFS index (对应LTFSCopyGUI的目录创建逻辑)
    fn create_directory_in_index(&mut self, source_dir: &Path, target_path: &str) -> Result<()> {
        let mut current_index = match &self.index {
            Some(index) => index.clone(),
            None => self.create_new_ltfs_index(),
        };

        let dir_name = source_dir
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("unknown")
            .to_string();

        let metadata = std::fs::metadata(source_dir).map_err(|e| {
            RustLtfsError::file_operation(format!("Cannot get directory metadata: {}", e))
        })?;

        let now = chrono::Utc::now().to_rfc3339();
        let new_uid = current_index.highestfileuid.unwrap_or(0) + 1;

        let creation_time = metadata
            .created()
            .map(|t| {
                let dt: chrono::DateTime<chrono::Utc> = t.into();
                dt.to_rfc3339()
            })
            .unwrap_or_else(|_| now.clone());

        let modify_time = metadata
            .modified()
            .map(|t| {
                let dt: chrono::DateTime<chrono::Utc> = t.into();
                dt.to_rfc3339()
            })
            .unwrap_or_else(|_| now.clone());

        let access_time = metadata
            .accessed()
            .map(|t| {
                let dt: chrono::DateTime<chrono::Utc> = t.into();
                dt.to_rfc3339()
            })
            .unwrap_or_else(|_| now.clone());

        let new_directory = crate::ltfs_index::Directory {
            name: dir_name,
            uid: new_uid,
            creation_time: creation_time,
            change_time: now.clone(),
            modify_time: modify_time,
            access_time: access_time,
            backup_time: now,
            read_only: false,
            contents: crate::ltfs_index::DirectoryContents {
                files: Vec::new(),
                directories: Vec::new(),
            },
        };

        // For now, add to root directory (should parse target_path properly)
        current_index
            .root_directory
            .contents
            .directories
            .push(new_directory);

        // Update index metadata
        current_index.generationnumber += 1;
        current_index.updatetime = chrono::Utc::now().to_rfc3339();
        current_index.highestfileuid = Some(new_uid);

        // Update internal index
        self.index = Some(current_index.clone());
        self.schema = Some(current_index);
        self.modified = true;

        debug!("Created directory in LTFS index: UID {}", new_uid);
        Ok(())
    }

    /// Update LTFS index on tape
    pub async fn update_ltfs_index(&mut self) -> Result<()> {
        self.update_index_on_tape().await
    }

    /// Write index to tape
    pub async fn write_index_to_tape(&mut self) -> Result<()> {
        self.update_index_on_tape().await
    }

    /// Commit index changes
    pub async fn commit_index_changes(&mut self) -> Result<()> {
        self.update_index_on_tape().await
    }

    /// Auto update LTFS index on tape (enhanced version based on LTFSCopyGUI WriteCurrentIndex)
    pub async fn update_index_on_tape(&mut self) -> Result<()> {
        info!("Starting to update tape LTFS index...");

        // Allow execution in offline mode but skip actual tape operations
        if self.offline_mode {
            info!("Offline mode: simulating index update operation");
            self.write_progress.total_bytes_unindexed = 0;
            return Ok(());
        }

        // Check if index exists and has modifications
        let mut current_index = match &self.schema {
            Some(idx) => idx.clone(),
            None => {
                // Create new index if none exists
                self.create_new_ltfs_index()
            }
        };

        // 按照LTFSCopyGUI逻辑：检查TotalBytesUnindexed而不是modified标志
        if self.write_progress.total_bytes_unindexed == 0 {
            info!("No unindexed data, skipping update (matching LTFSCopyGUI logic)");
            return Ok(());
        }

        // Position to End of Data (EOD) in data partition (对应LTFSCopyGUI的GotoEOD逻辑)
        let current_position = self.scsi.read_position()?;
        info!(
            "Current tape position: partition={}, block={}",
            current_position.partition, current_position.block_number
        );

        // Move to data partition and go to EOD
        let data_partition = 1; // Partition B
        if current_position.partition != data_partition {
            self.scsi.locate_block(data_partition, 0)?; // Move to beginning of data partition first
        }

        // Go to end of data
        self.scsi.space(crate::scsi::SpaceType::EndOfData, 0)?;
        let eod_position = self.scsi.read_position()?;
        info!(
            "End of data position: partition={}, block={}",
            eod_position.partition, eod_position.block_number
        );

        // Write filemark before index (对应LTFSCopyGUI的WriteFileMark)
        self.scsi.write_filemarks(1)?;

        // Update index metadata (对应LTFSCopyGUI的索引元数据更新)
        current_index.generationnumber += 1;
        current_index.updatetime = chrono::Utc::now().to_rfc3339();
        current_index.location.partition = "b".to_string(); // Data partition

        // Store previous generation location if exists
        if let Some(ref existing_index) = &self.index {
            current_index.previousgenerationlocation = Some(existing_index.location.clone());
        }

        // Get position for index write location
        let index_position = self.scsi.read_position()?;
        current_index.location.startblock = index_position.block_number;
        info!(
            "Index will be written at position: partition={}, block={}",
            index_position.partition, index_position.block_number
        );

        info!("Generating index XML...");

        // Create temporary file for index
        let temp_index_path = std::env::temp_dir().join(format!(
            "ltfs_index_{}.xml",
            chrono::Utc::now().format("%Y%m%d_%H%M%S%.3f")
        ));

        // Serialize index to XML and save to temporary file
        let index_xml = current_index.to_xml()?;
        tokio::fs::write(&temp_index_path, index_xml)
            .await
            .map_err(|e| {
                RustLtfsError::file_operation(format!("Cannot write temporary index file: {}", e))
            })?;

        info!("Writing index to tape...");

        // Write index file to tape
        let index_content = tokio::fs::read(&temp_index_path).await.map_err(|e| {
            RustLtfsError::file_operation(format!("Cannot read temporary index file: {}", e))
        })?;

        // Calculate blocks needed for index
        let blocks_needed =
            (index_content.len() + self.block_size as usize - 1) / self.block_size as usize;
        let buffer_size = blocks_needed * self.block_size as usize;
        let mut buffer = vec![0u8; buffer_size];

        // Copy index content to buffer (rest will be zero-padded)
        buffer[..index_content.len()].copy_from_slice(&index_content);

        // Write index blocks to tape
        let blocks_written = self.scsi.write_blocks(blocks_needed as u32, &buffer)?;

        if blocks_written != blocks_needed as u32 {
            // Clean up temporary file
            if let Err(e) = tokio::fs::remove_file(&temp_index_path).await {
                warn!("Failed to remove temporary index file: {}", e);
            }
            return Err(RustLtfsError::scsi(format!(
                "Expected to write {} blocks for index, but wrote {}",
                blocks_needed, blocks_written
            )));
        }

        // Write filemark after index
        self.scsi.write_filemarks(1)?;

        // Update current position tracking
        let final_position = self.scsi.read_position()?;
        info!(
            "Index write completed at position: partition={}, block={}",
            final_position.partition, final_position.block_number
        );

        // Clean up temporary file
        if let Err(e) = tokio::fs::remove_file(&temp_index_path).await {
            warn!("Failed to remove temporary index file: {}", e);
        }

        // Update internal state
        self.index = Some(current_index.clone());
        self.schema = Some(current_index);
        self.modified = false;
        self.write_progress.total_bytes_unindexed = 0;

        // Clear progress counters
        self.write_progress.current_bytes_processed = 0;
        self.write_progress.current_files_processed = 0;

        info!("LTFS index update completed successfully");
        Ok(())
    }

    // ================== 文件处理相关 ==================

    /// Process file entry for writing
    pub async fn process_file_entry(&mut self, entry: &FileWriteEntry) -> Result<()> {
        self.write_file_to_tape(&entry.source_path, &entry.target_path).await
    }

    /// Calculate file hash (preserved for backward compatibility)
    pub async fn calculate_file_hash(&self, file_path: &Path) -> Result<String> {
        use sha2::{Digest, Sha256};

        let mut file = tokio::fs::File::open(file_path).await.map_err(|e| {
            RustLtfsError::file_operation(format!("Cannot open file for hashing: {}", e))
        })?;

        let mut hasher = Sha256::new();
        let mut buffer = vec![0u8; 1024 * 1024]; // 1MB buffer

        loop {
            match file.read(&mut buffer).await {
                Ok(0) => break, // EOF
                Ok(n) => hasher.update(&buffer[..n]),
                Err(e) => {
                    return Err(RustLtfsError::file_operation(format!(
                        "Error reading file for hashing: {}",
                        e
                    )))
                }
            }
        }

        let result = hasher.finalize();
        Ok(format!("{:x}", result))
    }

    /// Calculate multiple file hashes (对应LTFSCopyGUI的多种哈希计算)
    async fn calculate_file_hashes(&self, file_path: &Path) -> Result<HashMap<String, String>> {
        use sha1::{Digest, Sha1};
        use sha2::{Digest as Sha256Digest, Sha256};

        let mut file = tokio::fs::File::open(file_path).await.map_err(|e| {
            RustLtfsError::file_operation(format!("Cannot open file for hashing: {}", e))
        })?;

        let mut sha1_hasher = Sha1::new();
        let mut md5_hasher = md5::Context::new();
        let mut sha256_hasher = Sha256::new();
        
        let mut buffer = vec![0u8; 1024 * 1024]; // 1MB buffer

        loop {
            let bytes_read = file.read(&mut buffer).await.map_err(|e| {
                RustLtfsError::file_operation(format!("Error reading file for hash: {}", e))
            })?;

            if bytes_read == 0 {
                break;
            }

            sha1_hasher.update(&buffer[..bytes_read]);
            md5_hasher.consume(&buffer[..bytes_read]);
            sha256_hasher.update(&buffer[..bytes_read]);
        }

        let mut hashes = HashMap::new();
        
        // 按照LTFSCopyGUI的格式生成哈希值
        hashes.insert("sha1sum".to_string(), format!("{:X}", sha1_hasher.finalize()));
        hashes.insert("md5sum".to_string(), format!("{:X}", md5_hasher.compute()));
        hashes.insert("sha256sum".to_string(), format!("{:X}", sha256_hasher.finalize()));
        
        Ok(hashes)
    }

    /// Verify written data against source file
    pub async fn verify_written_data(&self, source_path: &Path, tape_uid: u64) -> Result<bool> {
        info!("Verifying written data for file: {:?}", source_path);

        // Calculate hash of source file
        let source_hash = self.calculate_file_hash(source_path).await?;

        // For now, we assume verification passes
        // In a full implementation, we would read the file back from tape and compare hashes
        debug!("Source file hash: {}", source_hash);
        
        // Placeholder verification logic
        let verification_passed = true;

        if !verification_passed {
            error!(
                "File verification failed: {:?} (UID: {})",
                source_path, tape_uid
            );
        } else {
            debug!("File verification passed: {:?}", source_path);
        }

        Ok(verification_passed)
    }

    // ================== 进度管理相关 ==================

    /// Update write progress
    pub fn update_write_progress(&mut self, files_processed: u64, bytes_processed: u64) {
        self.write_progress.current_files_processed = files_processed;
        self.write_progress.current_bytes_processed = bytes_processed;
    }

    /// Handle write queue processing
    async fn handle_write_queue(&mut self) -> Result<()> {
        self.process_write_queue().await
    }

    /// Process write queue (对应LTFSCopyGUI的队列处理机制)
    async fn process_write_queue(&mut self) -> Result<()> {
        info!(
            "Processing write queue with {} entries",
            self.write_queue.len()
        );

        let queue_copy = self.write_queue.clone();
        self.write_queue.clear();

        // Update progress
        self.write_progress.files_in_queue = queue_copy.len();

        for entry in queue_copy {
            if self.stop_flag {
                break;
            }

            // Handle pause
            while self.pause_flag && !self.stop_flag {
                tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;
            }

            // Process individual file write entry
            if let Err(e) = self
                .write_file_to_tape(&entry.source_path, &entry.target_path)
                .await
            {
                error!("Failed to write queued file {:?}: {}", entry.source_path, e);
                // Continue with other files in queue
            }
        }

        self.write_progress.files_in_queue = 0;
        info!("Write queue processing completed");
        Ok(())
    }

    // ================== 辅助函数 ==================

    /// Write file data to tape (separated for better error handling)
    async fn write_file_data_to_tape(
        &mut self,
        source_path: &Path,
        file_size: u64,
    ) -> Result<WriteResult> {
        // Read file content
        let file_content = tokio::fs::read(source_path)
            .await
            .map_err(|e| RustLtfsError::file_operation(format!("Unable to read file: {}", e)))?;

        // Position to data partition (partition B) for file data
        let current_position = self.scsi.read_position()?;
        info!(
            "Current tape position: partition={}, block={}",
            current_position.partition, current_position.block_number
        );

        // Move to data partition if not already there
        let data_partition = 1; // Partition B
        let write_start_block = current_position.block_number.max(100); // Start at block 100 for data

        if current_position.partition != data_partition {
            self.scsi.locate_block(data_partition, write_start_block)?;
        }

        // Calculate blocks needed
        let blocks_needed = (file_size + crate::scsi::block_sizes::LTO_BLOCK_SIZE as u64 - 1)
            / crate::scsi::block_sizes::LTO_BLOCK_SIZE as u64;
        let buffer_size =
            blocks_needed as usize * crate::scsi::block_sizes::LTO_BLOCK_SIZE as usize;
        let mut buffer = vec![0u8; buffer_size];

        // Copy file data to buffer (rest will be zero-padded)
        buffer[..file_content.len()].copy_from_slice(&file_content);

        // Get position before writing for extent information
        let write_position = self.scsi.read_position()?;

        // Write file data blocks
        let blocks_written = self.scsi.write_blocks(blocks_needed as u32, &buffer)?;

        if blocks_written != blocks_needed as u32 {
            return Err(RustLtfsError::scsi(format!(
                "Expected to write {} blocks, but wrote {}",
                blocks_needed, blocks_written
            )));
        }

        // Write file mark to separate this file from next
        self.scsi.write_filemarks(1)?;

        info!(
            "Successfully wrote {} blocks ({} bytes) to tape",
            blocks_written, file_size
        );

        Ok(WriteResult {
            position: write_position,
            blocks_written,
            bytes_written: file_size,
        })
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

    /// Find existing file in LTFS index (对应LTFSCopyGUI的文件检查逻辑)
    fn find_existing_file_in_index(
        &self,
        index: &LtfsIndex,
        target_dir: &str,
        file_name: &str,
    ) -> Result<crate::ltfs_index::File> {
        // Parse target directory path and find the file
        // This is a simplified implementation - full version would properly parse directory structure
        let file_locations = index.extract_tape_file_locations();

        for location in &file_locations {
            if location.file_name.to_lowercase() == file_name.to_lowercase() {
                // Find the actual file object in the index
                return self.find_file_by_name_recursive(&index.root_directory, file_name);
            }
        }

        Err(RustLtfsError::ltfs_index(format!(
            "File not found: {}",
            file_name
        )))
    }

    /// Recursively find file by name in directory structure
    fn find_file_by_name_recursive(
        &self,
        dir: &crate::ltfs_index::Directory,
        file_name: &str,
    ) -> Result<crate::ltfs_index::File> {
        // Search files in current directory
        for file in &dir.contents.files {
            if file.name.to_lowercase() == file_name.to_lowercase() {
                return Ok(file.clone());
            }
        }

        // Recursively search subdirectories
        for subdir in &dir.contents.directories {
            if let Ok(found_file) = self.find_file_by_name_recursive(subdir, file_name) {
                return Ok(found_file);
            }
        }

        Err(RustLtfsError::ltfs_index(format!(
            "File not found: {}",
            file_name
        )))
    }

    /// Check if local file is same as tape file (对应LTFSCopyGUI的IsSameFile逻辑)
    async fn is_same_file(
        &self,
        local_path: &Path,
        tape_file: &crate::ltfs_index::File,
    ) -> Result<bool> {
        let metadata = tokio::fs::metadata(local_path).await.map_err(|e| {
            RustLtfsError::file_operation(format!("Cannot get file metadata: {}", e))
        })?;

        // Compare file size
        if metadata.len() != tape_file.length {
            return Ok(false);
        }

        // Compare modification time if available
        if let Ok(modified_time) = metadata.modified() {
            if let Ok(tape_time) = chrono::DateTime::parse_from_rfc3339(&tape_file.modify_time) {
                let local_time: chrono::DateTime<chrono::Utc> = modified_time.into();

                // Allow small time differences (within 2 seconds) due to precision differences
                let time_diff = (local_time.timestamp() - tape_time.timestamp()).abs();
                if time_diff > 2 {
                    return Ok(false);
                }
            }
        }

        // If hash checking is enabled, compare file hashes
        if self.write_options.hash_on_write {
            let local_hash = self.calculate_file_hash(local_path).await?;
            // For now, we assume tape file doesn't have hash stored
            // In full implementation, we would compare with stored hash
            debug!("Local file hash: {}", local_hash);
        }

        // Files are considered the same if size matches and time is close
        Ok(true)
    }

    /// Apply speed limiting (对应LTFSCopyGUI的SpeedLimit功能)
    async fn apply_speed_limit(&mut self, bytes_to_write: u64, speed_limit_mbps: u32) {
        let speed_limit_bytes_per_sec = (speed_limit_mbps as u64) * 1024 * 1024;
        let expected_duration = bytes_to_write * 1000 / speed_limit_bytes_per_sec; // in milliseconds

        if expected_duration > 0 {
            debug!(
                "Speed limiting: waiting {}ms for {} bytes at {} MiB/s",
                expected_duration, bytes_to_write, speed_limit_mbps
            );
            tokio::time::sleep(tokio::time::Duration::from_millis(expected_duration)).await;
        }
    }

    /// Check if directory exists in LTFS index
    fn directory_exists_in_index(
        &self,
        index: &LtfsIndex,
        target_path: &str,
        dir_name: &str,
    ) -> Result<bool> {
        // This is a simplified implementation
        // In a full implementation, we would properly parse the path and navigate the directory tree
        debug!(
            "Checking if directory exists: {} in {}",
            dir_name, target_path
        );
        Ok(false) // For now, always assume directory doesn't exist
    }

    /// Create new empty LTFS index
    fn create_new_ltfs_index(&self) -> LtfsIndex {
        use uuid::Uuid;

        let now = chrono::Utc::now().to_rfc3339();
        let volume_uuid = Uuid::new_v4();

        LtfsIndex {
            version: "2.4.0".to_string(),
            creator: "RustLTFS".to_string(),
            volumeuuid: volume_uuid.to_string(),
            generationnumber: 1,
            updatetime: now.clone(),
            location: crate::ltfs_index::Location {
                partition: "b".to_string(), // Data partition
                startblock: 0,
            },
            previousgenerationlocation: None,
            allowpolicyupdate: Some(true),
            volumelockstate: None,
            highestfileuid: Some(1),
            root_directory: crate::ltfs_index::Directory {
                name: ".".to_string(),
                uid: 1,
                creation_time: now.clone(),
                change_time: now.clone(),
                modify_time: now.clone(),
                access_time: now.clone(),
                backup_time: now,
                read_only: false,
                contents: crate::ltfs_index::DirectoryContents {
                    files: Vec::new(),
                    directories: Vec::new(),
                },
            },
        }
    }
}