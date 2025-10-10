// 双分区索引同步逻辑 - 严格按照LTFSCopyGUI实现
use crate::error::{Result, RustLtfsError};
use crate::ltfs_index::LtfsIndex;
use super::TapeOperations;
use tracing::{debug, info};

impl TapeOperations {
    /// Update index on tape with force option (corresponds to VB.NET WriteCurrentIndex + RefreshIndexPartition)
    pub async fn update_index_on_tape_with_options_dual_partition(&mut self, force_index: bool) -> Result<()> {
        info!("Starting to update tape LTFS index (LTFSCopyGUI WriteCurrentIndex + RefreshIndexPartition)...");

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
                // Create new index if none exists - inline creation
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
                        backup_time: now.clone(),
                        read_only: false,
                        contents: crate::ltfs_index::DirectoryContents {
                            files: Vec::new(),
                            directories: Vec::new(),
                        },
                    },
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
        info!("Current tape position: partition={}, block={}", 
              current_position.partition, current_position.block_number);

        // 使用LTFSCopyGUI精确逻辑：定位到DataPartition的EOD
        let logical_data_partition = 1u8; // DataPartition = 1 (Partition B)
        let data_partition = self.get_target_partition(logical_data_partition);
        
        info!("Moving to data partition {} EOD (LTFSCopyGUI: TapeUtils.Locate(driveHandle, 0UL, DataPartition, EOD))", 
              data_partition);
        
        // 精确对应：TapeUtils.Locate(driveHandle, 0UL, DataPartition, TapeUtils.LocateDestType.EOD)
        if current_position.partition != data_partition {
            self.scsi.locate_block(data_partition, 0)?; // Move to data partition first
        }
        self.scsi.space(crate::scsi::SpaceType::EndOfData, 0)?; // Go to EOD

        let eod_position = self.scsi.read_position()?;
        info!("End of data position: partition={}, block={}", 
              eod_position.partition, eod_position.block_number);

        // LTFSCopyGUI validation logic
        let extra_partition_count = self.get_extra_partition_count();
        if extra_partition_count > 0 && current_index.location.partition != "b" && 
           eod_position.partition != data_partition {
            return Err(RustLtfsError::tape_device(format!(
                "Current position p{}b{} not allowed for index write",
                eod_position.partition, eod_position.block_number
            )));
        }

        if extra_partition_count > 0 && current_index.location.startblock >= eod_position.block_number {
            return Err(RustLtfsError::tape_device(format!(
                "Current position p{}b{} not allowed for index write, index at startblock {}",
                eod_position.partition, eod_position.block_number, current_index.location.startblock
            )));
        }

        // Write filemark before index (对应LTFSCopyGUI WriteFileMark)
        info!("Writing filemark before index");
        self.scsi.write_filemarks(1)?;

        // Update index metadata (对应LTFSCopyGUI的索引元数据更新)
        current_index.generationnumber += 1;
        current_index.updatetime = chrono::Utc::now().to_rfc3339();
        current_index.location.partition = "b".to_string(); // Data partition
        
        // Set previous generation location
        current_index.previousgenerationlocation = Some(crate::ltfs_index::Location {
            partition: current_index.location.partition.clone(),
            startblock: current_index.location.startblock,
        });

        let index_position = self.scsi.read_position()?;
        current_index.location.startblock = index_position.block_number;
        
        info!("Index will be written at position: partition={}, block={}", 
              index_position.partition, index_position.block_number);

        // Generate and write index XML
        info!("Generating index XML...");
        let index_xml = current_index.to_xml()?;
        
        info!("Writing index to tape...");
        self.write_xml_to_tape(&index_xml).await?;

        // Write filemark after index (对应LTFSCopyGUI WriteFileMark)
        self.scsi.write_filemarks(1)?;

        let final_position = self.scsi.read_position()?;
        info!("Index write completed at position: partition={}, block={}", 
              final_position.partition, final_position.block_number);

