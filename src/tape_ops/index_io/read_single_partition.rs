use crate::error::Result;
use tracing::{debug, info};
use crate::scsi::block_sizes;

/// TapeOperations 单分区读取操作实现
impl super::super::TapeOperations {
    /// 单分区模式：从partition 0读取索引（FM-1策略）
    /// 对应 VB.NET: Locate(EOD) -> Locate(FM-1)
    pub(super) async fn try_read_index_single_partition(&mut self) -> Result<String> {
        info!("Reading index from single-partition tape (FM-1 strategy)");
        
        let partition = 0u8;
        
        // Step 1: 定位到EOD
        debug!("Step 1: Locating to partition 0 EOD");
        self.scsi.locate_to_eod(partition)?;
        
        let eod_position = self.scsi.read_position()?;
        debug!(
            "EOD position: P{} B{} FM{}",
            eod_position.partition, eod_position.block_number, eod_position.file_number
        );
        
        // Step 2: 检查FileNumber
        if eod_position.file_number <= 1 {
            debug!("FileMark number {} <= 1, attempting fallback strategy", eod_position.file_number);
            return self.ltfscopygui_disable_partition_fallback();
        }
        
        // Step 3: 定位到 FM-1
        let target_fm = eod_position.file_number - 1;
        debug!("Step 3: Locating to FileMark {} (FM-1)", target_fm);
        self.scsi.locate_to_filemark(target_fm as u64, partition)?;  // filemark FM-1, partition 0
        
        // Step 4: ReadFileMark - 跳过FileMark标记
        debug!("Step 4: Skipping FileMark using ReadFileMark");
        self.scsi.read_file_mark()?;
        
        // Step 5: ReadToFileMark - 读取索引
        debug!("Step 5: Reading index content");
        let block_size = self
            .partition_label
            .as_ref()
            .map(|plabel| plabel.blocksize as usize)
            .unwrap_or(block_sizes::LTO_BLOCK_SIZE as usize);
        
        let index_data = self.read_to_file_mark_with_temp_file(block_size)?;
        
        // 🎯 验证并处理内容 (LTFSCopyGUI逻辑)
        let xml_content = index_data;
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

    /// LTFSCopyGUI的DisablePartition后备策略 (对应TapeUtils.Space6(-2, FileMark))
    pub(super) fn ltfscopygui_disable_partition_fallback(&mut self) -> Result<String> {
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

        // 🎯 验证并处理内容
        let xml_content = String::from_utf8_lossy(&index_data).to_string();
        if xml_content.contains("XMLSchema") {
            debug!("✅ Successfully read LTFS index using DisablePartition fallback: {} bytes (contains XMLSchema)", xml_content.len());
            Ok(xml_content)
        } else {
            // 🔧 LTFSCopyGUI备选路径
            let processed_content = self.ltfscopygui_from_schema_text(xml_content)?;
            info!(
                "✅ Successfully processed LTFS schema text format: {} bytes",
                processed_content.len()
            );
            Ok(processed_content)
        }
    }
}
