use crate::error::{Result, RustLtfsError};
use super::PartitionStrategy;
use super::TapeFormatAnalysis;

use tracing::{debug, info, warn};
use chrono;
use crate::scsi::block_sizes;

// LtfsPartitionLabel 在 format_operations.rs 中定义
// 通过模块重新导出使用

/// TapeOperations读取操作实现
impl super::TapeOperations {
    /// 验证并处理索引 - 增强版本：添加详细调试信息
    pub async fn validate_and_process_index(&mut self, xml_content: &str) -> Result<bool> {
        debug!("🔍 Validating index content: {} bytes", xml_content.len());

        // 🔍 添加详细的验证日志
        let preview = xml_content.chars().take(300).collect::<String>();
        debug!("🔍 Index content preview: {:?}", preview);

        // 基本验证XML格式
        if !xml_content.contains("<ltfsindex") || !xml_content.contains("</ltfsindex>") {
            debug!("❌ Basic XML validation failed - missing LTFS index tags");
            debug!(
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

        debug!("✅ Basic XML validation passed - LTFS index tags found");

        // 解析并设置索引
        match crate::ltfs_index::LtfsIndex::from_xml(xml_content) {
            Ok(index) => {
                debug!("✅ XML parsing successful - setting index");
                debug!("   Volume UUID: {}", index.volumeuuid);
                debug!("   Generation: {}", index.generationnumber);
                debug!(
                    "   Files count: {}",
                    self.count_files_in_directory(&index.root_directory)
                );
                self.index = Some(index);
                Ok(true)
            }
            Err(e) => {
                warn!("❌ XML parsing failed: {}", e);
                debug!("🔍 Failed XML content length: {} bytes", xml_content.len());
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
        debug!("🔧 Detecting partition strategy using opened SCSI device (fixing device handle inconsistency)");

        // 直接使用已初始化的ExtraPartitionCount，避免创建新的PartitionManager实例
        let extra_partition_count = self.get_extra_partition_count();

        debug!(
            "Determining partition strategy based on ExtraPartitionCount = {}",
            extra_partition_count
        );

        match extra_partition_count {
            0 => {
                debug!("Single-partition strategy (ExtraPartitionCount = 0)");
                Ok(PartitionStrategy::SinglePartitionFallback)
            }
            1 => {
                debug!("Dual-partition strategy (ExtraPartitionCount = 1)");
                Ok(PartitionStrategy::StandardMultiPartition)
            }
            _ => {
                debug!(
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



        debug!("=== LTFSCopyGUI Compatible Index Reading Process ===");

        // Step 1 (最高优先级): LTFSCopyGUI兼容方法
        debug!("Step 1 (Highest Priority): LTFSCopyGUI compatible method");

        // 检测分区策略并决定读取顺序
        let extra_partition_count = self.get_extra_partition_count();

        if extra_partition_count > 0 {
            // 双分区磁带：使用LTFSCopyGUI方法从数据分区读取索引
            debug!("Dual-partition detected, using LTFSCopyGUI method from data partition");

            // LTFSCopyGUI method removed as part of cleanup
            debug!("LTFSCopyGUI method skipped (cleanup)");

            // 🔧 双分区backup策略：尝试从索引分区(partition 0) EOD读取
            debug!("🔧 Trying dual-partition backup strategy: index partition EOD");
            match self.try_read_latest_index_from_eod(0).await {
                Ok(xml_content) => {
                    if self.validate_and_process_index(&xml_content).await? {
                        debug!("✅ Step 1 succeeded - index read from index partition EOD (dual-partition fallback)");
                        info!("Index loaded successfully ({} files)", self.index.as_ref().map(|i| self.count_files_in_directory(&i.root_directory)).unwrap_or(0));
                        return Ok(());
                    }
                }
                Err(e) => {
                    debug!("Index partition EOD strategy failed: {}", e);
                }
            }
        } else {
            // 单分区磁带：从partition=0读取索引
            debug!("Single-partition detected, reading from partition 0");

            match self.try_read_latest_index_from_eod(0).await {
                Ok(xml_content) => {
                    if self.validate_and_process_index(&xml_content).await? {
                        debug!("✅ Step 1 succeeded - index read from partition 0 EOD (single-partition logic)");
                        info!("Index loaded successfully ({} files)", self.index.as_ref().map(|i| self.count_files_in_directory(&i.root_directory)).unwrap_or(0));
                        return Ok(());
                    }
                }
                Err(e) => {
                    debug!("Single-partition EOD strategy failed: {}", e);
                }
            }
        }

        // Step 2: 标准流程作为备用策略
        debug!("Step 2: Standard LTFS reading process as fallback");

        // 定位到索引分区并读取VOL1标签
        self.scsi.locate_block(0, 0)?;
        let mut label_buffer = vec![0u8; crate::scsi::block_sizes::LTO_BLOCK_SIZE as usize];
        self.scsi.read_blocks(1, &mut label_buffer)?;

        let vol1_valid = self.parse_vol1_label(&label_buffer)?;

        if vol1_valid {
            debug!("VOL1 label validation passed, trying standard reading");

            let partition_strategy = self.detect_partition_strategy().await?;

            match partition_strategy {
                PartitionStrategy::StandardMultiPartition => {
                    // 尝试数据分区EOD策略
                    match self.try_read_latest_index_from_data_partition_eod().await {
                        Ok(xml_content) => {
                            if self.validate_and_process_index(&xml_content).await? {
                                debug!("✅ Standard reading (data partition EOD) succeeded");
                                info!("Index loaded successfully ({} files)", self.index.as_ref().map(|i| self.count_files_in_directory(&i.root_directory)).unwrap_or(0));
                                return Ok(());
                            }
                        }
                        Err(e) => debug!("Data partition EOD reading failed: {}", e),
                    }

                    // 使用ReadToFileMark方法读取整个索引文件
                    match self.read_index_xml_from_tape_with_file_mark() {
                        Ok(xml_content) => {
                            if self.validate_and_process_index(&xml_content).await? {
                                debug!("✅ Standard reading strategy succeeded");
                                info!("Index loaded successfully ({} files)", self.index.as_ref().map(|i| self.count_files_in_directory(&i.root_directory)).unwrap_or(0));
                                return Ok(());
                            }
                        }
                        Err(e) => debug!("Standard reading failed: {}", e),
                    }
                }
                PartitionStrategy::SinglePartitionFallback => {
                    let xml = self.try_read_latest_index_from_eod(0).await?;
                    if self.validate_and_process_index(&xml).await? {
                        return Ok(());
                    } else {
                        return Err(RustLtfsError::ltfs_index("Index validation failed"));
                    }
                }

            }
        }

        // Step 3: 最后的多分区策略回退
        debug!("Step 3: Final multi-partition strategy fallback");

        let partition_strategy = self
            .detect_partition_strategy()
            .await
            .unwrap_or(PartitionStrategy::StandardMultiPartition);

        match partition_strategy {
            PartitionStrategy::SinglePartitionFallback => {
                debug!("🔄 Trying single-partition fallback strategy");
                let xml = self.try_read_latest_index_from_eod(0).await?;
                if self.validate_and_process_index(&xml).await? {
                    Ok(())
                } else {
                    Err(RustLtfsError::ltfs_index("Index validation failed"))
                }
            }

            PartitionStrategy::StandardMultiPartition => {
                debug!("🔄 Trying standard multi-partition strategy without VOL1 validation");

                // 最后尝试：有限的固定位置搜索（仅作为最后手段）
                let standard_locations = vec![6, 5, 2, 0]; // block 6仍然保留以兼容特殊情况

                for &block in &standard_locations {
                    debug!("Trying final fallback at p0 block {}", block);
                    match self.scsi.locate_block(0, block) {
                        Ok(()) => match self.read_index_xml_from_tape_with_file_mark() {
                            Ok(xml_content) => {
                                if self.validate_and_process_index(&xml_content).await? {
                                    debug!("✅ Successfully read index from p0 block {} (final fallback)", block);
                                    info!("Index loaded successfully ({} files)", self.index.as_ref().map(|i| self.count_files_in_directory(&i.root_directory)).unwrap_or(0));
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

                debug!(
                    "🔄 All standard locations failed, falling back to single-partition strategy"
                );
                match self.search_index_copies_in_data_partition() {
                    Ok(xml_content) => {
                        debug!(
                            "🔍 LTFSCopyGUI method returned {} bytes of content",
                            xml_content.len()
                        );
                        match self.validate_and_process_index(&xml_content).await? {
                            true => {
                                debug!("✅ Step 1 succeeded - LTFS index read using LTFSCopyGUI method (dual-partition)");
                                info!("Index loaded successfully ({} files)", self.index.as_ref().map(|i| self.count_files_in_directory(&i.root_directory)).unwrap_or(0));
                                return Ok(());
                            }
                            false => {
                                warn!("⚠️ LTFSCopyGUI method read data but XML validation failed");
                                debug!("🔍 This suggests the data at FileMark 1 position is not valid LTFS XML");
                            }
                        }
                    }
                    Err(e) => {
                        warn!("❌ LTFSCopyGUI method failed completely: {}", e);
                        debug!("LTFSCopyGUI method failed: {}", e);
                    }
                }
                
                // Fallback to simple EOD read if LTFSCopyGUI method fails
                let xml = self.try_read_latest_index_from_eod(0).await?;
                if self.validate_and_process_index(&xml).await? {
                    Ok(())
                } else {
                    Err(RustLtfsError::ltfs_index("Index validation failed"))
                }
            }
        }
    }

    /// 解析volume label中的索引位置信息


    /// 在当前位置尝试读取索引（简化版本）


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


    /// Parse LTFS volume label to extract index location


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


    /// 检查临时文件是否包含XML结束标记


    /// Read index XML data from tape with progressive expansion


    /// Validate index XML structure


    /// Load index from local file


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








    /// 检查设备是否就绪


    /// 检测磁带分区数量 (对应LTFSCopyGUI的ExtraPartitionCount检测逻辑)



    /// 尝试从数据分区读取索引副本


    /// 单分区磁带的扩展搜索


    /// 保存索引并返回内容








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

    /// 使用临时文件读取到文件标记 (精准对应TapeUtils.ReadToFileMark)
    pub fn read_to_file_mark_with_temp_file(&self, block_size: usize) -> Result<String> {
        use std::io::Write;

        // 创建临时文件 (对应LTFSCopyGUI的tmpFile)
        let temp_dir = std::env::temp_dir();
        let temp_filename = format!(
            "LTFSIndex_{}.tmp",
            chrono::Utc::now().format("%Y%m%d_%H%M%S")
        );
        let temp_path = temp_dir.join(temp_filename);

        info!("Creating temporary index file: {:?}", temp_path);

        let mut temp_file = std::fs::File::create(&temp_path)?;
        let mut total_bytes_read = 0u64;
        let mut blocks_read = 0;
        // Start conservatively and expand if we detect a '<?xml' start tag in the temporary file.
        // hard_max_blocks is an absolute safety cap (matches previous fixed limit).
        let hard_max_blocks = 200u32; // 对应LTFSCopyGUI的固定限制上限（安全上限）
        let mut max_blocks = 50u32; // 初始较小值，避免一次读太多无效数据
        let mut consecutive_errors = 0;
        const MAX_CONSECUTIVE_ERRORS: u32 = 3;

        debug!(
            "Starting ReadToFileMark with blocksize {}, max {} blocks (enhanced SCSI error handling)",
            block_size, max_blocks
        );

        // 精准模仿LTFSCopyGUI的ReadToFileMark循环 + 增强错误处理
        loop {
            // 安全限制 - 防止无限读取（对应LTFSCopyGUI逻辑）
            if blocks_read >= max_blocks {
                warn!("Reached maximum block limit ({}), stopping", max_blocks);
                break;
            }

            let mut buffer = vec![0u8; block_size];

            // 执行SCSI READ命令 (对应ScsiRead调用) + 增强错误处理
            match self.scsi.read_blocks(1, &mut buffer) {
                Ok(blocks_read_count) => {
                    consecutive_errors = 0; // 重置错误计数器
                    debug!("SCSI read returned: {} blocks", blocks_read_count);

                    // 对应: If bytesRead = 0 Then Exit Do
                    if blocks_read_count == 0 {
                        debug!("✅ Reached file mark (blocks_read_count = 0), stopping read");
                        break;
                    }

                    // 添加数据采样调试（仅DEBUG级别输出）
                    if tracing::enabled!(tracing::Level::DEBUG) {
                        let sample_size = std::cmp::min(32, buffer.len());
                        let sample_data: Vec<String> = buffer[..sample_size]
                            .iter()
                            .map(|&b| format!("{:02X}", b))
                            .collect();
                        debug!(
                            "Buffer sample (first {} bytes): {}",
                            sample_size,
                            sample_data.join(" ")
                        );
                    }

                    // 写入到输出文件 (对应fileStream.Write(buffer, 0, bytesRead))
                    temp_file.write_all(&buffer)?;
                    total_bytes_read += block_size as u64;
                    blocks_read += 1;

                    debug!(
                        "Read block {}: {} bytes, total: {} bytes",
                        blocks_read, block_size, total_bytes_read
                    );

                    // 动态扩展策略：
                    // 如果我们尚未扩大到硬上限，并且临时文件中检测到了 "<?xml"（意味着索引开始出现），
                    // 则将 max_blocks 扩展到 hard_max_blocks，以便继续读取直至找到完整的 </ltfsindex>（或达到硬上限）。
                    if max_blocks < hard_max_blocks {
                        if let Ok(mut f) = std::fs::File::open(&temp_path) {
                            use std::io::{Read, Seek, SeekFrom};
                            if let Ok(file_len) = f.seek(SeekFrom::End(0)) {
                                // 检查文件末尾的一小段（最多 4KB），通常足以检测 "<?xml" 或其他索引起始标识
                                let check_len = std::cmp::min(4096, file_len) as usize;
                                if check_len > 0 {
                                    if f.seek(SeekFrom::End(-(check_len as i64))).is_ok() {
                                        let mut tail_buf = vec![0u8; check_len];
                                        if f.read_exact(&mut tail_buf).is_ok() {
                                            if String::from_utf8_lossy(&tail_buf).contains("<?xml")
                                            {
                                                debug!(
                                                    "Detected '<?xml' in temporary index file; expanding max_blocks: {} -> {}",
                                                    max_blocks, hard_max_blocks
                                                );
                                                max_blocks = hard_max_blocks;
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
                Err(e) => {
                    consecutive_errors += 1;
                    warn!(
                        "SCSI read error #{} after {} blocks: {}",
                        consecutive_errors, blocks_read, e
                    );

                    // 增强的SCSI错误分类和恢复
                    let error_handled =
                        self.handle_scsi_read_error(&e, blocks_read, consecutive_errors)?;

                    if !error_handled {
                        // 如果没有读取任何数据就失败，返回错误
                        if blocks_read == 0 {
                            return Err(RustLtfsError::ltfs_index(format!(
                                "No data could be read from tape after {} consecutive errors: {}",
                                consecutive_errors, e
                            )));
                        }
                        // 如果已经读取了一些数据，就停止并尝试解析
                        break;
                    }

                    // 如果连续错误过多，停止尝试
                    if consecutive_errors >= MAX_CONSECUTIVE_ERRORS {
                        warn!(
                            "Too many consecutive SCSI errors ({}), stopping read operation",
                            consecutive_errors
                        );
                        if blocks_read == 0 {
                            return Err(RustLtfsError::scsi(format!(
                                "Failed to read any data after {} consecutive SCSI errors",
                                consecutive_errors
                            )));
                        }
                        break;
                    }
                }
            }
        }

        temp_file.flush()?;
        drop(temp_file);

        debug!(
            "ReadToFileMark completed: {} blocks read, {} total bytes",
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

        debug!(
            "Extracted XML content: {} bytes (after cleanup)",
            cleaned_xml.len()
        );
        Ok(cleaned_xml)
    }

    /// 增强的SCSI读取错误处理
    /// 返回true表示错误已处理，可以继续；返回false表示应该停止
    fn handle_scsi_read_error(
        &self,
        error: &RustLtfsError,
        blocks_read: u32,
        error_count: u32,
    ) -> Result<bool> {
        let error_str = error.to_string();

        // 错误分类和处理策略
        if error_str.contains("Direct block read operation failed") {
            debug!(
                "Detected direct block read failure - possibly reached end of data or file mark"
            );

            // 如果已经读取了一些数据，这可能是正常的文件结束
            if blocks_read > 0 {
                debug!(
                    "Block read failure after {} blocks - likely reached end of index data",
                    blocks_read
                );
                return Ok(false); // 正常结束
            } else {
                warn!("Block read failure on first block - may indicate positioning or hardware issue");
                return Ok(error_count <= 2); // 允许重试前2次
            }
        }

        if error_str.contains("Device not ready") || error_str.contains("Unit attention") {
            warn!("Device status issue detected - attempting recovery");

            // 尝试设备状态恢复
            match self.scsi.test_unit_ready() {
                Ok(_) => {
                    debug!("Device status recovered, can continue reading");
                    return Ok(true);
                }
                Err(e) => {
                    warn!("Device status recovery failed: {}", e);
                    return Ok(error_count <= 1); // 仅重试一次
                }
            }
        }

        if error_str.contains("Medium error") || error_str.contains("Unrecovered read error") {
            warn!("Medium/read error detected - this may indicate tape defect or wear");

            // 对于介质错误，如果已有数据就停止，否则尝试一次
            if blocks_read > 10 {
                debug!(
                    "Medium error after reading {} blocks - stopping to preserve data",
                    blocks_read
                );
                return Ok(false);
            } else {
                warn!("Early medium error - attempting one retry");
                return Ok(error_count <= 1);
            }
        }

        if error_str.contains("Illegal request") || error_str.contains("Invalid field") {
            warn!("SCSI command error detected - likely programming issue");
            return Ok(false); // 不重试命令错误
        }

        if error_str.contains("Hardware error") || error_str.contains("Communication failure") {
            warn!("Hardware/communication error - attempting limited retry");
            return Ok(error_count <= 1); // 有限重试
        }

        // 未知错误的保守处理
        debug!(
            "Unknown SCSI error type: {} - attempting conservative retry",
            error_str
        );
        Ok(error_count <= 2) // 允许有限重试
    }

    pub fn search_index_copies_in_data_partition(&self) -> Result<String> {
        info!("🔧 Starting LTFSCopyGUI-compatible index search (exact replication)");

        // 精确复制LTFSCopyGUI的读取索引逻辑
        self.read_index_ltfscopygui_method()
    }

    /// 精确复制LTFSCopyGUI的索引读取逻辑 (一比一实现)
    /// 支持单分区和多分区磁带的统一处理策略
    fn read_index_ltfscopygui_method(&self) -> Result<String> {
        debug!("🎯 Executing LTFSCopyGUI exact index reading method");

        // 步骤1: 检测ExtraPartitionCount (对应LTFSCopyGUI的分区检测)
        let extra_partition_count = match self.scsi.mode_sense_partition_page_0x11() {
            Ok(mode_data) if mode_data.len() >= 4 => {
                let count = mode_data[3];
                debug!("📊 ExtraPartitionCount detected from MODE SENSE: {}", count);
                count
            }
            _ => {
                debug!("📊 Cannot read ExtraPartitionCount, assuming single partition");
                0
            }
        };

        if extra_partition_count == 0 {
            // 🔧 单分区磁带策略 (对应LTFSCopyGUI的ExtraPartitionCount = 0逻辑)
            debug!("🎯 Single partition tape detected (ExtraPartitionCount=0)");
            self.read_index_single_partition_ltfscopygui()
        } else {
            // 🔧 多分区磁带策略 (对应LTFSCopyGUI的数据分区索引读取)
            debug!(
                "🎯 Multi-partition tape detected (ExtraPartitionCount={})",
                extra_partition_count
            );
            self.read_index_multi_partition_ltfscopygui(extra_partition_count)
        }
    }

    /// LTFSCopyGUI单分区索引读取策略 (精确复制"读取索引ToolStripMenuItem_Click"的单分区逻辑)
    fn read_index_single_partition_ltfscopygui(&self) -> Result<String> {
        debug!("🔧 LTFSCopyGUI single partition index reading (ExtraPartitionCount=0)");

        // 步骤1: 定位到分区0的EOD
        debug!("Step 1: Locating to partition 0 EOD");
        self.scsi.locate_to_eod(0)?;

        // 步骤2: 获取当前FileMark编号
        let position = self.scsi.read_position()?;
        let current_fm = position.file_number;

        debug!(
            "🔍 Current position at EOD: P{} B{} FM{} SET{}",
            position.partition, position.block_number, position.file_number, position.set_number
        );

        // 步骤3: LTFSCopyGUI的关键检查 - FM <= 1 则失败
        if current_fm <= 1 {
            return Err(RustLtfsError::ltfs_index(format!(
                "Invalid LTFS tape: FileMark number {} <= 1, this is not a valid LTFS tape",
                current_fm
            )));
        }

        // 步骤4: LTFSCopyGUI真实策略 - 定位到FileMark 1 (不是FM-1!)
        // 对应LTFSCopyGUI代码: TapeUtils.Locate(driveHandle, 1UL, partition, TapeUtils.LocateDestType.FileMark)
        debug!("Step 4: Locating to FileMark 1 (LTFSCopyGUI standard strategy)");
        self.scsi.locate_to_filemark(0, 1)?; // partition=0, filemark=1

        // 步骤5: ReadFileMark - 跳过FileMark标记
        debug!("Step 5: Skipping FileMark using ReadFileMark method");
        self.scsi.read_file_mark()?;

        // 步骤6: ReadToFileMark - 读取索引内容
        debug!("Step 6: Reading index content using ReadToFileMark");
        let index_data = self
            .scsi
            .read_to_file_mark(block_sizes::LTO_BLOCK_SIZE_512K)?;

        // 🎯 完全按照LTFSCopyGUI的验证逻辑：检查是否包含"XMLSchema"
        let xml_content = String::from_utf8_lossy(&index_data).to_string();
        if xml_content.contains("XMLSchema") {
            debug!("✅ Successfully read LTFS index using single partition method: {} bytes (contains XMLSchema)", xml_content.len());
            Ok(xml_content)
        } else {
            // 🔧 LTFSCopyGUI备选路径：FromSchemaText处理
            let processed_content = self.ltfscopygui_from_schema_text(xml_content)?;
            debug!(
                "✅ Successfully processed LTFS schema text format: {} bytes",
                processed_content.len()
            );
            Ok(processed_content)
        }
    }

    /// LTFSCopyGUI多分区索引读取策略 (精确复制"读取数据区索引ToolStripMenuItem_Click"逻辑)
    fn read_index_multi_partition_ltfscopygui(&self, extra_partition_count: u8) -> Result<String> {
        debug!(
            "🔧 LTFSCopyGUI multi-partition index reading (ExtraPartitionCount={})",
            extra_partition_count
        );

        // 🎯 关键修复：明确使用数据分区进行索引读取 (对应LTFSCopyGUI Line 4636逻辑)
        let data_partition = 1u8; // 数据分区固定为1
        debug!("🔧 Step 1: Targeting data partition {} for index reading (LTFSCopyGUI data partition strategy)", data_partition);

        // 步骤1a: 先切换到数据分区Block 0 (对应LTFSCopyGUI Line 4635)
        debug!(
            "Step 1a: Switching to data partition {} Block 0 (LTFSCopyGUI prerequisite)",
            data_partition
        );
        self.scsi.locate_block(data_partition, 0)?;

        // 步骤1b: 然后定位到数据分区的EOD (对应LTFSCopyGUI Line 4636)
        debug!("Step 1b: Locating to data partition EOD");
        self.scsi.locate_to_eod(data_partition)?;

        // 步骤3: 获取当前FileMark编号
        let position = self.scsi.read_position()?;
        let current_fm = position.file_number;

        debug!(
            "🔍 Data partition EOD position: P{} B{} FM{} SET{}",
            position.partition, position.block_number, position.file_number, position.set_number
        );

        // 🎯 应用LTFSCopyGUI Line 7138的核心逻辑：TapeUtils.Locate(driveHandle, CULng(FM - 1), DataPartition, TapeUtils.LocateDestType.FileMark)
        if current_fm > 1 {
            let target_fm = current_fm - 1;
            debug!("Step 2: Using LTFSCopyGUI FM-1 strategy: locating to FileMark {} on data partition", target_fm);
            self.scsi.locate_to_filemark(target_fm, data_partition)?;

            // 步骤3: ReadFileMark - 跳过FileMark
            debug!("Step 3: Skipping FileMark using ReadFileMark");
            self.scsi.read_file_mark()?;

            // 步骤4: ReadToFileMark - 读取索引 (使用动态blocksize)
            debug!(
                "Step 4: Reading data partition index using ReadToFileMark (LTFSCopyGUI blocksize)"
            );

            // 🔧 关键修复：使用plabel.blocksize而非固定大小 (对应LTFSCopyGUI Line 4661)
            let dynamic_blocksize = self
                .partition_label
                .as_ref()
                .map(|label| label.blocksize)
                .unwrap_or(block_sizes::LTO_BLOCK_SIZE);

            debug!(
                "🔧 Using dynamic blocksize: {} bytes (from partition label)",
                dynamic_blocksize
            );

            // 🔍 添加当前位置详细诊断
            let current_pos = self.scsi.read_position()?;
            debug!(
                "🔍 Current position before ReadToFileMark: P{} B{} FM{}",
                current_pos.partition, current_pos.block_number, current_pos.file_number
            );

            let index_data = self.scsi.read_to_file_mark(dynamic_blocksize)?;

            // 🎯 完全按照LTFSCopyGUI的验证逻辑：检查是否包含"XMLSchema"
            let xml_content = String::from_utf8_lossy(&index_data).to_string();

            // 🔍 添加详细诊断日志
            debug!(
                "🔍 Data partition index content length: {} bytes",
                xml_content.len()
            );
            let preview = xml_content.chars().take(200).collect::<String>();
            debug!("🔍 Data partition index content preview: {:?}", preview);
            let contains_xmlschema = xml_content.contains("XMLSchema");
            debug!(
                "🔍 Data partition XMLSchema check result: {}",
                contains_xmlschema
            );

            if contains_xmlschema {
                debug!("✅ Successfully read LTFS index from data partition using FM-1 strategy: {} bytes (contains XMLSchema)", xml_content.len());
                Ok(xml_content)
            } else {
                debug!("🔧 Data partition XMLSchema not found, applying FromSchemaText processing");
                // 🔧 LTFSCopyGUI备选路径：FromSchemaText处理
                let processed_content = self.ltfscopygui_from_schema_text(xml_content)?;
                debug!(
                    "✅ Successfully processed data partition LTFS schema text format: {} bytes",
                    processed_content.len()
                );
                Ok(processed_content)
            }
        } else {
            // 步骤4: LTFSCopyGUI的关键检查和策略选择
            debug!("Step 2: FM <= 1, using DisablePartition fallback (Space6 -2 FileMark)");
            self.ltfscopygui_disable_partition_fallback()
        }
    }

    /// LTFSCopyGUI的DisablePartition后备策略 (对应TapeUtils.Space6(-2, FileMark))
    fn ltfscopygui_disable_partition_fallback(&self) -> Result<String> {
        debug!("🔧 Executing LTFSCopyGUI DisablePartition fallback strategy");

        // 步骤1: Space6(-2, FileMark) - 后退2个FileMark
        debug!("Step 1: Moving back 2 FileMarks using Space6 command");
        self.scsi.space(crate::scsi::SpaceType::FileMarks, -2)?;

        // 步骤2: ReadFileMark - 跳过FileMark
        debug!("Step 2: Skipping FileMark using ReadFileMark");
        self.scsi.read_file_mark()?;

        // 步骤3: ReadToFileMark - 读取索引
        debug!("Step 3: Reading index using ReadToFileMark");
        let index_data = self
            .scsi
            .read_to_file_mark(block_sizes::LTO_BLOCK_SIZE_512K)?;

        // 🎯 完全按照LTFSCopyGUI的验证逻辑：检查是否包含"XMLSchema"
        let xml_content = String::from_utf8_lossy(&index_data).to_string();
        if xml_content.contains("XMLSchema") {
            debug!("✅ Successfully read LTFS index using DisablePartition fallback: {} bytes (contains XMLSchema)", xml_content.len());
            Ok(xml_content)
        } else {
            // 🔧 LTFSCopyGUI备选路径：FromSchemaText处理
            let processed_content = self.ltfscopygui_from_schema_text(xml_content)?;
            info!(
                "✅ Successfully processed LTFS schema text format: {} bytes",
                processed_content.len()
            );
            Ok(processed_content)
        }
    }



    /// 完全复刻LTFSCopyGUI的FromSchemaText方法 (Schema.vb:542-553)
    /// 精确对应VB.NET代码的字符串替换和处理逻辑
    fn ltfscopygui_from_schema_text(&self, mut s: String) -> Result<String> {
        debug!("🔧 Applying LTFSCopyGUI FromSchemaText transformations");

        // 记录原始数据信息用于调试
        let original_len = s.len();
        let non_null_count = s.chars().filter(|&c| c != '\0').count();
        debug!(
            "📊 Original data: {} bytes, {} non-null chars ({:.1}% content)",
            original_len,
            non_null_count,
            (non_null_count as f64 / original_len as f64) * 100.0
        );

        // 移除null字符（对应.NET字符串处理）
        s = s.replace('\0', "");

        // 检查处理后的数据
        debug!("📊 After null removal: {} bytes", s.len());
        if s.len() < 20 {
            debug!(
                "⚠️ Content sample: {:?}",
                s.chars().take(100).collect::<String>()
            );

            // LTFSCopyGUI兼容性：如果数据太短，可能是空白磁带或错误位置
            // 返回一个更具体的错误信息，但允许上层逻辑继续尝试其他策略
            return Err(RustLtfsError::ltfs_index(
                format!("Schema text too short after null removal: {} bytes (original: {} bytes, {:.1}% null)",
                       s.len(), original_len, ((original_len - s.len()) as f64 / original_len as f64) * 100.0)
            ));
        }

        // 🔧 修复：移除LTFSCopyGUI写入的非标准标签
        // LTFSCopyGUI在写入时会添加 <_directory> 和 <_file> 包裹标签
        // 这些标签不是标准LTFS格式，需要在读取时移除以兼容标准XML解析器
        s = s.replace("<_directory>", "");
        s = s.replace("</_directory>", "");
        s = s.replace("<_file>", "");
        s = s.replace("</_file>", "");
        s = s.replace("%25", "%");

        // 基础验证：确保包含必要的LTFS结构
        if !s.contains("ltfsindex") && !s.contains("directory") && !s.contains("file") {
            debug!(
                "⚠️ No LTFS structure found. Content preview: {:?}",
                s.chars().take(200).collect::<String>()
            );
            return Err(RustLtfsError::ltfs_index(format!(
                "No LTFS structure found in {} bytes of processed text",
                s.len()
            )));
        }

        debug!(
            "✅ LTFSCopyGUI FromSchemaText processing completed: {} bytes",
            s.len()
        );
        Ok(s)
    }

}
