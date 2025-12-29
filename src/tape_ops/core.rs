use super::LtfsPartitionLabel;
use super::{WriteOptions, WriteProgress};
use crate::error::{Result, RustLtfsError};
use crate::ltfs_index::LtfsIndex;
use tracing::{debug, info, warn};





/// 操作类型枚举
#[derive(Debug, Clone, Copy)]
pub enum OperationType {
    Space,  // 只需要设备初始化
    Write,  // 需要设备初始化 + 索引加载
    Read,   // 需要设备初始化 + 索引加载 + 内容显示
}

/// Tape operations - core functionality from LTFSCopyGUI
pub struct TapeOperations {
    pub(crate) device_path: String,

    pub(crate) index: Option<LtfsIndex>,
    pub(crate) schema: Option<LtfsIndex>,
    pub(crate) block_size: u32,
    pub(crate) scsi: crate::scsi::ScsiInterface,
    pub(crate) partition_label: Option<LtfsPartitionLabel>, // 对应LTFSCopyGUI的plabel

    pub(crate) write_progress: WriteProgress,
    pub(crate) write_options: WriteOptions,
    pub(crate) modified: bool,   // 对应LTFSCopyGUI的Modified标志
    pub(crate) extra_partition_count: Option<u8>, // 对应LTFSCopyGUI的ExtraPartitionCount
    pub(crate) max_extra_partition_allowed: u8, // 对应LTFSCopyGUI的MaxExtraPartitionAllowed
}

impl TapeOperations {
    /// Create new tape operations instance
    pub fn new(device: &str) -> Self {
        Self {
            device_path: device.to_string(),

            index: None,
            schema: None,
            block_size: crate::scsi::block_sizes::LTO_BLOCK_SIZE, // Default block size (64KB)
            scsi: crate::scsi::ScsiInterface::new(),
            partition_label: None, // 初始化为None，稍后读取

            write_progress: WriteProgress::default(),
            write_options: WriteOptions::default(),
            modified: false,

            extra_partition_count: None, // Will be detected during initialization
            max_extra_partition_allowed: 1, // LTO standard maximum
        }
    }

















    /// Get current write progress
    pub fn get_write_progress(&self) -> &WriteProgress {
        &self.write_progress
    }

    /// Set write options
    pub fn set_write_options(&mut self, options: WriteOptions) {
        self.block_size = options.block_size;
        self.write_options = options;
    }







    /// 初始化分区检测 (精确对应LTFSCopyGUI的初始化逻辑)
    /// 检测ExtraPartitionCount并设置分区策略 - 修复版本：直接使用已打开的SCSI设备
    pub async fn initialize_partition_detection(&mut self) -> Result<()> {
        debug!(
            "Initializing partition detection (LTFSCopyGUI compatible) - using opened SCSI device"
        );



        // 直接使用已打开的self.scsi进行MODE SENSE检测 (对应LTFSCopyGUI的MODE SENSE检测)
        // 🔧 FIX: 使用 Page 0x11 (Medium Partition) 而不是 Page 0x1D (Medium Configuration)
        // Page 0x1D 的 byte[3] 是 Block Descriptor Length，不是分区数！
        info!("🔧 Using MODE SENSE Page 0x11 for partition detection");

        match self.scsi.mode_sense_partition_page_0x11() {
            Ok(mode_data) => {
                // 记录原始数据以便调试
                info!(
                    "📊 MODE SENSE 0x11 returned {} bytes: {:02X?}",
                    mode_data.len(),
                    &mode_data[..std::cmp::min(16, mode_data.len())]
                );
                
                // LTFSCopyGUI逻辑: If PModeData.Length >= 4 Then ExtraPartitionCount = PModeData(3)
                // Page 0x11 byte[3] = Additional Partition Defined (分区数)
                if mode_data.len() >= 4 {
                    let detected_count = mode_data[3];
                    info!(
                        "✅ ExtraPartitionCount detected from MODE SENSE 0x11: {}",
                        detected_count
                    );

                    // 应用LTFSCopyGUI的验证逻辑: Math.Min(1, value)
                    let validated_count = std::cmp::min(1, detected_count);
                    let final_count =
                        std::cmp::min(validated_count, self.max_extra_partition_allowed);

                    if final_count != detected_count {
                        debug!(
                            "ExtraPartitionCount limited from {} to {} (Math.Min validation)",
                            detected_count, final_count
                        );
                    }

                    self.extra_partition_count = Some(final_count);
                    info!(
                        "✅ ExtraPartitionCount initialized: {} (detected: {}, validated: {})",
                        final_count, detected_count, final_count
                    );

                    // 设置modified状态 (对应LTFSCopyGUI的Modified = ExtraPartitionCount > 0)
                    self.modified = final_count > 0;
                } else {
                    warn!(
                        "MODE SENSE data too short ({} bytes), defaulting to single partition",
                        mode_data.len()
                    );
                    self.extra_partition_count = Some(0);
                    self.modified = false;
                }
            }
            Err(e) => {
                warn!(
                    "MODE SENSE 0x11 failed: {}, defaulting to single partition",
                    e
                );
                self.extra_partition_count = Some(0);
                self.modified = false;
            }
        }

        Ok(())
    }

