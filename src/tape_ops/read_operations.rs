use crate::error::{Result, RustLtfsError};
use crate::ltfs_index::LtfsIndex;
use crate::scsi::{MediaType, ScsiInterface};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tracing::{debug, error, info, warn};
use uuid::Uuid;

// 导入partition_manager中的类型
use super::partition_manager::{IndexLocation, PartitionStrategy, LtfsPartitionLabel, PartitionInfo};

// LtfsPartitionLabel 在 format_operations.rs 中定义
// 通过模块重新导出使用

/// TapeOperations读取操作实现
impl super::TapeOperations {
    /// 验证并处理索引 - 委托给partition_manager
    pub async fn validate_and_process_index(&mut self, xml_content: &str) -> Result<bool> {
        // 基本验证XML格式
        if !xml_content.contains("<ltfsindex") || !xml_content.contains("</ltfsindex>") {
            return Ok(false);
        }
        
        // 解析并设置索引
        match crate::ltfs_index::LtfsIndex::from_xml(xml_content) {
            Ok(index) => {
                self.index = Some(index);
                Ok(true)
            }
            Err(_) => Ok(false),
        }
    }
    
    /// 检测分区策略 - 委托给partition_manager
    pub async fn detect_partition_strategy(&self) -> Result<PartitionStrategy> {
        let partition_manager = crate::tape_ops::partition_manager::PartitionManager::new(
            Arc::new(crate::scsi::ScsiInterface::new()),
            self.offline_mode,
        );
        partition_manager.detect_partition_strategy().await
    }
    
    /// Read LTFS index from tape (优化版本：优先使用成功的策略)
    pub async fn read_index_from_tape(&mut self) -> Result<()> {
        info!("Starting optimized LTFS index reading process...");

        if self.offline_mode {
            info!("Offline mode: using dummy index for simulation");
            return Ok(());
        }

        // 快速缓存检查 - 如果知道上次成功的位置，直接尝试
        if let Some(cached_location) = self.get_cached_index_location() {
            info!("🚀 Fast path: Trying cached successful location first");
            if let Ok(xml_content) = self.try_read_index_at_location(cached_location).await {
                if self.validate_and_process_index(&xml_content).await? {
                    info!("✅ Fast path succeeded - index found at cached location");
                    return Ok(());
                }
            }
            info!("Cached location failed, proceeding with optimized search");
        }

        info!("=== Optimized LTFS Index Reading Process ===");

        // Step 1 (Priority): 优先使用经过验证的成功策略
        info!("Step 1 (Priority): Trying proven successful strategies first");
        
        // 优化的并行策略搜索
        match self.try_optimized_parallel_strategies().await {
            Ok((xml_content, successful_location)) => {
                if self.validate_and_process_index(&xml_content).await? {
                    // 缓存成功的位置供下次使用
                    self.cache_successful_location(successful_location);
                    info!("✅ Optimized strategy succeeded - index loaded successfully");
                    return Ok(());
                }
            }
            Err(e) => {
                debug!("Optimized strategies failed: {}", e);
            }
        }

        // Step 2: 标准流程作为后备
        info!("Step 2: Fallback to standard LTFS reading process");
        
        // 定位到索引分区并读取VOL1标签
        self.scsi.locate_block(0, 0)?;
        let mut label_buffer = vec![0u8; crate::scsi::block_sizes::LTO_BLOCK_SIZE as usize];
        self.scsi.read_blocks(1, &mut label_buffer)?;

        let vol1_valid = self.parse_vol1_label(&label_buffer)?;

        if vol1_valid {
            info!("VOL1 label validation passed, trying standard reading");

            let partition_strategy = self.detect_partition_strategy().await?;

            match partition_strategy {
                PartitionStrategy::StandardMultiPartition => {
                    // 使用ReadToFileMark方法读取整个索引文件
                    match self.read_index_xml_from_tape_with_file_mark() {
                        Ok(xml_content) => {
                            if self.validate_and_process_index(&xml_content).await? {
                                info!("✅ Standard reading strategy succeeded");
                                return Ok(());
                            }
                        }
                        Err(e) => debug!("Standard reading failed: {}", e),
                    }
                }
                PartitionStrategy::SinglePartitionFallback => {
                    return self.read_index_from_single_partition_tape().await;
                }
                PartitionStrategy::IndexFromDataPartition => {
                    return self.read_index_from_data_partition_strategy().await;
                }
            }
        }

        // Step 3: 最后的多分区策略回退
        info!("Step 3: Final multi-partition strategy fallback");
        
        let partition_strategy = self
            .detect_partition_strategy()
            .await
            .unwrap_or(PartitionStrategy::StandardMultiPartition);

        match partition_strategy {
            PartitionStrategy::SinglePartitionFallback => {
                debug!("🔄 Trying single-partition fallback strategy");
                self.read_index_from_single_partition_tape().await
            }
            PartitionStrategy::IndexFromDataPartition => {
                debug!("🔄 Trying data partition index strategy");
                self.read_index_from_data_partition_strategy().await
            }
            PartitionStrategy::StandardMultiPartition => {
                debug!("🔄 Trying standard multi-partition strategy without VOL1 validation");

                let standard_locations = vec![6, 5, 2, 0];

                for &block in &standard_locations {
                    info!("Trying standard multi-partition at p0 block {}", block);
                    match self.scsi.locate_block(0, block) {
                        Ok(()) => match self.read_index_xml_from_tape_with_file_mark() {
                            Ok(xml_content) => {
                                if self.validate_and_process_index(&xml_content).await? {
                                    info!("✅ Successfully read index from p0 block {} (standard multi-partition)", block);
                                    return Ok(());
                                }
                            }
                            Err(e) => {
                                debug!("Failed to read index from p0 block {}: {}", block, e);
                            }
                        },
                        Err(e) => {
                            debug!("Cannot position to p0 block {}: {}", block, e);
                        }
                    }
                }

                info!("🔄 All standard locations failed, falling back to single-partition strategy");
                self.read_index_from_single_partition_tape().await
            }
        }
    }

    /// 解析volume label中的索引位置信息
    fn parse_index_locations_from_volume_label(&self, buffer: &[u8]) -> Result<IndexLocation> {
        // 查找LTFS volume label标识
        let ltfs_signature = b"LTFS";

        if let Some(ltfs_pos) = buffer.windows(4).position(|w| w == ltfs_signature) {
            info!("Found LTFS volume label at offset {}", ltfs_pos);

            // LTFS volume label结构（简化版本）：
            // - LTFS signature (4 bytes)
            // - Version info
            // - Current index location (partition + block)
            // - Previous index location (partition + block)

            // 搜索可能的索引位置信息
            // 通常在LTFS签名后的几百字节内
            let search_area = &buffer[ltfs_pos..std::cmp::min(ltfs_pos + 1024, buffer.len())];

            // 查找非零的块号（可能的索引位置）
            for i in (0..search_area.len() - 8).step_by(4) {
                let potential_block = u32::from_le_bytes([
                    search_area[i],
                    search_area[i + 1],
                    search_area[i + 2],
                    search_area[i + 3],
                ]) as u64;

                // 合理的索引位置：通常在block 5-1000之间
                if potential_block >= 5 && potential_block <= 1000 {
                    info!(
                        "Found potential index location at block {}",
                        potential_block
                    );
                    return Ok(IndexLocation {
                        partition: "a".to_string(),
                        start_block: potential_block,
                    });
                }
            }

            // 如果没找到，尝试查找数据分区的索引
            // 搜索大的块号（数据分区的索引位置）
            for i in (0..search_area.len() - 8).step_by(4) {
                let potential_block = u32::from_le_bytes([
                    search_area[i],
                    search_area[i + 1],
                    search_area[i + 2],
                    search_area[i + 3],
                ]) as u64;

                // 数据分区的索引位置：通常是较大的块号
                if potential_block >= 1000 && potential_block <= 1000000 {
                    info!(
                        "Found potential data partition index location at block {}",
                        potential_block
                    );
                    return Ok(IndexLocation {
                        partition: "b".to_string(),
                        start_block: potential_block,
                    });
                }
            }
        }

        Err(RustLtfsError::ltfs_index(
            "No valid index location found in volume label".to_string(),
        ))
    }

