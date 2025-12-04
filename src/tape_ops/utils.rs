//! Utility Functions Module
//!
//! This module provides common utility functions for LTFS tape operations,
//! including timestamp formatting and other helper functions.

/// Generate LTFS-compatible Z-format timestamp (matching LTFSCopyGUI XML format)
/// Converts RFC3339 format with +00:00 to Z format for XML compatibility
pub fn format_ltfs_timestamp(datetime: chrono::DateTime<chrono::Utc>) -> String {
    format!("{}Z", datetime.format("%Y-%m-%dT%H:%M:%S%.9f"))
}

/// Get current timestamp in LTFS-compatible format
pub fn get_current_ltfs_timestamp() -> String {
    format_ltfs_timestamp(chrono::Utc::now())
}

/// Convert system time to LTFS-compatible timestamp
pub fn system_time_to_ltfs_timestamp(time: std::time::SystemTime) -> String {
    let dt: chrono::DateTime<chrono::Utc> = time.into();
    format_ltfs_timestamp(dt)
}