    /// 获取当前ExtraPartitionCount
    pub fn get_extra_partition_count(&self) -> u8 {
        self.extra_partition_count.unwrap_or(0)
    }

    /// 获取目标分区号 (正确的LTFS分区映射逻辑)
    /// 修复关键Bug：之前的Math.Min逻辑导致数据写入错误分区
    pub fn get_target_partition(&self, logical_partition: u8) -> u8 {
        let extra_partition_count = self.get_extra_partition_count();

        debug!(
            "Computing target partition: logical={}, ExtraPartitionCount={}",
            logical_partition, extra_partition_count
        );

        match extra_partition_count {
            0 => {
                // 单分区磁带：所有数据和索引都在分区0
                debug!("Single-partition tape: all data goes to partition 0");
                0
            }
            1 => {
                // 双分区磁带：分区0=索引分区，分区1=数据分区
                match logical_partition {
                    0 => {
                        debug!(
                            "Dual-partition tape: index data goes to partition 0 (index partition)"
                        );
                        0 // 索引分区
                    }
                    1 => {
                        debug!(
                            "Dual-partition tape: file data goes to partition 1 (data partition)"
                        );
                        1 // 数据分区
                    }
                    _ => {
                        warn!(
                            "Unexpected logical partition {}, defaulting to data partition",
                            logical_partition
                        );
                        1
                    }
                }
            }
            _ => {
                warn!(
                    "Unexpected ExtraPartitionCount {}, using dual-partition logic",
                    extra_partition_count
                );
                if logical_partition == 0 {
                    0
                } else {
                    1
                }
            }
        }
    }




    /// Wait for device ready using TestUnitReady retry logic (对应LTFSCopyGUI的TestUnitReady重试逻辑)
    pub async fn wait_for_device_ready(&self) -> Result<()> {
        debug!("Starting TestUnitReady retry logic");

        let max_retries = 5; // 对应LTFSCopyGUI的5次重试
        let retry_delay_ms = 200; // 对应LTFSCopyGUI的200ms延迟

        for retry_count in (1..=max_retries).rev() {
            debug!(
                "TestUnitReady attempt {} (remaining: {})",
                max_retries - retry_count + 1,
                retry_count
            );

            // 执行SCSI Test Unit Ready命令
            match self.scsi.test_unit_ready() {
                Ok(sense_data) => {
                    if sense_data.is_empty() {
                        // 无sense数据表示设备就绪
                        debug!("✅ Device is ready (TestUnitReady successful, no sense data)");
                        return Ok(());
                    } else {
                        // 有sense数据，需要分析
                        let sense_info = self.scsi.parse_sense_data(&sense_data);
                        debug!("TestUnitReady returned sense data: {}", sense_info);

                        // 检查是否为"设备准备就绪"的状态
                        if sense_info.contains("No additional sense information") ||
                           sense_info.contains("ready") ||  // 改为小写匹配
                           sense_info.contains("Ready") ||
                           sense_info.contains("Good") ||
                           sense_info == "Device ready"
                        {
                            // 精确匹配SCSI返回的"Device ready"
                            debug!(
                                "✅ Device is ready (TestUnitReady with ready sense: {})",
                                sense_info
                            );
                            return Ok(());
                        }

                        // 检查是否为可重试的错误
                        if sense_info.contains("Not ready")
                            || sense_info.contains("Unit attention")
                            || sense_info.contains("Medium may have changed")
                        {
                            if retry_count > 1 {
                                debug!("⏳ Device not ready ({}), retrying in {}ms (attempts remaining: {})",
                                     sense_info, retry_delay_ms, retry_count - 1);
                                tokio::time::sleep(tokio::time::Duration::from_millis(
                                    retry_delay_ms,
                                ))
                                .await;
                                continue;
                            } else {
                                warn!(
                                    "❌ Device not ready after {} attempts: {}",
                                    max_retries, sense_info
                                );
                                return Err(RustLtfsError::scsi(format!(
                                    "Device not ready after {} retries: {}",
                                    max_retries, sense_info
                                )));
                            }
                        } else {
                            // 非可重试错误，立即返回
                            return Err(RustLtfsError::scsi(format!(
                                "TestUnitReady failed: {}",
                                sense_info
                            )));
                        }
                    }
                }
                Err(e) => {
                    if retry_count > 1 {
                        warn!("🔄 TestUnitReady SCSI command failed: {}, retrying in {}ms (attempts remaining: {})",
                             e, retry_delay_ms, retry_count - 1);
                        tokio::time::sleep(tokio::time::Duration::from_millis(retry_delay_ms))
                            .await;
                        continue;
                    } else {
                        return Err(RustLtfsError::scsi(format!(
                            "TestUnitReady failed after {} retries: {}",
                            max_retries, e
                        )));
                    }
                }
            }
        }

        Ok(())
    }

