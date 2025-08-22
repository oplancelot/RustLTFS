//! RustLTFS Library
//! 
//! A Rust library for IBM LTFS direct tape operations, providing low-level
//! SCSI commands and high-level LTFS file system operations.

pub mod cli;
pub mod error;
pub mod logger;
pub mod scsi;
pub mod tape;
pub mod ltfs;
pub mod ltfs_index;
pub mod file_ops;
pub mod display;

// Re-export key types for easier use
pub use error::{Result, RustLtfsError};
pub use ltfs::{LtfsDirectAccess, TapeCapacity, create_ltfs_access};
pub use scsi::{ScsiInterface, MediaType, TapePosition, MamAttribute, SpaceType};
pub use ltfs_index::{LtfsIndex, PathType, DirectoryEntry, File, FileExtent};

#[cfg(test)]
mod tests;