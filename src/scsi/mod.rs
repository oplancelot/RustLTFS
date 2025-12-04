//! SCSI Module
//!
//! This module provides low-level SCSI operations for tape devices.
//! It is refactored into submodules for better organization.

pub mod constants;
pub mod types;
pub mod ffi;
pub mod core;
mod sense;
mod device;
mod commands;

pub use constants::*;
pub use types::{DriveType, MediaType, TapePosition, SpaceType};
pub use ffi::*;
pub use core::ScsiInterface;
