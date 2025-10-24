use crate::error::{Result, RustLtfsError};
use crate::ltfs_index::LtfsIndex;
use crate::scsi::MediaType;
use std::path::Path;
use tracing::{debug, info, warn};

// 导入partition_manager中的类型
use super::partition_manager::{IndexLocation, PartitionStrategy};
use super::TapeFormatAnalysis; // 导入增强版VOL1验证需要的枚举

// LtfsPartitionLabel 在 format_operations.rs 中定义
// 通过模块重新导出使用

/// TapeOperations读取操作实现
impl super::TapeOperations {
    /// 验证并处理索引 - 增强版本：添加详细调试信息
    pub async fn validate_and_process_index(&mut self, xml_content: &str) -> Result<bool> {
        info!("🔍 Validating index content: {} bytes", xml_content.len());

        // 🔍 添加详细的验证日志
        let preview = xml_content.chars().take(300).collect::<String>();
        info!("🔍 Index content preview: {:?}", preview);

        // 基本验证XML格式
        if !xml_content.contains("<ltfsindex") || !xml_content.contains("</ltfsindex>") {
            warn!("❌ Basic XML validation failed - missing LTFS index tags");
            info!(
                "🔍 Missing tags check: contains('<ltfsindex'): {}, contains('</ltfsindex>'): {}",
                xml_content.contains("<ltfsindex"),
                xml_content.contains("</ltfsindex>")
            );
            debug!(
                "Content preview: {}",
                &xml_content[..std::cmp::min(200, xml_content.len())]
            );
            return Ok(false);
        }

        info!("✅ Basic XML validation passed - LTFS index tags found");

        // 解析并设置索引
        match crate::ltfs_index::LtfsIndex::from_xml(xml_content) {
            Ok(index) => {
                info!("✅ XML parsing successful - setting index");
                info!("   Volume UUID: {}", index.volumeuuid);
                info!("   Generation: {}", index.generationnumber);
                info!(
                    "   Files count: {}",
                    self.count_files_in_directory(&index.root_directory)
                );
                self.index = Some(index);
                Ok(true)
            }
            Err(e) => {
                warn!("❌ XML parsing failed: {}", e);
                info!("🔍 Failed XML content length: {} bytes", xml_content.len());
                debug!(
                    "Failed XML content preview: {}",
                    &xml_content[..std::cmp::min(500, xml_content.len())]
                );
                Ok(false)
            }
        }
    }

    /// 计算目录中的文件数量
    fn count_files_in_directory(&self, dir: &crate::ltfs_index::Directory) -> usize {
        let mut count = dir.contents.files.len();
        for subdir in &dir.contents.directories {
            count += self.count_files_in_directory(subdir);
        }
        count
    }

    /// 检测分区策略 - 修复版本：直接使用已打开的SCSI设备
    pub async fn detect_partition_strategy(&self) -> Result<PartitionStrategy> {
        info!("🔧 Detecting partition strategy using opened SCSI device (fixing device handle inconsistency)");

        // 直接使用已初始化的ExtraPartitionCount，避免创建新的PartitionManager实例
        let extra_partition_count = self.get_extra_partition_count();

        info!(
            "Determining partition strategy based on ExtraPartitionCount = {}",
            extra_partition_count
        );

        match extra_partition_count {
            0 => {
                info!("Single-partition strategy (ExtraPartitionCount = 0)");
                Ok(PartitionStrategy::SinglePartitionFallback)
            }
            1 => {
                info!("Dual-partition strategy (ExtraPartitionCount = 1)");
                Ok(PartitionStrategy::StandardMultiPartition)
            }
            _ => {
                warn!(
                    "Unexpected ExtraPartitionCount value: {}, using dual-partition strategy",
                    extra_partition_count
                );
                Ok(PartitionStrategy::StandardMultiPartition)
            }
        }
    }

