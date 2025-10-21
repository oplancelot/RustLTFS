use crate::error::{Result, RustLtfsError};
use crate::ltfs_index::LtfsIndex;
use crate::scsi::{MediaType, ScsiInterface, block_sizes};
use std::sync::Arc;
use std::io::Write;
use tracing::{debug, info, warn};
use chrono;

/// LTFS分区标签结构 (对应LTFSCopyGUI的ltfslabel)
#[derive(Debug, Clone)]
pub struct LtfsPartitionLabel {
    pub volume_uuid: String,
    pub blocksize: u32,
    pub compression: bool,
    pub index_partition: u8, // 通常是0 (partition a)
    pub data_partition: u8,  // 通常是1 (partition b)
    pub format_time: String,
}

impl Default for LtfsPartitionLabel {
    fn default() -> Self {
        Self {
            volume_uuid: String::new(),
            blocksize: crate::scsi::block_sizes::LTO_BLOCK_SIZE, // 默认64KB
            compression: false,
            index_partition: 0,
            data_partition: 1,
            format_time: String::new(),
        }
    }
}

/// Partition reading strategy (对应LTFSCopyGUI的ExtraPartitionCount处理策略)
#[derive(Debug, Clone, PartialEq)]
pub enum PartitionStrategy {
    /// 标准多分区磁带：索引在partition A，数据在partition B
    StandardMultiPartition,
    /// 单分区磁带回退策略：需要从数据分区读取索引副本
    SinglePartitionFallback,
    /// 从数据分区读取索引：当索引位置指向partition B时
    IndexFromDataPartition,
}

/// Partition size information (对应LTFSCopyGUI的分区大小检测)
#[derive(Debug, Clone)]
pub struct PartitionInfo {
    pub partition_0_size: u64, // p0分区大小（索引分区）
    pub partition_1_size: u64, // p1分区大小（数据分区）
    pub has_multi_partition: bool,
}

/// Index location information
#[derive(Debug, Clone)]
pub struct IndexLocation {
    pub partition: String,
    pub start_block: u64,
}

/// Partition Manager - 专门处理磁带分区管理的结构体
pub struct PartitionManager {
    scsi: Arc<ScsiInterface>,
    offline_mode: bool,
    partition_label: Option<LtfsPartitionLabel>,
}

impl PartitionManager {
    /// 创建新的分区管理器实例
    pub fn new(scsi: Arc<ScsiInterface>, offline_mode: bool) -> Self {
        Self {
            scsi,
            offline_mode,
            partition_label: None,
        }
    }

    /// 检测ExtraPartitionCount (精确对应LTFSCopyGUI逻辑)
    /// 使用MODE SENSE 0x11命令从磁带直接读取分区配置
    pub async fn detect_extra_partition_count(&self) -> Result<u8> {
        info!("Detecting ExtraPartitionCount using MODE SENSE 0x11 (LTFSCopyGUI exact logic)");

        if self.offline_mode {
            info!("Offline mode: assuming dual-partition (ExtraPartitionCount = 1)");
            return Ok(1);
        }

        // 执行MODE SENSE 0x11命令 (对应LTFSCopyGUI的ModeSense(driveHandle, &H11))
        match self.scsi.mode_sense_partition_info() {
            Ok(mode_data) => {
                // 精确匹配LTFSCopyGUI逻辑: If PModeData.Length >= 4 Then ExtraPartitionCount = PModeData(3)
                if mode_data.len() >= 4 {
                    let extra_partition_count = mode_data[3];
                    info!(
                        "✅ ExtraPartitionCount detected from MODE SENSE: {}",
                        extra_partition_count
                    );
                    
                    // 应用LTFSCopyGUI的验证逻辑: Math.Min(1, value)
                    let validated_count = std::cmp::min(1, extra_partition_count);
                    
                    if validated_count != extra_partition_count {
                        warn!(
                            "ExtraPartitionCount limited from {} to {} (Math.Min validation)",
                            extra_partition_count, validated_count
                        );
                    }
                    
                    Ok(validated_count)
                } else {
                    warn!(
                        "MODE SENSE data too short ({} bytes), defaulting to single partition",
                        mode_data.len()
                    );
                    Ok(0)
                }
            }
            Err(e) => {
                warn!(
                    "MODE SENSE 0x11 failed: {}, defaulting to single partition",
                    e
                );
                Ok(0)
            }
        }
    }

    /// 根据ExtraPartitionCount确定分区策略 (对应LTFSCopyGUI的策略选择)
    pub async fn determine_partition_strategy(&self, extra_partition_count: u8) -> PartitionStrategy {
        info!(
            "Determining partition strategy based on ExtraPartitionCount = {}",
            extra_partition_count
        );

        match extra_partition_count {
            0 => {
                info!("Single-partition strategy (ExtraPartitionCount = 0)");
                PartitionStrategy::SinglePartitionFallback
            }
            1 => {
                info!("Dual-partition strategy (ExtraPartitionCount = 1)");
                PartitionStrategy::StandardMultiPartition
            }
            _ => {
                warn!(
                    "Unexpected ExtraPartitionCount value: {}, using dual-partition strategy",
                    extra_partition_count
                );
                PartitionStrategy::StandardMultiPartition
            }
        }
    }

    /// 分区号映射 (修复版本：正确的LTFS分区布局)
    /// 之前的Math.Min逻辑导致数据写入错误分区，现已修复
    pub fn map_partition_number(&self, logical_partition: u8, extra_partition_count: u8) -> u8 {
        debug!("Computing partition mapping: logical={}, ExtraPartitionCount={}", 
               logical_partition, extra_partition_count);
        
        let physical_partition = match extra_partition_count {
            0 => {
                // 单分区磁带：所有数据和索引都在分区0
                debug!("Single-partition tape: mapping logical {} to physical 0", logical_partition);
                0
            }
            1 => {
                // 双分区磁带：分区0=索引分区，分区1=数据分区
                match logical_partition {
                    0 => {
                        debug!("Dual-partition tape: index partition (logical 0 -> physical 0)");
                        0  // 索引分区
                    }
                    1 => {
                        debug!("Dual-partition tape: data partition (logical 1 -> physical 1)");
                        1  // 数据分区
                    }
                    _ => {
                        warn!("Unexpected logical partition {}, defaulting to data partition", logical_partition);
                        1
                    }
                }
            }
            _ => {
                warn!("Unexpected ExtraPartitionCount {}, using dual-partition logic", extra_partition_count);
                if logical_partition == 0 { 0 } else { 1 }
            }
        };
        
        debug!(
            "Partition mapping result: logical={} -> physical={} (ExtraPartitionCount={})",
            logical_partition, physical_partition, extra_partition_count
        );
        
        physical_partition
    }

    /// 验证和标准化ExtraPartitionCount值 (对应LTFSCopyGUI的双重Math.Min验证)
    /// 对应: Math.Min(1, value) 和 Math.Min(value, MaxExtraPartitionAllowed)
    pub fn validate_extra_partition_count(&self, value: u8, max_allowed: u8) -> u8 {
        // 第一层验证: Math.Min(1, value)
        let step1 = std::cmp::min(1, value);
        
        // 第二层验证: Math.Min(step1, MaxExtraPartitionAllowed)
        let final_value = std::cmp::min(step1, max_allowed);
        
        if final_value != value {
            warn!(
                "ExtraPartitionCount normalized: {} -> {} (limits: max=1, max_allowed={})",
                value, final_value, max_allowed
            );
        }
        
        final_value
    }

    /// 获取目标分区号用于定位操作 (对应LTFSCopyGUI的分区选择逻辑)
    /// 对应: Math.Min(ExtraPartitionCount, IndexPartition) 和 Math.Min(ExtraPartitionCount, ext.partition)
    pub fn get_target_partition(&self, logical_partition: u8, extra_partition_count: u8) -> u8 {
        self.map_partition_number(logical_partition, extra_partition_count)
    }

    /// 检查磁带多分区支持 (对应LTFSCopyGUI的ExtraPartitionCount检测)
    /// 使用SCSI MODE SENSE命令来准确检测分区结构，而不是依赖数据读取测试
    async fn check_multi_partition_support(&self) -> Result<bool> {
        debug!("Checking multi-partition support using SCSI MODE SENSE (ExtraPartitionCount detection)");

        if self.offline_mode {
            debug!("Offline mode: assuming dual-partition support");
            return Ok(true);
        }

        // 使用我们实现的SCSI MODE SENSE命令来准确检测分区
        // 这比尝试读取数据更可靠，因为分区可能存在但为空
        match self.scsi.mode_sense_partition_info() {
            Ok(mode_data) => {
                debug!("MODE SENSE successful, parsing partition information");

                match self.scsi.parse_partition_info(&mode_data) {
                    Ok((p0_size, p1_size)) => {
                        let has_multi_partition = p1_size > 0;
                        if has_multi_partition {
                            info!(
                                "✅ Multi-partition detected via MODE SENSE: p0={}GB, p1={}GB",
                                p0_size / 1_000_000_000,
                                p1_size / 1_000_000_000
                            );
                        } else {
                            info!(
                                "📋 Single partition detected via MODE SENSE: total={}GB",
                                p0_size / 1_000_000_000
                            );
                        }
                        Ok(has_multi_partition)
                    }
                    Err(e) => {
                        debug!(
                            "MODE SENSE data parsing failed: {}, falling back to position test",
                            e
                        );
                        self.fallback_partition_detection().await
                    }
                }
            }
            Err(e) => {
                debug!(
                    "MODE SENSE command failed: {}, falling back to position test",
                    e
                );
                self.fallback_partition_detection().await
            }
        }
    }