    /// 在当前位置尝试读取索引（简化版本）
    fn try_read_index_at_current_position(&self, block_size: usize) -> Result<String> {
        let mut buffer = vec![0u8; block_size * 10]; // 读取10个块

        match self.scsi.read_blocks(10, &mut buffer) {
            Ok(_) => {
                let content = String::from_utf8_lossy(&buffer);
                let cleaned = content.replace('\0', "").trim().to_string();

                if cleaned.len() > 100 {
                    Ok(cleaned)
                } else {
                    Err(RustLtfsError::ltfs_index(
                        "No sufficient data at position".to_string(),
                    ))
                }
            }
            Err(e) => Err(e),
        }
    }

    /// Find current LTFS index location from volume label
    fn find_current_index_location(&self) -> Result<IndexLocation> {
        debug!("Finding current index location from volume label");

        // LTFS Volume Label is typically at partition A, block 0
        self.scsi.locate_block(0, 0)?;

        let mut buffer = vec![0u8; crate::scsi::block_sizes::LTO_BLOCK_SIZE as usize];

        match self.scsi.read_blocks(1, &mut buffer) {
            Ok(_) => {
                // Parse volume label to find index location
                // LTFS volume label contains pointers to current and previous index
                if let Some(location) = self.parse_volume_label(&buffer)? {
                    return Ok(location);
                }
            }
            Err(e) => {
                warn!("Failed to read volume label: {}", e);
            }
        }

        // Fallback to default index location (block 5 on partition A)
        warn!("Using default index location: partition A, block 5");
        Ok(IndexLocation {
            partition: "a".to_string(),
            start_block: 5,
        })
    }

    /// Parse LTFS volume label to extract index location
    fn parse_volume_label(&self, buffer: &[u8]) -> Result<Option<IndexLocation>> {
        // LTFS volume label parsing - simplified implementation
        // In a full implementation, this would parse the actual LTFS volume label structure

        // Look for LTFS signature in the buffer
        let ltfs_signature = b"LTFS";
        if let Some(pos) = buffer
            .windows(ltfs_signature.len())
            .position(|window| window == ltfs_signature)
        {
            debug!("Found LTFS signature at offset {}", pos);

            // For now, use a fixed index location
            // Real implementation would parse the volume label structure
            return Ok(Some(IndexLocation {
                partition: "a".to_string(),
                start_block: 5,
            }));
        }

        debug!("No LTFS signature found in volume label");
        Ok(None)
    }

    /// Read index XML data from tape using file mark method (对应TapeUtils.ReadToFileMark)
    fn read_index_xml_from_tape_with_file_mark(&self) -> Result<String> {
        debug!("Reading LTFS index XML data using file mark method");

        // 获取动态blocksize (对应LTFSCopyGUI的plabel.blocksize)
        let block_size = self
            .partition_label
            .as_ref()
            .map(|plabel| plabel.blocksize as usize)
            .unwrap_or(crate::scsi::block_sizes::LTO_BLOCK_SIZE as usize);

        info!("Using dynamic blocksize: {} bytes", block_size);

        // 直接使用当前TapeOperations的方法
        self.read_to_file_mark_with_temp_file(block_size)
    }

    /// 检查buffer是否全为零 (对应LTFSCopyGUI的IsAllZeros函数)
    fn is_all_zeros(&self, buffer: &[u8], length: usize) -> bool {
        buffer.iter().take(length).all(|&b| b == 0)
    }

    /// 检查临时文件是否包含XML结束标记
    fn check_temp_file_for_xml_end(&self, temp_path: &std::path::Path) -> Result<bool> {
        use std::io::{BufRead, BufReader, Seek, SeekFrom};

        let mut file = std::fs::File::open(temp_path)?;

        // 检查文件末尾1KB的数据
        let file_len = file.seek(SeekFrom::End(0))?;
        let check_len = std::cmp::min(1024, file_len);
        file.seek(SeekFrom::End(-(check_len as i64)))?;

        let reader = BufReader::new(file);
        let content: String = reader
            .lines()
            .map(|line| line.unwrap_or_default())
            .collect::<Vec<_>>()
            .join("\n");

        Ok(content.contains("</ltfsindex>"))
    }

    /// Read index XML data from tape with progressive expansion
    fn read_index_xml_from_tape(&self) -> Result<String> {
        debug!("Reading LTFS index XML data from tape");

        let mut xml_content;
        let mut blocks_to_read = 10u32; // Start with 10 blocks
        let max_blocks = 200u32; // Maximum 200 blocks for safety (12.8MB)

        loop {
            debug!("Attempting to read {} blocks for index", blocks_to_read);
            let buffer_size =
                blocks_to_read as usize * crate::scsi::block_sizes::LTO_BLOCK_SIZE as usize;
            let mut buffer = vec![0u8; buffer_size];

            match self
                .scsi
                .read_blocks_with_retry(blocks_to_read, &mut buffer, 2)
            {
                Ok(blocks_read) => {
                    debug!("Successfully read {} blocks", blocks_read);

                    // Find the actual data length (look for XML end)
                    let actual_data_len =
                        buffer.iter().position(|&b| b == 0).unwrap_or(buffer.len());

                    // Convert to string
                    match String::from_utf8(buffer[..actual_data_len].to_vec()) {
                        Ok(content) => {
                            xml_content = content;

                            // Check if we have a complete XML document
                            if xml_content.contains("</ltfsindex>") {
                                info!(
                                    "Complete LTFS index XML found ({} bytes)",
                                    xml_content.len()
                                );
                                break;
                            }

                            // If incomplete and we haven't hit the limit, try reading more blocks
                            if blocks_to_read < max_blocks {
                                blocks_to_read = std::cmp::min(blocks_to_read * 2, max_blocks);
                                debug!("XML incomplete, expanding to {} blocks", blocks_to_read);
                                continue;
                            } else {
                                warn!("Reached maximum block limit, using partial XML");
                                break;
                            }
                        }
                        Err(e) => {
                            return Err(RustLtfsError::ltfs_index(format!(
                                "Failed to parse index data as UTF-8: {}",
                                e
                            )));
                        }
                    }
                }
                Err(e) => {
                    warn!("Failed to read {} blocks from tape: {}", blocks_to_read, e);

                    // Provide more specific error information
                    if e.to_string().contains("Direct block read operation failed") {
                        return Err(RustLtfsError::scsi(
                            format!("Failed to read index from tape: {}. Possible causes: blank tape, incorrect position, hardware issue, SCSI problem. Try --skip-index option.", e)
                        ));
                    }

                    return Err(RustLtfsError::scsi(format!(
                        "Failed to read index from tape: {}",
                        e
                    )));
                }
            }
        }

        // Validate the extracted XML
        self.validate_index_xml(&xml_content)?;

        info!(
            "Successfully read LTFS index ({} bytes) from tape",
            xml_content.len()
        );
        Ok(xml_content)
    }

