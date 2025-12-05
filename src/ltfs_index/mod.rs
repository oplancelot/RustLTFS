//! LTFS Index Module
//!
//! This module provides complete LTFS index data structures and operations.
//! 
//! ## Structure
//! - `types`: Core data structure definitions
//! - `parser`: XML parsing functionality
//! - `serializer`: XML serialization functionality
//! - `validator`: Index validation logic

pub mod types;
pub mod parser;
pub mod validator;
pub mod serializer;

// Re-export public types for convenience
pub use types::{
    LtfsIndex,
    Location,
    Directory,
    DirectoryContents,
    File,
    FileExtent,
    ExtentInfo,
    ExtendedAttributes,
    ExtendedAttribute,
};
