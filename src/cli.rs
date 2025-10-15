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

/// 报告类型选项
#[derive(Debug, Clone, ValueEnum)]
pub enum ReportType {
    /// Summary report of all devices
    Summary,
    /// Detailed report for specific device
    Detailed,
    /// CSV inventory of all devices
    Inventory,
    /// Performance metrics report
    Performance,
    /// Health status report
    Health,
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

        /// Overwrite existing files without confirmation
        #[arg(long = "force")]
        force: bool,

        /// Verify written data using hash comparison
        #[arg(long)]
        verify: bool,

        /// Show detailed progress information
        #[arg(short, long)]
        progress: bool,

        /// Skip symbolic links during write operations
        #[arg(long = "skip-symlinks")]
        skip_symlinks: bool,

        /// Enable parallel file processing (faster but higher memory usage)
        #[arg(long = "parallel")]
        parallel: bool,

        /// Limit write speed in MiB/s (e.g., 100 for 100 MiB/s)
        #[arg(long = "speed-limit", value_name = "MBPS")]
        speed_limit: Option<u32>,

        /// Index update interval in GiB (default: 36 GiB)
        #[arg(long = "index-interval", value_name = "GIB", default_value = "36")]
        index_interval: u32,

        /// File extensions to exclude (comma-separated, e.g., .tmp,.log)
        #[arg(long = "exclude", value_name = "EXTENSIONS")]
        exclude_extensions: Option<String>,

        /// Resume write from previous interruption
        #[arg(long = "resume")]
        resume: bool,

        /// Dry run mode - show what would be written without actual operation
        #[arg(long = "dry-run")]
        dry_run: bool,

        /// Set compression level (0=none, 1=low, 2=medium, 3=high)
        #[arg(long = "compress", value_name = "LEVEL")]
        compression_level: Option<u8>,

        /// Enable encryption with password prompt
        #[arg(long = "encrypt")]
        encrypt: bool,

        /// Create checkpoint every N files (useful for large operations)
        #[arg(long = "checkpoint", value_name = "COUNT")]
        checkpoint_interval: Option<u32>,

        /// Maximum file size to process in one operation (in GiB)
        #[arg(long = "max-file-size", value_name = "GIB")]
        max_file_size: Option<u32>,

        /// Use quiet mode (minimal output)
        #[arg(short = 'q', long = "quiet")]
        quiet: bool,
    },

    /// Read from tape (对应LTFSCopyGUI的读取索引和提取功能)
    /// 
    /// 从LTFS磁带下载文件或目录
    Read {
        /// Tape device path (e.g. \\.\TAPE0)
        #[arg(short = 't', long = "tape", value_name = "DEVICE")]
        device: String,

        /// Source path in tape (optional - if not provided, list root directory)
        #[arg(value_name = "SOURCE")]
        source: Option<PathBuf>,

        /// Local destination path (optional - if not provided, download to current directory)
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

    /// Device management and monitoring commands
    /// 
    /// 磁带设备管理和监控功能，包括设备发现、状态检查和报告生成
    Device {
        #[command(subcommand)]
        action: DeviceAction,
    },
}

#[derive(Subcommand)]
pub enum DeviceAction {
    /// Discover available tape devices
    /// 
    /// 扫描系统中可用的磁带设备
    Discover {
        /// Show detailed device information
        #[arg(short = 'd', long = "detailed")]
        detailed: bool,
    },

    /// Show device status and health information
    /// 
    /// 显示指定设备的详细状态和健康信息
    Status {
        /// Tape device path (e.g. \\.\TAPE0)
        #[arg(value_name = "DEVICE")]
        device: String,

        /// Enable continuous monitoring mode
        #[arg(short = 'm', long = "monitor")]
        monitor: bool,

        /// Monitoring interval in seconds (default: 30)
        #[arg(short = 'i', long = "interval", default_value = "30")]
        interval: u64,
    },

    /// Generate device reports
    /// 
    /// 生成设备状态报告和清单
    Report {
        /// Report type
        #[arg(short = 't', long = "type", value_enum, default_value = "summary")]
        report_type: ReportType,

        /// Specific device to report on (optional)
        #[arg(short = 'd', long = "device")]
        device: Option<String>,

        /// Output file path (optional, prints to stdout if not specified)
        #[arg(short = 'o', long = "output")]
        output: Option<PathBuf>,
    },

    /// Run health check on devices
    /// 
    /// 对指定设备执行健康检查
    HealthCheck {
        /// Tape device path (e.g. \\.\TAPE0), or 'all' for all devices
        #[arg(value_name = "DEVICE")]
        device: String,

        /// Run comprehensive health check
        #[arg(short = 'c', long = "comprehensive")]
        comprehensive: bool,
    },
}

impl Cli {
    pub fn parse_args() -> Self {
        Self::parse()
    }
}