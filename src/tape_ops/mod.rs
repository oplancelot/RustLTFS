// Re-export modules
pub mod capacity_manager;
pub mod core;
pub mod dual_partition_index;
pub mod index_validator;
pub mod partition_manager;
pub mod read_operations;
pub mod write_operations;

pub use self::core::*;
// 选择性导出避免重名冲突
// (format_operations types were previously re-exported here for MKLTFS.
//  MKLTFS command and related helpers have been removed from the CLI,
//  so the re-export is no longer necessary.)

use std::path::PathBuf;

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

/// LTFS格式化状态枚举（基于LTFSCopyGUI的检测策略）
#[derive(Debug, Clone, PartialEq)]
pub enum LtfsFormatStatus {
    /// 磁带已正常格式化为LTFS（包含索引大小）
    LtfsFormatted(usize),
    /// 磁带为空白（未写入任何数据）
    BlankTape,
    /// 磁带有数据但不是LTFS格式
    NonLtfsFormat,
    /// LTFS索引损坏或不完整
    CorruptedIndex,
    /// 磁带定位失败
    PositioningFailed,
    /// 硬件错误或通信问题
    HardwareError,
    /// 未知状态（无法确定）
    Unknown,
}

impl LtfsFormatStatus {
    /// Get human-readable description of the status
    pub fn description(&self) -> &'static str {
        match self {
            LtfsFormatStatus::LtfsFormatted(_) => "LTFS formatted tape with valid index",
            LtfsFormatStatus::BlankTape => "Blank tape (no data written)",
            LtfsFormatStatus::NonLtfsFormat => "Non-LTFS formatted tape",
            LtfsFormatStatus::CorruptedIndex => "LTFS tape with corrupted index",
            LtfsFormatStatus::PositioningFailed => "Failed to position on tape",
            LtfsFormatStatus::HardwareError => "Hardware communication error",
            LtfsFormatStatus::Unknown => "Unknown format status",
        }
    }

    /// Check if the tape is usable for LTFS operations
    pub fn is_usable(&self) -> bool {
        matches!(self, LtfsFormatStatus::LtfsFormatted(_))
    }

    /// Check if the tape is LTFS formatted
    pub fn is_ltfs_formatted(&self) -> bool {
        matches!(self, LtfsFormatStatus::LtfsFormatted(_))
    }
}

/// Path content types for describing tape path contents
#[derive(Debug, Clone)]
pub enum PathContent {
    /// Directory containing other files and directories
    Directory {
        entries: Vec<DirectoryEntry>,
        subdirs: Vec<String>,
    },
    /// Regular file
    File(FileInfo),
    /// Path does not exist
    NotFound,
}

/// Directory entry information
#[derive(Debug, Clone)]
pub struct DirectoryEntry {
    pub name: String,
    pub is_directory: bool,
    pub size: Option<u64>,
    pub modified_time: Option<String>,
    pub uid: Option<u64>,
}

/// File information
#[derive(Debug, Clone)]
pub struct FileInfo {
    pub name: String,
    pub size: u64,
    pub modified_time: String,
    pub uid: u64,
    pub checksum: Option<String>,
}

/// Extraction result information
#[derive(Debug, Clone)]
pub struct ExtractionResult {
    pub extracted_files: Vec<String>,
    pub total_bytes: u64,
    pub errors: Vec<String>,
    pub skipped_files: Vec<String>,
}

/// Tape medium information including barcode
#[derive(Debug, Clone)]
pub struct TapeMediumInfo {
    pub barcode: String,
    pub volume_uuid: String,
    pub format_time: String,
    pub blocksize: u32,
}

/// Tape space information
#[derive(Debug, Clone)]
pub struct TapeSpaceInfo {
    pub used_capacity: u64,
    pub remaining_capacity: u64,
    pub total_capacity: u64,
    pub compression_ratio: f64,
}

/// Write queue entry for file operations
#[derive(Debug, Clone)]
pub struct FileWriteEntry {
    pub source_path: PathBuf,
    pub target_path: String,
    pub tape_path: String,
    pub file_size: u64,
    pub size: u64,
    pub is_directory: bool,
    pub preserve_permissions: bool,
    pub modified: bool,
    pub overwrite: bool,
    pub hash: Option<String>,
}

/// Write progress information
#[derive(Debug, Clone, Default)]
pub struct WriteProgress {
    pub total_files_processed: u64,
    pub current_files_processed: u64,
    pub total_bytes_processed: u64,
    pub current_bytes_processed: u64,
    pub total_bytes_unindexed: u64,
    pub files_in_queue: usize,
    pub files_written: u64,
    pub bytes_written: u64,
    pub total_files: u64,
    pub total_bytes: u64,
    pub current_file: String,
    pub errors: Vec<String>,
    pub duplicates_skipped: u64, // 添加：跳过的重复文件数
    pub space_saved: u64,        // 添加：通过跳过重复文件节省的空间
}

