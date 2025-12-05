use crate::error::Result;
use super::PartitionStrategy;
use super::volume;
use tracing::debug;

// LtfsPartitionLabel 在 format_operations.rs 中定义
// 通过模块重新导出使用

/// TapeOperations读取操作实现
impl super::TapeOperations {
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

    /// Enhanced VOL1 label validation with comprehensive format detection
    /// 增强版 VOL1 标签验证：支持多种磁带格式检测和详细诊断
    /// 
    /// This method delegates to the volume module for cleaner code organization
    pub fn parse_vol1_label(&self, buffer: &[u8]) -> Result<bool> {
        volume::parse_vol1_label(buffer)
    }
}