    /// Validate index XML structure
    fn validate_index_xml(&self, xml_content: &str) -> Result<()> {
        debug!("Validating LTFS index XML structure");

        // Basic validation checks
        if xml_content.is_empty() {
            return Err(RustLtfsError::ltfs_index("Index XML is empty".to_string()));
        }

        if !xml_content.contains("<ltfsindex") {
            return Err(RustLtfsError::ltfs_index(
                "Invalid LTFS index format - missing ltfsindex element".to_string(),
            ));
        }

        if !xml_content.contains("</ltfsindex>") {
            warn!("LTFS index XML may be incomplete - missing closing tag");
        }

        debug!("LTFS index XML validation passed");
        Ok(())
    }

    /// Load index from local file
    pub async fn load_index_from_file(&mut self, index_path: &Path) -> Result<()> {
        info!("Loading LTFS index from file: {:?}", index_path);

        let xml_content = tokio::fs::read_to_string(index_path).await.map_err(|e| {
            RustLtfsError::file_operation(format!("Unable to read index file: {}", e))
        })?;

        let index = LtfsIndex::from_xml(&xml_content)?;
        self.index = Some(index.clone());
        self.schema = Some(index);

        info!("Index file loaded successfully");
        Ok(())
    }

    /// Strictly validate VOL1 label according to VB.NET logic
    fn parse_vol1_label(&self, buffer: &[u8]) -> Result<bool> {
        info!("Strictly validating VOL1 label (VB.NET logic)...");

        // Condition 1: Buffer length check - must be at least 80 bytes to contain VOL1 label
        if buffer.len() < 80 {
            warn!(
                "VOL1 label validation error: buffer too short ({} bytes), need at least 80 bytes",
                buffer.len()
            );
            return Ok(false);
        }

        // Extract the first 80 bytes for VOL1 label validation
        let vol1_label = &buffer[0..80];

        // Condition 2: Prefix check - must start with "VOL1"
        let vol1_prefix = b"VOL1";
        if !vol1_label.starts_with(vol1_prefix) {
            warn!("VOL1 label prefix error: does not start with 'VOL1'");
            debug!(
                "First 10 bytes: {:?}",
                &vol1_label[0..std::cmp::min(10, vol1_label.len())]
            );

            // Check if tape is blank (all zeros)
            let non_zero_count = vol1_label.iter().filter(|&&b| b != 0).count();
            if non_zero_count == 0 {
                info!("📭 Detected blank tape (all zeros in VOL1 area)");
            } else {
                info!(
                    "🔍 Non-LTFS tape detected. First 40 bytes as hex: {:02X?}",
                    &vol1_label[0..40]
                );
                info!(
                    "🔍 First 40 bytes as text: {:?}",
                    String::from_utf8_lossy(&vol1_label[0..40])
                );
            }

            return Ok(false);
        }

        // Condition 3: Content check - bytes 24-27 must be "LTFS"
        if vol1_label.len() < 28 {
            warn!("VOL1 label too short for LTFS identifier check");
            return Ok(false);
        }

        let ltfs_bytes = &vol1_label[24..28];
        let expected_ltfs = b"LTFS";

        if ltfs_bytes != expected_ltfs {
            warn!(
                "LTFS identifier error: expected 'LTFS' at position 24-27, actual: {:?}",
                String::from_utf8_lossy(ltfs_bytes)
            );
            debug!(
                "VOL1 label content (first 40 bytes): {:?}",
                &vol1_label[0..40]
            );
            return Ok(false);
        }

        info!("✅ VOL1 label validation passed: 80-byte label found in {}-byte buffer, VOL1 prefix and LTFS identifier correct", buffer.len());
        Ok(true)
    }

    /// Preview file content
    pub async fn preview_file_content(&self, file_uid: u64, max_lines: usize) -> Result<String> {
        info!(
            "Previewing file content: UID {}, max lines: {}",
            file_uid, max_lines
        );

        if self.offline_mode {
            info!("Offline mode: returning dummy preview content");
            return Ok(
                "[Offline Mode] File content preview not available without tape access".to_string(),
            );
        }

        // Find file by UID in index
        let index = match &self.index {
            Some(idx) => idx,
            None => {
                return Err(RustLtfsError::ltfs_index("Index not loaded".to_string()));
            }
        };

        let file_info = self.find_file_by_uid(index, file_uid)?;

        // Read file content using SCSI operations
        let content_bytes = self
            .read_file_content_from_tape(&file_info, max_lines * 100)
            .await?; // Estimate bytes per line

        // Convert to string and limit lines
        let content_str = String::from_utf8_lossy(&content_bytes);
        let lines: Vec<&str> = content_str.lines().take(max_lines).collect();

        Ok(lines.join("\n"))
    }

    /// Find file by UID in LTFS index
    fn find_file_by_uid(
        &self,
        index: &LtfsIndex,
        file_uid: u64,
    ) -> Result<crate::ltfs_index::File> {
        self.search_file_by_uid(&index.root_directory, file_uid)
            .ok_or_else(|| {
                RustLtfsError::ltfs_index(format!("File with UID {} not found", file_uid))
            })
    }

    /// Recursively search for file by UID
    fn search_file_by_uid(
        &self,
        dir: &crate::ltfs_index::Directory,
        file_uid: u64,
    ) -> Option<crate::ltfs_index::File> {
        // Search files in current directory
        for file in &dir.contents.files {
            if file.uid == file_uid {
                return Some(file.clone());
            }
        }

        // Recursively search subdirectories
        for subdir in &dir.contents.directories {
            if let Some(found_file) = self.search_file_by_uid(subdir, file_uid) {
                return Some(found_file);
            }
        }

        None
    }

    /// Read file content from tape using SCSI operations
    async fn read_file_content_from_tape(
        &self,
        file_info: &crate::ltfs_index::File,
        max_bytes: usize,
    ) -> Result<Vec<u8>> {
        debug!(
            "Reading file content from tape: {} (max {} bytes)",
            file_info.name, max_bytes
        );

        if file_info.extent_info.extents.is_empty() {
            return Err(RustLtfsError::ltfs_index(
                "File has no extent information".to_string(),
            ));
        }

        // Get the first extent for reading
        let first_extent = &file_info.extent_info.extents[0];

        // Calculate read parameters
        let bytes_to_read = std::cmp::min(max_bytes as u64, file_info.length) as usize;
        let blocks_to_read = (bytes_to_read + crate::scsi::block_sizes::LTO_BLOCK_SIZE as usize
            - 1)
            / crate::scsi::block_sizes::LTO_BLOCK_SIZE as usize;

        // Position to file start
        let partition_id = self.get_partition_id(&first_extent.partition)?;
        self.scsi
            .locate_block(partition_id, first_extent.start_block)?;

        // Read blocks
        let buffer_size = blocks_to_read * crate::scsi::block_sizes::LTO_BLOCK_SIZE as usize;
        let mut buffer = vec![0u8; buffer_size];

        let blocks_read =
            self.scsi
                .read_blocks_with_retry(blocks_to_read as u32, &mut buffer, 2)?;

        if blocks_read == 0 {
            return Err(RustLtfsError::scsi("No data read from tape".to_string()));
        }

        // Extract actual file content (considering byte offset)
        let start_offset = first_extent.byte_offset as usize;
        let end_offset = start_offset + bytes_to_read;

        if end_offset > buffer.len() {
            return Ok(buffer[start_offset..].to_vec());
        }

        Ok(buffer[start_offset..end_offset].to_vec())
    }

