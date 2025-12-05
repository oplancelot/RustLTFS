//! LTFS Index I/O Operations Module
//!
//! This module handles tape-level index I/O operations including:
//! - Reading LTFS index from tape
//! - Writing LTFS index to tape
//! - Index backup and recovery
//!
//! Note: For index data structures and format handling, see `ltfs_index` module.

pub mod read;
pub mod write;
pub mod sync;

// Future: Re-export commonly used items when implementations are complete
// pub use read::*;
// pub use write::*;
