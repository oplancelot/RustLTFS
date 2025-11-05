use clap::{Parser, Subcommand};
use std::path::PathBuf;

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
    /// 从LTFS磁带读取目录和文件列表
    Read {
        /// Tape device path (e.g. \\.\TAPE0)
        #[arg(short = 't', long = "tape", value_name = "DEVICE")]
        device: String,

        /// Source path in tape (optional - if not provided, list root directory)
        #[arg(value_name = "SOURCE")]
        source: Option<PathBuf>,
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
}

impl Cli {
    pub fn parse_args() -> Self {
        Self::parse()
    }
}