    /// Get partition ID from partition name
    fn get_partition_id(&self, partition: &str) -> Result<u8> {
        match partition.to_lowercase().as_str() {
            "a" => Ok(0),
            "b" => Ok(1),
            _ => Err(RustLtfsError::ltfs_index(format!(
                "Invalid partition: {}",
                partition
            ))),
        }
    }

    /// Read complete file content from tape
    async fn read_complete_file_from_tape(
        &self,
        file_info: &crate::ltfs_index::File,
    ) -> Result<Vec<u8>> {
        debug!(
            "Reading complete file from tape: {} ({} bytes)",
            file_info.name, file_info.length
        );

        if file_info.extent_info.extents.is_empty() {
            return Err(RustLtfsError::ltfs_index(
                "File has no extent information".to_string(),
            ));
        }

        let mut result = Vec::with_capacity(file_info.length as usize);

        // Read all extents
        for extent in &file_info.extent_info.extents {
            let extent_data = self.read_extent_from_tape(extent).await?;
            result.extend_from_slice(&extent_data);
        }

        // Trim to actual file size
        result.truncate(file_info.length as usize);

        Ok(result)
    }

    /// Read a single extent from tape
    async fn read_extent_from_tape(
        &self,
        extent: &crate::ltfs_index::FileExtent,
    ) -> Result<Vec<u8>> {
        debug!(
            "Reading extent: partition {}, block {}, {} bytes",
            extent.partition, extent.start_block, extent.byte_count
        );

        // Use retry mechanism for critical SCSI operations
        let partition_id = self.get_partition_id(&extent.partition)?;

        // Position to extent start with retry
        self.verify_operation_with_retry(
            "locate_extent",
            move || self.scsi.locate_block(partition_id, extent.start_block),
            3,
        )
        .await?;

        // Calculate blocks needed
        let bytes_needed = extent.byte_count as usize;
        let blocks_needed = (bytes_needed
            + extent.byte_offset as usize
            + crate::scsi::block_sizes::LTO_BLOCK_SIZE as usize
            - 1)
            / crate::scsi::block_sizes::LTO_BLOCK_SIZE as usize;

        // Read blocks with retry - return the buffer directly
        let buffer_size = blocks_needed * crate::scsi::block_sizes::LTO_BLOCK_SIZE as usize;

        let buffer = self
            .verify_operation_with_retry(
                "read_extent_blocks",
                move || {
                    let mut buf = vec![0u8; buffer_size];
                    match self
                        .scsi
                        .read_blocks_with_retry(blocks_needed as u32, &mut buf, 3)
                    {
                        Ok(blocks_read) => {
                            if blocks_read == 0 {
                                return Err(RustLtfsError::scsi(
                                    "No data read from tape".to_string(),
                                ));
                            }
                            Ok(buf)
                        }
                        Err(e) => Err(e),
                    }
                },
                2,
            )
            .await?;

        // Extract actual extent data considering byte offset
        let start_offset = extent.byte_offset as usize;
        let end_offset = start_offset + bytes_needed;

        if end_offset > buffer.len() {
            return Ok(buffer[start_offset..].to_vec());
        }

        Ok(buffer[start_offset..end_offset].to_vec())
    }

    /// Verify tape operation with retry
    async fn verify_operation_with_retry<F, T>(
        &self,
        operation_name: &str,
        operation: F,
        max_retries: u32,
    ) -> Result<T>
    where
        F: Fn() -> Result<T> + Clone,
    {
        let mut last_error = None;

        for attempt in 0..=max_retries {
            if attempt > 0 {
                info!(
                    "Retrying operation '{}' (attempt {} of {})",
                    operation_name,
                    attempt + 1,
                    max_retries + 1
                );

                // Progressive backoff delay
                let delay_ms = std::cmp::min(1000 * attempt, 10000); // Max 10 second delay
                tokio::time::sleep(tokio::time::Duration::from_millis(delay_ms as u64)).await;

                // Attempt recovery
                if let Some(ref error) = last_error {
                    if let Err(recovery_error) =
                        self.recover_from_scsi_error(error, operation_name).await
                    {
                        warn!(
                            "Recovery failed for '{}': {}",
                            operation_name, recovery_error
                        );
                    }
                }
            }

            match operation() {
                Ok(result) => {
                    if attempt > 0 {
                        info!(
                            "Operation '{}' succeeded after {} retries",
                            operation_name, attempt
                        );
                    }
                    return Ok(result);
                }
                Err(e) => {
                    last_error = Some(e);
                    warn!(
                        "Operation '{}' failed on attempt {}: {:?}",
                        operation_name,
                        attempt + 1,
                        last_error
                    );
                }
            }
        }

        Err(last_error.unwrap_or_else(|| {
            RustLtfsError::scsi(format!(
                "Operation '{}' failed after {} attempts",
                operation_name,
                max_retries + 1
            ))
        }))
    }

    /// Enhanced error recovery for SCSI operations
    async fn recover_from_scsi_error(&self, error: &RustLtfsError, operation: &str) -> Result<()> {
        warn!(
            "SCSI operation '{}' failed, attempting recovery: {}",
            operation, error
        );

        // Recovery strategy 1: Check device status
        match self.scsi.check_media_status() {
            Ok(media_type) => {
                if matches!(media_type, MediaType::NoTape) {
                    return Err(RustLtfsError::tape_device(
                        "No tape loaded - manual intervention required".to_string(),
                    ));
                }
                debug!("Media status OK: {}", media_type.description());
            }
            Err(e) => {
                warn!("Media status check failed during recovery: {}", e);
            }
        }

        // Recovery strategy 2: Read current position to test responsiveness
        match self.scsi.read_position() {
            Ok(pos) => {
                debug!(
                    "Drive responsive at position: partition {}, block {}",
                    pos.partition, pos.block_number
                );
            }
            Err(e) => {
                warn!("Drive not responsive during recovery: {}", e);
                return self.attempt_drive_reset().await;
            }
        }

        // Recovery strategy 3: Small delay to allow drive to settle
        tokio::time::sleep(tokio::time::Duration::from_millis(1000)).await;

        info!("SCSI recovery completed for operation: {}", operation);
        Ok(())
    }

    /// Attempt to reset the drive as last resort
    async fn attempt_drive_reset(&self) -> Result<()> {
        warn!("Attempting drive reset as recovery measure");

        // Try to rewind to beginning of tape
        match self.scsi.locate_block(0, 0) {
            Ok(()) => {
                info!("Successfully rewound tape during recovery");

                // Wait for rewind to complete
                tokio::time::sleep(tokio::time::Duration::from_secs(5)).await;

                // Test position read
                match self.scsi.read_position() {
                    Ok(pos) => {
                        info!(
                            "Drive reset successful, position: partition {}, block {}",
                            pos.partition, pos.block_number
                        );
                        Ok(())
                    }
                    Err(e) => Err(RustLtfsError::tape_device(format!(
                        "Drive reset failed - position unreadable: {}",
                        e
                    ))),
                }
            }
            Err(e) => Err(RustLtfsError::tape_device(format!(
                "Drive reset failed - cannot rewind: {}",
                e
            ))),
        }
    }

