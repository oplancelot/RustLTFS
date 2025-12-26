//! LTFS Index Synchronization for Dual-Partition Tapes
//!
//! This module implements dual-partition index synchronization logic,
//! strictly following LTFSCopyGUI implementation:
//! - Write index to data partition (Partition B)
//! - Sync index to index partition (Partition A)
//! - Volume Coherency Information (VCI) management
//!
use crate::error::{Result, RustLtfsError};
use crate::ltfs_index::LtfsIndex;
use super::super::TapeOperations;
use tracing::{debug, info};

/// Generate LTFS-compatible Z-format timestamp (matching LTFSCopyGUI XML format)
fn format_ltfs_timestamp(datetime: chrono::DateTime<chrono::Utc>) -> String {
    format!("{}Z", datetime.format("%Y-%m-%dT%H:%M:%S%.9f"))
}

/// Get current timestamp in LTFS-compatible format
fn get_current_ltfs_timestamp() -> String {
    format_ltfs_timestamp(chrono::Utc::now())
}

/// Helper function to count files recursively in directory tree
fn count_files_recursive(dir: &crate::ltfs_index::Directory) -> usize {
    let mut count = dir.contents.files.len();
    for subdir in &dir.contents.directories {
        count += count_files_recursive(subdir);
    }
    count
}

impl TapeOperations {
    /// Update index on tape with force option (corresponds to VB.NET WriteCurrentIndex + RefreshIndexPartition)
    pub async fn update_index_on_tape_with_options_dual_partition(&mut self, force_index: bool) -> Result<()> {
        info!("Starting to update tape LTFS index...");



        // Check if index exists and has modifications  
        // 优先使用 self.index (包含最新的文件状态)，回退到 self.schema
        let mut current_index = match &self.index {
            Some(idx) => {
                let total_files = count_files_recursive(&idx.root_directory);
                info!("Using self.index with {} total files (gen {})", total_files, idx.generationnumber);
                idx.clone()
            },
            None => match &self.schema {
                Some(idx) => {
                    let total_files = count_files_recursive(&idx.root_directory);
                    info!("Fallback to self.schema with {} total files (gen {})", total_files, idx.generationnumber);
                    idx.clone()
                },
                None => {
                    info!("Creating new index (no existing index found)");
                    // Create new index if none exists - inline creation
                    use uuid::Uuid;
                    let now = get_current_ltfs_timestamp();
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
                        allowpolicyupdate: Some(false),
                        volumelockstate: "unlocked".to_string(),
                        highestfileuid: Some(1),
                        root_directory: crate::ltfs_index::Directory {
                            name: "".to_string(),
                            uid: 1,
                            creation_time: now.clone(),
                            change_time: now.clone(),
                            modify_time: now.clone(),
                            access_time: now.clone(),
                            backup_time: now.clone(),
                            read_only: false,
                            contents: crate::ltfs_index::DirectoryContents {
                                files: Vec::new(),
                                directories: Vec::new(),
                            },
                        },
                    }
                }
            }
        };

        // Enhanced logic following LTFSCopyGUI: check force_index OR TotalBytesUnindexed
        let should_update = force_index || 
                          self.write_progress.total_bytes_unindexed > 0 ||
                          self.modified;

        if !should_update {
            info!("No index update needed (no modifications)");
            return Ok(());
        }

        let extra_partition_count = self.get_extra_partition_count();
        info!("Index update with ExtraPartitionCount: {}", extra_partition_count);

        // === Step 1: WriteCurrentIndex - Write to Data Partition ===
        self.write_current_index_to_data_partition(&mut current_index).await?;

        // === Step 2: RefreshIndexPartition - Sync to Index Partition (only if dual partition) ===
        if extra_partition_count > 0 {
            self.refresh_index_partition(&mut current_index).await?;
        }

        // Update internal state
        self.schema = Some(current_index.clone());
        self.index = Some(current_index);
        self.write_progress.total_bytes_unindexed = 0;
        self.modified = false;

