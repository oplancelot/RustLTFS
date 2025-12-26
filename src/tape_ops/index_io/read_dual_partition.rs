use crate::error::{Result, RustLtfsError};
use tracing::{debug, info};
use crate::scsi::block_sizes;

/// TapeOperations 双分区读取操作实现
impl super::super::TapeOperations {
    /// 双分区模式：从索引分区读取索引（FileMark 3）
    /// 对应 VB.NET: TapeUtils.Locate(driveHandle, 3UL, IndexPartition, FileMark)
    pub(super) async fn try_read_index_dual_partition(&mut self) -> Result<String> {
        info!("Reading index from dual-partition tape (FileMark 3 strategy)");
        
        let index_partition = 0u8;  // 索引分区固定为0
        
        // Step 1: 定位到 FileMark 3
        debug!("Step 1: Locating to FileMark 3 on index partition");
        self.scsi.locate_to_filemark(3, index_partition)?;  // filemark 3, partition 0
        
        let position = self.scsi.read_position()?;
        debug!(
            "Positioned after FileMark 3: P{} B{} FM{}",
            position.partition, position.block_number, position.file_number
        );
        
        // Step 2: ReadFileMark - 跳过FileMark标记
        debug!("Step 2: Skipping FileMark using ReadFileMark");
        self.scsi.read_file_mark()?;
        
        // Step 3: ReadToFileMark - 读取索引
        debug!("Step 3: Reading index content");
        let block_size = self
            .partition_label
            .as_ref()
            .map(|plabel| plabel.blocksize as usize)
            .unwrap_or(block_sizes::LTO_BLOCK_SIZE as usize);
        
        let index_data = self.read_to_file_mark_with_temp_file(block_size)?;
        
        Ok(index_data)
    }

    /// 按照LTFSCopyGUI逻辑从数据分区EOD读取最新索引（双分区专用）
    /// 对应VB.NET读取数据区索引ToolStripMenuItem_Click的核心逻辑
    pub(super) async fn read_index_from_data_partition_eod(&mut self) -> Result<String> {
        info!("Reading latest index from data partition end");

        let data_partition = 1; // 数据分区

        // Step 1: 定位到数据分区EOD
        info!("Locating to data partition {} EOD", data_partition);

        self.scsi.locate_block(data_partition, 0)?;
        info!("Successfully positioned to data partition {}, block 0", data_partition);

        // 使用LOCATE命令进行EOD定位
        info!("Using LOCATE command for end-of-data positioning");
        self.scsi.locate_to_eod(data_partition)?;
        info!("Successfully located to End of Data in partition {}", data_partition);

        let eod_position = self.scsi.read_position()?;
        info!(
            "Data partition EOD position: partition={}, block={}, file_number={}",
            eod_position.partition, eod_position.block_number, eod_position.file_number
        );

        // Step 2: 检查FileNumber
        if eod_position.file_number <= 1 {
            return Err(RustLtfsError::ltfs_index(
                "Insufficient file marks in data partition for index reading".to_string(),
            ));
        }

        // Step 3: 定位到FM-1
        let target_filemark = eod_position.file_number - 1;
        info!("Locating to FileMark {} in data partition", target_filemark);
        self.scsi.locate_to_filemark(target_filemark, data_partition)?;
        info!("Successfully positioned to FileMark {}", target_filemark);

        // Step 4: 跳过FileMark并读取索引
        self.scsi.space(crate::scsi::SpaceType::FileMarks, 1)?;
        info!("Skipped FileMark, now reading latest index content");

        let position_after_fm = self.scsi.read_position()?;
        info!(
            "Position after FileMark: partition={}, block={}",
            position_after_fm.partition, position_after_fm.block_number
        );

        // 读取索引内容
        let block_size = self
            .partition_label
            .as_ref()
            .map(|plabel| plabel.blocksize as usize)
            .unwrap_or(block_sizes::LTO_BLOCK_SIZE as usize);

        let xml_content = self.read_to_file_mark_with_temp_file(block_size)?;

        if xml_content.contains("<ltfsindex") && xml_content.contains("</ltfsindex>") {
            info!("✅ Successfully read latest index from data partition EOD at FileMark {}", target_filemark);
            Ok(xml_content)
        } else {
            Err(RustLtfsError::ltfs_index(
                "Content at data partition EOD is not valid LTFS index".to_string(),
            ))
        }
    }
}