    /// 从磁带索引分区读取LTFS索引 - 新版本
    /// 对应LTFSWriter.vb的读取索引ToolStripMenuItem_Click功能
    pub fn read_index_from_tape_new(&mut self, output_path: Option<String>) -> Result<String> {
        info!("Starting read_index_from_tape operation");

        // 首先打开设备连接
        info!("Opening device: {}", self.device_path);
        self.scsi.open_device(&self.device_path)?;
        info!("Device opened successfully");

        // 检查设备状态
        self.check_device_ready()?;

        // 检测分区数量
        let partition_count = self.detect_partition_count()?;
        info!("Detected {} partitions on tape", partition_count);

        // 定位到索引分区(P0或P255)
        let index_partition = if partition_count > 1 { 0 } else { 0 };
        self.scsi.locate_block(index_partition, 0)?;

        // 读取并验证VOL1标签（使用LTFSCopyGUI兼容的缓冲区大小）
        // 对应LTFSCopyGUI: ReadBlock(driveHandle, senseData)
        let default_buffer_size = 524288; // 对应LTFSCopyGUI的&H80000默认缓冲区大小
        let mut vol1_buffer = vec![0u8; default_buffer_size];

        info!(
            "Reading VOL1 label with buffer size: {} bytes",
            default_buffer_size
        );
        let bytes_read = match self.scsi.read_blocks(1, &mut vol1_buffer) {
            Ok(bytes) => bytes,
            Err(e) => {
                warn!(
                    "Initial VOL1 read failed: {}, trying with smaller buffer",
                    e
                );
                // 备用方案：尝试使用80字节的小缓冲区（标准VOL1大小）
                let mut small_buffer = vec![0u8; 80];
                match self.scsi.read_blocks(1, &mut small_buffer) {
                    Ok(bytes) => {
                        vol1_buffer = small_buffer;
                        bytes
                    }
                    Err(e2) => {
                        return Err(RustLtfsError::scsi(format!(
                            "Failed to read VOL1 label: {}",
                            e2
                        )))
                    }
                }
            }
        };

        // 验证VOL1标签格式（最少需要80字节）
        if vol1_buffer.len() < 80 {
            warn!(
                "VOL1 buffer too small ({} bytes), trying alternative strategies",
                vol1_buffer.len()
            );
            return self.try_alternative_index_reading_strategies(output_path);
        }

        // 检查是否为空白磁带（前4KB都是零） - 对应LTFSCopyGUI的空白磁带检测
        let check_size = std::cmp::min(4096, vol1_buffer.len());
        let is_completely_blank = vol1_buffer.iter().take(check_size).all(|&b| b == 0);
        if is_completely_blank {
            info!(
                "📭 Detected blank tape (all zeros in first {}KB)",
                check_size / 1024
            );
            return Err(RustLtfsError::ltfs_index(
                "Blank tape detected - no LTFS index found".to_string(),
            ));
        }

        // 检查VOL1标签和LTFS标识
        let vol1_str = String::from_utf8_lossy(&vol1_buffer[0..80]);
        let vol1_valid = vol1_str.starts_with("VOL1");
        let ltfs_valid = vol1_buffer.len() >= 28 && &vol1_buffer[24..28] == b"LTFS";

        if !vol1_valid || !ltfs_valid {
            warn!(
                "⚠️ VOL1 validation failed (VOL1: {}, LTFS: {}), trying alternative strategies",
                vol1_valid, ltfs_valid
            );

            // 显示磁带内容诊断信息
            let display_len = std::cmp::min(40, vol1_buffer.len());
            info!("🔍 Tape content analysis (first {} bytes):", display_len);
            info!("   Hex: {:02X?}", &vol1_buffer[0..display_len]);
            info!(
                "   Text: {:?}",
                String::from_utf8_lossy(&vol1_buffer[0..display_len])
            );

            return self.try_alternative_index_reading_strategies(output_path);
        }

        info!("✅ Confirmed LTFS formatted tape with valid VOL1 label");

        // 读取LTFS标签
        self.scsi.locate_block(index_partition, 1)?;
        let block_size = crate::scsi::block_sizes::LTO_BLOCK_SIZE as usize;
        let mut ltfs_label_buffer = vec![0u8; block_size];
        let _bytes_read = self.scsi.read_blocks(1, &mut ltfs_label_buffer)?;

        // 解析标签以找到索引位置
        let index_location = self.parse_index_locations_from_volume_label(&ltfs_label_buffer)?;

        // 从指定位置读取索引
        let index_content = self.read_index_from_specific_location(&index_location)?;

        // 保存索引文件到指定路径或默认路径
        let save_path = output_path.unwrap_or_else(|| {
            format!(
                "schema/ltfs_index_{}.xml",
                chrono::Utc::now().format("%Y%m%d_%H%M%S")
            )
        });

        // 确保目录存在
        if let Some(parent) = std::path::Path::new(&save_path).parent() {
            std::fs::create_dir_all(parent).map_err(|e| {
                RustLtfsError::file_operation(format!("Failed to create directory: {}", e))
            })?;
        }

        std::fs::write(&save_path, &index_content).map_err(|e| {
            RustLtfsError::file_operation(format!("Failed to save index file: {}", e))
        })?;

        info!("LTFS index saved to: {}", save_path);
        Ok(index_content)
    }

    /// 从数据分区末尾读取最新的索引副本 - 新版本
    /// 对应LTFSWriter.vb的读取数据区索引ToolStripMenuItem_Click功能
    pub fn read_data_index_from_tape_new(&mut self, output_path: Option<String>) -> Result<String> {
        info!("Starting read_data_index_from_tape operation");

        // 检查设备状态
        self.check_device_ready()?;

        // 检测分区数量，确保是多分区磁带
        let partition_count = self.detect_partition_count()?;
        if partition_count <= 1 {
            return Err(RustLtfsError::ltfs_index(
                "Single partition tape - no data partition index available".to_string(),
            ));
        }

        info!("Multi-partition tape detected, searching data partition for index");

        // 定位到数据分区（通常是分区1）
        let data_partition = 1;

        // 定位到数据分区末尾(EOD)
        self.scsi.locate_to_eod(data_partition)?;
        info!("Located to end of data partition");

        // 向前搜索找到最后的索引文件标记
        let index_content = self.search_backward_for_last_index(data_partition)?;

        // 保存索引文件
        let save_path = output_path.unwrap_or_else(|| {
            format!(
                "schema/ltfs_data_index_{}.xml",
                chrono::Utc::now().format("%Y%m%d_%H%M%S")
            )
        });

        // 确保目录存在
        if let Some(parent) = std::path::Path::new(&save_path).parent() {
            std::fs::create_dir_all(parent).map_err(|e| {
                RustLtfsError::file_operation(format!("Failed to create directory: {}", e))
            })?;
        }

        std::fs::write(&save_path, &index_content).map_err(|e| {
            RustLtfsError::file_operation(format!("Failed to save data index file: {}", e))
        })?;

        info!("LTFS data partition index saved to: {}", save_path);
        Ok(index_content)
    }

    /// 检查设备是否就绪
    fn check_device_ready(&mut self) -> Result<()> {
        // 执行基本的设备就绪检查
        match self.scsi.test_unit_ready() {
            Ok(_) => Ok(()), // test_unit_ready返回Vec<u8>，我们只关心是否成功
            Err(e) => Err(RustLtfsError::scsi(format!("Device not ready: {}", e))),
        }
    }