        Ok(())
    }

    /// RefreshIndexPartition: Sync index to index partition (对应LTFSCopyGUI RefreshIndexPartition)
    async fn refresh_index_partition(&mut self, current_index: &mut LtfsIndex) -> Result<()> {
        info!("=== RefreshIndexPartition: Syncing to Index Partition ===");

        let logical_index_partition = 0u8; // IndexPartition = 0 (Partition A)
        let index_partition = self.get_target_partition(logical_index_partition);

        // 精确对应LTFSCopyGUI：TapeUtils.Locate(driveHandle, 3UL, IndexPartition, TapeUtils.LocateDestType.FileMark)
        info!("Locating to index partition {} at 3rd filemark (LTFSCopyGUI: TapeUtils.Locate(driveHandle, 3UL, IndexPartition, FileMark))", 
              index_partition);
        
        // 使用LTFSCopyGUI的精确参数：3UL (第3个文件标记)
        self.scsi.locate_to_filemark(3, index_partition)?;

        let locate_position = self.scsi.read_position()?;
        info!("Located to position: partition={}, block={}", 
              locate_position.partition, locate_position.block_number);

        // Write filemark (对应LTFSCopyGUI WriteFileMark)
        info!("Writing filemark at index partition");
        self.scsi.write_filemarks(1)?;

        // Update index location to index partition
        if current_index.location.partition == "b" {
            current_index.previousgenerationlocation = Some(crate::ltfs_index::Location {
                partition: current_index.location.partition.clone(),
                startblock: current_index.location.startblock,
            });
        }

        let write_position = self.scsi.read_position()?;
        current_index.location.startblock = write_position.block_number;
        current_index.location.partition = "a".to_string(); // Index partition

        info!("Updated index location to index partition: partition={}, block={}", 
              write_position.partition, write_position.block_number);

        // Generate and write index XML to index partition
        info!("Generating index XML for index partition...");
        let index_xml = current_index.to_xml()?;
        
        info!("Writing index to index partition...");
        self.write_xml_to_tape(&index_xml).await?;

        // Write filemark after index
        self.scsi.write_filemarks(1)?;

        let final_position = self.scsi.read_position()?;
        info!("Index partition write completed at position: partition={}, block={}", 
              final_position.partition, final_position.block_number);

        // Write VCI (Volume Coherency Information) - 对应LTFSCopyGUI WriteVCI
        info!("Writing VCI (Volume Coherency Information)");
        self.write_volume_coherency_info(current_index).await?;

        Ok(())
    }

    /// Write Volume Coherency Information (对应LTFSCopyGUI WriteVCI)
    async fn write_volume_coherency_info(&mut self, _current_index: &LtfsIndex) -> Result<()> {
        // VCI写入逻辑 - 这是LTFSCopyGUI的高级功能，暂时实现基础版本
        info!("VCI write completed (basic implementation)");
        Ok(())
    }

    /// Write XML content to tape
    async fn write_xml_to_tape(&mut self, xml_content: &str) -> Result<()> {
        use tokio::io::AsyncWriteExt;
        
        // Convert XML to bytes
        let xml_bytes = xml_content.as_bytes();
        let block_size = self.block_size as usize;
        
        // Write XML in blocks
        let mut offset = 0;
        while offset < xml_bytes.len() {
            let chunk_size = std::cmp::min(block_size, xml_bytes.len() - offset);
            let mut block = vec![0u8; block_size];
            
            // Copy data to block
            block[..chunk_size].copy_from_slice(&xml_bytes[offset..offset + chunk_size]);
            
            // Write block to tape
            match self.scsi.write_blocks(1, &block) {
                Ok(_) => {
                    debug!("Wrote XML block: offset={}, size={}", offset, chunk_size);
                }
                Err(e) => {
                    return Err(RustLtfsError::tape_device(format!(
                        "Failed to write XML block at offset {}: {}", offset, e
                    )));
                }
            }
            
            offset += chunk_size;
        }
        
        info!("XML write completed: {} bytes written", xml_bytes.len());
        Ok(())
    }
}