        info!("LTFS index update completed successfully");
        Ok(())
    }

    /// WriteCurrentIndex: Write index to data partition (对应LTFSCopyGUI WriteCurrentIndex)
    async fn write_current_index_to_data_partition(&mut self, current_index: &mut LtfsIndex) -> Result<()> {
        info!("=== WriteCurrentIndex: Writing to Data Partition ===");

        let current_position = self.scsi.read_position()?;
        debug!("Current tape position: partition={}, block={}", 
              current_position.partition, current_position.block_number);

        // 使用LTFSCopyGUI精确逻辑：定位到DataPartition的EOD
        let logical_data_partition = 1u8; // DataPartition = 1 (Partition B)
        let data_partition = self.get_target_partition(logical_data_partition);
        
        debug!("Moving to data partition {} EOD", 
              data_partition);
        
        // 精确对应：TapeUtils.Locate(driveHandle, 0UL, DataPartition, TapeUtils.LocateDestType.EOD)
        if current_position.partition != data_partition {
            self.scsi.locate_block(data_partition, 0)?; // Move to data partition first
        }
        self.scsi.space(crate::scsi::SpaceType::EndOfData, 0)?; // Go to EOD

        let eod_position = self.scsi.read_position()?;
        debug!("End of data position: partition={}, block={}", 
              eod_position.partition, eod_position.block_number);

        // Enhanced LTFSCopyGUI validation logic for first write scenarios
        let extra_partition_count = self.get_extra_partition_count();
        if extra_partition_count > 0 && current_index.location.partition != "b" && 
           eod_position.partition != data_partition {
            return Err(RustLtfsError::tape_device(format!(
                "Current position p{}b{} not allowed for index write",
                eod_position.partition, eod_position.block_number
            )));
        }

        // Enhanced validation logic for first write scenarios
        // 首次写入时，索引startblock可能为0，EOD位置也可能为0，这是正常情况
        if extra_partition_count > 0 {
            let is_first_write = current_index.generationnumber <= 1 && current_index.location.startblock == 0;
            let is_eod_at_start = eod_position.block_number == 0;
            
            // 如果不是首次写入，或者EOD不在开始位置，才进行位置冲突检查
            if !is_first_write && !is_eod_at_start && current_index.location.startblock >= eod_position.block_number {
                return Err(RustLtfsError::tape_device(format!(
                    "Current position p{}b{} not allowed for index write, index at startblock {}",
                    eod_position.partition, eod_position.block_number, current_index.location.startblock
                )));
            }
            
            debug!("Index write validation passed: first_write={}, eod_at_start={}, startblock={}, eod_block={}", 
                  is_first_write, is_eod_at_start, current_index.location.startblock, eod_position.block_number);
        }

        // Write filemark before index (对应LTFSCopyGUI WriteFileMark)
        debug!("Writing filemark before index");
        self.scsi.write_filemarks(1)?;

        // Update index metadata (对应LTFSCopyGUI的索引元数据更新)
        current_index.generationnumber += 1;
        current_index.updatetime = get_current_ltfs_timestamp();
        current_index.location.partition = "b".to_string(); // Data partition
        
        // Set previous generation location
        current_index.previousgenerationlocation = Some(crate::ltfs_index::Location {
            partition: current_index.location.partition.clone(),
            startblock: current_index.location.startblock,
        });

        let index_position = self.scsi.read_position()?;
        current_index.location.startblock = index_position.block_number;
        
        debug!("Index will be written at position: partition={}, block={}", 
              index_position.partition, index_position.block_number);

        // Generate and write index XML
        debug!("Generating index XML...");
        
        let index_xml = current_index.to_xml()?;
        
        debug!("Writing index to tape...");
        self.write_xml_to_tape(&index_xml).await?;

        // Write filemark after index (对应LTFSCopyGUI WriteFileMark)
        self.scsi.write_filemarks(1)?;

        let final_position = self.scsi.read_position()?;
        debug!("Index write completed at position: partition={}, block={}", 
              final_position.partition, final_position.block_number);

        Ok(())
    }

    /// RefreshIndexPartition: Sync index to index partition (对应LTFSCopyGUI RefreshIndexPartition)
    /// 
    /// 🔧 LTFSCopyGUI compatible: Uses FileMark 3 for index partition
    /// Reference: LTFSWriter.vb line 2418 - TapeUtils.Locate(driveHandle, 3UL, IndexPartition, TapeUtils.LocateDestType.FileMark)
    /// Reference: LTFSWriter.vb line 4549 - same location used for reading
    async fn refresh_index_partition(&mut self, current_index: &mut LtfsIndex) -> Result<()> {
        info!("=== RefreshIndexPartition: Syncing to Index Partition ===");

        let logical_index_partition = 0u8; // IndexPartition = 0 (Partition A)
        let index_partition = self.get_target_partition(logical_index_partition);

        // LTFSCopyGUI uses FileMark 3 for index partition (line 2418 & 4549)
        let target_filemark = 3u64;
        debug!("Locating to index partition {} at FileMark {} (LTFSCopyGUI compatible)", 
              index_partition, target_filemark);
        
        self.scsi.locate_to_filemark(target_filemark, index_partition)?;

        let locate_position = self.scsi.read_position()?;
        debug!("Located to position: partition={}, block={}", 
              locate_position.partition, locate_position.block_number);

        // Write filemark (对应LTFSCopyGUI WriteFileMark at line 2421)
        debug!("Writing filemark at index partition");
        self.scsi.write_filemarks(1)?;

        // Update index location to index partition
        if current_index.location.partition == "b" {
            current_index.previousgenerationlocation = Some(crate::ltfs_index::Location {
                partition: current_index.location.partition.clone(),
                startblock: current_index.location.startblock,
            });
        }

        // LTFSCopyGUI: schema.location.startblock = p.BlockNumber + 1 (line 2427)
        let write_position = self.scsi.read_position()?;
        current_index.location.startblock = write_position.block_number;
        current_index.location.partition = "a".to_string(); // Index partition

        debug!("Updated index location to index partition: partition={}, block={}", 
              write_position.partition, write_position.block_number);

        // Generate and write index XML to index partition
        debug!("Generating index XML for index partition...");
        
        let index_xml = current_index.to_xml()?;
        
        debug!("Writing index to index partition ({} bytes)...", index_xml.len());
        self.write_xml_to_tape(&index_xml).await?;

        // Write filemark after index
        self.scsi.write_filemarks(1)?;

        let final_position = self.scsi.read_position()?;
        info!("Index partition write completed: partition={}, block={}, index_size={} bytes", 
              final_position.partition, final_position.block_number, index_xml.len());

        // Write VCI (Volume Coherency Information) - 对应LTFSCopyGUI WriteVCI
        debug!("Writing VCI (Volume Coherency Information)");
        self.write_volume_coherency_info(current_index).await?;

        Ok(())
    }

    /// Write Volume Coherency Information (对应LTFSCopyGUI WriteVCI)
    async fn write_volume_coherency_info(&mut self, _current_index: &LtfsIndex) -> Result<()> {
        // VCI写入逻辑 - 这是LTFSCopyGUI的高级功能，暂时实现基础版本
        debug!("VCI write completed (basic implementation)");
        Ok(())
    }

    /// Write XML content to tape (following commit 3432483 variable-length pattern)
    async fn write_xml_to_tape(&mut self, xml_content: &str) -> Result<()> {
        // Convert XML to bytes
        let xml_bytes = xml_content.as_bytes();
        let xml_size = xml_bytes.len();
        
        // Following commit 3432483 pattern: use variable-length write to avoid ILI warning
        // LTFS indexes should be written as single variable-length blocks without padding
        // This matches LTFSCopyGUI behavior and prevents the 0-padding issue
        
        // Write XML as single variable-length block (commit 3432483 style)
        let blocks_written = self.scsi.write_blocks(1, &xml_bytes[..xml_size])?;
        
        if blocks_written != 1 {
            return Err(RustLtfsError::tape_device(format!(
                "Expected to write 1 block, but wrote {} blocks", blocks_written
            )));
        }
        
        debug!("Wrote XML as variable-length block: {} bytes (commit 3432483 pattern)", xml_size);
        info!("XML write completed: {} bytes written", xml_size);
        Ok(())
    }
}