    /// Initialize tape operations
    pub async fn initialize(&mut self, operation_type: Option<OperationType>) -> Result<()> {
        let op_type = operation_type.unwrap_or(OperationType::Write); // 默认为写入模式



        // 设备初始化（所有操作都需要）
        self.scsi.open_device(&self.device_path)?;
        self.wait_for_device_ready().await?;

        match self.scsi.check_media_status()? {
            crate::scsi::MediaType::NoTape => {
                return Err(RustLtfsError::tape_device("No tape loaded".to_string()));
            }
            crate::scsi::MediaType::Unknown(_) => {
                // Continue with unknown media
            }
            _ => {
                // Media detected, continue
            }
        }

        self.initialize_partition_detection().await?;

        match op_type {
            OperationType::Space => {
                debug!("Device initialization completed");
                return Ok(());
            }
            OperationType::Write => {
                debug!("Device initialization completed");
                
                // 尝试加载现有的LTFS索引
                match self.read_index_from_tape().await {
                    Ok(()) => {
                        let file_count = self
                            .index
                            .as_ref()
                            .map(|idx| idx.root_directory.contents.files.len())
                            .unwrap_or(0);
                        debug!("Index loaded successfully ({} files)", file_count);
                    }
                    Err(_) => {
                        info!("Will create new index");
                    }
                }
            }
            OperationType::Read => {
                debug!("Device initialization completed");
                
                // 读取操作必须成功加载索引
                match self.read_index_from_tape().await {
                    Ok(()) => {
                        debug!("Index loaded successfully");
                        
                        // 显示索引内容概览
                        if let Some(stats) = self.get_index_statistics() {
                            info!("Tape contents: {} files, {} directories", stats.total_files, stats.total_directories);
                        }
                    }
                    Err(e) => {
                        return Err(RustLtfsError::ltfs_index(format!("Index reading failed: {}", e)));
                    }
                }
            }
        }

        self.partition_label = Some(LtfsPartitionLabel::default());
        Ok(())
    }

    /// 保存索引到文件
    pub async fn save_index_to_file(&self, file_path: &std::path::Path) -> Result<()> {
        debug!("Saving LTFS index to file: {:?}", file_path);

        if let Some(ref index) = self.index {
            let xml_content = index.to_xml()?;
            std::fs::write(file_path, xml_content)?;
            debug!("Index saved successfully to {:?}", file_path);
            Ok(())
        } else {
            Err(RustLtfsError::ltfs_index(
                "No index loaded to save".to_string(),
            ))
        }
    }