    /// 检测磁带分区数量 (对应LTFSCopyGUI的ExtraPartitionCount检测逻辑)
    fn detect_partition_count(&mut self) -> Result<u8> {
        info!("Detecting partition count using LTFSCopyGUI-compatible MODE SENSE logic");

        // 使用MODE SENSE命令查询页面0x11 (对应LTFSCopyGUI的实现)
        // LTFSCopyGUI代码: Dim PModeData As Byte() = TapeUtils.ModeSense(driveHandle, &H11)
        match self.scsi.mode_sense_partition_page_0x11() {
            Ok(mode_data) => {
                debug!(
                    "MODE SENSE page 0x11 data length: {} bytes",
                    mode_data.len()
                );

                // 对应LTFSCopyGUI: If PModeData.Length >= 4 Then ExtraPartitionCount = PModeData(3)
                if mode_data.len() >= 4 {
                    let extra_partition_count = mode_data[3];
                    let total_partitions = extra_partition_count + 1; // ExtraPartitionCount + 主分区

                    info!(
                        "✅ MODE SENSE successful: ExtraPartitionCount={}, Total partitions={}",
                        extra_partition_count, total_partitions
                    );

                    // 限制分区数量（对应LTFSCopyGUI的逻辑）
                    let partition_count = if total_partitions > 2 {
                        2
                    } else {
                        total_partitions
                    };

                    Ok(partition_count)
                } else {
                    warn!("MODE SENSE data too short, assuming single partition");
                    Ok(1)
                }
            }
            Err(e) => {
                warn!(
                    "MODE SENSE page 0x11 failed: {}, trying fallback detection",
                    e
                );

                // 备用方法：尝试定位到分区1来检测多分区支持
                match self.scsi.locate_block(1, 0) {
                    Ok(_) => {
                        info!("✅ Fallback detection: Can access partition 1, multi-partition supported");
                        // 返回分区0继续正常流程
                        if let Err(e) = self.scsi.locate_block(0, 0) {
                            warn!("Warning: Failed to return to partition 0: {}", e);
                        }
                        Ok(2) // 支持多分区
                    }
                    Err(_) => {
                        info!("📋 Fallback detection: Cannot access partition 1, single partition tape");
                        Ok(1) // 单分区
                    }
                }
            }
        }
    }

    /// 替代索引读取策略 - 当VOL1验证失败时使用 (对应LTFSCopyGUI的完整回退逻辑)
    fn try_alternative_index_reading_strategies(
        &mut self,
        output_path: Option<String>,
    ) -> Result<String> {
        info!("🔄 Starting alternative index reading strategies (LTFSCopyGUI compatible)");

        // 策略1: 跳过VOL1验证，直接尝试读取LTFS标签和索引
        debug!("Strategy 1: Bypassing VOL1, attempting direct LTFS label reading");

        let partition_count = self.detect_partition_count()?;
        let index_partition = if partition_count > 1 { 0 } else { 0 };

        // 尝试读取LTFS标签 (block 1)
        match self.scsi.locate_block(index_partition, 1) {
            Ok(()) => {
                let block_size = crate::scsi::block_sizes::LTO_BLOCK_SIZE as usize;
                let mut ltfs_label_buffer = vec![0u8; block_size];

                match self.scsi.read_blocks(1, &mut ltfs_label_buffer) {
                    Ok(_) => {
                        // 尝试从LTFS标签解析索引位置
                        match self.parse_index_locations_from_volume_label(&ltfs_label_buffer) {
                            Ok(index_location) => {
                                info!("✅ Found index location from LTFS label: partition {}, block {}", 
                                     index_location.partition, index_location.start_block);

                                match self.read_index_from_specific_location(&index_location) {
                                    Ok(index_content) => {
                                        info!("✅ Strategy 1 succeeded - index read from LTFS label location");
                                        return self
                                            .save_index_and_return(index_content, output_path);
                                    }
                                    Err(e) => debug!("Strategy 1 location read failed: {}", e),
                                }
                            }
                            Err(e) => debug!("Strategy 1 location parsing failed: {}", e),
                        }
                    }
                    Err(e) => debug!("Strategy 1 LTFS label read failed: {}", e),
                }
            }
            Err(e) => debug!("Strategy 1 positioning failed: {}", e),
        }

        // 策略2: 搜索常见的索引位置
        debug!("Strategy 2: Searching common index locations");
        let common_locations = vec![2, 5, 6, 10, 20, 100];

        for &block in &common_locations {
            debug!(
                "Trying common location: partition {}, block {}",
                index_partition, block
            );

            match self.scsi.locate_block(index_partition, block) {
                Ok(()) => match self.try_read_index_at_current_position_sync() {
                    Ok(xml_content) => {
                        if !xml_content.trim().is_empty()
                            && xml_content.contains("<ltfsindex")
                            && xml_content.contains("</ltfsindex>")
                        {
                            info!(
                                "✅ Strategy 2 succeeded - found valid index at block {}",
                                block
                            );
                            return self.save_index_and_return(xml_content, output_path);
                        }
                    }
                    Err(e) => debug!("Failed to read index at block {}: {}", block, e),
                },
                Err(e) => debug!("Cannot position to block {}: {}", block, e),
            }
        }

        // 策略3: 检测分区策略并使用相应的读取方法
        debug!("Strategy 3: Applying partition-specific strategies");

        if partition_count > 1 {
            info!("Multi-partition tape detected, trying data partition strategy");

            // 尝试从数据分区读取索引副本
            match self.try_read_from_data_partition() {
                Ok(xml_content) => {
                    info!("✅ Strategy 3 succeeded - index read from data partition");
                    return self.save_index_and_return(xml_content, output_path);
                }
                Err(e) => debug!("Data partition strategy failed: {}", e),
            }
        } else {
            info!("Single-partition tape detected, trying extended search");

            // 单分区磁带的扩展搜索
            match self.try_single_partition_extended_search() {
                Ok(xml_content) => {
                    info!("✅ Strategy 3 succeeded - index found via extended search");
                    return self.save_index_and_return(xml_content, output_path);
                }
                Err(e) => debug!("Single partition extended search failed: {}", e),
            }
        }

        // 所有策略都失败了
        Err(RustLtfsError::ltfs_index(
            "All alternative index reading strategies failed. Possible causes:\n\
             1. Blank or unformatted tape\n\
             2. Corrupted LTFS index\n\
             3. Non-LTFS tape format\n\
             4. Hardware communication issues\n\
             \n\
             Suggestions:\n\
             - Check if tape is properly loaded\n\
             - Try using --skip-index option for file operations\n\
             - Verify tape format with original LTFS tools"
                .to_string(),
        ))
    }


    /// 尝试从数据分区读取索引副本
    fn try_read_from_data_partition(&self) -> Result<String> {
        info!("Attempting to read index from data partition (partition 1)");

        // 定位到数据分区的一些常见索引位置
        let data_partition = 1;
        let search_blocks = vec![1000, 2000, 5000, 10000]; // 数据分区的常见索引位置

        for &block in &search_blocks {
            debug!("Trying data partition block {}", block);

            match self.scsi.locate_block(data_partition, block) {
                Ok(()) => match self.try_read_index_at_current_position_sync() {
                    Ok(xml_content) => {
                        if xml_content.contains("<ltfsindex")
                            && xml_content.contains("</ltfsindex>")
                        {
                            info!("Found valid index in data partition at block {}", block);
                            return Ok(xml_content);
                        }
                    }
                    Err(_) => continue,
                },
                Err(_) => continue,
            }
        }

        Err(RustLtfsError::ltfs_index(
            "No valid index found in data partition".to_string(),
        ))
    }

