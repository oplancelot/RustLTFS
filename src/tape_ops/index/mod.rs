//! LTFS Index Operations Module
//!
//! This module handles all LTFS index-related operations including:
//! - Index reading from tape
//! - Index writing to tape
//! - Index validation and processing

pub mod read;
pub mod write;

// Future: Re-export commonly used items when implementations are complete
// pub use read::*;
// pub use write::*;
