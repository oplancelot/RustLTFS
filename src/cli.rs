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

    /// Enable verbose output
    #[arg(short, long, global = true)]
    pub verbose: bool,

    /// Specify configuration file path
    #[arg(short, long, global = true)]
    pub config: Option<PathBuf>,
}

#[derive(Subcommand)]
pub enum Commands {
    /// Write files or folders to tape
    Copy {
        /// Source file or folder path
        #[arg(value_name = "SOURCE")]
        source: PathBuf,

        /// Tape device path (e.g. \\.\TAPE0)
        #[arg(value_name = "DEVICE")]
        device: String,

        /// Target tape path
        #[arg(value_name = "DESTINATION")]
        destination: PathBuf,

        /// Skip confirmation prompt
        #[arg(short, long)]
        force: bool,

        /// Verify written data
        #[arg(long)]
        verify: bool,

        /// Show detailed progress
        #[arg(short, long)]
        progress: bool,
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

    /// Read tape content (display or copy to local)
    Read {
        /// Tape device path
        #[arg(value_name = "DEVICE")]
        device: String,

        /// Source path in tape
        #[arg(value_name = "SOURCE")]
        source: PathBuf,

        /// Local destination path (optional - if not provided, display content)
        #[arg(value_name = "DESTINATION")]
        destination: Option<PathBuf>,

        /// Verify read data
        #[arg(long)]
        verify: bool,

        /// Limit output lines for file content display
        #[arg(long, default_value = "50")]
        lines: usize,
    },

    /// Check tape status
    Status {
        /// Tape device path
        #[arg(value_name = "DEVICE")]
        device: String,
    },
}

impl Cli {
    pub fn parse_args() -> Self {
        Self::parse()
    }
}