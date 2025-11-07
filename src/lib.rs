//! RustLTFS Library
//!
//! A Rust library for IBM LTFS direct tape operations, providing low-level
//! SCSI commands and high-level LTFS file system operations.

pub mod cli;
pub mod display;
pub mod error;
pub mod logger;
pub mod ltfs_index;
pub mod scsi;
pub mod tape;
pub mod tape_ops;
pub mod utils;

// Re-export key types for easier use
pub use error::{Result, RustLtfsError};
pub use ltfs_index::{DirectoryEntry, File, FileExtent, LtfsIndex, PathType};
pub use scsi::{
    locate_block, locate_to_eod, locate_to_filemark, locate_with_drive_type, locate_with_type,
    DriveType, LocateDestType, MamAttribute, MediaType, ScsiInterface, SpaceType, TapePosition,
};