    /// Read LTFS index from tape (LTFSCopyGUI兼容方法)
    pub async fn read_index_from_tape(&mut self) -> Result<()> {
        info!("Starting LTFS index reading process with LTFSCopyGUI compatible method...");

        if self.offline_mode {
            info!("Offline mode: using dummy index for simulation");
            return Ok(());
        }

        info!("=== LTFSCopyGUI Compatible Index Reading Process ===");

        // Step 1 (最高优先级): LTFSCopyGUI兼容方法
        info!("Step 1 (Highest Priority): LTFSCopyGUI compatible method");

        // 检测分区策略并决定读取顺序
        let extra_partition_count = self.get_extra_partition_count();

        if extra_partition_count > 0 {
            // 双分区磁带：使用LTFSCopyGUI方法从数据分区读取索引
            info!("Dual-partition detected, using LTFSCopyGUI method from data partition");

            match self.search_index_copies_in_data_partition() {
                Ok(xml_content) => {
                    info!(
                        "🔍 LTFSCopyGUI method returned {} bytes of content",
                        xml_content.len()
                    );
                    match self.validate_and_process_index(&xml_content).await? {
                        true => {
                            info!("✅ Step 1 succeeded - LTFS index read using LTFSCopyGUI method (dual-partition)");
                            return Ok(());
                        }
                        false => {
                            warn!("⚠️ LTFSCopyGUI method read data but XML validation failed");
                            info!("🔍 This suggests the data at FileMark 1 position is not valid LTFS XML");
                            // 不要立即fallback到单分区逻辑，先尝试dual-partition的backup策略
                        }
                    }
                }
                Err(e) => {
                    warn!("❌ LTFSCopyGUI method failed completely: {}", e);
                    debug!("LTFSCopyGUI method failed: {}", e);
                }
            }

            // 🔧 双分区backup策略：尝试从索引分区(partition 0) EOD读取
            info!("🔧 Trying dual-partition backup strategy: index partition EOD");
            match self.try_read_latest_index_from_eod(0).await {
                Ok(xml_content) => {
                    if self.validate_and_process_index(&xml_content).await? {
                        info!("✅ Step 1 succeeded - index read from index partition EOD (dual-partition fallback)");
                        return Ok(());
                    }
                }
                Err(e) => {
                    debug!("Index partition EOD strategy failed: {}", e);
                }
            }
        } else {
            // 单分区磁带：从partition=0读取索引
            info!("Single-partition detected, reading from partition 0");

            match self.try_read_latest_index_from_eod(0).await {
                Ok(xml_content) => {
                    if self.validate_and_process_index(&xml_content).await? {
                        info!("✅ Step 1 succeeded - index read from partition 0 EOD (single-partition logic)");
                        return Ok(());
                    }
                }
                Err(e) => {
                    debug!("Single-partition EOD strategy failed: {}", e);
                }
            }
        }

        // Step 2: 标准流程作为备用策略
        info!("Step 2: Standard LTFS reading process as fallback");

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
                    // 尝试数据分区EOD策略
                    match self.try_read_latest_index_from_data_partition_eod().await {
                        Ok(xml_content) => {
                            if self.validate_and_process_index(&xml_content).await? {
                                info!("✅ Standard reading (data partition EOD) succeeded");
                                return Ok(());
                            }
                        }
                        Err(e) => debug!("Data partition EOD reading failed: {}", e),
                    }

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

                // 最后尝试：有限的固定位置搜索（仅作为最后手段）
                let standard_locations = vec![6, 5, 2, 0]; // block 6仍然保留以兼容特殊情况

                for &block in &standard_locations {
                    info!("Trying final fallback at p0 block {}", block);
                    match self.scsi.locate_block(0, block) {
                        Ok(()) => match self.read_index_xml_from_tape_with_file_mark() {
                            Ok(xml_content) => {
                                if self.validate_and_process_index(&xml_content).await? {
                                    info!("✅ Successfully read index from p0 block {} (final fallback)", block);
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

                info!(
                    "🔄 All standard locations failed, falling back to single-partition strategy"
                );
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

    /// 同步版本：在当前位置尝试读取索引（使用动态block size）
    fn try_read_index_at_current_position_with_filemarks(&self) -> Result<String> {
        // 获取动态blocksize (对应LTFSCopyGUI的plabel.blocksize)
        let block_size = self
            .partition_label
            .as_ref()
            .map(|plabel| plabel.blocksize as usize)
            .unwrap_or(crate::scsi::block_sizes::LTO_BLOCK_SIZE as usize);

        debug!(
            "Using dynamic blocksize: {} bytes for index reading",
            block_size
        );

        // 直接使用当前TapeOperations的read_to_file_mark方法
        self.read_to_file_mark_with_temp_file(block_size)
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

    /// Enhanced VOL1 label validation with comprehensive format detection
    /// 增强版 VOL1 标签验证：支持多种磁带格式检测和详细诊断
    fn parse_vol1_label(&self, buffer: &[u8]) -> Result<bool> {
        info!(
            "🔍 Enhanced VOL1 validation (LTFSCopyGUI compatible with extended format support)..."
        );

        // Enhanced Condition 1: Dynamic buffer length check with detailed analysis
        if buffer.len() < 80 {
            warn!(
                "❌ VOL1 validation failed: buffer too short ({} bytes), need at least 80 bytes",
                buffer.len()
            );

            // Provide diagnostic information for short buffers
            if buffer.len() > 0 {
                let preview_len = std::cmp::min(buffer.len(), 40);
                info!(
                    "🔧 Buffer content preview ({} bytes): hex={:02X?}",
                    preview_len,
                    &buffer[0..preview_len]
                );
                info!(
                    "🔧 Buffer content preview ({} bytes): text={:?}",
                    preview_len,
                    String::from_utf8_lossy(&buffer[0..preview_len])
                );
            }

            return Ok(false);
        }

        // Extract the standard 80-byte VOL1 label area
        let vol1_label = &buffer[0..80];

        // Enhanced Condition 2: Multi-format tape detection with detailed analysis
        let vol1_prefix = b"VOL1";
        if !vol1_label.starts_with(vol1_prefix) {
            info!("⚠️ VOL1 prefix not found, performing enhanced format detection...");

            // Comprehensive tape format analysis
            let tape_analysis = self.analyze_tape_format_enhanced(vol1_label);
            match tape_analysis {
                TapeFormatAnalysis::BlankTape => {
                    info!("📭 Confirmed: Blank tape detected (all zeros in VOL1 area)");
                    return Ok(false);
                }
                TapeFormatAnalysis::LegacyTape(format_name) => {
                    info!("🏛️ Legacy tape format detected: {}", format_name);
                    info!("💡 Tip: This tape may contain data but is not LTFS formatted");
                    return Ok(false);
                }
                TapeFormatAnalysis::CorruptedLabel => {
                    warn!("💥 Corrupted or damaged VOL1 label detected");
                    info!("🔧 Suggestion: Try cleaning the tape drive or using a different tape");
                    return Ok(false);
                }
                TapeFormatAnalysis::UnknownFormat => {
                    info!("❓ Unknown tape format detected");
                    self.log_detailed_tape_analysis(vol1_label);
                    return Ok(false);
                }
                TapeFormatAnalysis::PossibleLTFS => {
                    info!("🤔 Possible LTFS tape with non-standard VOL1, proceeding with extended validation...");
                    // Continue to LTFS identifier check
                }
            }
        } else {
            info!("✅ VOL1 prefix validation passed");
        }

        // Enhanced Condition 3: Advanced LTFS identifier validation with fallback strategies
        if vol1_label.len() < 28 {
            warn!(
                "❌ VOL1 label too short for LTFS identifier check (need 28+ bytes, got {})",
                vol1_label.len()
            );
            return Ok(false);
        }

        let ltfs_bytes = &vol1_label[24..28];
        let expected_ltfs = b"LTFS";

        if ltfs_bytes == expected_ltfs {
            info!("✅ Standard LTFS identifier found at position 24-27");
            return self.validate_extended_ltfs_properties(vol1_label);
        }

        // Enhanced fallback strategies for LTFS detection
        info!("🔄 Standard LTFS identifier not found, trying enhanced detection strategies...");

        // Strategy 1: Search for LTFS identifier in alternative positions
        if let Some(ltfs_position) = self.search_ltfs_identifier_in_vol1(vol1_label) {
            info!(
                "🎯 Found LTFS identifier at alternative position: {}",
                ltfs_position
            );
            return self.validate_extended_ltfs_properties(vol1_label);
        }

        // Strategy 2: Check for LTFS version indicators
        if self.detect_ltfs_version_indicators(vol1_label) {
            info!("🔍 LTFS version indicators detected, likely LTFS tape with non-standard label");
            return self.validate_extended_ltfs_properties(vol1_label);
        }

        // Strategy 3: Pattern-based LTFS detection
        if self.detect_ltfs_patterns(vol1_label) {
            info!("📊 LTFS patterns detected in VOL1 label");
            return Ok(true); // Allow with pattern-based detection
        }

        // Final diagnostic report
        warn!(
            "❌ LTFS identifier validation failed: expected 'LTFS' at position 24-27, found: {:?}",
            String::from_utf8_lossy(ltfs_bytes)
        );
        info!("🔧 Enhanced diagnostic: checking for partial LTFS compatibility...");

        // Check if this might be a partially formatted or corrupted LTFS tape
        if self.detect_partial_ltfs_formatting(vol1_label) {
            warn!("⚠️ Partial LTFS formatting detected - tape may be recoverable");
            info!("💡 Suggestion: Try reformatting with mkltfs or use recovery tools");
        }

        self.log_detailed_tape_analysis(vol1_label);
        Ok(false)
    }

    /// Enhanced tape format analysis with detailed classification
    fn analyze_tape_format_enhanced(&self, vol1_label: &[u8]) -> TapeFormatAnalysis {
        // Check for blank tape (all zeros)
        let non_zero_count = vol1_label.iter().filter(|&&b| b != 0).count();
        if non_zero_count == 0 {
            return TapeFormatAnalysis::BlankTape;
        }

        // Check for very sparse data (likely blank or minimally written)
        let sparse_threshold = 5; // Less than 5 non-zero bytes in 80 bytes
        if non_zero_count < sparse_threshold {
            debug!(
                "Sparse data detected: only {} non-zero bytes",
                non_zero_count
            );
            return TapeFormatAnalysis::BlankTape;
        }

        // Check for common legacy tape formats
        if vol1_label.starts_with(b"HDR1") || vol1_label.starts_with(b"HDR2") {
            return TapeFormatAnalysis::LegacyTape("ANSI Standard Label (HDR)".to_string());
        }

        if vol1_label.starts_with(b"UHL1") || vol1_label.starts_with(b"UHL2") {
            return TapeFormatAnalysis::LegacyTape("User Header Label (UHL)".to_string());
        }

        if vol1_label.starts_with(b"EOF1") || vol1_label.starts_with(b"EOF2") {
            return TapeFormatAnalysis::LegacyTape("End of File Label (EOF)".to_string());
        }

        if vol1_label.starts_with(b"EOV1") || vol1_label.starts_with(b"EOV2") {
            return TapeFormatAnalysis::LegacyTape("End of Volume Label (EOV)".to_string());
        }

        // Check for IBM tape formats
        if vol1_label[0..4] == [0xE5, 0xD6, 0xD3, 0xF1] {
            // EBCDIC "VOL1"
            return TapeFormatAnalysis::LegacyTape("IBM EBCDIC VOL1 Label".to_string());
        }

        // Check for potential LTFS with damaged VOL1
        if self.contains_ltfs_patterns(vol1_label) {
            return TapeFormatAnalysis::PossibleLTFS;
        }

        // Check for corrupted label (has data but unrecognizable pattern)
        let ascii_count = vol1_label.iter().filter(|&&b| b >= 32 && b <= 126).count();
        let ascii_ratio = ascii_count as f64 / vol1_label.len() as f64;

        if ascii_ratio < 0.3 {
            return TapeFormatAnalysis::CorruptedLabel;
        }

        TapeFormatAnalysis::UnknownFormat
    }

    /// Search for LTFS identifier in alternative positions within VOL1 label
    fn search_ltfs_identifier_in_vol1(&self, vol1_label: &[u8]) -> Option<usize> {
        let ltfs_signature = b"LTFS";

        // Search in common alternative positions (some LTFS implementations may vary)
        let search_positions = [20, 28, 32, 36, 40, 44, 48]; // Alternative positions to check

        for &pos in &search_positions {
            if pos + 4 <= vol1_label.len() {
                if &vol1_label[pos..pos + 4] == ltfs_signature {
                    return Some(pos);
                }
            }
        }

        // Broader search within the entire VOL1 label
        for i in 0..=(vol1_label.len().saturating_sub(4)) {
            if &vol1_label[i..i + 4] == ltfs_signature {
                return Some(i);
            }
        }

        None
    }

    /// Detect LTFS version indicators in VOL1 label
    fn detect_ltfs_version_indicators(&self, vol1_label: &[u8]) -> bool {
        let vol1_text = String::from_utf8_lossy(vol1_label).to_lowercase();

        // Look for version patterns commonly found in LTFS labels
        let version_patterns = [
            "ltfs",
            "2.4",
            "2.2",
            "2.0",
            "1.0",
            "version",
            "ltfscopygui",
            "rustltfs",
        ];

        let mut pattern_count = 0;
        for pattern in &version_patterns {
            if vol1_text.contains(pattern) {
                pattern_count += 1;
                debug!("Found LTFS version indicator: '{}'", pattern);
            }
        }

        pattern_count >= 2 // Require at least 2 patterns for confidence
    }

    /// Detect LTFS-specific patterns in VOL1 label
    fn detect_ltfs_patterns(&self, vol1_label: &[u8]) -> bool {
        // Check for characteristic byte patterns found in LTFS labels
        let patterns_found = [
            self.contains_ltfs_patterns(vol1_label),
            self.has_ltfs_block_size_indicators(vol1_label),
            self.has_ltfs_partition_indicators(vol1_label),
        ];

        patterns_found.iter().filter(|&&found| found).count() >= 2
    }

    /// Check if VOL1 contains LTFS-specific patterns
    fn contains_ltfs_patterns(&self, vol1_label: &[u8]) -> bool {
        let vol1_text = String::from_utf8_lossy(vol1_label);

        // Look for case-insensitive LTFS patterns
        let ltfs_indicators = ["ltfs", "linear", "tape", "file", "system"];
        let found_indicators = ltfs_indicators
            .iter()
            .filter(|&pattern| vol1_text.to_lowercase().contains(pattern))
            .count();

        found_indicators >= 2
    }

    /// Check for LTFS block size indicators
    fn has_ltfs_block_size_indicators(&self, vol1_label: &[u8]) -> bool {
        // Look for typical LTFS block sizes in the label
        let common_block_sizes = [524288u32, 65536u32, 32768u32]; // Common LTFS block sizes

        for &block_size in &common_block_sizes {
            let size_bytes = block_size.to_le_bytes();
            if vol1_label.windows(4).any(|window| window == size_bytes) {
                debug!("Found potential block size indicator: {}", block_size);
                return true;
            }

            let size_bytes_be = block_size.to_be_bytes();
            if vol1_label.windows(4).any(|window| window == size_bytes_be) {
                debug!("Found potential block size indicator (BE): {}", block_size);
                return true;
            }
        }

        false
    }

    /// Check for LTFS partition indicators
    fn has_ltfs_partition_indicators(&self, vol1_label: &[u8]) -> bool {
        // Look for partition-related information typical in LTFS
        let vol1_text = String::from_utf8_lossy(vol1_label).to_lowercase();
        let partition_patterns = ["partition", "part", "index", "data"];

        partition_patterns
            .iter()
            .any(|&pattern| vol1_text.contains(pattern))
    }

    /// Detect partial LTFS formatting that might be recoverable
    fn detect_partial_ltfs_formatting(&self, vol1_label: &[u8]) -> bool {
        // Look for signs of interrupted or partial LTFS formatting
        let vol1_text = String::from_utf8_lossy(vol1_label);

        // Check for partial signatures or formatting indicators
        let partial_indicators = [
            vol1_text.contains("LTF"), // Partial "LTFS"
            vol1_text.contains("TFS"), // Partial "LTFS"
            vol1_text.contains("vol"), // Partial volume info
            vol1_label.windows(2).any(|window| window == [0x4C, 0x54]), // Partial "LT" bytes
        ];

        partial_indicators.iter().any(|&found| found)
    }

    /// Validate extended LTFS properties in VOL1 label
    fn validate_extended_ltfs_properties(&self, vol1_label: &[u8]) -> Result<bool> {
        info!("🔍 Validating extended LTFS properties in VOL1 label...");

        // Basic validation passed, now check additional LTFS properties
        let mut validation_score = 0u32;
        let max_score = 10u32;

        // Check 1: Volume serial number area (bytes 4-10)
        if vol1_label.len() >= 11 {
            let volume_serial = &vol1_label[4..11];
            if volume_serial.iter().any(|&b| b != 0 && b != 0x20) {
                // Not all zeros or spaces
                validation_score += 2;
                debug!("✓ Volume serial number present");
            }
        }

        // Check 2: Owner identifier area (bytes 37-50)
        if vol1_label.len() >= 51 {
            let owner_id = &vol1_label[37..51];
            if owner_id.iter().any(|&b| b != 0 && b != 0x20) {
                validation_score += 1;
                debug!("✓ Owner identifier present");
            }
        }

        // Check 3: Label standard version (typically at byte 79)
        if vol1_label.len() >= 80 {
            let label_std_version = vol1_label[79];
            if label_std_version >= 0x30 && label_std_version <= 0x39 {
                // ASCII digit
                validation_score += 2;
                debug!(
                    "✓ Valid label standard version: {}",
                    label_std_version as char
                );
            }
        }

        // Check 4: Overall ASCII compliance
        let ascii_count = vol1_label
            .iter()
            .filter(|&&b| (b >= 0x20 && b <= 0x7E) || b == 0x00)
            .count();
        let ascii_ratio = ascii_count as f64 / vol1_label.len() as f64;
        if ascii_ratio >= 0.8 {
            validation_score += 2;
            debug!("✓ Good ASCII compliance: {:.1}%", ascii_ratio * 100.0);
        }

        // Check 5: Reasonable data distribution (not too repetitive)
        let unique_bytes = vol1_label
            .iter()
            .collect::<std::collections::HashSet<_>>()
            .len();
        if unique_bytes >= 10 {
            validation_score += 2;
            debug!("✓ Good data diversity: {} unique bytes", unique_bytes);
        }

        // Check 6: LTFS-specific structural validation
        if self.validate_ltfs_vol1_structure(vol1_label) {
            validation_score += 1;
            debug!("✓ LTFS VOL1 structure validation passed");
        }

        let validation_percentage = (validation_score as f64 / max_score as f64) * 100.0;
        info!(
            "📊 Extended LTFS validation score: {}/{} ({:.1}%)",
            validation_score, max_score, validation_percentage
        );

        if validation_score >= 6 {
            info!("✅ Extended LTFS properties validation passed with high confidence");
            Ok(true)
        } else if validation_score >= 4 {
            info!("⚠️ Extended LTFS properties validation passed with medium confidence");
            Ok(true) // Allow with warnings
        } else {
            warn!("❌ Extended LTFS properties validation failed - score too low");
            Ok(false)
        }
    }

    /// Validate LTFS-specific VOL1 label structure
    fn validate_ltfs_vol1_structure(&self, vol1_label: &[u8]) -> bool {
        // LTFS VOL1 should have specific structural characteristics

        // Check for proper field separators and lengths
        let mut structure_score = 0u32;

        // Field 1: Volume identifier (4 bytes "VOL1")
        if vol1_label.starts_with(b"VOL1") {
            structure_score += 1;
        }

        // Field 2: Volume serial (6 bytes, typically alphanumeric)
        if vol1_label.len() >= 10 {
            let vol_serial = &vol1_label[4..10];
            if vol_serial
                .iter()
                .all(|&b| b.is_ascii_alphanumeric() || b == 0x20)
            {
                structure_score += 1;
            }
        }

        // Field 3: Security byte (should be space or ASCII)
        if vol1_label.len() >= 11 && (vol1_label[10] == 0x20 || vol1_label[10].is_ascii()) {
            structure_score += 1;
        }

        structure_score >= 2
    }

    /// Log detailed tape analysis for diagnostic purposes
    fn log_detailed_tape_analysis(&self, vol1_label: &[u8]) {
        info!("🔧 === Detailed Tape Analysis Report ===");

        // Basic statistics
        let total_bytes = vol1_label.len();
        let non_zero_bytes = vol1_label.iter().filter(|&&b| b != 0).count();
        let ascii_bytes = vol1_label
            .iter()
            .filter(|&&b| b >= 0x20 && b <= 0x7E)
            .count();
        let control_bytes = vol1_label.iter().filter(|&&b| b < 0x20).count();

        info!(
            "📊 Statistics: {} total bytes, {} non-zero, {} ASCII printable, {} control",
            total_bytes, non_zero_bytes, ascii_bytes, control_bytes
        );

        // Hex dump of first 40 bytes
        let preview_len = std::cmp::min(40, vol1_label.len());
        info!(
            "🔍 Hex dump (first {} bytes): {:02X?}",
            preview_len,
            &vol1_label[0..preview_len]
        );

        // ASCII representation
        let ascii_repr = vol1_label[0..preview_len]
            .iter()
            .map(|&b| {
                if b >= 0x20 && b <= 0x7E {
                    b as char
                } else {
                    '.'
                }
            })
            .collect::<String>();
        info!("🔤 ASCII representation: '{}'", ascii_repr);

        // Pattern analysis
        let unique_bytes = vol1_label
            .iter()
            .collect::<std::collections::HashSet<_>>()
            .len();
        info!("🎨 Data diversity: {} unique byte values", unique_bytes);

        // Look for any recognizable patterns
        if let Some(pattern) = self.identify_tape_patterns(vol1_label) {
            info!("🔍 Identified pattern: {}", pattern);
        }
    }

    /// Identify recognizable patterns in tape data
    fn identify_tape_patterns(&self, data: &[u8]) -> Option<String> {
        let text = String::from_utf8_lossy(data).to_lowercase();

        // Check for various tape-related patterns
        if text.contains("backup") || text.contains("archive") {
            return Some("Backup/Archive software signature".to_string());
        }

        if text.contains("tar") || text.contains("cpio") {
            return Some("Unix archive format signature".to_string());
        }

        if text.contains("ibm") || text.contains("tivoli") {
            return Some("IBM software signature".to_string());
        }

        if text.contains("hp") || text.contains("veritas") {
            return Some("Enterprise backup software signature".to_string());
        }

        // Check for filesystem signatures
        if data.windows(2).any(|window| window == [0x53, 0xEF]) {
            // ext2/3/4 magic
            return Some("Linux filesystem signature".to_string());
        }

        None
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
                Ok(()) => match self.try_read_index_at_current_position_with_filemarks() {
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
                Ok(()) => match self.try_read_index_at_current_position_with_filemarks() {
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
                Ok(()) => match self.try_read_index_at_current_position_with_filemarks() {
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

        // 策略0 (最高优先级): 按照LTFSCopyGUI逻辑优先读取数据分区EOD最新索引
        info!("Strategy 0 (Highest Priority): Reading latest index from data partition EOD (LTFSCopyGUI logic)");

        if partition_count > 1 {
            // 多分区磁带：按照LTFSCopyGUI的"读取数据区索引"逻辑，优先从数据分区EOD读取最新索引
            match self.try_read_latest_index_from_data_partition_eod().await {
                Ok(xml_content) => {
                    info!("✅ Strategy 0 succeeded - latest index read from data partition EOD (LTFSCopyGUI priority)");
                    return Ok(xml_content);
                }
                Err(e) => {
                    warn!("Strategy 0 (data partition EOD priority) failed: {}", e);
                    info!("EOD strategy failure details: {}", e);
                }
            }
        } else {
            // 单分区磁带：按照LTFSCopyGUI逻辑，从主分区EOD读取最新索引
            match self.try_read_latest_index_from_eod(0).await {
                Ok(xml_content) => {
                    info!("✅ Strategy 0 succeeded - latest index read from partition 0 EOD (single partition)");
                    return Ok(xml_content);
                }
                Err(e) => {
                    warn!("Strategy 0 (partition 0 EOD) failed: {}", e);
                    info!("Single partition EOD strategy failure details: {}", e);
                }
            }
        }

        // 策略1 (最高优先级): 标准LTFS读取策略 - 总是成功的策略
        info!("Strategy 1 (Highest Priority): Standard LTFS reading strategy");
        match self.try_standard_ltfs_reading().await {
            Ok(xml_content) => {
                info!("✅ Strategy 1 succeeded - standard LTFS reading completed");
                return Ok(xml_content);
            }
            Err(e) => debug!("Strategy 1 (standard LTFS reading) failed: {}", e),
        }

        // 策略2 (高优先级): 使用成功的扩展搜索策略 - 作为备用策略
        info!("Strategy 2 (High Priority): Using proven extended search strategy as backup");
        match self.try_single_partition_extended_search_async().await {
            Ok(xml_content) => {
                info!("✅ Strategy 2 succeeded - index found via proven extended search");
                return Ok(xml_content);
            }
            Err(e) => debug!("Strategy 2 (extended search) failed: {}", e),
        }

        // 策略3 (次级优先): 搜索常见的索引位置 - 包含LTFSCopyGUI成功位置
        info!("Strategy 3 (Secondary): Searching common index locations including LTFSCopyGUI-compatible positions");
        let common_locations = vec![6, 10, 2, 5, 20, 100]; // 将block 6放在最前面，这是LTFSCopyGUI找到索引的位置

        for &block in &common_locations {
            debug!(
                "Trying common location: partition {}, block {}",
                index_partition, block
            );

            match self.scsi.locate_block(index_partition, block) {
                Ok(()) => match self.try_read_index_at_current_position_with_filemarks() {
                    Ok(xml_content) => {
                        if !xml_content.trim().is_empty()
                            && xml_content.contains("<ltfsindex")
                            && xml_content.contains("</ltfsindex>")
                        {
                            info!(
                                "✅ Strategy 3 succeeded - found valid index at block {}",
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

        // 策略4: 跳过VOL1验证，直接尝试读取LTFS标签和索引
        info!("Strategy 4: Bypassing VOL1, attempting direct LTFS label reading");

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
                                        info!("✅ Strategy 4 succeeded - index read from LTFS label location");
                                        return Ok(index_content);
                                    }
                                    Err(e) => debug!("Strategy 4 location read failed: {}", e),
                                }
                            }
                            Err(e) => debug!("Strategy 4 location parsing failed: {}", e),
                        }
                    }
                    Err(e) => debug!("Strategy 4 LTFS label read failed: {}", e),
                }
            }
            Err(e) => debug!("Strategy 4 positioning failed: {}", e),
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
                // 使用LOCATE命令而非SPACE命令进行EOD定位（LTFSCopyGUI兼容）
                match self.scsi.locate_to_eod(data_partition) {
                    Ok(()) => {
                        let eod_position = self.scsi.read_position()?;
                        info!(
                            "Data partition EOD at partition={}, block={}",
                            eod_position.partition, eod_position.block_number
                        );

                        // 从EOD向前查找索引，类似LTFSCopyGUI的FM-1定位
                        if eod_position.file_number > 1 {
                            // 向前定位到最后一个FileMark之前
                            match self
                                .scsi
                                .locate_to_filemark(eod_position.file_number - 1, data_partition)
                            {
                                Ok(()) => {
                                    // 跳过FileMark，向前移动一个filemark
                                    match self.scsi.space(crate::scsi::SpaceType::FileMarks, 1) {
                                        Ok(_) => {
                                            // 现在应该在最后的索引位置，尝试读取
                                            match self
                                                .try_read_index_at_current_position_with_filemarks()
                                            {
                                                Ok(xml_content) => {
                                                    if xml_content.contains("<ltfsindex")
                                                        && xml_content.contains("</ltfsindex>")
                                                    {
                                                        info!("✅ Found valid index at data partition EOD-1");
                                                        return Ok(xml_content);
                                                    }
                                                }
                                                Err(e) => {
                                                    debug!("Failed to read index at EOD-1: {}", e)
                                                }
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
        // 修复：添加小块号，覆盖新写入的索引位置
        let search_blocks = vec![50, 100, 500, 1000, 2000, 5000, 10000]; // 从小到大搜索

        for &block in &search_blocks {
            debug!("Trying data partition block {}", block);

            match self.scsi.locate_block(data_partition, block) {
                Ok(()) => match self.try_read_index_at_current_position_with_filemarks() {
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
        info!("Performing extended search on single-partition tape with LTFSCopyGUI-compatible locations");

        // 包含小block号和大block号，确保覆盖LTFSCopyGUI找到的位置
        let extended_locations = vec![6, 2, 5, 10, 20, 50, 100, 200, 500, 1000, 2000];

        for &block in &extended_locations {
            debug!("Extended search: trying block {}", block);

            match self.scsi.locate_block(0, block) {
                Ok(()) => match self.try_read_index_at_current_position_with_filemarks() {
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
                    match self.try_read_index_at_current_position_with_filemarks() {
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

    /// 获取缓存的索引位置 - 智能持久化缓存机制
    fn get_cached_index_location(&self) -> Option<u64> {
        // 构建缓存文件路径
        let cache_path = self.get_cache_file_path();

        // 尝试从缓存文件读取成功位置
        match std::fs::read_to_string(&cache_path) {
            Ok(content) => {
                // 解析缓存内容：格式为 "device_path:block_number:timestamp"
                for line in content.lines() {
                    let parts: Vec<&str> = line.split(':').collect();
                    if parts.len() >= 3 {
                        // 检查设备路径是否匹配
                        if parts[0] == self.device_path {
                            if let Ok(cached_location) = parts[1].parse::<u64>() {
                                // 检查缓存时间戳是否在有效期内 (24小时)
                                if let Ok(timestamp) = parts[2].parse::<i64>() {
                                    let current_time = chrono::Utc::now().timestamp();
                                    let cache_validity_hours = 24;

                                    if current_time - timestamp < cache_validity_hours * 3600 {
                                        debug!("✅ Found valid cached index location: block {} for device {}",
                                               cached_location, self.device_path);
                                        return Some(cached_location);
                                    } else {
                                        debug!(
                                            "⏰ Cached location expired for device {}",
                                            self.device_path
                                        );
                                    }
                                }
                            }
                            break;
                        }
                    }
                }
                debug!(
                    "❌ No valid cached location found for device {}",
                    self.device_path
                );
                None
            }
            Err(_) => {
                debug!("📝 No cache file found, will create on first success");
                None
            }
        }
    }

    /// 缓存成功的索引位置 - 持久化实现
    fn cache_successful_location(&self, location: u64) {
        info!(
            "📋 Caching successful index location: block {} for device {}",
            location, self.device_path
        );

        let cache_path = self.get_cache_file_path();
        let timestamp = chrono::Utc::now().timestamp();
        let cache_entry = format!("{}:{}:{}\n", self.device_path, location, timestamp);

        // 创建缓存目录
        if let Some(parent) = cache_path.parent() {
            if let Err(e) = std::fs::create_dir_all(parent) {
                debug!("Failed to create cache directory: {}", e);
                return;
            }
        }

        // 读取现有缓存文件（如果存在）
        let existing_content = std::fs::read_to_string(&cache_path).unwrap_or_default();
        let mut new_content = String::new();

        // 过滤掉当前设备的旧缓存条目
        for line in existing_content.lines() {
            let parts: Vec<&str> = line.split(':').collect();
            if parts.len() >= 1 && parts[0] != self.device_path {
                new_content.push_str(line);
                new_content.push('\n');
            }
        }

        // 添加新的缓存条目
        new_content.push_str(&cache_entry);

        // 写入缓存文件
        match std::fs::write(&cache_path, new_content) {
            Ok(()) => {
                debug!("✅ Successfully cached index location to {:?}", cache_path);
            }
            Err(e) => {
                debug!("❌ Failed to write cache file: {}", e);
            }
        }
    }

    /// 获取缓存文件路径
    fn get_cache_file_path(&self) -> std::path::PathBuf {
        // 使用系统临时目录或用户配置目录
        let cache_dir = if let Some(config_dir) = dirs::config_dir() {
            config_dir.join("rustltfs")
        } else {
            std::env::temp_dir().join("rustltfs")
        };

        cache_dir.join("index_location_cache.txt")
    }

    /// 优化的并行策略搜索 - 基于测试结果优化
    async fn try_optimized_parallel_strategies(&mut self) -> Result<(String, u64)> {
        info!("🚀 Starting optimized index search with intelligent strategies");

        // 基于实际测试结果和LTFSCopyGUI兼容性的优化位置列表
        let priority_locations = vec![
            6,    // LTFSCopyGUI找到索引的位置 - 最高优先级
            50,   // 测试中成功的位置 - 高优先级
            1000, // 原有的成功位置
            2, 5, // 标准LTFS位置
            10, 20, // 常见位置
            100, 200, 500, // 中等距离位置
            2000, 5000, // 较远位置
        ];

        info!("Trying {} priority locations with block 6 (LTFSCopyGUI-compatible) as highest priority", priority_locations.len());

        // 串行搜索优先位置（避免并行磁带操作的复杂性）
        for &block in &priority_locations {
            if let Ok(()) = self.scsi.locate_block(0, block) {
                debug!("🎯 Testing priority location: block {}", block);

                // 使用智能读取方法
                match self.try_read_index_intelligently(block) {
                    Ok(xml_content) => {
                        if xml_content.contains("<ltfsindex")
                            && xml_content.contains("</ltfsindex>")
                        {
                            info!(
                                "✅ Found index at priority location: block {} (intelligent read)",
                                block
                            );
                            self.cache_successful_location(block);
                            return Ok((xml_content, block));
                        }
                    }
                    Err(e) => {
                        debug!("Intelligent read failed at block {}: {}", block, e);
                    }
                }
            }
        }

        // 如果优先位置都失败，回退到原有的完整搜索
        info!("Priority locations failed, falling back to comprehensive search");
        match self.try_alternative_index_reading_strategies_async().await {
            Ok(xml_content) => {
                // 估算找到的位置（实际实现中应该记录确切位置）
                Ok((xml_content, 1000)) // 默认位置
            }
            Err(e) => Err(e),
        }
    }

    /// 智能索引读取 - 在指定位置使用优化方法
    fn try_read_index_intelligently(&self, block: u64) -> Result<String> {
        info!("🎯 Trying intelligent index read at block {}", block);

        // 获取动态blocksize
        let block_size = self
            .partition_label
            .as_ref()
            .map(|plabel| plabel.blocksize as usize)
            .unwrap_or(crate::scsi::block_sizes::LTO_BLOCK_SIZE as usize);

        debug!("Using blocksize {} bytes for intelligent read", block_size);

        // 使用智能读取方法
        self.read_index_intelligently_with_partitions(block_size)
    }

    /// 智能索引读取方法 - 高效版本
    /// 解决260MB数据获取12KB索引的效率问题
    pub fn read_index_intelligently(&self, block_size: usize) -> Result<String> {
        info!("🚀 Starting intelligent index reading (optimized for efficiency)");

        // Phase 1: 快速预验证 - 读取前几个块检测索引标记
        const PREVIEW_BLOCKS: usize = 2; // 只读取2个块进行预验证
        let preview_size = block_size * PREVIEW_BLOCKS;
        let mut preview_buffer = vec![0u8; preview_size];

        info!(
            "Phase 1: Quick preview - reading {} blocks ({} bytes) for validation",
            PREVIEW_BLOCKS, preview_size
        );

        match self
            .scsi
            .read_blocks(PREVIEW_BLOCKS as u32, &mut preview_buffer)
        {
            Ok(blocks_read) => {
                if blocks_read == 0 {
                    debug!("Preview read returned 0 blocks - no data at current position");
                    return Err(RustLtfsError::ltfs_index(
                        "No data at current position".to_string(),
                    ));
                }

                // 转换为字符串进行快速检测
                let preview_text = String::from_utf8_lossy(&preview_buffer);

                // 检测LTFS索引标记
                if !preview_text.contains("<ltfsindex") {
                    debug!(
                        "❌ Preview validation failed - no <ltfsindex> found in first {} blocks",
                        PREVIEW_BLOCKS
                    );
                    return Err(RustLtfsError::ltfs_index(
                        "No LTFS index marker found in preview".to_string(),
                    ));
                }

                info!(
                    "✅ Preview validation passed - LTFS index detected, proceeding with full read"
                );
            }
            Err(e) => {
                debug!("Preview read failed: {}", e);
                return Err(RustLtfsError::ltfs_index(format!(
                    "Preview read failed: {}",
                    e
                )));
            }
        }

        // Phase 2: 智能完整读取 - 既然检测到索引，进行优化的完整读取
        info!("Phase 2: Intelligent full read with early termination");

        // 创建临时文件
        let temp_dir = std::env::temp_dir();
        let temp_filename = format!(
            "LTFSIndex_Smart_{}.tmp",
            chrono::Utc::now().format("%Y%m%d_%H%M%S")
        );
        let temp_path = temp_dir.join(temp_filename);
        info!("Creating temporary index file: {:?}", temp_path);

        let mut temp_file = std::fs::File::create(&temp_path)?;

        // 首先写入已经读取的预览数据
        use std::io::Write;
        temp_file.write_all(&preview_buffer)?;

        let mut total_bytes_read = preview_size as u64;
        let mut blocks_read = PREVIEW_BLOCKS;
        let max_blocks = 50; // 减少最大限制从200到50
        let mut consecutive_zero_blocks = 0;
        const MAX_CONSECUTIVE_ZEROS: usize = 3; // 连续3个零块就停止

        info!(
            "Continuing read from block {} with max {} total blocks",
            blocks_read + 1,
            max_blocks
        );

        // 继续读取剩余数据
        loop {
            if blocks_read >= max_blocks {
                info!("Reached intelligent block limit ({}), stopping", max_blocks);
                break;
            }

            let mut buffer = vec![0u8; block_size];

            match self.scsi.read_blocks(1, &mut buffer) {
                Ok(read_count) => {
                    if read_count == 0 {
                        info!("✅ Reached file mark (read_count = 0), stopping");
                        break;
                    }

                    // 检测零块（可能表示数据结束）
                    let is_zero_block = buffer.iter().all(|&b| b == 0);
                    if is_zero_block {
                        consecutive_zero_blocks += 1;
                        debug!(
                            "Zero block detected ({}/{})",
                            consecutive_zero_blocks, MAX_CONSECUTIVE_ZEROS
                        );

                        if consecutive_zero_blocks >= MAX_CONSECUTIVE_ZEROS {
                            info!("✅ Detected {} consecutive zero blocks, stopping read (likely end of data)",
                                  consecutive_zero_blocks);
                            break;
                        }
                    } else {
                        consecutive_zero_blocks = 0; // 重置零块计数器

                        // 检测索引结束标记
                        let text_chunk = String::from_utf8_lossy(&buffer);
                        if text_chunk.contains("</ltfsindex>") {
                            // 找到索引结束，写入这最后一块然后停止
                            temp_file.write_all(&buffer)?;
                            total_bytes_read += block_size as u64;
                            blocks_read += 1;
                            info!(
                                "✅ Found </ltfsindex> marker, index complete after {} blocks",
                                blocks_read
                            );
                            break;
                        }
                    }

                    temp_file.write_all(&buffer)?;
                    total_bytes_read += block_size as u64;
                    blocks_read += 1;

                    // 每10个块报告一次进度
                    if blocks_read % 10 == 0 {
                        debug!(
                            "Read {} blocks, {} bytes so far",
                            blocks_read, total_bytes_read
                        );
                    }
                }
                Err(e) => {
                    debug!("SCSI read error after {} blocks: {}", blocks_read, e);
                    if blocks_read <= PREVIEW_BLOCKS {
                        return Err(RustLtfsError::ltfs_index(
                            "Failed to read beyond preview data".to_string(),
                        ));
                    }
                    // 已经有一些数据，尝试解析
                    break;
                }
            }
        }

        temp_file.flush()?;
        drop(temp_file);

        info!(
            "🎯 Intelligent read completed: {} blocks read, {} total bytes (vs old method: ~13MB)",
            blocks_read, total_bytes_read
        );

        // 读取并清理临时文件
        let xml_content = std::fs::read_to_string(&temp_path)?;

        // 清理临时文件
        if let Err(e) = std::fs::remove_file(&temp_path) {
            warn!("Failed to remove temporary file {:?}: {}", temp_path, e);
        }

        // 清理XML内容
        let cleaned_xml = xml_content.replace('\0', "").trim().to_string();

        if cleaned_xml.is_empty() {
            return Err(RustLtfsError::ltfs_index(
                "Cleaned XML is empty".to_string(),
            ));
        }

        // 验证XML完整性
        if !cleaned_xml.contains("<ltfsindex") || !cleaned_xml.contains("</ltfsindex>") {
            return Err(RustLtfsError::ltfs_index(
                "Incomplete LTFS index XML".to_string(),
            ));
        }

        info!(
            "✅ Intelligent read extracted {} bytes of valid index data",
            cleaned_xml.len()
        );
        Ok(cleaned_xml)
    }

    /// 智能索引读取方法 - 直接使用当前TapeOperations的实现
    fn read_index_intelligently_with_partitions(&self, block_size: usize) -> Result<String> {
        // 直接使用当前TapeOperations实例的智能读取实现
        self.read_index_intelligently(block_size)
    }

    /// 按照LTFSCopyGUI逻辑从数据分区EOD读取最新索引
    /// 对应VB.NET读取数据区索引ToolStripMenuItem_Click的核心逻辑
    async fn try_read_latest_index_from_data_partition_eod(&mut self) -> Result<String> {
        info!("Reading latest index from data partition EOD (matching LTFSCopyGUI 读取数据区索引)");

        let data_partition = 1; // 数据分区

        // Step 1: 定位到数据分区EOD (对应LTFSCopyGUI: TapeUtils.Locate(driveHandle, 0UL, DataPartition, TapeUtils.LocateDestType.EOD))
        info!("Locating to data partition {} EOD", data_partition);

        match self.scsi.locate_block(data_partition, 0) {
            Ok(()) => info!(
                "Successfully positioned to data partition {}, block 0",
                data_partition
            ),
            Err(e) => {
                warn!(
                    "Failed to locate to data partition {}, block 0: {}",
                    data_partition, e
                );
                return Err(RustLtfsError::ltfs_index(format!(
                    "Cannot position to data partition: {}",
                    e
                )));
            }
        }

        // 使用LOCATE命令而非SPACE命令进行EOD定位（LTFSCopyGUI兼容）
        info!("Using LOCATE command for EOD positioning (LTFSCopyGUI compatible)");
        match self.scsi.locate_to_eod(data_partition) {
            Ok(()) => info!(
                "Successfully located to End of Data in partition {}",
                data_partition
            ),
            Err(e) => {
                warn!(
                    "Failed to locate to End of Data in partition {}: {}",
                    data_partition, e
                );
                return Err(RustLtfsError::ltfs_index(format!(
                    "Cannot locate to EOD: {}",
                    e
                )));
            }
        }

        let eod_position = self.scsi.read_position()?;
        info!(
            "Data partition EOD position: partition={}, block={}, file_number={}",
            eod_position.partition, eod_position.block_number, eod_position.file_number
        );

        // Step 2: 检查 FileNumber，确保有足够的 FileMark (对应LTFSCopyGUI: If FM <= 1 Then)
        if eod_position.file_number <= 1 {
            return Err(RustLtfsError::ltfs_index(
                "Insufficient file marks in data partition for index reading".to_string(),
            ));
        }

        // Step 3: 定位到最后一个FileMark之前 (对应LTFSCopyGUI: TapeUtils.Locate(handle:=driveHandle, BlockAddress:=FM - 1, Partition:=DataPartition, DestType:=TapeUtils.LocateDestType.FileMark))
        let target_filemark = eod_position.file_number - 1;
        info!("Locating to FileMark {} in data partition", target_filemark);

        match self
            .scsi
            .locate_to_filemark(target_filemark, data_partition)
        {
            Ok(()) => {
                info!("Successfully positioned to FileMark {}", target_filemark);

                // Step 4: 跳过FileMark并读取索引内容 (对应LTFSCopyGUI: TapeUtils.ReadFileMark + TapeUtils.ReadToFileMark)
                match self.scsi.space(crate::scsi::SpaceType::FileMarks, 1) {
                    Ok(_) => {
                        info!("Skipped FileMark, now reading latest index content");
                        let position_after_fm = self.scsi.read_position()?;
                        info!(
                            "Position after FileMark: partition={}, block={}",
                            position_after_fm.partition, position_after_fm.block_number
                        );

                        // 读取索引内容
                        match self.try_read_index_at_current_position_with_filemarks() {
                            Ok(xml_content) => {
                                if xml_content.contains("<ltfsindex")
                                    && xml_content.contains("</ltfsindex>")
                                {
                                    info!("✅ Successfully read latest index from data partition EOD at FileMark {}", target_filemark);
                                    return Ok(xml_content);
                                } else {
                                    warn!("Content at data partition EOD FileMark {} is not valid LTFS index", target_filemark);
                                }
                            }
                            Err(e) => {
                                debug!(
                                    "Failed to read content at data partition EOD FileMark {}: {}",
                                    target_filemark, e
                                );
                            }
                        }
                    }
                    Err(e) => {
                        debug!("Failed to skip FileMark {}: {}", target_filemark, e);
                    }
                }
            }
            Err(e) => {
                debug!(
                    "Failed to locate to FileMark {} in data partition: {}",
                    target_filemark, e
                );
            }
        }

        Err(RustLtfsError::ltfs_index(
            "No valid latest index found at data partition EOD".to_string(),
        ))
    }

    /// 按照LTFSCopyGUI逻辑从指定分区EOD读取最新索引
    /// 对应单分区磁带的索引读取逻辑
    async fn try_read_latest_index_from_eod(&mut self, partition: u8) -> Result<String> {
        info!("Reading latest index from partition {} EOD", partition);

        // Step 1: 定位到指定分区EOD
        info!("Locating to partition {} EOD", partition);
        self.scsi.locate_block(partition, 0)?;
        // 使用LOCATE命令定位到指定分区的EOD（LTFSCopyGUI兼容）
        match self.scsi.locate_to_eod(partition) {
            Ok(()) => info!(
                "Successfully located to End of Data in partition {}",
                partition
            ),
            Err(e) => {
                warn!(
                    "Failed to locate to End of Data in partition {}: {}",
                    partition, e
                );
                return Err(RustLtfsError::ltfs_index(format!(
                    "Cannot locate to EOD: {}",
                    e
                )));
            }
        }

        let eod_position = self.scsi.read_position()?;
        info!(
            "Partition {} EOD position: partition={}, block={}, file_number={}",
            partition, eod_position.partition, eod_position.block_number, eod_position.file_number
        );

        // Step 2: 检查 FileNumber，确保有足够的 FileMark
        if eod_position.file_number <= 1 {
            return Err(RustLtfsError::ltfs_index(format!(
                "Insufficient file marks in partition {} for index reading",
                partition
            )));
        }

        // Step 3: 根据分区类型确定目标FileMark
        // 🔧 修复：索引分区(P0)使用固定的FileMark 1（LTFS标准位置）
        // 数据分区(P1)使用FM-1策略（最新索引在EOD之前）
        let target_filemark = if partition == 0 {
            // 索引分区：LTFS标准索引位置在FileMark 1之后
            info!("Index partition (P0): using standard LTFS location FileMark 1");
            1
        } else {
            // 数据分区：最新索引在最后一个FileMark之前
            info!("Data partition (P{}): using FM-1 strategy", partition);
            eod_position.file_number - 1
        };
        info!(
            "Locating to FileMark {} in partition {}",
            target_filemark, partition
        );

        match self.scsi.locate_to_filemark(target_filemark, partition) {
            Ok(()) => {
                info!(
                    "Successfully positioned to FileMark {} in partition {}",
                    target_filemark, partition
                );

                // Step 4: 跳过FileMark并读取索引内容
                match self.scsi.space(crate::scsi::SpaceType::FileMarks, 1) {
                    Ok(_) => {
                        info!("Skipped FileMark, now reading latest index content");

                        // 读取索引内容
                        match self.try_read_index_at_current_position_with_filemarks() {
                            Ok(xml_content) => {
                                if xml_content.contains("<ltfsindex")
                                    && xml_content.contains("</ltfsindex>")
                                {
                                    info!("✅ Successfully read latest index from partition {} EOD at FileMark {}", partition, target_filemark);
                                    return Ok(xml_content);
                                } else {
                                    warn!("Content at partition {} EOD FileMark {} is not valid LTFS index", partition, target_filemark);
                                }
                            }
                            Err(e) => {
                                debug!(
                                    "Failed to read content at partition {} EOD FileMark {}: {}",
                                    partition, target_filemark, e
                                );
                            }
                        }
                    }
                    Err(e) => {
                        debug!(
                            "Failed to skip FileMark {} in partition {}: {}",
                            target_filemark, partition, e
                        );
                    }
                }
            }
            Err(e) => {
                debug!(
                    "Failed to locate to FileMark {} in partition {}: {}",
                    target_filemark, partition, e
                );
            }
        }

        Err(RustLtfsError::ltfs_index(format!(
            "No valid latest index found at partition {} EOD",
            partition
        )))
    }

    /// 标准LTFS读取策略 - 基于成功的标准读取流程
    async fn try_standard_ltfs_reading(&mut self) -> Result<String> {
        info!("🔍 Starting standard LTFS reading strategy");

        // 定位到索引分区并读取VOL1标签
        self.scsi.locate_block(0, 0)?;
        let mut label_buffer = vec![0u8; crate::scsi::block_sizes::LTO_BLOCK_SIZE as usize];
        self.scsi.read_blocks(1, &mut label_buffer)?;

        let vol1_valid = self.parse_vol1_label(&label_buffer)?;

        if vol1_valid {
            info!("VOL1 label validation passed, trying standard reading");

            let partition_strategy = self.detect_partition_strategy().await?;

            match partition_strategy {
                super::partition_manager::PartitionStrategy::StandardMultiPartition => {
                    // 首先尝试从索引分区EOD读取最新索引（LTFSCopyGUI逻辑）
                    info!("Multi-partition: trying index partition EOD first (LTFSCopyGUI logic)");
                    match self.try_read_latest_index_from_eod(0).await {
                        Ok(xml_content) => {
                            info!("✅ Standard LTFS reading (index partition EOD) succeeded");
                            return Ok(xml_content);
                        }
                        Err(e) => debug!("Index partition EOD reading failed: {}", e),
                    }

                    // 备用：使用ReadToFileMark方法读取整个索引文件
                    match self.read_index_xml_from_tape_with_file_mark() {
                        Ok(xml_content) => {
                            if !xml_content.trim().is_empty()
                                && xml_content.contains("<ltfsindex")
                                && xml_content.contains("</ltfsindex>")
                            {
                                info!("✅ Standard LTFS reading (ReadToFileMark) succeeded");
                                return Ok(xml_content);
                            }
                        }
                        Err(e) => debug!("Standard reading failed: {}", e),
                    }
                }
                super::partition_manager::PartitionStrategy::SinglePartitionFallback => {
                    // 单分区策略：从partition 0 EOD读取最新索引
                    match self.try_read_latest_index_from_eod(0).await {
                        Ok(xml_content) => {
                            info!("✅ Standard LTFS reading (single partition EOD) succeeded");
                            return Ok(xml_content);
                        }
                        Err(e) => debug!("Single partition EOD reading failed: {}", e),
                    }

                    // 备用：单分区策略读取
                    match self.try_single_partition_extended_search_async().await {
                        Ok(xml_content) => {
                            info!("✅ Standard LTFS reading (single partition extended) succeeded");
                            return Ok(xml_content);
                        }
                        Err(e) => debug!("Single partition standard reading failed: {}", e),
                    }
                }
                super::partition_manager::PartitionStrategy::IndexFromDataPartition => {
                    // 数据分区索引策略
                    match self.try_read_from_data_partition_async().await {
                        Ok(xml_content) => {
                            info!("✅ Standard LTFS reading (data partition) succeeded");
                            return Ok(xml_content);
                        }
                        Err(e) => debug!("Data partition standard reading failed: {}", e),
                    }
                }
            }
        } else {
            warn!("VOL1 label validation failed, trying fallback strategies");
        }

        // VOL1验证失败或标准策略失败时的回退策略
        info!("Trying fallback strategy: index partition EOD");

        // 首先尝试从索引分区EOD读取（不依赖VOL1验证）
        match self.try_read_latest_index_from_eod(0).await {
            Ok(xml_content) => {
                info!("✅ Standard LTFS reading (fallback EOD) succeeded");
                return Ok(xml_content);
            }
            Err(e) => debug!("Fallback EOD reading failed: {}", e),
        }

        // 最后尝试：直接搜索已知位置（仅作为最后手段）
        info!("Trying final fallback: direct location search");
        let fallback_locations = vec![6, 2, 5, 10, 20, 100]; // block 6是LTFSCopyGUI找到索引的位置

        for &block in &fallback_locations {
            info!(
                "Trying final fallback location: partition 0, block {}",
                block
            );

            match self.scsi.locate_block(0, block) {
                Ok(()) => {
                    match self.try_read_index_at_current_position_with_filemarks() {
                        Ok(xml_content) => {
                            if !xml_content.trim().is_empty()
                                && xml_content.contains("<ltfsindex")
                                && xml_content.contains("</ltfsindex>")
                            {
                                info!("✅ Standard LTFS reading (final fallback) succeeded at block {}", block);
                                return Ok(xml_content);
                            }
                        }
                        Err(e) => debug!("Failed to read index at fallback block {}: {}", block, e),
                    }
                }
                Err(e) => debug!("Cannot position to fallback block {}: {}", block, e),
            }
        }

        Err(RustLtfsError::ltfs_index(
            "Standard LTFS reading strategy failed (including all fallbacks)".to_string(),
        ))
    }
}
