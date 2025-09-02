use clap::{Parser, Subcommand, ValueEnum};
use std::path::PathBuf;

/// 导出格式选项
#[derive(Debug, Clone, ValueEnum)]
pub enum ExportFormat {
    /// Tab-separated values (Partition, Startblock, Length, Path)
    Tsv,
    /// JSON format
    Json,
    /// XML format
    Xml,
    /// Batch script for file writing
    Batch,
}

#[derive(Parser)]
#[command(name = "rustltfs")]
#[command(about = "A Rust CLI tool for IBM tape direct read/write operations")]
#[command(version = "0.1.0")]
#[command(author = "lance <oplancelot@gmail.com>")]
pub struct Cli {
    #[command(subcommand)]
    pub command: Commands,

    /// Enable verbose output
    #[arg(short, long, global = true)]
    pub verbose: bool,

    /// Specify configuration file path
    #[arg(short, long, global = true)]
    pub config: Option<PathBuf>,
}

#[derive(Subcommand)]
pub enum Commands {
    /// Write files or folders to tape (对应LTFSCopyGUI的写入数据功能)
    /// 
    /// 将本地文件或目录写入到LTFS磁带，写入完成后自动更新索引
    Write {
        /// Source file or folder path
        #[arg(value_name = "SOURCE")]
        source: PathBuf,

        /// Tape device path (e.g. \\.\TAPE0)
        #[arg(short = 't', long = "tape", value_name = "DEVICE")]
        device: String,

        /// Target tape path
        #[arg(value_name = "DESTINATION")]
        destination: PathBuf,

        /// Skip automatic index reading (offline mode)
        #[arg(short = 's', long = "skip-index")]
        skip_index: bool,

        /// Load index from local file instead of tape
        #[arg(short = 'f', long = "index-file", value_name = "INDEX_FILE")]
        index_file: Option<PathBuf>,

        /// Skip confirmation prompt
        #[arg(long = "force")]
        force: bool,

        /// Verify written data
        #[arg(long)]
        verify: bool,

        /// Show detailed progress
        #[arg(short, long)]
        progress: bool,
    },

    /// Read from tape (对应LTFSCopyGUI的读取索引和提取功能)
    /// 
    /// 从LTFS磁带读取文件、目录或查看内容
    Read {
        /// Tape device path (e.g. \\.\TAPE0)
        #[arg(short = 't', long = "tape", value_name = "DEVICE")]
        device: String,

        /// Source path in tape (optional - if not provided, list root)
        #[arg(value_name = "SOURCE")]
        source: Option<PathBuf>,

        /// Local destination path (optional - if not provided, display content)
        #[arg(value_name = "DESTINATION")]
        destination: Option<PathBuf>,

        /// Skip automatic index reading (offline mode)
        #[arg(short = 's', long = "skip-index")]
        skip_index: bool,

        /// Load index from local file instead of tape
        #[arg(short = 'f', long = "index-file", value_name = "INDEX_FILE")]
        index_file: Option<PathBuf>,

        /// Verify read data
        #[arg(long)]
        verify: bool,

        /// Limit output lines for file content display
        #[arg(long, default_value = "50")]
        lines: usize,

        /// Show detailed file information
        #[arg(short = 'd', long = "detailed")]
        detailed: bool,
    },

    /// Load and view local LTFS index file (对应LTFSCopyGUI的索引查看功能)
    /// 
    /// 解析并显示本地保存的LTFS索引文件内容
    ViewIndex {
        /// LTFS index file path (.schema file)
        #[arg(value_name = "INDEX_FILE")]
        index_file: PathBuf,

        /// Show detailed file information
        #[arg(short, long)]
        detailed: bool,

        /// Export file list to specified format
        #[arg(long, value_enum)]
        export_format: Option<ExportFormat>,

        /// Output file for export
        #[arg(short, long)]
        output: Option<PathBuf>,
    },

    /// List tape devices
    List {
        /// Show detailed device information
        #[arg(short, long)]
        detailed: bool,
    },

    /// Get tape information
    Info {
        /// Tape device path
        #[arg(value_name = "DEVICE")]
        device: String,
    },

    /// Check tape status
    Status {
        /// Tape device path
        #[arg(value_name = "DEVICE")]
        device: String,
    },

