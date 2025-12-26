use crate::error::{Result, RustLtfsError};
use super::super::PartitionStrategy;

use tracing::{debug, info, warn};
use chrono;
use crate::scsi::block_sizes;

// LtfsPartitionLabel 在 format_operations.rs 中定义
// 通过模块重新导出使用

/// TapeOperations读取操作实现
impl super::super::TapeOperations {
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

    /// Read LTFS index from tape (LTFSCopyGUI兼容方法)
    pub async fn read_index_from_tape(&mut self) -> Result<()> {
        info!("Starting LTFS index reading process");



        debug!("=== LTFSCopyGUI Compatible Index Reading Process ===");

        // Step 1 (最高优先级): LTFSCopyGUI兼容方法
        debug!("Step 1 (Highest Priority): LTFSCopyGUI compatible method");

        // 检测分区策略并决定读取顺序
        let extra_partition_count = self.get_extra_partition_count();

        if extra_partition_count > 0 {
            // 双分区磁带：使用专门的双分区读取逻辑（FileMark 3）
            debug!("Dual-partition detected, using FileMark 3 strategy");
            
            match self.try_read_index_dual_partition().await {
                Ok(xml_content) => {
                    if self.validate_and_process_index(&xml_content).await? {
                        debug!("✅ Step 1 succeeded - index read from dual-partition (FileMark 3)");
                        info!("Index loaded successfully ({} files)", self.index.as_ref().map(|i| self.count_files_in_directory(&i.root_directory)).unwrap_or(0));
                        return Ok(());
                    }
                }
                Err(e) => {
                    debug!("Dual-partition FileMark 3 strategy failed: {}", e);
                }
            }
        } else {
            // 单分区磁带：使用FM-1策略从partition 0读取索引
            debug!("Single-partition detected, using FM-1 strategy");

            match self.try_read_index_single_partition().await {
                Ok(xml_content) => {
                    if self.validate_and_process_index(&xml_content).await? {
                        debug!("✅ Step 1 succeeded - index read from single-partition (FM-1 strategy)");
                        info!("Index loaded successfully ({} files)", self.index.as_ref().map(|i| self.count_files_in_directory(&i.root_directory)).unwrap_or(0));
                        return Ok(());
                    }
                }
                Err(e) => {
                    debug!("Single-partition FM-1 strategy failed: {}", e);
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
                    // 尝试数据分区EOD策略（双分区专用函数）
                    match self.read_index_from_data_partition_eod().await {
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
                match self.search_index_copies_in_data_partition().await {
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
        // 🔧 FIX: 索引分区(P0)使用FileMark 3（与LTFSCopyGUI一致）
        // Reference: LTFSWriter.vb line 4549 - TapeUtils.Locate(driveHandle, 3UL, IndexPartition, TapeUtils.LocateDestType.FileMark)
        // 数据分区(P1)使用FM-1策略（最新索引在EOD之前）
        let target_filemark = if partition == 0 {
            // 索引分区：使用FileMark 3（LTFSCopyGUI兼容）
            info!("Index partition (P0): using FileMark 3 (LTFSCopyGUI compatible)");
            3
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

    pub async fn search_index_copies_in_data_partition(&mut self) -> Result<String> {
        info!("Starting index search from standard locations (LTFSCopyGUI method)");

        // 步骤1: 检测ExtraPartitionCount (对应LTFSCopyGUI的分区检测)
        let extra_partition_count = self.get_extra_partition_count();

        if extra_partition_count == 0 {
            // 🔧 单分区磁带策略
            debug!("🎯 Single partition tape detected (ExtraPartitionCount=0)");
            self.try_read_index_single_partition().await
        } else {
            // 🔧 多分区磁带策略
            debug!(
                "🎯 Multi-partition tape detected (ExtraPartitionCount={})",
                extra_partition_count
            );
            // 这里我们使用 read_index_from_data_partition_eod，因为这是多分区的数据区读取逻辑
            self.read_index_from_data_partition_eod().await
        }
    }









    /// 完全复刻LTFSCopyGUI的FromSchemaText方法 (Schema.vb:542-553)
    /// 精确对应VB.NET代码的字符串替换和处理逻辑
    pub(crate) fn ltfscopygui_from_schema_text(&self, mut s: String) -> Result<String> {
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
