//! RustLTFS Library
//!
//! A Rust library for IBM LTFS direct tape operations, providing low-level
//! SCSI commands and high-level LTFS file system operations.

pub mod cli;
pub mod error;
pub mod logger;
pub mod ltfs_index;
pub mod scsi;
pub mod tape;
pub mod tape_ops;
pub mod utils;

// Re-export key types for easier use
pub use error::{Result, RustLtfsError};
pub use ltfs_index::{File, FileExtent, LtfsIndex};
pub use scsi::{
    DriveType, LocateDestType, MediaType, ScsiInterface, SpaceType, TapePosition,
};