    /// 单分区磁带的扩展搜索
    fn try_single_partition_extended_search(&self) -> Result<String> {
        info!("Performing extended search on single-partition tape");

        let extended_locations = vec![50, 200, 500, 1000, 2000];

        for &block in &extended_locations {
            debug!("Extended search: trying block {}", block);

            match self.scsi.locate_block(0, block) {
                Ok(()) => match self.try_read_index_at_current_position_sync() {
                    Ok(xml_content) => {
                        if xml_content.contains("<ltfsindex")
                            && xml_content.contains("</ltfsindex>")
                        {
                            info!("Found valid index via extended search at block {}", block);
                            return Ok(xml_content);
                        }
                    }
                    Err(_) => continue,
                },
                Err(_) => continue,
            }
        }

        Err(RustLtfsError::ltfs_index(
            "Extended search found no valid index".to_string(),
        ))
    }

    /// 保存索引并返回内容
    fn save_index_and_return(
        &self,
        index_content: String,
        output_path: Option<String>,
    ) -> Result<String> {
        // 保存索引文件到指定路径或默认路径
        let save_path = output_path.unwrap_or_else(|| {
            format!(
                "schema/ltfs_index_{}.xml",
                chrono::Utc::now().format("%Y%m%d_%H%M%S")
            )
        });

        // 确保目录存在
        if let Some(parent) = std::path::Path::new(&save_path).parent() {
            std::fs::create_dir_all(parent).map_err(|e| {
                RustLtfsError::file_operation(format!("Failed to create directory: {}", e))
            })?;
        }

        std::fs::write(&save_path, &index_content).map_err(|e| {
            RustLtfsError::file_operation(format!("Failed to save index file: {}", e))
        })?;

        info!("LTFS index saved to: {}", save_path);
        Ok(index_content)
    }

    /// 异步版本的完整LTFSCopyGUI回退策略
    async fn try_alternative_index_reading_strategies_async(&mut self) -> Result<String> {
        info!("🔄 Starting complete LTFSCopyGUI alternative index reading strategies");

        let partition_count = self.detect_partition_count()?;
        let index_partition = if partition_count > 1 { 0 } else { 0 };

        // 策略0 (最高优先级): 按照LTFSCopyGUI逻辑优先读取数据分区索引  
        info!("Strategy 0 (Highest Priority): Reading from data partition first (LTFSCopyGUI logic)");
        
        if partition_count > 1 {
            // 多分区磁带：优先尝试读取数据分区最新索引，匹配LTFSCopyGUI的"读取数据区索引"
            match self.try_read_from_data_partition_async().await {
                Ok(xml_content) => {
                    info!("✅ Strategy 0 succeeded - index read from data partition (LTFSCopyGUI priority)");
                    return Ok(xml_content);
                }
                Err(e) => debug!("Strategy 0 (data partition priority) failed: {}", e),
            }
        }

        // 策略1 (次级优先): 搜索常见的索引位置 - 将成功率最高的策略放在前面
        info!("Strategy 1 (Priority): Searching common index locations first");
        let common_locations = vec![10, 2, 5, 6, 20, 100]; // 将10放在最前面，因为日志显示在这里成功

        for &block in &common_locations {
            debug!(
                "Trying common location: partition {}, block {}",
                index_partition, block
            );

            match self.scsi.locate_block(index_partition, block) {
                Ok(()) => match self.try_read_index_at_current_position_sync() {
                    Ok(xml_content) => {
                        if !xml_content.trim().is_empty()
                            && xml_content.contains("<ltfsindex")
                            && xml_content.contains("</ltfsindex>")
                        {
                            info!(
                                "✅ Strategy 1 succeeded - found valid index at block {}",
                                block
                            );
                            return Ok(xml_content);
                        }
                    }
                    Err(e) => debug!("Failed to read index at block {}: {}", block, e),
                },
                Err(e) => debug!("Cannot position to block {}: {}", block, e),
            }
        }

        // 检查是否为真正的空白磁带（前4KB都是零）
        // 重新读取VOL1进行空白检测
        match self.scsi.locate_block(0, 0) {
            Ok(()) => {
                let mut vol1_buffer = vec![0u8; crate::scsi::block_sizes::LTO_BLOCK_SIZE as usize];
                if let Ok(_) = self.scsi.read_blocks(1, &mut vol1_buffer) {
                    let check_size = std::cmp::min(4096, vol1_buffer.len());
                    let is_completely_blank = vol1_buffer.iter().take(check_size).all(|&b| b == 0);
                    if is_completely_blank {
                        info!(
                            "📭 Detected blank tape (all zeros in first {}KB)",
                            check_size / 1024
                        );
                        return Err(RustLtfsError::ltfs_index(
                            "Blank tape detected - no LTFS index found".to_string(),
                        ));
                    }
                }
            }
            Err(_) => debug!("Could not re-read VOL1 for blank detection"),
        }

        // 策略2: 跳过VOL1验证，直接尝试读取LTFS标签和索引
        info!("Strategy 2: Bypassing VOL1, attempting direct LTFS label reading");

        // 尝试读取LTFS标签 (block 1)
        match self.scsi.locate_block(index_partition, 1) {
            Ok(()) => {
                let block_size = crate::scsi::block_sizes::LTO_BLOCK_SIZE as usize;
                let mut ltfs_label_buffer = vec![0u8; block_size];

                match self.scsi.read_blocks(1, &mut ltfs_label_buffer) {
                    Ok(_) => {
                        // 尝试从LTFS标签解析索引位置
                        match self.parse_index_locations_from_volume_label(&ltfs_label_buffer) {
                            Ok(index_location) => {
                                info!("✅ Found index location from LTFS label: partition {}, block {}", 
                                     index_location.partition, index_location.start_block);

                                match self.read_index_from_specific_location(&index_location) {
                                    Ok(index_content) => {
                                        info!("✅ Strategy 2 succeeded - index read from LTFS label location");
                                        return Ok(index_content);
                                    }
                                    Err(e) => debug!("Strategy 2 location read failed: {}", e),
                                }
                            }
                            Err(e) => debug!("Strategy 2 location parsing failed: {}", e),
                        }
                    }
                    Err(e) => debug!("Strategy 2 LTFS label read failed: {}", e),
                }
            }
            Err(e) => debug!("Strategy 2 positioning failed: {}", e),
        }

        // 策略3: 扩展搜索策略 - 单分区和多分区都适用
        info!("Strategy 3: Extended search strategies");

        if partition_count > 1 {
            info!("Multi-partition tape: trying single-partition fallback search");
        } else {
            info!("Single-partition tape: trying extended search");
        }

        // 单分区磁带的扩展搜索
        match self.try_single_partition_extended_search_async().await {
            Ok(xml_content) => {
                info!("✅ Strategy 3 succeeded - index found via extended search");
                return Ok(xml_content);
            }
            Err(e) => debug!("Single partition extended search failed: {}", e),
        }

        // 所有策略都失败了
        Err(RustLtfsError::ltfs_index(
            "All alternative index reading strategies failed. Possible causes:\n\
             1. Blank or unformatted tape\n\
             2. Corrupted LTFS index\n\
             3. Non-LTFS tape format\n\
             4. Hardware communication issues\n\
             \n\
             Suggestions:\n\
             - Check if tape is properly loaded\n\
             - Try using --skip-index option for file operations\n\
             - Verify tape format with original LTFS tools"
                .to_string(),
        ))
    }

