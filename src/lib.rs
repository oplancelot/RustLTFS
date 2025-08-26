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

// Re-export key types for easier use
pub use error::{Result, RustLtfsError};
<<<<<<< HEAD
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
=======
pub use ltfs::{create_ltfs_access, LtfsDirectAccess, TapeCapacity};
pub use ltfs_index::{DirectoryEntry, File, FileExtent, LtfsIndex, PathType};
pub use scsi::{MamAttribute, MediaType, ScsiInterface, SpaceType, TapePosition};

// #[cfg(test)]
// mod tests;

// #[cfg(test)]
// mod xml_test;

// #[cfg(test)]
// mod ltfs_index_real_test;
>>>>>>> e8bb472 (✨ 精准实现LTFSCopyGUI索引读取机制并优化代码结构)
