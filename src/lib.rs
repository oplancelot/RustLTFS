//! RustLTFS Library
//!
//! A Rust library for IBM LTFS direct tape operations, providing low-level
//! SCSI commands and high-level LTFS file system operations.

pub mod cli;
pub mod display;
pub mod error;
pub mod file_ops;
pub mod logger;
pub mod ltfs;
pub mod ltfs_index;
pub mod scsi;
pub mod tape;
pub mod tape_ops;
pub mod utils;

// Re-export key types for easier use
pub use error::{Result, RustLtfsError};
pub use ltfs::{LtfsDirectAccess, TapeCapacity, create_ltfs_access};
pub use scsi::{
    ScsiInterface, MediaType, TapePosition, MamAttribute, SpaceType,
    LocateDestType, DriveType,
    locate_block, locate_with_type, locate_with_drive_type,
    locate_to_filemark, locate_to_eod
};
pub use ltfs_index::{LtfsIndex, PathType, DirectoryEntry, File, FileExtent};

#[cfg(test)]
mod locate_test;