/// Write options configuration (Enhanced for LTFSCopyGUI compatibility)
#[derive(Debug, Clone)]
pub struct WriteOptions {
    pub overwrite: bool,
    pub verify: bool,
    pub hash_on_write: bool,
    pub skip_symlinks: bool,
    pub parallel_add: bool,
    pub speed_limit: Option<u32>,  // MiB/s
    pub index_write_interval: u64, // bytes
    pub excluded_extensions: Vec<String>,
    pub compression: bool,
    pub verify_writes: bool,
    pub preserve_permissions: bool,
    pub block_size: u32,
    pub buffer_size: usize,
    pub max_retry_attempts: u32,

    // New LTFSCopyGUI compatible options
    pub goto_eod_on_write: bool,      // Go to End of Data on write
    pub force_index: bool,            // Force index update
    pub dedupe: bool,                 // Enable deduplication (SHA1-based)
    pub skip_duplicates: bool, // Skip writing duplicate files (对应LTFSCopyGUI的跳过重复文件)
    pub preload_file_count: u32, // Number of files to preload
    pub ignore_volume_overflow: bool, // Ignore volume overflow warnings
    pub auto_clean_enabled: bool, // Auto clean on write
    pub power_policy_on_write: bool, // Change power policy during write

    // Hash algorithm enables (for compatibility with LTFSCopyGUI settings)
    pub hash_sha1_enabled: bool,
    pub hash_md5_enabled: bool,
    pub hash_blake3_enabled: bool,
    pub hash_sha256_enabled: bool,
    pub hash_xxhash3_enabled: bool,
    pub hash_xxhash128_enabled: bool,
    pub extended_hashing: bool,   // Enable extended hashing algorithms
    pub compatibility_mode: bool, // MD5 compatibility mode
}

impl Default for WriteOptions {
    fn default() -> Self {
        Self {
            overwrite: false,
            verify: false,
            hash_on_write: true,
            skip_symlinks: false,
            parallel_add: true,
            speed_limit: None,
            index_write_interval: 38_654_705_664, // 36GiB (matching LTFSCopyGUI)
            excluded_extensions: vec![".xattr".to_string()],
            compression: false,
            verify_writes: true,
            preserve_permissions: true,
            block_size: crate::scsi::block_sizes::LTO_BLOCK_SIZE_512K,  // 512KB (LTFSCopyGUI standard)
            buffer_size: 1024 * 1024, // 1MB
            max_retry_attempts: 3,

            // LTFSCopyGUI compatible defaults
            goto_eod_on_write: true,
            force_index: false,
            dedupe: false,
            skip_duplicates: true, // 默认跳过重复文件以节省空间
            preload_file_count: 8,
            ignore_volume_overflow: false,
            auto_clean_enabled: false,
            power_policy_on_write: false,

            // Hash algorithms (enable common ones by default)
            hash_sha1_enabled: true,
            hash_md5_enabled: true,
            hash_blake3_enabled: false,
            hash_sha256_enabled: true,
            hash_xxhash3_enabled: false,
            hash_xxhash128_enabled: false,
            extended_hashing: false,
            compatibility_mode: true, // 默认启用MD5兼容模式
        }
    }
}

/// Tape capacity information (对应LTFSCopyGUI的容量信息)
#[derive(Debug, Clone)]
pub struct TapeCapacityInfo {
    pub total_capacity: u64,
    pub used_capacity: u64,
    pub available_capacity: u64,
    pub compression_ratio: f64,
    pub estimated_remaining_hours: f32,
}

/// Drive cleaning status (对应LTFSCopyGUI的清洁状态)
#[derive(Debug, Clone)]
pub struct CleaningStatus {
    pub cleaning_required: bool,
    pub cleaning_media_expired: bool,
    pub operations_since_clean: u32,
}

/// Encryption status (对应LTFSCopyGUI的加密状态)
#[derive(Debug, Clone)]
pub struct EncryptionStatus {
    pub enabled: bool,
    pub key_format: String,
    pub method: String,
}

/// Write result information
#[derive(Debug, Clone)]
pub struct WriteResult {
    pub position: crate::scsi::TapePosition,
    pub blocks_written: u32,
    pub bytes_written: u64,
}

/// Index statistics structure
#[derive(Debug, Clone)]
pub struct IndexStatistics {
    pub total_files: u64,
    pub total_directories: u64,
    pub total_size: u64,
    pub index_generation: u64,
    pub format_time: String,
    pub volume_uuid: String,
}

// IndexViewer removed - `view-index` CLI command was deleted and IndexViewer utilities are no longer needed.
// Retained index-related core functionality lives in `ltfs_index` and read_operations modules.