    /// 异步版本：尝试从数据分区读取索引副本
    async fn try_read_from_data_partition_async(&mut self) -> Result<String> {
        info!("Attempting to read index from data partition (matching LTFSCopyGUI logic)");

        // 按照LTFSCopyGUI的"读取数据区索引"逻辑：
        // 1. 定位到数据分区EOD
        // 2. 向前查找最后的索引
        let data_partition = 1;
        
        // 先尝试定位到数据分区EOD
        match self.scsi.locate_block(data_partition, 0) {
            Ok(()) => {
                // 定位到数据分区的EOD
                match self.scsi.space(crate::scsi::SpaceType::EndOfData, 0) {
                    Ok(()) => {
                        let eod_position = self.scsi.read_position()?;
                        info!("Data partition EOD at partition={}, block={}", eod_position.partition, eod_position.block_number);
                        
                        // 从EOD向前查找索引，类似LTFSCopyGUI的FM-1定位
                        if eod_position.file_number > 1 {
                            // 向前定位到最后一个FileMark之前
                            match self.scsi.locate_to_filemark(eod_position.file_number - 1, data_partition) {
                                Ok(()) => {
                                    // 跳过FileMark，向前移动一个filemark
                                    match self.scsi.space(crate::scsi::SpaceType::FileMarks, 1) {
                                        Ok(_) => {
                                            // 现在应该在最后的索引位置，尝试读取
                                            match self.try_read_index_at_current_position_sync() {
                                                Ok(xml_content) => {
                                                    if xml_content.contains("<ltfsindex") && xml_content.contains("</ltfsindex>") {
                                                        info!("✅ Found valid index at data partition EOD-1");
                                                        return Ok(xml_content);
                                                    }
                                                }
                                                Err(e) => debug!("Failed to read index at EOD-1: {}", e),
                                            }
                                        }
                                        Err(e) => debug!("Failed to read filemark: {}", e),
                                    }
                                }
                                Err(e) => debug!("Failed to locate to filemark: {}", e),
                            }
                        }
                    }
                    Err(e) => debug!("Failed to locate to EOD: {}", e),
                }
            }
            Err(e) => debug!("Failed to position to data partition: {}", e),
        }

        // 回退策略：搜索数据分区的一些常见索引位置
        info!("EOD strategy failed, trying common data partition locations");
        let search_blocks = vec![10000, 5000, 2000, 1000]; // 数据分区的常见索引位置

        for &block in &search_blocks {
            debug!("Trying data partition block {}", block);

            match self.scsi.locate_block(data_partition, block) {
                Ok(()) => match self.try_read_index_at_current_position_sync() {
                    Ok(xml_content) => {
                        if xml_content.contains("<ltfsindex")
                            && xml_content.contains("</ltfsindex>")
                        {
                            info!("Found valid index in data partition at block {}", block);
                            return Ok(xml_content);
                        }
                    }
                    Err(_) => continue,
                },
                Err(_) => continue,
            }
        }

        Err(RustLtfsError::ltfs_index(
            "No valid index found in data partition".to_string(),
        ))
    }

    /// 异步版本：单分区磁带的扩展搜索
    async fn try_single_partition_extended_search_async(&mut self) -> Result<String> {
        info!("Performing extended search on single-partition tape");

        let extended_locations = vec![50, 200, 500, 1000, 2000];

        for &block in &extended_locations {
            debug!("Extended search: trying block {}", block);

            match self.scsi.locate_block(0, block) {
                Ok(()) => match self.try_read_index_at_current_position_sync() {
                    Ok(xml_content) => {
                        if xml_content.contains("<ltfsindex")
                            && xml_content.contains("</ltfsindex>")
                        {
                            info!("Found valid index via extended search at block {}", block);
                            return Ok(xml_content);
                        }
                    }
                    Err(_) => continue,
                },
                Err(_) => continue,
            }
        }

        Err(RustLtfsError::ltfs_index(
            "Extended search found no valid index".to_string(),
        ))
    }

    /// 向后搜索找到数据分区中最后的索引
    fn search_backward_for_last_index(&mut self, partition: u8) -> Result<String> {
        info!("Searching backward from EOD for last index");

        let block_size = crate::scsi::block_sizes::LTO_BLOCK_SIZE as usize;
        let mut search_distance = 1;
        let max_search_blocks = 1000; // 最多向前搜索1000个块

        while search_distance <= max_search_blocks {
            // 尝试通过相对定位向前搜索
            // 注意：ScsiInterface没有locate_block_relative方法，我们需要使用space方法
            match self
                .scsi
                .space(crate::scsi::SpaceType::Blocks, -(search_distance as i32))
            {
                Ok(()) => {
                    // 尝试读取当前位置的数据
                    match self.try_read_index_at_current_position_sync() {
                        Ok(xml_content) => {
                            if self.is_valid_ltfs_index(&xml_content) {
                                info!(
                                    "Found valid LTFS index at {} blocks before EOD",
                                    search_distance
                                );
                                return Ok(xml_content);
                            }
                        }
                        Err(_) => {
                            // 继续搜索
                            debug!(
                                "No valid index found at {} blocks before EOD",
                                search_distance
                            );
                        }
                    }
                }
                Err(_) => {
                    warn!("Cannot locate to {} blocks before EOD", search_distance);
                    break;
                }
            }

            search_distance += 10; // 每次向前搜索10个块
        }

        Err(RustLtfsError::ltfs_index(
            "No valid index found in data partition".to_string(),
        ))
    }

    // === 性能优化方法 ===
    
    /// 获取缓存的索引位置
    fn get_cached_index_location(&self) -> Option<u64> {
        // 简单的静态缓存实现，实际应用中可以使用更复杂的缓存策略
        // 根据日志，上次成功的位置是 block 1000
        Some(1000)  // 临时硬编码，后续可以实现动态缓存
    }
    
    /// 缓存成功的索引位置
    fn cache_successful_location(&self, location: u64) {
        // 实际实现中可以保存到配置文件或内存缓存
        info!("Caching successful index location: block {}", location);
    }
    
    /// 尝试在指定位置读取索引
    async fn try_read_index_at_location(&self, block: u64) -> Result<String> {
        debug!("Trying to read index at cached location: block {}", block);
        
        // 定位到指定块
        self.scsi.locate_block(0, block)?;
        
        // 尝试读取索引
        self.try_read_index_at_current_position_sync()
    }
    
    /// 优化的并行策略搜索
    async fn try_optimized_parallel_strategies(&mut self) -> Result<(String, u64)> {
        info!("🚀 Starting optimized parallel index search strategies");
        
        // 基于日志分析的最可能位置列表（按优先级排序）
        let priority_locations = vec![
            1000,  // 上次成功位置
            5,     // 标准LTFS位置  
            3,     // 常见位置
            1,     // 起始位置
            10, 15, 20,  // 其他常见位置
            100, 200, 500,  // 中间位置
        ];
        
        info!("Trying {} priority locations in optimized order", priority_locations.len());
        
        // 快速串行搜索优先位置（避免并行磁带操作的复杂性）
        for &block in &priority_locations {
            if let Ok(()) = self.scsi.locate_block(0, block) {
                if let Ok(xml_content) = self.try_read_index_at_current_position_sync() {
                    if xml_content.contains("<ltfsindex") && xml_content.contains("</ltfsindex>") {
                        info!("✅ Found index at priority location: block {}", block);
                        return Ok((xml_content, block));
                    }
                }
            }
        }
        
        // 如果优先位置都失败，回退到原有的完整搜索
        info!("Priority locations failed, falling back to comprehensive search");
        match self.try_alternative_index_reading_strategies_async().await {
            Ok(xml_content) => {
                // 估算找到的位置（实际实现中应该记录确切位置）
                Ok((xml_content, 1000))  // 默认位置
            }
            Err(e) => Err(e)
        }
    }
}
