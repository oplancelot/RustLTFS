//! Utility functions for RustLTFS

/// Format bytes in human-readable format (B, KB, MB, GB, TB)
pub fn format_bytes(bytes: u64) -> String {
    const UNITS: &[&str] = &["B", "KB", "MB", "GB", "TB", "PB"];
    const THRESHOLD: u64 = 1024;
    
    if bytes == 0 {
        return "0 B".to_string();
    }
    
    let mut size = bytes as f64;
    let mut unit_index = 0;
    
    while size >= THRESHOLD as f64 && unit_index < UNITS.len() - 1 {
        size /= THRESHOLD as f64;
        unit_index += 1;
    }
    
    if unit_index == 0 {
        format!("{} {}", bytes, UNITS[unit_index])
    } else {
        format!("{:.2} {}", size, UNITS[unit_index])
    }
}

/// Format duration in human-readable format
pub fn format_duration(duration_secs: f64) -> String {
    if duration_secs < 60.0 {
        format!("{:.1}s", duration_secs)
    } else if duration_secs < 3600.0 {
        let minutes = (duration_secs / 60.0) as u32;
        let seconds = duration_secs % 60.0;
        format!("{}m {:.1}s", minutes, seconds)
    } else {
        let hours = (duration_secs / 3600.0) as u32;
        let minutes = ((duration_secs % 3600.0) / 60.0) as u32;
        let seconds = duration_secs % 60.0;
        format!("{}h {}m {:.1}s", hours, minutes, seconds)
    }
}

/// Calculate transfer speed in human-readable format
pub fn format_speed(bytes: u64, duration_secs: f64) -> String {
    if duration_secs <= 0.0 {
        return "0 B/s".to_string();
    }
    
    let speed = bytes as f64 / duration_secs;
    format!("{}/s", format_bytes(speed as u64))
}

/// Truncate string to specified length with ellipsis
pub fn truncate_string(s: &str, max_len: usize) -> String {
    if s.len() <= max_len {
        s.to_string()
    } else if max_len <= 3 {
        "...".to_string()
    } else {
        format!("{}...", &s[..max_len - 3])
    }
}

/// Convert file path to display-friendly string
pub fn path_to_string(path: &std::path::Path) -> String {
    path.to_string_lossy().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_format_bytes() {
        assert_eq!(format_bytes(0), "0 B");
        assert_eq!(format_bytes(512), "512 B");
        assert_eq!(format_bytes(1024), "1.00 KB");
        assert_eq!(format_bytes(1048576), "1.00 MB");
        assert_eq!(format_bytes(1073741824), "1.00 GB");
        assert_eq!(format_bytes(1536), "1.50 KB");
    }

    #[test]
    fn test_format_duration() {
        assert_eq!(format_duration(30.5), "30.5s");
        assert_eq!(format_duration(90.0), "1m 30.0s");
        assert_eq!(format_duration(3661.5), "1h 1m 1.5s");
    }

    #[test]
    fn test_format_speed() {
        assert_eq!(format_speed(1024, 1.0), "1.00 KB/s");
        assert_eq!(format_speed(0, 5.0), "0 B/s");
        assert_eq!(format_speed(1024, 0.0), "0 B/s");
    }

    #[test]
    fn test_truncate_string() {
        assert_eq!(truncate_string("hello", 10), "hello");
        assert_eq!(truncate_string("hello world", 8), "hello...");
        assert_eq!(truncate_string("hi", 2), "hi");
        assert_eq!(truncate_string("hello", 3), "...");
    }
}