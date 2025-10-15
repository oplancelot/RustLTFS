// Re-export modules
pub mod core;
pub mod read_operations;
pub mod format_operations;
pub mod write_operations;
pub mod partition_manager;
pub mod capacity_manager;
pub mod dual_partition_index;
pub mod deduplication;

pub use self::core::*;
// 选择性导出避免重名冲突
pub use self::format_operations::{MkltfsParams, MkltfsProgressCallback, MkltfsFinishCallback, MkltfsErrorCallback};

use crate::error::{Result, RustLtfsError};
use crate::ltfs_index::LtfsIndex;
use std::path::{Path, PathBuf};
use tracing::info;

/// Format bytes in human-readable format
fn format_bytes(bytes: u64) -> String {
    const UNITS: &[&str] = &["B", "KB", "MB", "GB", "TB", "PB"];
    const THRESHOLD: u64 = 1024;

    if bytes == 0 {
        return "0 B".to_string();
    }

    let mut size = bytes as f64;
    let mut unit_index = 0;

    while size >= THRESHOLD as f64 && unit_index < UNITS.len() - 1 {
        size /= THRESHOLD as f64;
        unit_index += 1;
    }

    if unit_index == 0 {
        format!("{} {}", bytes, UNITS[unit_index])
    } else {
        format!("{:.2} {}", size, UNITS[unit_index])
    }
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

/// LTFS access interface for tape device operations
pub struct LtfsAccess {
    device_path: String,
}

impl LtfsAccess {
    /// Create new LTFS access instance
    pub fn new(device_path: String) -> Self {
        Self { device_path }
    }

    /// Get tape medium information
    pub async fn get_medium_info(&self) -> Result<TapeMediumInfo> {
        // Implementation would go here
        Err(RustLtfsError::unsupported("get_medium_info".to_string()))
    }
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
    pub duplicates_skipped: u64,  // 添加：跳过的重复文件数
    pub space_saved: u64,         // 添加：通过跳过重复文件节省的空间
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
    pub goto_eod_on_write: bool,        // Go to End of Data on write
    pub force_index: bool,              // Force index update
    pub dedupe: bool,                   // Enable deduplication (SHA1-based)
    pub skip_duplicates: bool,          // Skip writing duplicate files (对应LTFSCopyGUI的跳过重复文件)
    pub preload_file_count: u32,        // Number of files to preload
    pub ignore_volume_overflow: bool,   // Ignore volume overflow warnings
    pub auto_clean_enabled: bool,       // Auto clean on write
    pub power_policy_on_write: bool,    // Change power policy during write
    
    // Hash algorithm enables (for compatibility with LTFSCopyGUI settings)
    pub hash_sha1_enabled: bool,
    pub hash_md5_enabled: bool,
    pub hash_blake3_enabled: bool,
    pub hash_sha256_enabled: bool,
    pub hash_xxhash3_enabled: bool,
    pub hash_xxhash128_enabled: bool,
    pub extended_hashing: bool,         // Enable extended hashing algorithms
    pub compatibility_mode: bool,       // MD5 compatibility mode
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
            block_size: crate::scsi::block_sizes::LTO_BLOCK_SIZE,
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

/// View index utilities
pub struct IndexViewer;

impl IndexViewer {
    /// Create a new index viewer instance
    pub fn new() -> Self {
        Self
    }

    /// Load and display index from file
    pub async fn display_index_from_file(&self, index_path: &Path) -> Result<()> {
        let xml_content = tokio::fs::read_to_string(index_path).await.map_err(|e| {
            RustLtfsError::file_operation(format!("Unable to read index file: {}", e))
        })?;

        let index = LtfsIndex::from_xml(&xml_content)?;
        self.display_index_info(&index).await
    }

    /// Display detailed index information
    pub async fn display_index_info(&self, index: &LtfsIndex) -> Result<()> {
        println!("=== LTFS Index Information ===");
        println!("Volume UUID: {}", index.volumeuuid);
        println!("Generation: {}", index.generationnumber);
        println!("Update time: {}", index.updatetime);
        println!("Creator: {}", index.creator);

        // Collect statistics
        let stats = self.collect_statistics(index).await?;
        
        println!("\n=== Statistics ===");
        println!("Total files: {}", stats.total_files);
        println!("Total directories: {}", stats.total_directories);
        println!("Total size: {}", format_bytes(stats.total_size));

        // Display directory tree
        println!("\n=== Directory Structure ===");
        self.display_directory_tree(&index.root_directory, 0).await?;

        Ok(())
    }

    /// Collect statistics from index
    async fn collect_statistics(&self, index: &LtfsIndex) -> Result<IndexStatistics> {
        let mut stats = IndexStatistics {
            total_files: 0,
            total_directories: 0,
            total_size: 0,
            index_generation: index.generationnumber,
            format_time: index.updatetime.clone(),
            volume_uuid: index.volumeuuid.clone(),
        };

        self.collect_directory_stats(&index.root_directory, &mut stats).await?;
        Ok(stats)
    }

    /// Recursively collect directory statistics
    fn collect_directory_stats<'a>(
        &'a self,
        dir: &'a crate::ltfs_index::Directory,
        stats: &'a mut IndexStatistics,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<()>> + 'a>> {
        Box::pin(async move {
            stats.total_directories += 1;

            // Count files in this directory
            for file in &dir.contents.files {
                stats.total_files += 1;
                stats.total_size += file.length;
            }

            // Recursively process subdirectories
            for subdir in &dir.contents.directories {
                self.collect_directory_stats(subdir, stats).await?;
            }

            Ok(())
        })
    }

    /// Display directory tree recursively
    fn display_directory_tree<'a>(
        &'a self,
        dir: &'a crate::ltfs_index::Directory,
        depth: usize,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<()>> + 'a>> {
        Box::pin(async move {
            let indent = "  ".repeat(depth);

            // Display current directory
            if depth > 0 {
                println!("{}📁 {}/", indent, dir.name);
            } else {
                println!("📁 / (root)");
            }

            // Display files in current directory
            for file in &dir.contents.files {
                println!(
                    "{}📄 {} ({}, UID: {})",
                    indent,
                    file.name,
                    format_bytes(file.length),
                    file.uid
                );
            }

            // Recursively display subdirectories
            for subdir in &dir.contents.directories {
                self.display_directory_tree(subdir, depth + 1).await?;
            }

            Ok(())
        })
    }

    /// Find and display specific file information
    pub async fn find_file_info(&self, index: &LtfsIndex, file_uid: u64) -> Result<FileInfo> {
        if let Some(file) = self.find_file_by_uid(&index.root_directory, file_uid) {
            Ok(FileInfo {
                name: file.name.clone(),
                size: file.length,
                modified_time: file.modify_time.clone(),
                uid: file.uid,
                checksum: None, // Would need to be calculated or extracted if available
            })
        } else {
            Err(RustLtfsError::ltfs_index(format!(
                "File with UID {} not found",
                file_uid
            )))
        }
    }

    /// Recursively search for file by UID
    fn find_file_by_uid(
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
            if let Some(found_file) = self.find_file_by_uid(subdir, file_uid) {
                return Some(found_file);
            }
        }

        None
    }
    
    /// 处理查看索引命令
    pub fn handle_view_index_command(
        index_path: &str,
        detailed: Option<bool>,
        format: Option<&str>,
        output_file: Option<&str>,
    ) -> Result<()> {
        info!("Handling view index command for: {}", index_path);
        
        // 读取索引文件
        let xml_content = std::fs::read_to_string(index_path)
            .map_err(|e| RustLtfsError::file_operation(format!("Unable to read index file: {}", e)))?;
        
        let index = LtfsIndex::from_xml(&xml_content)?;
        
        // 根据格式显示索引
        match format.unwrap_or("tree") {
            "tree" => {
                println!("LTFS Index Tree View:");
                Self::print_directory_tree(&index.root_directory, 0);
            }
            "list" => {
                println!("LTFS Index File List:");
                Self::print_file_list(&index.root_directory, "");
            }
            "json" => {
                // 简化的JSON输出
                println!("{{");
                println!("  \"volume_name\": \"{}\",", index.root_directory.name);
                println!("  \"creation_time\": \"{}\",", index.updatetime);
                println!("  \"files\": [");
                Self::print_files_json(&index.root_directory, "");
                println!("  ]");
                println!("}}");
            }
            _ => {
                return Err(RustLtfsError::parameter_validation(
                    "Unsupported format. Use 'tree', 'list', or 'json'".to_string()
                ));
            }
        }
        
        // 如果指定了输出文件，则保存结果（这里简化处理）
        if let Some(output_path) = output_file {
            info!("Output would be saved to: {}", output_path);
        }
        
        Ok(())
    }
    
    fn print_directory_tree(dir: &crate::ltfs_index::Directory, depth: usize) {
        let indent = "  ".repeat(depth);
        
        // 打印文件
        for file in &dir.contents.files {
            println!("{}📄 {} ({} bytes)", indent, file.name, file.length);
        }
        // 打印并递归子目录
        for subdir in &dir.contents.directories {
            println!("{}📁 {}/", indent, subdir.name);
            Self::print_directory_tree(subdir, depth + 1);
        }
    }
    
    fn print_file_list(dir: &crate::ltfs_index::Directory, path_prefix: &str) {
        for file in &dir.contents.files {
            println!("{}{} ({} bytes)", path_prefix, file.name, file.length);
        }
        for subdir in &dir.contents.directories {
            let new_prefix = format!("{}{}/", path_prefix, subdir.name);
            Self::print_file_list(subdir, &new_prefix);
        }
    }
    
    fn print_files_json(dir: &crate::ltfs_index::Directory, path_prefix: &str) {
        for file in &dir.contents.files {
            println!("    {{\"path\": \"{}{}\", \"size\": {}}},", path_prefix, file.name, file.length);
        }
        for subdir in &dir.contents.directories {
            let new_prefix = format!("{}{}/", path_prefix, subdir.name);
            Self::print_files_json(subdir, &new_prefix);
        }
    }
}