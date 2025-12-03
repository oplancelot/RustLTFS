use clap::builder::styling::AnsiColor;
use clap::builder::Styles;
use clap::{Parser, Subcommand};
use std::path::PathBuf;

const CLAP_STYLING: Styles = Styles::styled()
    .header(AnsiColor::BrightGreen.on_default().bold())
    .usage(AnsiColor::BrightGreen.on_default().bold())
    .literal(AnsiColor::BrightCyan.on_default().bold())
    .placeholder(AnsiColor::BrightCyan.on_default());

#[derive(Parser)]
#[command(name = "rustltfs")]
#[command(about = "A Rust CLI tool for IBM tape direct read/write operations")]
#[command(version)]
#[command(author = "lance <oplancelot@gmail.com>")]
#[command(styles = CLAP_STYLING)]
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
        /// Tape device path (e.g. \\.\TAPE0)
        #[arg(short = 't', long = "tape", value_name = "DEVICE")]
        device: String,

        /// Target tape path
        #[arg(short = 'o', long = "output", value_name = "DESTINATION")]
        destination: PathBuf,

        /// Source file or folder path (if not provided, read from stdin)
        #[arg(value_name = "SOURCE")]
        source: Option<PathBuf>,

        /// Verify written data using hash comparison
        #[arg(long)]
        verify: bool,

        /// Show detailed progress information
        #[arg(short, long)]
        progress: bool,
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
