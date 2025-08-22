use crate::error::Result;
use crate::ltfs_index::{DirectoryEntry, File};
use crate::ltfs::LtfsDirectAccess;
use tracing::{debug, info};
use chrono::{DateTime, Utc};

/// Display directory listing to stdout
pub fn display_directory_listing(entries: Vec<DirectoryEntry>) {
    println!("{:<40} {:>12} {:>20}", "Name", "Size", "Modified");
    println!("{:-<74}", "");
    
    for entry in entries {
        match entry {
            DirectoryEntry::Directory(dir) => {
                println!("{:<40} {:>12} {:>20}", 
                    format!("{}/", dir.name), 
                    "-", 
                    format_time(&dir.modify_time)
                );
            },
            DirectoryEntry::File(file) => {
                println!("{:<40} {:>12} {:>20}", 
                    file.name, 
                    format_size(file.length), 
                    format_time(&file.modify_time)
                );
            }
        }
    }
}

/// Display file content to stdout  
pub async fn display_file_content(ltfs: &LtfsDirectAccess, file: &File, max_lines: usize) -> Result<()> {
    info!("Displaying file content: {} (max {} lines)", file.name, max_lines);
    
    // Read initial content to determine if it's text or binary
    let sample_size = std::cmp::min(8192, file.total_size());
    let content = ltfs.read_file_content(file, 0, Some(sample_size)).await?;
    
    if is_text_content(&content) {
        display_text_content(&content, max_lines, file.total_size() > sample_size);
    } else {
        display_binary_content(&content, file);
    }
    
    Ok(())
}

/// Display text content with line limiting
fn display_text_content(content: &[u8], max_lines: usize, is_truncated: bool) {
    let text = String::from_utf8_lossy(content);
    let lines: Vec<&str> = text.lines().collect();
    
    let display_lines = std::cmp::min(lines.len(), max_lines);
    
    for i in 0..display_lines {
        println!("{}", lines[i]);
    }
    
    if lines.len() > max_lines {
        println!("... ({} more lines, use destination parameter to save full file)", 
            lines.len() - max_lines);
    } else if is_truncated {
        println!("... (file truncated for preview, use destination parameter to save full file)");
    }
}

/// Display binary content as hex dump
fn display_binary_content(content: &[u8], file: &File) {
    println!("Binary file: {} ({} bytes)", file.name, file.total_size());
    println!("Hex dump (first {} bytes):", content.len());
    println!();
    
    for (i, chunk) in content.chunks(16).enumerate() {
        print!("{:08x}  ", i * 16);
        
        // Hex bytes
        for (j, byte) in chunk.iter().enumerate() {
            if j == 8 { print!(" "); }
            print!("{:02x} ", byte);
        }
        
        // Padding for incomplete lines
        for j in chunk.len()..16 {
            if j == 8 { print!(" "); }
            print!("   ");
        }
        
        print!(" |");
        
        // ASCII representation
        for byte in chunk {
            let ch = if byte.is_ascii_graphic() || *byte == b' ' {
                *byte as char
            } else {
                '.'
            };
            print!("{}", ch);
        }
        
        println!("|");
        
        // Limit hex dump to first few lines
        if i >= 10 {
            println!("... (use destination parameter to save full file)");
            break;
        }
    }
}

/// Check if content appears to be text
fn is_text_content(content: &[u8]) -> bool {
    if content.is_empty() {
        return true;
    }
    
    // Check for null bytes (common in binary files)
    if content.contains(&0) {
        return false;
    }
    
    // Count printable ASCII characters
    let printable_count = content.iter()
        .filter(|&&b| b.is_ascii_graphic() || b.is_ascii_whitespace())
        .count();
    
    // If more than 95% of characters are printable ASCII, consider it text
    let ratio = printable_count as f64 / content.len() as f64;
    ratio > 0.95
}