    /// 备用分区检测方法 - 当MODE SENSE不可用时使用定位测试
    async fn fallback_partition_detection(&self) -> Result<bool> {
        info!("Using fallback method: testing partition access");

        // 尝试定位到partition 1来测试多分区支持
        match self.scsi.locate_block(1, 0) {
            Ok(()) => {
                debug!("Successfully positioned to partition 1 - multi-partition supported");

                // 不依赖数据读取，仅测试定位能力
                info!("✅ Multi-partition support confirmed (can position to partition 1)");

                // 返回partition 0以继续正常流程
                if let Err(e) = self.scsi.locate_block(0, 0) {
                    warn!("Warning: Failed to return to partition 0: {}", e);
                }

                Ok(true)
            }
            Err(e) => {
                debug!(
                    "Cannot position to partition 1: {} - single partition tape",
                    e
                );
                Ok(false)
            }
        }
    }

    /// 检查volume label中的索引位置 (对应LTFSCopyGUI的索引位置检测)
    async fn check_index_location_from_volume_label(&self) -> Result<IndexLocation> {
        debug!("Checking index location from volume label");

        // 确保在partition A的开始位置
        self.scsi.locate_block(0, 0)?;

        let mut buffer = vec![0u8; crate::scsi::block_sizes::LTO_BLOCK_SIZE as usize];
        self.scsi.read_blocks(1, &mut buffer)?;

        // 解析volume label中的索引位置信息
        self.parse_index_locations_from_volume_label(&buffer)
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

    /// 检测分区大小 (对应LTFSCopyGUI的分区大小检测逻辑)
    pub async fn detect_partition_sizes(&self) -> Result<PartitionInfo> {
        info!("Detecting partition sizes (LTFSCopyGUI compatible)");

        // 首先检查是否有多分区支持
        let has_multi_partition = self.check_multi_partition_support().await.unwrap_or(false);

        if !has_multi_partition {
            info!("Single partition detected, using full capacity");
            let total_capacity = self.estimate_tape_capacity_bytes();
            return Ok(PartitionInfo {
                partition_0_size: total_capacity,
                partition_1_size: 0,
                has_multi_partition: false,
            });
        }

        info!("Multi-partition detected, reading partition sizes");

        // 对于多分区磁带，尝试从不同位置获取分区信息
        // 对应LTFSCopyGUI中的分区大小检测逻辑

        // 方法1：从媒体类型估算标准分区大小
        let (p0_size, p1_size) = self.estimate_standard_partition_sizes().await;

        // 方法2：尝试从磁带读取实际分区信息（如果支持的话）
        match self.read_partition_info_from_tape().await {
            Ok((actual_p0, actual_p1)) => {
                info!(
                    "✅ Successfully read actual partition sizes from tape: p0={}GB, p1={}GB",
                    actual_p0 / 1_000_000_000,
                    actual_p1 / 1_000_000_000
                );
                Ok(PartitionInfo {
                    partition_0_size: actual_p0,
                    partition_1_size: actual_p1,
                    has_multi_partition: true,
                })
            }
            Err(e) => {
                debug!(
                    "Failed to read actual partition info: {}, using estimates",
                    e
                );
                info!(
                    "📊 Using estimated partition sizes: p0={}GB, p1={}GB",
                    p0_size / 1_000_000_000,
                    p1_size / 1_000_000_000
                );
                Ok(PartitionInfo {
                    partition_0_size: p0_size,
                    partition_1_size: p1_size,
                    has_multi_partition: true,
                })
            }
        }
    }

    /// 估算标准分区大小 (基于LTFSCopyGUI的mkltfs P0Size/P1Size逻辑)
    async fn estimate_standard_partition_sizes(&self) -> (u64, u64) {
        let total_capacity = self.estimate_tape_capacity_bytes();

        // 基于LTFSCopyGUI Resources.Designer.vb中的分区计算逻辑
        // P0Size: 分区0大小，默认为1GB，但实际应用中常设置为更大值
        // P1Size: 分区1大小，默认为65535（表示取剩余空间）

        match self.scsi.check_media_status() {
            Ok(MediaType::Lto7Rw) | Ok(MediaType::Lto7Worm) | Ok(MediaType::Lto7Ro) => {
                // LTO-7: 基于实际观察到的分区配置
                // p0=99.78GB, p1=5388.34GB，说明索引分区约占1.8%
                let index_partition_gb = 100; // 约100GB索引分区
                let p0_size = (index_partition_gb * 1_000_000_000) as u64;
                let p1_size = total_capacity.saturating_sub(p0_size);

                debug!(
                    "LTO-7 partition estimation: p0={}GB, p1={}GB",
                    p0_size / 1_000_000_000,
                    p1_size / 1_000_000_000
                );

                (p0_size, p1_size)
            }
            Ok(MediaType::Lto8Rw) | Ok(MediaType::Lto8Worm) | Ok(MediaType::Lto8Ro) => {
                // LTO-8: 按照相似比例估算
                let index_partition_gb = 200; // 约200GB索引分区（约1.7%）
                let p0_size = (index_partition_gb * 1_000_000_000) as u64;
                let p1_size = total_capacity.saturating_sub(p0_size);

                debug!(
                    "LTO-8 partition estimation: p0={}GB, p1={}GB",
                    p0_size / 1_000_000_000,
                    p1_size / 1_000_000_000
                );

                (p0_size, p1_size)
            }
            _ => {
                // 通用逻辑：索引分区约占1.8-2%，参考实际LTFSCopyGUI行为
                // 不是简单的固定1GB，而是基于磁带容量的比例
                let index_ratio = 0.018; // 1.8%，基于实际观察
                let min_index_size = 1_000_000_000u64; // 最小1GB
                let max_index_size = 500_000_000_000u64; // 最大500GB

                let calculated_index_size = (total_capacity as f64 * index_ratio) as u64;
                let p0_size = calculated_index_size.clamp(min_index_size, max_index_size);
                let p1_size = total_capacity.saturating_sub(p0_size);

                debug!(
                    "Generic partition estimation: p0={}GB, p1={}GB ({}% index ratio)",
                    p0_size / 1_000_000_000,
                    p1_size / 1_000_000_000,
                    index_ratio * 100.0
                );

                (p0_size, p1_size)
            }
        }
    }

    /// 从磁带读取实际分区信息 (对应LTFSCopyGUI的分区检测逻辑)
    async fn read_partition_info_from_tape(&self) -> Result<(u64, u64)> {
        info!("🔍 Reading actual partition information from tape using SCSI commands");

        // 首先尝试MODE SENSE命令读取分区表
        match self.scsi.mode_sense_partition_info() {
            Ok(mode_sense_data) => {
                debug!("MODE SENSE command successful, parsing partition data");

                // 解析MODE SENSE返回的分区信息
                match self.scsi.parse_partition_info(&mode_sense_data) {
                    Ok((p0_size, p1_size)) => {
                        info!("✅ Successfully parsed partition sizes from MODE SENSE:");
                        info!(
                            "   - p0 (index): {}GB ({} bytes)",
                            p0_size / 1_000_000_000,
                            p0_size
                        );
                        info!(
                            "   - p1 (data):  {}GB ({} bytes)",
                            p1_size / 1_000_000_000,
                            p1_size
                        );
                        return Ok((p0_size, p1_size));
                    }
                    Err(e) => {
                        debug!("MODE SENSE data parsing failed: {}", e);
                        // 继续尝试其他方法
                    }
                }
            }
            Err(e) => {
                debug!("MODE SENSE command failed: {}", e);
                // 继续尝试其他方法
            }
        }

        // 如果MODE SENSE失败，尝试READ POSITION获取当前位置信息
        debug!("Trying READ POSITION as fallback");
        match self.scsi.read_position_raw() {
            Ok(position_data) => {
                debug!("READ POSITION command successful");

                // READ POSITION主要用于获取当前位置，不直接提供分区大小
                // 但可以确认分区存在性
                if position_data.len() >= 32 {
                    let current_partition = position_data[1];
                    debug!(
                        "Current partition from READ POSITION: {}",
                        current_partition
                    );

                    // 如果能读取到分区信息，说明是多分区磁带
                    // 但READ POSITION不提供分区大小，需要使用其他方法
                    debug!("Confirmed multi-partition tape, but READ POSITION doesn't provide partition sizes");
                }

                // READ POSITION无法提供分区大小信息，使用估算值
                return Err(RustLtfsError::scsi(
                    "READ POSITION doesn't provide partition size information".to_string(),
                ));
            }
            Err(e) => {
                debug!("READ POSITION command also failed: {}", e);
            }
        }

        // 所有SCSI命令都失败，返回错误让调用者使用估算值
        Err(RustLtfsError::scsi(
            "All SCSI partition detection methods failed, will use estimated values".to_string(),
        ))
    }

    /// Estimate tape capacity based on media type
    fn estimate_tape_capacity_bytes(&self) -> u64 {
        // Default to LTO-8 capacity
        // In real implementation, this would query the device for actual capacity
        match self.scsi.check_media_status() {
            Ok(media_type) => {
                match media_type {
                    MediaType::Lto8Rw | MediaType::Lto8Worm | MediaType::Lto8Ro => {
                        12_000_000_000_000
                    } // 12TB
                    MediaType::Lto7Rw | MediaType::Lto7Worm | MediaType::Lto7Ro => {
                        6_000_000_000_000
                    } // 6TB
                    MediaType::Lto6Rw | MediaType::Lto6Worm | MediaType::Lto6Ro => {
                        2_500_000_000_000
                    } // 2.5TB
                    MediaType::Lto5Rw | MediaType::Lto5Worm | MediaType::Lto5Ro => {
                        1_500_000_000_000
                    } // 1.5TB
                    _ => 12_000_000_000_000, // Default to LTO-8
                }
            }
            Err(_) => 12_000_000_000_000, // Default capacity
        }
    }

    /// 切换到指定分区
    pub fn switch_to_partition(&self, partition: u8) -> Result<()> {
        info!("Switching to partition {}", partition);

        if self.offline_mode {
            info!("Offline mode: simulating partition switch");
            return Ok(());
        }

        self.scsi.locate_block(partition, 0)?;
        info!("Successfully switched to partition {}", partition);
        Ok(())
    }

    /// 定位到指定分区的指定块
    pub fn position_to_partition(&self, partition: u8, block: u64) -> Result<()> {
        info!("Positioning to partition {}, block {}", partition, block);

        if self.offline_mode {
            info!("Offline mode: simulating partition positioning");
            return Ok(());
        }

        self.scsi.locate_block(partition, block)?;
        info!("Successfully positioned to partition {}, block {}", partition, block);
        Ok(())
    }

    /// 获取分区信息
    pub async fn get_partition_info(&self) -> Result<PartitionInfo> {
        self.detect_partition_sizes().await
    }

    /// 读取分区标签
    pub async fn read_partition_labels(&mut self) -> Result<LtfsPartitionLabel> {
        info!("Reading LTFS partition label from tape");

        if self.offline_mode {
            return Ok(LtfsPartitionLabel::default());
        }

        // LTFS分区标签通常位于分区a的block 0
        // 首先定位到开头
        self.scsi.locate_block(0, 0)?; // 分区a, 块0 (相当于rewind)

        // 读取第一个块，包含LTFS卷标签
        let mut buffer = vec![0u8; crate::scsi::block_sizes::LTO_BLOCK_SIZE as usize];
        self.scsi.read_blocks(1, &mut buffer)?;

        // 解析LTFS卷标签
        let plabel = self.parse_ltfs_volume_label(&buffer)?;
        self.partition_label = Some(plabel.clone());
        Ok(plabel)
    }

    /// 解析LTFS卷标签获取分区标签信息（使用严格的VOL1验证）
    fn parse_ltfs_volume_label(&self, buffer: &[u8]) -> Result<LtfsPartitionLabel> {
        // 首先进行严格的VOL1标签验证
        if self.parse_vol1_label(buffer)? {
            info!("找到有效的VOL1标签");

            let mut plabel = LtfsPartitionLabel::default();

            // 从VOL1标签中提取额外信息（基于标准VOL1格式）
            // VOL1标签格式：
            // 位置0-3: "VOL1"
            // 位置4-9: 卷序列号
            // 位置10-79: 其他信息
            // 位置24-27: "LTFS"标识（已验证）

            // 尝试从标签中提取blocksize信息（位置40-43或类似位置）
            if buffer.len() >= 44 {
                let blocksize_bytes = &buffer[40..44];
                if let Ok(blocksize_str) = std::str::from_utf8(blocksize_bytes) {
                    if let Ok(blocksize) = blocksize_str.trim().parse::<u32>() {
                        if [65536, 524288, 1048576, 262144, 131072].contains(&blocksize) {
                            info!("从VOL1标签提取到blocksize: {}", blocksize);
                            plabel.blocksize = blocksize;
                        }
                    }
                }
            }

            Ok(plabel)
        } else {
            warn!("VOL1标签验证失败");
            // VOL1验证失败时，使用启发式方法作为后备
            self.detect_blocksize_heuristic(buffer)
        }
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

    /// 启发式检测blocksize
    fn detect_blocksize_heuristic(&self, buffer: &[u8]) -> Result<LtfsPartitionLabel> {
        info!("Using heuristic blocksize detection");

        let mut plabel = LtfsPartitionLabel::default();

        // 分析buffer中的模式来猜测blocksize
        // 如果buffer主要是零，可能使用了较大的blocksize
        let non_zero_count = buffer.iter().filter(|&&b| b != 0).count();
        let zero_ratio = (buffer.len() - non_zero_count) as f64 / buffer.len() as f64;

        info!("Buffer analysis: {:.1}% zeros", zero_ratio * 100.0);

        if zero_ratio > 0.8 {
            // 高零比率，可能是大blocksize
            plabel.blocksize = 524288; // 512KB
            info!("High zero ratio detected, using 512KB blocksize");
        } else if non_zero_count > 32768 {
            // 较多数据，可能是标准blocksize
            plabel.blocksize = 65536; // 64KB
            info!("Standard data pattern detected, using 64KB blocksize");
        } else {
            // 默认使用常见的512KB
            plabel.blocksize = 524288;
            info!("Using default 512KB blocksize");
        }

        Ok(plabel)
    }

    /// 检查分区支持情况
    pub async fn check_partition_support(&self) -> Result<bool> {
        self.check_multi_partition_support().await
    }

    /// 验证分区配置
    pub async fn validate_partition_configuration(&self) -> Result<bool> {
        let partition_info = self.detect_partition_sizes().await?;
        
        if partition_info.has_multi_partition {
            // 验证多分区配置
            if partition_info.partition_0_size == 0 || partition_info.partition_1_size == 0 {
                warn!("Invalid multi-partition configuration: zero-sized partition detected");
                return Ok(false);
            }
            
            info!("Multi-partition configuration validated: p0={}GB, p1={}GB", 
                  partition_info.partition_0_size / 1_000_000_000,
                  partition_info.partition_1_size / 1_000_000_000);
            Ok(true)
        } else {
            // 单分区配置
            info!("Single partition configuration validated: {}GB", 
                  partition_info.partition_0_size / 1_000_000_000);
            Ok(true)
        }
    }

    /// 获取分区大小
    pub async fn get_partition_sizes(&self) -> Result<(u64, u64)> {
        let partition_info = self.detect_partition_sizes().await?;
        Ok((partition_info.partition_0_size, partition_info.partition_1_size))
    }

    /// 分区健康检查
    pub async fn partition_health_check(&self) -> Result<bool> {
        info!("Performing partition health check");

        if self.offline_mode {
            info!("Offline mode: simulating partition health check");
            return Ok(true);
        }

        // 检查是否能成功访问所有分区
        let partition_info = self.detect_partition_sizes().await?;
        
        // 测试分区0访问
        match self.scsi.locate_block(0, 0) {
            Ok(()) => debug!("Partition 0 access: OK"),
            Err(e) => {
                warn!("Partition 0 access failed: {}", e);
                return Ok(false);
            }
        }

        // 如果是多分区磁带，测试分区1访问
        if partition_info.has_multi_partition {
            match self.scsi.locate_block(1, 0) {
                Ok(()) => {
                    debug!("Partition 1 access: OK");
                    // 返回分区0
                    self.scsi.locate_block(0, 0)?;
                }
                Err(e) => {
                    warn!("Partition 1 access failed: {}", e);
                    return Ok(false);
                }
            }
        }

        info!("Partition health check passed");
        Ok(true)
    }

    /// 获取当前分区标签信息
    pub fn get_partition_label(&self) -> Option<&LtfsPartitionLabel> {
        self.partition_label.as_ref()
    }

    /// 设置分区标签信息
    pub fn set_partition_label(&mut self, label: LtfsPartitionLabel) {
        self.partition_label = Some(label);
    }
}

/// 为TapeOperations实现分区管理功能
impl crate::tape_ops::TapeOperations {

    /// 检测分区大小 - 修复版本：直接使用已打开的SCSI设备
    pub async fn detect_partition_sizes(&self) -> Result<PartitionInfo> {
        info!("🔧 Detecting partition sizes using opened SCSI device (fixing device handle inconsistency)");
        
        // 使用已经初始化的ExtraPartitionCount结果
        let extra_partition_count = self.get_extra_partition_count();
        let has_multi_partition = extra_partition_count > 0;
        
        if !has_multi_partition {
            info!("Single partition detected (ExtraPartitionCount={}), using full capacity", extra_partition_count);
            
            // 简化版本：使用默认容量估算
            let total_capacity = match self.scsi.check_media_status() {
                Ok(media_type) => {
                    match media_type {
                        crate::scsi::MediaType::Lto8Rw | crate::scsi::MediaType::Lto8Worm | crate::scsi::MediaType::Lto8Ro => {
                            12_000_000_000_000
                        } // 12TB
                        crate::scsi::MediaType::Lto7Rw | crate::scsi::MediaType::Lto7Worm | crate::scsi::MediaType::Lto7Ro => {
                            6_000_000_000_000
                        } // 6TB
                        _ => 12_000_000_000_000, // Default to LTO-8
                    }
                }
                Err(_) => 12_000_000_000_000, // Default capacity
            };
            
            return Ok(PartitionInfo {
                partition_0_size: total_capacity,
                partition_1_size: 0,
                has_multi_partition: false,
            });
        }

        info!("Multi-partition detected (ExtraPartitionCount={}), using estimated partition sizes", extra_partition_count);

        // 对于多分区磁带，使用简化的估算方法
        let total_capacity = match self.scsi.check_media_status() {
            Ok(crate::scsi::MediaType::Lto7Rw) | Ok(crate::scsi::MediaType::Lto7Worm) | Ok(crate::scsi::MediaType::Lto7Ro) => {
                // LTO-7: 基于实际观察到的分区配置
                let index_partition_gb = 100; // 约100GB索引分区
                let p0_size = (index_partition_gb * 1_000_000_000) as u64;
                let p1_size = 6_000_000_000_000u64.saturating_sub(p0_size);
                (p0_size, p1_size)
            }
            Ok(crate::scsi::MediaType::Lto8Rw) | Ok(crate::scsi::MediaType::Lto8Worm) | Ok(crate::scsi::MediaType::Lto8Ro) => {
                // LTO-8: 按照相似比例估算
                let index_partition_gb = 200; // 约200GB索引分区
                let p0_size = (index_partition_gb * 1_000_000_000) as u64;
                let p1_size = 12_000_000_000_000u64.saturating_sub(p0_size);
                (p0_size, p1_size)
            }
            _ => {
                // 通用逻辑：索引分区约占1.8%
                let total = 12_000_000_000_000u64;
                let index_ratio = 0.018; // 1.8%
                let p0_size = (total as f64 * index_ratio) as u64;
                let p1_size = total.saturating_sub(p0_size);
                (p0_size, p1_size)
            }
        };
        
        info!(
            "📊 Using estimated partition sizes: p0={}GB, p1={}GB",
            total_capacity.0 / 1_000_000_000,
            total_capacity.1 / 1_000_000_000
        );
        
        Ok(PartitionInfo {
            partition_0_size: total_capacity.0,
            partition_1_size: total_capacity.1,
            has_multi_partition: true,
        })
    }

    /// 检查多分区支持 - 修复版本：直接使用已初始化的ExtraPartitionCount
    pub async fn check_multi_partition_support(&self) -> Result<bool> {
        info!("🔧 Checking multi-partition support using ExtraPartitionCount (avoiding new SCSI instance)");
        
        let extra_partition_count = self.get_extra_partition_count();
        let has_multi_partition = extra_partition_count > 0;
        
        info!("✅ Multi-partition support result: {} (ExtraPartitionCount={})", 
              has_multi_partition, extra_partition_count);
        
        Ok(has_multi_partition)
    }

    /// 验证分区配置 - 修复版本：直接使用已打开的SCSI设备
    pub async fn validate_partition_configuration(&self) -> Result<bool> {
        info!("🔧 Validating partition configuration using opened SCSI device");
        
        let partition_info = self.detect_partition_sizes().await?;
        
        if partition_info.has_multi_partition {
            // 验证多分区配置
            if partition_info.partition_0_size == 0 || partition_info.partition_1_size == 0 {
                warn!("Invalid multi-partition configuration: zero-sized partition detected");
                return Ok(false);
            }
            
            info!("Multi-partition configuration validated: p0={}GB, p1={}GB", 
                  partition_info.partition_0_size / 1_000_000_000,
                  partition_info.partition_1_size / 1_000_000_000);
            Ok(true)
        } else {
            // 单分区配置
            info!("Single partition configuration validated: {}GB", 
                  partition_info.partition_0_size / 1_000_000_000);
            Ok(true)
        }
    }

    /// 获取分区大小 - 修复版本：直接使用已打开的SCSI设备
    pub async fn get_partition_sizes(&self) -> Result<(u64, u64)> {
        info!("🔧 Getting partition sizes using opened SCSI device");
        
        let partition_info = self.detect_partition_sizes().await?;
        Ok((partition_info.partition_0_size, partition_info.partition_1_size))
    }

    /// 分区健康检查 - 修复版本：直接使用已打开的SCSI设备
    pub async fn partition_health_check(&self) -> Result<bool> {
        info!("🔧 Performing partition health check using opened SCSI device");

        if self.offline_mode {
            info!("Offline mode: simulating partition health check");
            return Ok(true);
        }

        // 检查是否能成功访问所有分区
        let partition_info = self.detect_partition_sizes().await?;
        
        // 测试分区0访问
        match self.scsi.locate_block(0, 0) {
            Ok(()) => debug!("Partition 0 access: OK"),
            Err(e) => {
                warn!("Partition 0 access failed: {}", e);
                return Ok(false);
            }
        }

        // 如果是多分区磁带，测试分区1访问
        if partition_info.has_multi_partition {
            match self.scsi.locate_block(1, 0) {
                Ok(()) => {
                    debug!("Partition 1 access: OK");
                    // 返回分区0
                    self.scsi.locate_block(0, 0)?;
                }
                Err(e) => {
                    warn!("Partition 1 access failed: {}", e);
                    return Ok(false);
                }
            }
        }

        info!("Partition health check passed");
        Ok(true)
    }

    /// 切换到指定分区 - 修复版本：直接使用已打开的SCSI设备
    pub fn switch_to_partition(&self, partition: u8) -> Result<()> {
        info!("🔧 Switching to partition {} using opened SCSI device", partition);

        if self.offline_mode {
            info!("Offline mode: simulating partition switch");
            return Ok(());
        }

        self.scsi.locate_block(partition, 0)?;
        info!("Successfully switched to partition {}", partition);
        Ok(())
    }

    /// 定位到指定分区的指定块 - 修复版本：直接使用已打开的SCSI设备
    pub fn position_to_partition(&self, partition: u8, block: u64) -> Result<()> {
        info!("🔧 Positioning to partition {}, block {} using opened SCSI device", partition, block);

        if self.offline_mode {
            info!("Offline mode: simulating partition positioning");
            return Ok(());
        }

        self.scsi.locate_block(partition, block)?;
        info!("Successfully positioned to partition {}, block {}", partition, block);
        Ok(())
    }

    /// 获取分区信息 - 修复版本：直接使用已打开的SCSI设备
    pub async fn get_partition_info(&self) -> Result<PartitionInfo> {
        info!("🔧 Getting partition info using opened SCSI device");
        
        self.detect_partition_sizes().await
    }

    /// 从指定位置读取索引
    pub fn read_index_from_specific_location(&self, location: &IndexLocation) -> Result<String> {
        info!(
            "Reading index from partition {}, block {}",
            location.partition, location.start_block
        );

        let partition_id = match location.partition.to_lowercase().as_str() {
            "a" => 0,
            "b" => 1,
            _ => {
                return Err(RustLtfsError::ltfs_index(format!(
                    "Invalid partition: {}",
                    location.partition
                )))
            }
        };

        // 定位到指定位置
        self.scsi.locate_block(partition_id, location.start_block)?;

        // 使用动态blocksize读取
        let block_size = self
            .partition_label
            .as_ref()
            .map(|plabel| plabel.blocksize as usize)
            .unwrap_or(crate::scsi::block_sizes::LTO_BLOCK_SIZE as usize);

        self.read_to_file_mark_with_temp_file(block_size)
    }

    /// 读取单分区磁带索引读取策略 (对应LTFSCopyGUI的单分区处理逻辑)
    pub async fn read_index_from_single_partition_tape(&mut self) -> Result<()> {
        info!("Reading index from single-partition tape (LTFSCopyGUI fallback strategy)");

        // 在单分区磁带上，数据和索引都存储在同一分区
        // 需要搜索数据分区中的索引副本

        // 步骤1: 尝试从常见的索引位置读取（基于LTFSCopyGUI观察到的模式）
        // 从索引文件我们看到LTFS索引通常在block 6，而不是block 0
        let common_index_locations = vec![6, 5, 2, 10, 20, 100]; // 把block 6放在首位

        for &block in &common_index_locations {
            debug!(
                "Trying index location at block {} (single-partition strategy)",
                block
            );

            match self.scsi.locate_block(0, block) {
                Ok(()) => {
                    match self.try_read_index_at_current_position_advanced().await {
                        Ok(xml_content) => {
                            if self.validate_and_process_index(&xml_content).await? {
                                info!("✅ Successfully read index from single-partition tape at block {}", block);
                                return Ok(());
                            }
                        }
                        Err(_e) => {
                            // 使用debug级别而不是warn，减少日志噪音
                            debug!("No valid index at block {}", block);
                        }
                    }
                }
                Err(_e) => {
                    debug!("Cannot position to block {}", block);
                }
            }
        }

        // 步骤2: 有限的数据区域搜索（不是扩展搜索）
        info!("Common index locations failed, performing limited data area search");
        self.search_data_area_for_index().await
    }

    /// 数据分区索引读取策略 (对应LTFSCopyGUI的数据分区索引逻辑)
    pub async fn read_index_from_data_partition_strategy(&mut self) -> Result<()> {
        info!("Reading index from data partition strategy (LTFSCopyGUI data partition logic)");

        // 当volume label指示索引在partition B时使用此策略
        match self.read_latest_index_from_data_partition() {
            Ok(xml_content) => {
                if self.validate_and_process_index(&xml_content).await? {
                    info!("✅ Successfully read index from data partition");
                    Ok(())
                } else {
                    Err(RustLtfsError::ltfs_index(
                        "Index from data partition validation failed".to_string(),
                    ))
                }
            }
            Err(e) => {
                warn!(
                    "Data partition index reading failed: {}, trying fallback",
                    e
                );
                self.read_index_from_single_partition_tape().await
            }
        }
    }

    /// 异步版本的完整LTFSCopyGUI回退策略 (分区管理器版本) - 修复版本：直接使用已打开的SCSI设备
    pub async fn try_alternative_index_reading_strategies_partition_async(&mut self) -> Result<String> {
        info!("🔄 Starting complete LTFSCopyGUI alternative index reading strategies (using opened SCSI device)");

        // 直接使用已打开的self.scsi进行分区检测，避免创建新实例
        info!("🔧 Using opened SCSI device for partition detection (fixing device handle inconsistency)");
        
        // 使用我们已经修复的initialize_partition_detection结果
        let partition_count = if self.get_extra_partition_count() > 0 { 2 } else { 1 };
        let index_partition = if partition_count > 1 { 0 } else { 0 };

        info!("📋 Partition detection result: count={}, index_partition={}", partition_count, index_partition);

        // 策略0 (最高优先级): 按照LTFSCopyGUI逻辑优先读取数据分区索引  
        info!("Strategy 0 (Highest Priority): Reading from data partition first (LTFSCopyGUI logic)");
        
        if partition_count > 1 {
            // 多分区磁带：优先尝试读取数据分区最新索引，匹配LTFSCopyGUI的"读取数据区索引"
            match self.try_read_from_data_partition_partition_async().await {
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

    /// 读取数据区最新索引 (对应LTFSCopyGUI的"读取数据区最新索引"功能)
    pub fn read_latest_index_from_data_partition(&self) -> Result<String> {
        info!("Attempting to read latest index from data partition (partition B)");

        // LTFS标准：数据区（partition B）可能包含最新的索引副本
        // 这是LTFSCopyGUI特有的策略，用于处理索引分区损坏的情况

        // 第1步：尝试从volume label获取最新索引位置
        if let Ok(latest_location) = self.get_latest_index_location_from_volume_label() {
            info!(
                "Found latest index location from volume label: partition {}, block {}",
                latest_location.partition, latest_location.start_block
            );

            if let Ok(xml_content) = self.read_index_from_specific_location(&latest_location) {
                return Ok(xml_content);
            }
        }

        // 第2步：搜索数据分区中的索引副本
        self.search_index_copies_in_data_partition()
    }

    /// 从volume label获取最新索引位置
    fn get_latest_index_location_from_volume_label(&self) -> Result<IndexLocation> {
        info!("Reading volume label to find latest index location");

        // 定位到volume label (partition A, block 0)
        self.scsi.locate_block(0, 0)?;

        let mut buffer = vec![0u8; crate::scsi::block_sizes::LTO_BLOCK_SIZE as usize];
        self.scsi.read_blocks(1, &mut buffer)?;

        // 解析volume label中的索引位置指针
        // LTFS volume label格式包含：
        // - Current index location (当前索引位置)
        // - Previous index location (上一个索引位置)

        // 使用本地实现而不是调用其他模块的私有方法
        self.parse_index_location_from_buffer(&buffer)
    }

    /// 使用LTFSCopyGUI兼容的ReadToFileMark方法读取索引
    pub fn try_read_index_with_ltfscopygui_method(&self, block: u64) -> Result<String> {
        info!("Using LTFSCopyGUI-compatible ReadToFileMark method at block {}", block);
        
        // 修复：使用标准LTO块大小（64KB）以确保正确读取完整索引
        let block_size = crate::scsi::block_sizes::LTO_BLOCK_SIZE as usize; // 64KB标准块大小
        let max_blocks = 50; // 减少最大块数，专注于索引读取
        
        debug!("Fixed block size calculation: using standard LTO block size {} bytes, max_blocks: {}", block_size, max_blocks);
        
        // 创建临时文件用于存储读取的数据
        let temp_dir = std::env::temp_dir();
        let temp_file_name = format!("LTFSIndex_Block{}_{}.tmp", block, 
                                    chrono::Utc::now().format("%Y%m%d_%H%M%S"));
        let temp_path = temp_dir.join(&temp_file_name);
        
        info!("Creating temporary index file: {:?}", temp_path);
        
        let mut temp_file = std::fs::File::create(&temp_path)
            .map_err(|e| RustLtfsError::file_operation(format!("Cannot create temp file: {}", e)))?;
        
        let mut total_bytes = 0;
        let mut blocks_read = 0;
        
        // ReadToFileMark逻辑：持续读取直到遇到FileMark或达到限制
        for block_num in 0..max_blocks {
            let mut buffer = vec![0u8; block_size];
            
            match self.scsi.read_blocks(1, &mut buffer) {
                Ok(bytes_read) => {
                    debug!("SCSI read_blocks(1) at relative block {} returned {} bytes", block_num, bytes_read);
                    
                    if bytes_read == 0 {
                        debug!("No more data available at block {}", block_num);
                        break;
                    }
                    
                    // 对第一个块进行详细分析
                    if block_num == 0 {
                        let sample_size = std::cmp::min(64, bytes_read as usize);
                        let sample_data: Vec<String> = buffer[..sample_size]
                            .iter()
                            .map(|&b| format!("{:02X}", b))
                            .collect();
                        debug!("First block sample ({}B): {}", sample_size, sample_data.join(" "));
                        
                        // 检查是否包含XML或LTFS相关内容
                        let text_content = String::from_utf8_lossy(&buffer[..bytes_read as usize]);
                        if text_content.contains("ltfsindex") || text_content.contains("<?xml") {
                            info!("Found XML/LTFS content in first block!");
                        } else if bytes_read == 1 {
                            warn!("Only 1 byte read at block {}, might indicate positioning issue or EOD", block);
                            debug!("Single byte value: 0x{:02X}", buffer[0]);
                        }
                    }
                    
                    // 写入临时文件
                    temp_file.write_all(&buffer[..bytes_read as usize])
                        .map_err(|e| RustLtfsError::file_operation(format!("Cannot write to temp file: {}", e)))?;
                    
                    total_bytes += bytes_read;
                    blocks_read += 1;
                    
                    // 检查是否遇到FileMark（通常表示为短读取或特定模式）
                    if (bytes_read as usize) < block_size {
                        debug!("Encountered short read (possible FileMark) at block {}: read {} bytes < {} block size", 
                               block_num, bytes_read, block_size);
                        break;
                    }
                }
                Err(e) => {
                    debug!("Read error at block {}: {}", block_num, e);
                    break;
                }
            }
        }
        
        // 关闭文件并读取内容
        drop(temp_file);
        
        info!("ReadToFileMark completed: {} blocks read, {} total bytes", blocks_read, total_bytes);
        
        // 读取临时文件内容
        let xml_content = std::fs::read_to_string(&temp_path)
            .map_err(|e| RustLtfsError::file_operation(format!("Cannot read temp file: {}", e)))?;
        
        // 清理临时文件
        let _ = std::fs::remove_file(&temp_path);
        
        Ok(xml_content)
    }

    pub fn search_index_copies_in_data_partition(&self) -> Result<String> {
        info!("🔧 Starting LTFSCopyGUI-compatible index search (exact replication)");
        
        // 精确复制LTFSCopyGUI的读取索引逻辑
        self.read_index_ltfscopygui_method()
    }

    /// 精确复制LTFSCopyGUI的索引读取逻辑 (一比一实现)
    /// 支持单分区和多分区磁带的统一处理策略
    fn read_index_ltfscopygui_method(&self) -> Result<String> {
        info!("🎯 Executing LTFSCopyGUI exact index reading method");
        
        // 步骤1: 检测ExtraPartitionCount (对应LTFSCopyGUI的分区检测)
        let extra_partition_count = if self.offline_mode { 1 } else {
            match self.scsi.mode_sense_partition_page_0x11() {
                Ok(mode_data) if mode_data.len() >= 4 => {
                    let count = mode_data[3];
                    info!("📊 ExtraPartitionCount detected from MODE SENSE: {}", count);
                    count
                }
                _ => {
                    info!("📊 Cannot read ExtraPartitionCount, assuming single partition");
                    0
                }
            }
        };
        
        if extra_partition_count == 0 {
            // 🔧 单分区磁带策略 (对应LTFSCopyGUI的ExtraPartitionCount = 0逻辑)
            info!("🎯 Single partition tape detected (ExtraPartitionCount=0)");
            self.read_index_single_partition_ltfscopygui()
        } else {
            // 🔧 多分区磁带策略 (对应LTFSCopyGUI的数据分区索引读取)
            info!("🎯 Multi-partition tape detected (ExtraPartitionCount={})", extra_partition_count);
            self.read_index_multi_partition_ltfscopygui(extra_partition_count)
        }
    }

    /// LTFSCopyGUI单分区索引读取策略 (精确复制"读取索引ToolStripMenuItem_Click"的单分区逻辑)
    fn read_index_single_partition_ltfscopygui(&self) -> Result<String> {
        info!("🔧 LTFSCopyGUI single partition index reading (ExtraPartitionCount=0)");
        
        // 步骤1: 定位到分区0的EOD
        info!("Step 1: Locating to partition 0 EOD");
        self.scsi.locate_to_eod(0)?;
        
        // 步骤2: 获取当前FileMark编号
        let position = self.scsi.read_position()?;
        let current_fm = position.file_number;
        
        info!("🔍 Current position at EOD: P{} B{} FM{} SET{}", 
             position.partition, position.block_number, position.file_number, position.set_number);
        
        // 步骤3: LTFSCopyGUI的关键检查 - FM <= 1 则失败
        if current_fm <= 1 {
            return Err(RustLtfsError::ltfs_index(format!(
                "Invalid LTFS tape: FileMark number {} <= 1, this is not a valid LTFS tape", current_fm
            )));
        }
        
        // 步骤4: LTFSCopyGUI真实策略 - 定位到FileMark 1 (不是FM-1!)
        // 对应LTFSCopyGUI代码: TapeUtils.Locate(driveHandle, 1UL, partition, TapeUtils.LocateDestType.FileMark)
        info!("Step 4: Locating to FileMark 1 (LTFSCopyGUI standard strategy)");
        self.scsi.locate_to_filemark(0, 1)?; // partition=0, filemark=1
        
        // 步骤5: ReadFileMark - 跳过FileMark标记
        info!("Step 5: Skipping FileMark using ReadFileMark method");
        self.scsi.read_file_mark()?;
        
        // 步骤6: ReadToFileMark - 读取索引内容
        info!("Step 6: Reading index content using ReadToFileMark");
        let index_data = self.scsi.read_to_file_mark(block_sizes::LTO_BLOCK_SIZE_512K)?;
        
        // 🎯 完全按照LTFSCopyGUI的验证逻辑：检查是否包含"XMLSchema"
        let xml_content = String::from_utf8_lossy(&index_data).to_string();
        if xml_content.contains("XMLSchema") {
            info!("✅ Successfully read LTFS index using single partition method: {} bytes (contains XMLSchema)", xml_content.len());
            Ok(xml_content)
        } else {
            // 🔧 LTFSCopyGUI备选路径：FromSchemaText处理
            let processed_content = self.ltfscopygui_from_schema_text(xml_content)?;
            info!("✅ Successfully processed LTFS schema text format: {} bytes", processed_content.len());
            Ok(processed_content)
        }
    }

    /// LTFSCopyGUI多分区索引读取策略 (精确复制"读取数据区索引ToolStripMenuItem_Click"逻辑)
    fn read_index_multi_partition_ltfscopygui(&self, extra_partition_count: u8) -> Result<String> {
        info!("🔧 LTFSCopyGUI multi-partition index reading (ExtraPartitionCount={})", extra_partition_count);
        
        // 步骤1: 切换到数据分区 (通常是partition 1)
        let data_partition = 1u8;
        info!("Step 1: Switching to data partition {}", data_partition);
        self.scsi.locate_block(data_partition, 0)?;
        
        // 步骤2: 定位到数据分区的EOD
        info!("Step 2: Locating to data partition EOD");
        self.scsi.locate_to_eod(data_partition)?;
        
        // 步骤3: 获取当前FileMark编号
        let position = self.scsi.read_position()?;
        let current_fm = position.file_number;
        
        info!("🔍 Current position at data partition EOD: P{} B{} FM{} SET{}", 
             position.partition, position.block_number, position.file_number, position.set_number);
        
        // 步骤4: LTFSCopyGUI的关键检查和策略选择
        if current_fm <= 1 {
            // DisablePartition后备策略
            info!("Step 4a: FM <= 1, using DisablePartition fallback (Space6 -2 FileMark)");
            return self.ltfscopygui_disable_partition_fallback();
        } else {
            // 标准FileMark 1策略 (LTFSCopyGUI标准)
            info!("Step 4b: Using standard FileMark 1 strategy");
            return self.ltfscopygui_standard_filemark_strategy(data_partition);
        }
    }

    /// LTFSCopyGUI的DisablePartition后备策略 (对应TapeUtils.Space6(-2, FileMark))
    fn ltfscopygui_disable_partition_fallback(&self) -> Result<String> {
        info!("🔧 Executing LTFSCopyGUI DisablePartition fallback strategy");
        
        // 步骤1: Space6(-2, FileMark) - 后退2个FileMark
        info!("Step 1: Moving back 2 FileMarks using Space6 command");
        self.scsi.space(crate::scsi::SpaceType::FileMarks, -2)?;
        
        // 步骤2: ReadFileMark - 跳过FileMark
        info!("Step 2: Skipping FileMark using ReadFileMark");
        self.scsi.read_file_mark()?;
        
        // 步骤3: ReadToFileMark - 读取索引
        info!("Step 3: Reading index using ReadToFileMark");
        let index_data = self.scsi.read_to_file_mark(block_sizes::LTO_BLOCK_SIZE_512K)?;
        
        // 🎯 完全按照LTFSCopyGUI的验证逻辑：检查是否包含"XMLSchema"
        let xml_content = String::from_utf8_lossy(&index_data).to_string();
        if xml_content.contains("XMLSchema") {
            info!("✅ Successfully read LTFS index using DisablePartition fallback: {} bytes (contains XMLSchema)", xml_content.len());
            Ok(xml_content)
        } else {
            // 🔧 LTFSCopyGUI备选路径：FromSchemaText处理
            let processed_content = self.ltfscopygui_from_schema_text(xml_content)?;
            info!("✅ Successfully processed LTFS schema text format: {} bytes", processed_content.len());
            Ok(processed_content)
        }
    }

    /// LTFSCopyGUI的标准FileMark定位策略 (精确对应TapeUtils.Locate FileMark逻辑)
    fn ltfscopygui_standard_filemark_strategy(&self, partition: u8) -> Result<String> {
        info!("🔧 Executing LTFSCopyGUI standard FileMark 1 strategy");
        
        // 步骤1: 定位到FileMark 1 (LTFSCopyGUI标准)
        // 对应: TapeUtils.Locate(driveHandle, 1UL, partition, TapeUtils.LocateDestType.FileMark)
        info!("Step 1: Locating to FileMark 1 (LTFSCopyGUI standard)");
        self.scsi.locate_to_filemark(1, partition)?; // filemark=1, partition
        
        // 步骤2: ReadFileMark - 跳过FileMark
        info!("Step 2: Skipping FileMark using ReadFileMark");
        self.scsi.read_file_mark()?;
        
        // 步骤3: ReadToFileMark - 读取索引
        info!("Step 3: Reading index using ReadToFileMark");
        let index_data = self.scsi.read_to_file_mark(block_sizes::LTO_BLOCK_SIZE_512K)?;
        
        // 🎯 完全按照LTFSCopyGUI的验证逻辑：检查是否包含"XMLSchema"
        let xml_content = String::from_utf8_lossy(&index_data).to_string();
        if xml_content.contains("XMLSchema") {
            info!("✅ Successfully read LTFS index using FileMark 1 strategy: {} bytes (contains XMLSchema)", xml_content.len());
            Ok(xml_content)
        } else {
            // 🔧 LTFSCopyGUI备选路径：FromSchemaText处理
            let processed_content = self.ltfscopygui_from_schema_text(xml_content)?;
            info!("✅ Successfully processed LTFS schema text format: {} bytes", processed_content.len());
            Ok(processed_content)
        }
    }

    /// 本地实现：解析volume label中的索引位置信息
    fn parse_index_location_from_buffer(&self, buffer: &[u8]) -> Result<IndexLocation> {
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

    /// 检查是否是有效的LTFS索引
    pub fn is_valid_ltfs_index(&self, xml_content: &str) -> bool {
        xml_content.contains("<ltfsindex")
            && xml_content.contains("</ltfsindex>")
            && xml_content.contains("<directory")
            && xml_content.len() > 200
    }

    /// 尝试从当前位置读取索引 (同步版本，用于回退策略)
    pub fn try_read_index_at_current_position_sync(&self) -> Result<String> {
        let block_size = crate::scsi::block_sizes::LTO_BLOCK_SIZE as usize;
        let max_blocks = 50; // 限制读取块数，避免读取过多数据
        let mut xml_content = String::new();
        let mut blocks_read = 0;

        // 读取多个块直到找到完整的XML或达到限制
        for block_num in 0..max_blocks {
            let mut buffer = vec![0u8; block_size];

            match self.scsi.read_blocks(1, &mut buffer) {
                Ok(read_count) => {
                    if read_count == 0 {
                        break;
                    }

                    blocks_read += 1;

                    // 检查是否为全零块（可能的文件标记）
                    if buffer.iter().all(|&b| b == 0) {
                        debug!(
                            "Encountered zero block at {}, assuming end of data",
                            block_num
                        );
                        break;
                    }

                    // 转换为UTF-8并添加到内容
                    match String::from_utf8(buffer) {
                        Ok(block_content) => {
                            let trimmed = block_content.trim_end_matches('\0');
                            xml_content.push_str(trimmed);

                            // 检查是否已读取完整的XML
                            if xml_content.contains("</ltfsindex>") {
                                debug!(
                                    "Found complete LTFS index XML after {} blocks",
                                    blocks_read
                                );
                                break;
                            }
                        }
                        Err(_) => {
                            debug!("Non-UTF8 data encountered at block {}, stopping", block_num);
                            break;
                        }
                    }
                }
                Err(_) => {
                    debug!("Read error at block {}, stopping", block_num);
                    break;
                }
            }
        }

        let cleaned_xml = xml_content.replace('\0', "").trim().to_string();

        if cleaned_xml.is_empty() {
            Err(RustLtfsError::ltfs_index(
                "No XML content found at current position".to_string(),
            ))
        } else {
            Ok(cleaned_xml)
        }
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
        let max_blocks = 200; // 对应LTFSCopyGUI的固定限制
        let mut consecutive_errors = 0;
        const MAX_CONSECUTIVE_ERRORS: u32 = 3;

        info!(
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
                        info!("✅ Reached file mark (blocks_read_count = 0), stopping read");
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
                }
                Err(e) => {
                    consecutive_errors += 1;
                    warn!("SCSI read error #{} after {} blocks: {}", consecutive_errors, blocks_read, e);
                    
                    // 增强的SCSI错误分类和恢复
                    let error_handled = self.handle_scsi_read_error(&e, blocks_read, consecutive_errors)?;
                    
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
                        warn!("Too many consecutive SCSI errors ({}), stopping read operation", consecutive_errors);
                        if blocks_read == 0 {
                            return Err(RustLtfsError::scsi(format!(
                                "Failed to read any data after {} consecutive SCSI errors", consecutive_errors
                            )));
                        }
                        break;
                    }
                }
            }
        }

        temp_file.flush()?;
        drop(temp_file);

        info!(
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
            return Err(RustLtfsError::ltfs_index("Cleaned XML is empty".to_string()));
        }

        debug!(
            "Extracted XML content: {} bytes (after cleanup)",
            cleaned_xml.len()
        );
        Ok(cleaned_xml)
    }

    /// 增强的SCSI读取错误处理
    /// 返回true表示错误已处理，可以继续；返回false表示应该停止
    fn handle_scsi_read_error(&self, error: &RustLtfsError, blocks_read: u32, error_count: u32) -> Result<bool> {
        let error_str = error.to_string();
        
        // 错误分类和处理策略
        if error_str.contains("Direct block read operation failed") {
            debug!("Detected direct block read failure - possibly reached end of data or file mark");
            
            // 如果已经读取了一些数据，这可能是正常的文件结束
            if blocks_read > 0 {
                info!("Block read failure after {} blocks - likely reached end of index data", blocks_read);
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
                    info!("Device status recovered, can continue reading");
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
                info!("Medium error after reading {} blocks - stopping to preserve data", blocks_read);
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
        debug!("Unknown SCSI error type: {} - attempting conservative retry", error_str);
        Ok(error_count <= 2) // 允许有限重试
    }

    /// 支持高级搜索的索引读取方法 (对应LTFSCopyGUI高级功能)
    pub fn try_read_index_at_current_position_advanced_sync(&self) -> Result<String> {
        let block_size = self
            .partition_label
            .as_ref()
            .map(|plabel| plabel.blocksize as usize)
            .unwrap_or(crate::scsi::block_sizes::LTO_BLOCK_SIZE as usize);

        debug!("Advanced index reading with dynamic blocksize: {}", block_size);

        // 读取并清理临时文件
        let xml_content = self.read_to_file_mark_with_temp_file(block_size)?;

        // 清理临时文件已在read_to_file_mark_with_temp_file中处理

        // 清理XML内容（对应VB的Replace和Trim）
        let cleaned_xml = xml_content.replace('\0', "").trim().to_string();

        if cleaned_xml.is_empty() {
            debug!("No LTFS index data found");
            return Err(RustLtfsError::ltfs_index("Index XML is empty".to_string()));
        } else {
            info!(
                "Advanced index reading extracted {} bytes of index data",
                cleaned_xml.len()
            );
        }

        Ok(cleaned_xml)
    }

    /// 高级当前位置索引读取 (增强版本，支持更好的错误处理)
    pub async fn try_read_index_at_current_position_advanced(&self) -> Result<String> {
        let block_size = crate::scsi::block_sizes::LTO_BLOCK_SIZE as usize;

        info!(
            "Advanced index reading at current position with blocksize {}",
            block_size
        );

        // 使用ReadToFileMark方法，与标准流程保持一致
        self.read_to_file_mark_with_temp_file(block_size)
    }

    /// 搜索数据区域中的索引副本 (分区管理器版本)
    pub async fn search_data_area_for_index_partition(&mut self) -> Result<()> {
        info!("Searching data area for index copies (optimized search)");

        // 缩减搜索范围：如果磁带是空白的，不需要大范围搜索
        let limited_search_locations = vec![
            // 只搜索最可能的位置
            50, 100, 500, 1000, 2000,
        ];

        for &block in &limited_search_locations {
            debug!("Extended search: trying block {}", block);

            // 在单分区磁带上，所有数据都在partition 0
            match self.scsi.locate_block(0, block) {
                Ok(()) => match self.try_read_index_at_current_position_advanced().await {
                    Ok(xml_content) => {
                        if self.validate_and_process_index_partition(&xml_content).await? {
                            info!("✅ Found valid index in data area at block {}", block);
                            return Ok(());
                        }
                    }
                    Err(e) => {
                        debug!("No valid index at data block {}: {}", block, e);
                    }
                },
                Err(e) => {
                    debug!("Cannot position to data block {}: {}", block, e);
                }
            }

            // 更短的延迟
            if block > 1000 {
                tokio::time::sleep(tokio::time::Duration::from_millis(50)).await;
            }
        }

        Err(RustLtfsError::ltfs_index(
            "No valid index found in data area search".to_string(),
        ))
    }

    /// 验证并处理索引内容 (分区管理器版本)
    pub async fn validate_and_process_index_partition(&mut self, xml_content: &str) -> Result<bool> {
        if xml_content.trim().is_empty() {
            return Ok(false);
        }

        if !xml_content.contains("<ltfsindex") || !xml_content.contains("</ltfsindex>") {
            return Ok(false);
        }

        // 尝试解析索引
        match LtfsIndex::from_xml_streaming(xml_content) {
            Ok(index) => {
                info!("✅ Index validation successful, updating internal state");

                // 保存索引文件到当前目录（按时间命名）
                let timestamp = chrono::Utc::now().format("%Y%m%d_%H%M%S");
                let index_filename = format!("ltfs_index_{}.xml", timestamp);

                match std::fs::write(&index_filename, xml_content) {
                    Ok(()) => {
                        info!("📄 LTFS索引已保存到: {}", index_filename);
                    }
                    Err(e) => {
                        warn!("⚠️ 保存索引文件失败: {} - {}", index_filename, e);
                    }
                }

                Ok(true)
            }
            Err(e) => {
                debug!("Index parsing failed: {}", e);
                Ok(false)
            }
        }
    }

    /// 异步版本：尝试从数据分区读取索引副本 (分区管理器版本)
    pub async fn try_read_from_data_partition_partition_async(&mut self) -> Result<String> {
        info!("Attempting to read index from data partition (matching LTFSCopyGUI logic)");

        // 按照LTFSCopyGUI的"读取数据区索引"逻辑：
        // 1. 定位到数据分区EOD
        // 2. 向前查找最后的索引
        let data_partition = 1;
        
        // 先尝试定位到数据分区EOD
        match self.scsi.locate_block(data_partition, 0) {
            Ok(()) => {
                // 注意：这里需要实现EOD定位逻辑，目前作为占位符
                // TODO: 实现 scsi.space(EndOfData) 和相关的文件标记定位功能
                info!("Data partition positioning - EOD logic placeholder");
                
                // 搜索数据分区的一些常见索引位置
                let search_blocks = vec![10000, 5000, 2000, 1000];
                
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
            }
            Err(e) => debug!("Failed to position to data partition: {}", e),
        }

        Err(RustLtfsError::ltfs_index(
            "No valid index found in data partition".to_string(),
        ))
    }
    
    /// 在数据区搜索索引
    pub async fn search_data_area_for_index(&mut self) -> Result<()> {
        info!("Searching for index in data area");
        
        // 这个方法与search_index_copies_in_data_partition类似
        // 但是会设置index字段而不是返回XML内容
        match self.search_index_copies_in_data_partition() {
            Ok(xml_content) => {
                match crate::ltfs_index::LtfsIndex::from_xml(&xml_content) {
                    Ok(index) => {
                        self.index = Some(index);
                        Ok(())
                    }
                    Err(e) => Err(RustLtfsError::ltfs_index(format!(
                        "Failed to parse index XML: {}", e
                    )))
                }
            }
            Err(e) => Err(e),
        }
    }

    /// 完全复刻LTFSCopyGUI的FromSchemaText方法 (Schema.vb:542-553)
    /// 精确对应VB.NET代码的字符串替换和处理逻辑
    fn ltfscopygui_from_schema_text(&self, mut s: String) -> Result<String> {
        debug!("🔧 Applying LTFSCopyGUI FromSchemaText transformations");
        
        // 记录原始数据信息用于调试
        let original_len = s.len();
        let non_null_count = s.chars().filter(|&c| c != '\0').count();
        debug!("📊 Original data: {} bytes, {} non-null chars ({:.1}% content)", 
               original_len, non_null_count, (non_null_count as f64 / original_len as f64) * 100.0);
        
        // 移除null字符（对应.NET字符串处理）
        s = s.replace('\0', "");
        
        // 检查处理后的数据
        debug!("📊 After null removal: {} bytes", s.len());
        if s.len() < 20 {
            debug!("⚠️ Content sample: {:?}", s.chars().take(100).collect::<String>());
            
            // LTFSCopyGUI兼容性：如果数据太短，可能是空白磁带或错误位置
            // 返回一个更具体的错误信息，但允许上层逻辑继续尝试其他策略
            return Err(RustLtfsError::ltfs_index(
                format!("Schema text too short after null removal: {} bytes (original: {} bytes, {:.1}% null)", 
                       s.len(), original_len, ((original_len - s.len()) as f64 / original_len as f64) * 100.0)
            ));
        }
        
        // 精确对应LTFSCopyGUI的字符串替换操作
        s = s.replace("<directory>", "<_directory><directory>");
        s = s.replace("</directory>", "</directory></_directory>");
        s = s.replace("<file>", "<_file><file>");
        s = s.replace("</file>", "</file></_file>");
        s = s.replace("%25", "%");
        
        // 基础验证：确保包含必要的LTFS结构
        if !s.contains("ltfsindex") && !s.contains("directory") && !s.contains("file") {
            debug!("⚠️ No LTFS structure found. Content preview: {:?}", 
                   s.chars().take(200).collect::<String>());
            return Err(RustLtfsError::ltfs_index(
                format!("No LTFS structure found in {} bytes of processed text", s.len())
            ));
        }
        
        debug!("✅ LTFSCopyGUI FromSchemaText processing completed: {} bytes", s.len());
        Ok(s)
    }

    /// LTFSCopyGUI的LookforXMLEndPosition方法复刻 (Form1.vb:141-156)
    /// 递归查找XML标签的结束位置
    fn ltfscopygui_lookfor_xml_end_position(&self, s: &str, target: &str, start_pos: usize) -> usize {
        let target_bra = format!("<{}>", target);
        let target_ket = format!("</{}>", target);
        let mut i = start_pos;
        
        while i < s.len().saturating_sub(target_ket.len()) {
            i += 1;
            
            // 检查是否遇到开始标签（需要递归处理）
            if i + target_bra.len() <= s.len() {
                if &s[i..i + target_bra.len()] == target_bra {
                    i = self.ltfscopygui_lookfor_xml_end_position(s, target, i);
                    continue;
                }
            }
            
            // 检查是否遇到结束标签
            if i + target_ket.len() <= s.len() {
                if &s[i..i + target_ket.len()] == target_ket {
                    return i;
                }
            }
        }
        
        i
    }

    /// 在数据中查找模式 (用于调试方法)
    fn find_pattern_in_data(&self, data: &[u8], pattern: &[u8]) -> Option<usize> {
        data.windows(pattern.len()).position(|window| window == pattern)
    }

    /// 调试分析索引数据内容 (增强版本)
    fn debug_analyze_index_data(&self, data: &[u8], strategy_name: &str) {
        warn!("🔍 Debug analysis for {} - {} bytes total", strategy_name, data.len());
        
        // 基础统计
        let non_zero_count = data.iter().filter(|&&b| b != 0).count();
        let zero_ratio = (data.len() - non_zero_count) as f64 / data.len() as f64;
        warn!("📊 Data composition: {:.1}% zeros, {} non-zero bytes", zero_ratio * 100.0, non_zero_count);
        
        // 查找常见的XML模式
        let patterns_to_check = [
            ("<?xml", "XML declaration"),
            ("<ltfsindex", "LTFS index start"),
            ("XMLSchema", "XML Schema reference"),
            ("<directory", "Directory element"), 
            ("</ltfsindex>", "LTFS index end"),
            ("ltfs", "LTFS text (case insensitive)"),
        ];
        
        for (pattern, description) in &patterns_to_check {
            if let Some(pos) = self.find_pattern_in_data(data, pattern.as_bytes()) {
                warn!("🎯 Found {}: '{}' at position {}", description, pattern, pos);
            } else {
                // 大小写不敏感搜索
                let lower_data: Vec<u8> = data.iter().map(|&b| b.to_ascii_lowercase()).collect();
                if let Some(pos) = self.find_pattern_in_data(&lower_data, pattern.to_lowercase().as_bytes()) {
                    warn!("🎯 Found {} (case insensitive): '{}' at position {}", description, pattern, pos);
                }
            }
        }
        
        // 采样数据内容
        let sample_size = std::cmp::min(512, data.len());
        let sample_start = if data.len() > 1024 { 512 } else { 0 };
        let sample_end = std::cmp::min(sample_start + sample_size, data.len());
        
        if sample_start < sample_end {
            let sample_data = &data[sample_start..sample_end];
            let sample_text = String::from_utf8_lossy(sample_data);
            let printable_chars: String = sample_text.chars()
                .take(200)
                .map(|c| if c.is_ascii_graphic() || c.is_whitespace() { c } else { '.' })
                .collect();
            
            warn!("📄 Sample data (bytes {}-{}): {:?}", sample_start, sample_end, printable_chars);
        }
        
        // 检查是否全为特定字符
        if data.iter().all(|&b| b == 0) {
            warn!("⚠️ All data is null bytes - likely unwritten block");
        } else if data.iter().all(|&b| b == 0xFF) {
            warn!("⚠️ All data is 0xFF - possible error condition");
        } else if data.len() == 65536 && non_zero_count < 100 {
            warn!("⚠️ Mostly zeros in 64KB block - typical LTO padding pattern");
        }
        
        // 尝试找到XML的开始和结束
        if let Some(xml_start) = self.find_pattern_in_data(data, b"<") {
            if let Some(xml_end) = self.find_pattern_in_data(&data[xml_start..], b">") {
                let tag_end = xml_start + xml_end + 1;
                if tag_end < data.len() {
                    let first_tag = String::from_utf8_lossy(&data[xml_start..tag_end]);
                    warn!("🏷️ First XML-like tag: {}", first_tag);
                }
            }
        }
    }
}