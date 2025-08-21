use clap::{Parser, Subcommand};
use std::path::PathBuf;

#[derive(Parser)]
#[command(name = "rustltfs")]
#[command(about = "A Rust CLI tool for IBM tape direct read/write operations")]
#[command(version = "0.1.0")]
#[command(author = "Your Name <your.email@example.com>")]
pub struct Cli {
    #[command(subcommand)]
    pub command: Commands,

    /// 启用详细输出
    #[arg(short, long, global = true)]
    pub verbose: bool,

    /// 指定配置文件路径
    #[arg(short, long, global = true)]
    pub config: Option<PathBuf>,
}

#[derive(Subcommand)]
pub enum Commands {
    /// 将文件或文件夹写入到磁带
    Copy {
        /// 源文件或文件夹路径
        #[arg(value_name = "SOURCE")]
        source: PathBuf,

        /// 磁带设备路径 (例如: \\.\TAPE0)
        #[arg(value_name = "DEVICE")]
        device: String,

        /// 目标磁带路径
        #[arg(value_name = "DESTINATION")]
        destination: PathBuf,

        /// 跳过确认提示
        #[arg(short, long)]
        force: bool,

        /// 验证写入数据
        #[arg(short, long)]
        verify: bool,

        /// 显示详细进度
        #[arg(short, long)]
        progress: bool,
    },

    /// 列出磁带设备
    List {
        /// 显示详细设备信息
        #[arg(short, long)]
        detailed: bool,
    },

    /// 获取磁带信息
    Info {
        /// 磁带设备路径
        #[arg(value_name = "DEVICE")]
        device: String,
    },

    /// 读取磁带内容到本地
    Read {
        /// 磁带设备路径
        #[arg(value_name = "DEVICE")]
        device: String,

        /// 磁带中的源路径
        #[arg(value_name = "SOURCE")]
        source: PathBuf,

        /// 本地目标路径
        #[arg(value_name = "DESTINATION")]
        destination: PathBuf,

        /// 验证读取数据
        #[arg(short, long)]
        verify: bool,
    },

    /// 检查磁带状态
    Status {
        /// 磁带设备路径
        #[arg(value_name = "DEVICE")]
        device: String,
    },
}

impl Cli {
    pub fn parse_args() -> Self {
        Self::parse()
    }
}