    /// 获取索引统计信息
    pub fn get_index_statistics(&self) -> Option<IndexStatistics> {
        if let Some(ref index) = self.index {
            let mut stats = IndexStatistics::default();
            stats.total_files = count_files_in_directory(&index.root_directory);
            stats.total_directories = count_directories_in_directory(&index.root_directory);
            stats.total_size = calculate_total_size(&index.root_directory);
            stats.volume_uuid = index.volumeuuid.clone();
            stats.generation_number = index.generationnumber;
            stats.update_time = index.updatetime.clone();
            Some(stats)
        } else {
            None
        }
    }

    /// 打印目录树
    pub fn print_directory_tree(&self) {
        if let Some(ref index) = self.index {
            println!("LTFS Directory Tree:");
            print_directory_recursive(&index.root_directory, 0);
        } else {
            println!("No index loaded");
        }
    }

    /// 列出指定目录的内容
    pub fn list_directory_contents(&self, path: &str) -> Result<()> {
        if let Some(ref index) = self.index {
            if path.is_empty() || path == "/" {
                // 列出根目录
                self.print_directory_contents(&index.root_directory, 0);
            } else {
                // 查找指定目录
                let target_dir = self.find_directory_by_path(&index.root_directory, path);
                match target_dir {
                    Some(dir) => {
                        println!("📁 Contents of: {}", path);
                        self.print_directory_contents(dir, 0);
                    }
                    None => {
                        println!("❌ Directory not found: {}", path);
                        return Err(RustLtfsError::ltfs_index(format!("Directory not found: {}", path)));
                    }
                }
            }
        } else {
            return Err(RustLtfsError::ltfs_index("No index loaded".to_string()));
        }
        Ok(())
    }

    /// 打印目录内容（不递归）
    fn print_directory_contents(&self, dir: &crate::ltfs_index::Directory, depth: usize) {
        let indent = "  ".repeat(depth);
        
        // 打印文件
        for file in &dir.contents.files {
            println!("{}📄 {} ({} bytes)", indent, file.name, file.length);
        }
        
        // 打印子目录
        for subdir in &dir.contents.directories {
            println!("{}📁 {}/", indent, subdir.name);
        }
    }

