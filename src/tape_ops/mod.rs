// Re-export modules
pub mod capacity_manager;
pub mod core;
pub mod dual_partition_index;
pub mod read_operations;
pub mod write_operations;
pub mod hash;
pub mod utils;
pub mod volume;

pub use self::core::*;
// 选择性导出避免重名冲突
// (format_operations types were previously re-exported here for MKLTFS.
//  MKLTFS command and related helpers have been removed from the CLI,
//  so the re-export is no longer necessary.)

/// LTFS分区标签结构 (对应LTFSCopyGUI的ltfslabel)
#[derive(Debug, Clone)]
pub struct LtfsPartitionLabel {
    pub blocksize: u32,
}

impl Default for LtfsPartitionLabel {
    fn default() -> Self {
        Self {
            blocksize: crate::scsi::block_sizes::LTO_BLOCK_SIZE, // 默认64KB
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
}




/// Enhanced tape format analysis result (对应增强版VOL1验证)
#[derive(Debug, Clone, PartialEq)]
pub enum TapeFormatAnalysis {
    /// 空白磁带（全零或无数据）
    BlankTape,
    /// 传统磁带格式（ANSI、IBM等）
    LegacyTape(String),
    /// VOL1标签损坏或不可识别
    CorruptedLabel,
    /// 未知格式
    UnknownFormat,
    /// 可能是LTFS但VOL1非标准
    PossibleLTFS,
}



/// Path content types for describing tape path contents



/// Write queue entry for file operations


/// Write progress information
#[derive(Debug, Clone, Default)]
pub struct WriteProgress {

    pub current_files_processed: u64,
    pub current_bytes_processed: u64,
    pub total_bytes_unindexed: u64,

    pub files_written: u64,
    pub bytes_written: u64,

}

/// Write options configuration (Enhanced for LTFSCopyGUI compatibility)
#[derive(Debug, Clone)]
pub struct WriteOptions {

    pub verify: bool,
    pub hash_on_write: bool,
    pub skip_symlinks: bool,

    pub index_write_interval: u64, // bytes


    pub block_size: u32,


    // New LTFSCopyGUI compatible options
    pub goto_eod_on_write: bool,      // Go to End of Data on write
    pub force_index: bool,            // Force index update


    // Hash algorithm enables (for compatibility with LTFSCopyGUI settings)
    pub hash_sha1_enabled: bool,
    pub hash_md5_enabled: bool,
    pub hash_blake3_enabled: bool,

    pub hash_xxhash3_enabled: bool,
    pub hash_xxhash128_enabled: bool,

}

impl Default for WriteOptions {
    fn default() -> Self {
        Self {

            verify: false,
            hash_on_write: true,
            skip_symlinks: false,

            index_write_interval: 38_654_705_664, // 36GiB (matching LTFSCopyGUI)


            block_size: crate::scsi::block_sizes::LTO_BLOCK_SIZE_512K,  // 512KB (LTFSCopyGUI standard)


            // LTFSCopyGUI compatible defaults
            goto_eod_on_write: true,
            force_index: false,


            // Hash algorithms (enable common ones by default)
            hash_sha1_enabled: true,
            hash_md5_enabled: true,
            hash_blake3_enabled: false,

            hash_xxhash3_enabled: false,
            hash_xxhash128_enabled: false,

        }
    }
}




// IndexViewer removed - `view-index` CLI command was deleted and IndexViewer utilities are no longer needed.
// Retained index-related core functionality lives in `ltfs_index` and read_operations modules.