/// Format file size in human readable format
pub fn format_size(size: u64) -> String {
    const UNITS: &[&str] = &["B", "KB", "MB", "GB", "TB"];
    const THRESHOLD: f64 = 1024.0;
    
    if size == 0 {
        return "0 B".to_string();
    }
    
    let mut size_f = size as f64;
    let mut unit_index = 0;
    
    while size_f >= THRESHOLD && unit_index < UNITS.len() - 1 {
        size_f /= THRESHOLD;
        unit_index += 1;
    }
    
    if unit_index == 0 {
        format!("{} {}", size, UNITS[unit_index])
    } else {
        format!("{:.1} {}", size_f, UNITS[unit_index])
    }
}

/// Format timestamp in readable format
pub fn format_time(timestamp: &str) -> String {
    // LTFS timestamps are in ISO 8601 format: "2023-01-01T00:00:00.000000000Z"
    match DateTime::parse_from_rfc3339(timestamp) {
        Ok(dt) => dt.format("%Y-%m-%d %H:%M").to_string(),
        Err(_) => {
            // Try alternative parsing if RFC3339 fails
            if let Ok(dt) = DateTime::parse_from_str(timestamp, "%Y-%m-%dT%H:%M:%S%.fZ") {
                dt.format("%Y-%m-%d %H:%M").to_string()
            } else {
                // Fallback to original string if parsing fails
                timestamp.to_string()
            }
        }
    }
}

/// Display file information summary
pub fn display_file_info(file: &File) {
    println!("File Information:");
    println!("  Name: {}", file.name);
    println!("  Size: {} ({})", format_size(file.length), file.length);
    println!("  UID: {}", file.uid);
    println!("  Created: {}", format_time(&file.creation_time));
    println!("  Modified: {}", format_time(&file.modify_time));
    println!("  Read-only: {}", file.read_only);
    
    if file.is_symlink() {
        println!("  Type: Symbolic link -> {}", file.symlink.as_ref().unwrap());
    } else {
        println!("  Type: Regular file");
    }
    
    if file.has_extents() {
        println!("  Extents: {} extent(s)", file.extents.len());
        for (i, extent) in file.extents.iter().enumerate() {
            println!("    Extent {}: partition={}, block={}, size={}", 
                i + 1, extent.partition, extent.start_block, format_size(extent.byte_count));
        }
    } else {
        println!("  Extents: None (file may be empty or have issues)");
    }
    
    if let Some(ext_attrs) = &file.extended_attributes {
        if !ext_attrs.attributes.is_empty() {
            println!("  Extended attributes:");
            for attr in &ext_attrs.attributes {
                println!("    {}: {}", attr.key, attr.value);
            }
        }
    }
}

/// Display error message in consistent format
pub fn display_error(error: &str) {
    eprintln!("Error: {}", error);
}

/// Display warning message in consistent format
pub fn display_warning(warning: &str) {
    eprintln!("Warning: {}", warning);
}

/// Display operation progress
pub fn display_progress(current: u64, total: u64, operation: &str) {
    let percentage = if total > 0 {
        (current as f64 / total as f64 * 100.0) as u32
    } else {
        0
    };
    
    eprint!("\r{}: {} / {} ({}%)", operation, format_size(current), format_size(total), percentage);
    
    if current >= total {
        eprintln!(); // New line when complete
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_format_size() {
        assert_eq!(format_size(0), "0 B");
        assert_eq!(format_size(500), "500 B");
        assert_eq!(format_size(1024), "1.0 KB");
        assert_eq!(format_size(1536), "1.5 KB");
        assert_eq!(format_size(1048576), "1.0 MB");
        assert_eq!(format_size(1073741824), "1.0 GB");
    }

    #[test]
    fn test_is_text_content() {
        assert!(is_text_content(b"Hello, world!"));
        assert!(is_text_content(b"Line 1\nLine 2\nLine 3"));
        assert!(!is_text_content(b"Hello\x00world"));
        assert!(!is_text_content(&[0xff, 0xfe, 0xfd, 0xfc]));
        assert!(is_text_content(b"")); // Empty content is considered text
    }

    #[test]
    fn test_format_time() {
        let timestamp = "2023-01-01T12:30:45.123456789Z";
        let formatted = format_time(timestamp);
        assert!(formatted.contains("2023-01-01"));
        assert!(formatted.contains("12:30"));
    }
}