    /// 根据路径查找目录
    fn find_directory_by_path<'a>(&self, root: &'a crate::ltfs_index::Directory, path: &str) -> Option<&'a crate::ltfs_index::Directory> {
        // 标准化路径
        let path = path.trim_start_matches('/').trim_end_matches('/');
        if path.is_empty() {
            return Some(root);
        }

        let path_parts: Vec<&str> = path.split('/').collect();
        self.find_directory_recursive(root, &path_parts, 0)
    }

    /// 递归查找目录
    fn find_directory_recursive<'a>(&self, current_dir: &'a crate::ltfs_index::Directory, path_parts: &[&str], index: usize) -> Option<&'a crate::ltfs_index::Directory> {
        if index >= path_parts.len() {
            return Some(current_dir);
        }

        let target_name = path_parts[index];
        for subdir in &current_dir.contents.directories {
            if subdir.name == target_name {
                return self.find_directory_recursive(subdir, path_parts, index + 1);
            }
        }

        None
    }



    /// 刷新磁带容量信息（精确对应LTFSCopyGUI RefreshCapacity）
    pub async fn refresh_capacity(&mut self) -> Result<super::capacity_manager::TapeCapacityInfo> {
        info!("Refreshing tape capacity information");

        let mut capacity_info = super::capacity_manager::TapeCapacityInfo {
            p0_remaining: 0,
            p0_maximum: 0,
            p1_remaining: 0,
            p1_maximum: 0,
        };

        // 直接使用self.scsi来读取容量信息
        info!("Reading tape capacity log page (0x31)");
        let capacity_log_data = match self.scsi.log_sense(0x31, 1) {
            Ok(data) => {
                info!("📊 Capacity log data length: {} bytes", data.len());
                if data.len() > 0 {
                    info!("📊 Capacity log data preview: {:02X?}", &data[..std::cmp::min(32, data.len())]);
                }
                data
            },
            Err(e) => {
                warn!("Failed to read capacity log page: {}", e);
                return Ok(capacity_info);
            }
        };

        // 解析容量信息
        let capacity_parser = super::capacity_manager::CapacityPageParser::new(capacity_log_data);
        
        capacity_info.p0_remaining = capacity_parser.get_remaining_capacity(0).unwrap_or(0);
        capacity_info.p0_maximum = capacity_parser.get_maximum_capacity(0).unwrap_or(0);

        let extra_partition_count = self.get_extra_partition_count();
        if extra_partition_count > 0 {
            capacity_info.p1_remaining = capacity_parser.get_remaining_capacity(1).unwrap_or(0);
            capacity_info.p1_maximum = capacity_parser.get_maximum_capacity(1).unwrap_or(0);
        }

        info!("Capacity refresh completed: P0({:.2}/{:.2}) GB, P1({:.2}/{:.2}) GB", 
              capacity_info.p0_remaining as f64 / 1024.0,
              capacity_info.p0_maximum as f64 / 1024.0,
              capacity_info.p1_remaining as f64 / 1024.0, 
              capacity_info.p1_maximum as f64 / 1024.0);

        Ok(capacity_info)
    }



    /// 获取磁带容量信息（简化版本，用于向后兼容）
    pub async fn get_tape_capacity_info(&mut self) -> Result<TapeSpaceInfo> {
        let capacity_info = self.refresh_capacity().await?;

        // 根据ExtraPartitionCount决定使用哪个分区的容量
        let (used_space, total_capacity) = if self.get_extra_partition_count() > 0 {
            // 多分区磁带：显示P0+P1的总容量（剩余容量）
            let p0_remaining_bytes = capacity_info.p0_remaining * 1024; // KB转字节
            let p1_remaining_bytes = capacity_info.p1_remaining * 1024; // KB转字节
            let total_remaining = p0_remaining_bytes + p1_remaining_bytes;
            
            // 计算已使用空间（如果有最大容量数据）
            let used_space = if capacity_info.p0_maximum > 0 && capacity_info.p1_maximum > 0 {
                let p0_used = capacity_info.p0_maximum.saturating_sub(capacity_info.p0_remaining);
                let p1_used = capacity_info.p1_maximum.saturating_sub(capacity_info.p1_remaining);
                (p0_used + p1_used) * 1024 // KB转字节
            } else {
                // 如果没有最大容量数据，假设已使用很少
                0
            };
            
            (used_space, total_remaining)
        } else {
            // 单分区磁带：使用P0容量
            let used_p0 = capacity_info
                .p0_maximum
                .saturating_sub(capacity_info.p0_remaining);
            ((used_p0 * 1024), (capacity_info.p0_maximum * 1024)) // KB转换为字节
        };

        Ok(TapeSpaceInfo {
            total_capacity,
            used_space,
            available_space: total_capacity.saturating_sub(used_space),
        })
    }
}

/// 索引统计信息
#[derive(Debug, Default)]
pub struct IndexStatistics {
    pub total_files: u64,
    pub total_directories: u64,
    pub total_size: u64,
    pub volume_uuid: String,
    pub generation_number: u64,
    pub update_time: String,
}

/// 磁带空间信息
#[derive(Debug)]
pub struct TapeSpaceInfo {
    pub total_capacity: u64,
    pub used_space: u64,
    pub available_space: u64,
}


// 辅助函数
fn count_files_in_directory(dir: &crate::ltfs_index::Directory) -> u64 {
    let mut count = dir.contents.files.len() as u64;
    for subdir in &dir.contents.directories {
        count += count_files_in_directory(subdir);
    }
    count
}

fn count_directories_in_directory(dir: &crate::ltfs_index::Directory) -> u64 {
    let mut count = dir.contents.directories.len() as u64;
    for subdir in &dir.contents.directories {
        count += count_directories_in_directory(subdir);
    }
    count
}

fn calculate_total_size(dir: &crate::ltfs_index::Directory) -> u64 {
    let mut size = 0;
    // 计算文件大小
    for file in &dir.contents.files {
        size += file.length;
    }
    // 递归计算子目录大小
    for subdir in &dir.contents.directories {
        size += calculate_total_size(subdir);
    }
    size
}

fn print_directory_recursive(dir: &crate::ltfs_index::Directory, depth: usize) {
    let indent = "  ".repeat(depth);
    // 打印文件
    for file in &dir.contents.files {
        println!("{}📄 {} ({} bytes)", indent, file.name, file.length);
    }
    // 打印并递归子目录
    for subdir in &dir.contents.directories {
        println!("{}📁 {}/", indent, subdir.name);
        print_directory_recursive(subdir, depth + 1);
    }
}