    /// Diagnose tape drive and media status
    /// 
    /// 诊断磁带驱动器和媒体状态，用于排除故障
    Diagnose {
        /// Tape device path (e.g., \\.\TAPE0)
        #[arg(short = 't', long = "tape", value_name = "DEVICE")]
        device: String,
        
        /// Detailed diagnostic output
        #[arg(short = 'd', long = "detailed")]
        detailed: bool,
        
        /// Test basic read operations
        #[arg(short = 'r', long = "test-read")]
        test_read: bool,
    },

    /// Show tape space information (free/total)
    /// 
    /// 显示磁带的可用空间和总空间信息
    Space {
        /// Tape device path (e.g. \\.\TAPE0)
        #[arg(short = 't', long = "tape", value_name = "DEVICE")]
        device: String,

        /// Skip automatic index reading (offline mode)
        #[arg(short = 's', long = "skip-index")]
        skip_index: bool,

        /// Show detailed space breakdown
        #[arg(short = 'd', long = "detailed")]
        detailed: bool,
    },

    /// Format tape with LTFS file system (对应LTFSCopyGUI的mkltfs功能)
    /// 
    /// 使用LTFS文件系统格式化磁带，创建分区和设置MAM属性
    Mkltfs {
        /// Tape device path (e.g. \\.\TAPE0)
        #[arg(short = 't', long = "tape", value_name = "DEVICE")]
        device: String,

        /// Tape barcode (max 20 ASCII characters)
        #[arg(short = 'b', long = "barcode", value_name = "BARCODE")]
        barcode: Option<String>,

        /// Volume label
        #[arg(short = 'l', long = "label", value_name = "LABEL")]
        volume_label: Option<String>,

        /// Create partitions (0=single, 1=dual partition)
        #[arg(short = 'p', long = "partition", value_name = "COUNT", default_value = "1")]
        partition_count: u8,

        /// Block size in bytes (512-2097152)
        #[arg(long = "block-size", value_name = "SIZE", default_value = "524288")]
        block_size: u32,

        /// Tape capacity (0-65535, 65535=max)
        #[arg(short = 'c', long = "capacity", value_name = "CAPACITY", default_value = "65535")]
        capacity: u16,

        /// P0 partition size in GB (1-65535, 65535=remaining space)
        #[arg(long = "p0-size", value_name = "SIZE", default_value = "1")]
        p0_size: u16,

        /// P1 partition size in GB (1-65535, 65535=remaining space)
        #[arg(long = "p1-size", value_name = "SIZE", default_value = "65535")]
        p1_size: u16,

        /// Run in immediate mode (async)
        #[arg(long = "immediate")]
        immediate: bool,

        /// Skip confirmation prompt
        #[arg(long = "force")]
        force: bool,

        /// Show progress information
        #[arg(long = "progress")]
        progress: bool,
    },

    /// Read LTFS index from tape (对应LTFSCopyGUI的读取索引功能)
    /// 
    /// 从LTFS磁带的索引分区读取最新的LTFS索引文件并保存到本地
    ReadIndex {
        /// Tape device path (e.g. \\.\TAPE0)
        #[arg(short = 't', long = "tape", value_name = "DEVICE")]
        device: String,

        /// Output file path for the index
        #[arg(short = 'o', long = "output", value_name = "OUTPUT")]
        output: Option<PathBuf>,

        /// Show detailed information during operation
        #[arg(short = 'd', long = "detailed")]
        detailed: bool,
    },

    /// Read LTFS index from data partition (对应LTFSCopyGUI的读取数据区索引功能)
    /// 
    /// 从数据分区末尾读取最新写入的LTFS索引副本，用于数据恢复
    ReadDataIndex {
        /// Tape device path (e.g. \\.\TAPE0)
        #[arg(short = 't', long = "tape", value_name = "DEVICE")]
        device: String,

        /// Output file path for the index
        #[arg(short = 'o', long = "output", value_name = "OUTPUT")]
        output: Option<PathBuf>,

        /// Show detailed information during operation
        #[arg(short = 'd', long = "detailed")]
        detailed: bool,
    },

    /// Update LTFS index on tape (对应LTFSCopyGUI的更新数据区索引功能)
    /// 
    /// 手动触发LTFS索引更新，将当前索引写入数据分区末尾
    UpdateIndex {
        /// Tape device path (e.g. \\.\TAPE0)
        #[arg(short = 't', long = "tape", value_name = "DEVICE")]
        device: String,

        /// Force update even if no changes detected
        #[arg(short = 'f', long = "force")]
        force: bool,

        /// Show detailed information during operation
        #[arg(short = 'd', long = "detailed")]
        detailed: bool,
    },
}

impl Cli {
    pub fn parse_args() -> Self {
        Self::parse()
    }
}