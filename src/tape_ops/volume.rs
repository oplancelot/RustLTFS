//! VOL1 Label Parsing and Tape Format Analysis Module
//!
//! This module provides comprehensive VOL1 label validation and tape format detection.
//! Supports LTFS format detection with multiple fallback strategies and detailed diagnostics.

use crate::error::Result;
use super::TapeFormatAnalysis;
use tracing::{debug, info, warn};

/// Enhanced VOL1 label validation with comprehensive format detection
/// 增强版 VOL1 标签验证：支持多种磁带格式检测和详细诊断
pub fn parse_vol1_label(buffer: &[u8]) -> Result<bool> {
    info!("Validating VOL1 label");

    // Enhanced Condition 1: Dynamic buffer length check with detailed analysis
    if buffer.len() < 80 {
        warn!(
            "❌ VOL1 validation failed: buffer too short ({} bytes), need at least 80 bytes",
            buffer.len()
        );

        // Provide diagnostic information for short buffers
        if buffer.len() > 0 {
            let preview_len = std::cmp::min(buffer.len(), 40);
            info!(
                "🔧 Buffer content preview ({} bytes): hex={:02X?}",
                preview_len,
                &buffer[0..preview_len]
            );
            info!(
                "🔧 Buffer content preview ({} bytes): text={:?}",
                preview_len,
                String::from_utf8_lossy(&buffer[0..preview_len])
            );
        }

        return Ok(false);
    }

    //Extract the standard 80-byte VOL1 label area
    let vol1_label = &buffer[0..80];

    // Enhanced Condition 2: Multi-format tape detection with detailed analysis
    let vol1_prefix = b"VOL1";
    if !vol1_label.starts_with(vol1_prefix) {
        info!("VOL1 prefix not found, performing format detection");

        // Comprehensive tape format analysis
        let tape_analysis = analyze_tape_format_enhanced(vol1_label);
        match tape_analysis {
            TapeFormatAnalysis::BlankTape => {
                info!("Blank tape detected");
                return Ok(false);
            }
            TapeFormatAnalysis::LegacyTape(format_name) => {
                info!("Legacy tape format detected: {}", format_name);
                info!("This tape may contain data but is not LTFS formatted");
                return Ok(false);
            }
            TapeFormatAnalysis::CorruptedLabel => {
                warn!("💥 Corrupted or damaged VOL1 label detected");
                info!("Try cleaning the tape drive or using a different tape");
                return Ok(false);
            }
            TapeFormatAnalysis::UnknownFormat => {
                info!("Unknown tape format detected");
                log_detailed_tape_analysis(vol1_label);
                return Ok(false);
            }
            TapeFormatAnalysis::PossibleLTFS => {
                info!("Possible LTFS tape with non-standard VOL1, proceeding with validation");
                // Continue to LTFS identifier check
            }
        }
    } else {
        info!("VOL1 prefix validation passed");
    }

    // Enhanced Condition 3: Advanced LTFS identifier validation with fallback strategies
    if vol1_label.len() < 28 {
        warn!(
            "❌ VOL1 label too short for LTFS identifier check (need 28+ bytes, got {})",
            vol1_label.len()
        );
        return Ok(false);
    }

    let ltfs_bytes = &vol1_label[24..28];
    let expected_ltfs = b"LTFS";

    if ltfs_bytes == expected_ltfs {
        info!("LTFS identifier found at standard position");
        return validate_extended_ltfs_properties(vol1_label);
    }

    // Enhanced fallback strategies for LTFS detection
    info!("Standard LTFS identifier not found, trying alternative detection");

    // Strategy 1: Search for LTFS identifier in alternative positions
    if let Some(ltfs_position) = search_ltfs_identifier_in_vol1(vol1_label) {
        info!("Found LTFS identifier at alternative position: {}", ltfs_position);
        return validate_extended_ltfs_properties(vol1_label);
    }

    // Strategy 2: Check for LTFS version indicators
    if detect_ltfs_version_indicators(vol1_label) {
        info!("LTFS version indicators detected");
        return validate_extended_ltfs_properties(vol1_label);
    }

    // Strategy 3: Pattern-based LTFS detection
    if detect_ltfs_patterns(vol1_label) {
        info!("LTFS patterns detected in VOL1 label");
        return Ok(true); // Allow with pattern-based detection
    }

    // Final diagnostic report
    warn!(
        "❌ LTFS identifier validation failed: expected 'LTFS' at position 24-27, found: {:?}",
        String::from_utf8_lossy(ltfs_bytes)
    );
    info!("Checking for partial LTFS compatibility");

    // Check if this might be a partially formatted or corrupted LTFS tape
    if detect_partial_ltfs_formatting(vol1_label) {
        warn!("⚠️ Partial LTFS formatting detected - tape may be recoverable");
        info!("Try reformatting with mkltfs or use recovery tools");
    }

    log_detailed_tape_analysis(vol1_label);
    Ok(false)
}

/// Enhanced tape format analysis with detailed classification
pub fn analyze_tape_format_enhanced(vol1_label: &[u8]) -> TapeFormatAnalysis {
    // Check for blank tape (all zeros)
    let non_zero_count = vol1_label.iter().filter(|&&b| b != 0).count();
    if non_zero_count == 0 {
        return TapeFormatAnalysis::BlankTape;
    }

    // Check for very sparse data (likely blank or minimally written)
    let sparse_threshold = 5; // Less than 5 non-zero bytes in 80 bytes
    if non_zero_count < sparse_threshold {
        debug!(
            "Sparse data detected: only {} non-zero bytes",
            non_zero_count
        );
        return TapeFormatAnalysis::BlankTape;
    }

    // Check for common legacy tape formats
    if vol1_label.starts_with(b"HDR1") || vol1_label.starts_with(b"HDR2") {
        return TapeFormatAnalysis::LegacyTape("ANSI Standard Label (HDR)".to_string());
    }

    if vol1_label.starts_with(b"UHL1") || vol1_label.starts_with(b"UHL2") {
        return TapeFormatAnalysis::LegacyTape("User Header Label (UHL)".to_string());
    }

    if vol1_label.starts_with(b"EOF1") || vol1_label.starts_with(b"EOF2") {
        return TapeFormatAnalysis::LegacyTape("End of File Label (EOF)".to_string());
    }

    if vol1_label.starts_with(b"EOV1") || vol1_label.starts_with(b"EOV2") {
        return TapeFormatAnalysis::LegacyTape("End of Volume Label (EOV)".to_string());
    }

    // Check for IBM tape formats
    if vol1_label[0..4] == [0xE5, 0xD6, 0xD3, 0xF1] {
        // EBCDIC "VOL1"
        return TapeFormatAnalysis::LegacyTape("IBM EBCDIC VOL1 Label".to_string());
    }

    // Check for potential LTFS with damaged VOL1
    if contains_ltfs_patterns(vol1_label) {
        return TapeFormatAnalysis::PossibleLTFS;
    }

    // Check for corrupted label (has data but unrecognizable pattern)
    let ascii_count = vol1_label.iter().filter(|&&b| b >= 32 && b <= 126).count();
    let ascii_ratio = ascii_count as f64 / vol1_label.len() as f64;

    if ascii_ratio < 0.3 {
        return TapeFormatAnalysis::CorruptedLabel;
    }

    TapeFormatAnalysis::UnknownFormat
}

/// Search for LTFS identifier in alternative positions within VOL1 label
fn search_ltfs_identifier_in_vol1(vol1_label: &[u8]) -> Option<usize> {
    let ltfs_signature = b"LTFS";

    // Search in common alternative positions (some LTFS implementations may vary)
    let search_positions = [20, 28, 32, 36, 40, 44, 48]; // Alternative positions to check

    for &pos in &search_positions {
        if pos + 4 <= vol1_label.len() {
            if &vol1_label[pos..pos + 4] == ltfs_signature {
                return Some(pos);
            }
        }
    }

    // Broader search within the entire VOL1 label
    for i in 0..=(vol1_label.len().saturating_sub(4)) {
        if &vol1_label[i..i + 4] == ltfs_signature {
            return Some(i);
        }
    }

    None
}

/// Detect LTFS version indicators in VOL1 label
fn detect_ltfs_version_indicators(vol1_label: &[u8]) -> bool {
    let vol1_text = String::from_utf8_lossy(vol1_label).to_lowercase();

    // Look for version patterns commonly found in LTFS labels
    let version_patterns = [
        "ltfs",
        "2.4",
        "2.2",
        "2.0",
        "1.0",
        "version",
        "ltfscopygui",
        "rustltfs",
    ];

    let mut pattern_count = 0;
    for pattern in &version_patterns {
        if vol1_text.contains(pattern) {
            pattern_count += 1;
            debug!("Found LTFS version indicator: '{}'", pattern);
        }
    }

    pattern_count >= 2 // Require at least 2 patterns for confidence
}

/// Detect LTFS-specific patterns in VOL1 label
fn detect_ltfs_patterns(vol1_label: &[u8]) -> bool {
    // Check for characteristic byte patterns found in LTFS labels
    let patterns_found = [
        contains_ltfs_patterns(vol1_label),
        has_ltfs_block_size_indicators(vol1_label),
        has_ltfs_partition_indicators(vol1_label),
    ];

    patterns_found.iter().filter(|&&found| found).count() >= 2
}

/// Check if VOL1 contains LTFS-specific patterns
fn contains_ltfs_patterns(vol1_label: &[u8]) -> bool {
    let vol1_text = String::from_utf8_lossy(vol1_label);

    // Look for case-insensitive LTFS patterns
    let ltfs_indicators = ["ltfs", "linear", "tape", "file", "system"];
    let found_indicators = ltfs_indicators
        .iter()
        .filter(|&pattern| vol1_text.to_lowercase().contains(pattern))
        .count();

    found_indicators >= 2
}

/// Check for LTFS block size indicators
fn has_ltfs_block_size_indicators(vol1_label: &[u8]) -> bool {
    // Look for typical LTFS block sizes in the label
    let common_block_sizes = [524288u32, 65536u32, 32768u32]; // Common LTFS block sizes

    for &block_size in &common_block_sizes {
        let size_bytes = block_size.to_le_bytes();
        if vol1_label.windows(4).any(|window| window == size_bytes) {
            debug!("Found potential block size indicator: {}", block_size);
            return true;
        }

        let size_bytes_be = block_size.to_be_bytes();
        if vol1_label.windows(4).any(|window| window == size_bytes_be) {
            debug!("Found potential block size indicator (BE): {}", block_size);
            return true;
        }
    }

    false
}

/// Check for LTFS partition indicators
fn has_ltfs_partition_indicators(vol1_label: &[u8]) -> bool {
    // Look for partition-related information typical in LTFS
    let vol1_text = String::from_utf8_lossy(vol1_label).to_lowercase();
    let partition_patterns = ["partition", "part", "index", "data"];

    partition_patterns
        .iter()
        .any(|&pattern| vol1_text.contains(pattern))
}

/// Detect partial LTFS formatting that might be recoverable
fn detect_partial_ltfs_formatting(vol1_label: &[u8]) -> bool {
    // Look for signs of interrupted or partial LTFS formatting
    let vol1_text = String::from_utf8_lossy(vol1_label);

    // Check for partial signatures or formatting indicators
    let partial_indicators = [
        vol1_text.contains("LTF"), // Partial "LTFS"
        vol1_text.contains("TFS"), // Partial "LTFS"
        vol1_text.contains("vol"), // Partial volume info
        vol1_label.windows(2).any(|window| window == [0x4C, 0x54]), // Partial "LT" bytes
    ];

    partial_indicators.iter().any(|&found| found)
}

/// Validate extended LTFS properties in VOL1 label
fn validate_extended_ltfs_properties(vol1_label: &[u8]) -> Result<bool> {
    info!("Validating extended LTFS properties in VOL1 label");

    // Basic validation passed, now check additional LTFS properties
    let mut validation_score = 0u32;
    let max_score = 10u32;

    // Check 1: Volume serial number area (bytes 4-10)
    if vol1_label.len() >= 11 {
        let volume_serial = &vol1_label[4..11];
        if volume_serial.iter().any(|&b| b != 0 && b != 0x20) {
            // Not all zeros or spaces
            validation_score += 2;
            debug!("✓ Volume serial number present");
        }
    }

    // Check 2: Owner identifier area (bytes 37-50)
    if vol1_label.len() >= 51 {
        let owner_id = &vol1_label[37..51];
        if owner_id.iter().any(|&b| b != 0 && b != 0x20) {
            validation_score += 1;
            debug!("✓ Owner identifier present");
        }
    }

    // Check 3: Label standard version (typically at byte 79)
    if vol1_label.len() >= 80 {
        let label_std_version = vol1_label[79];
        if label_std_version >= 0x30 && label_std_version <= 0x39 {
            // ASCII digit
            validation_score += 2;
            debug!(
                "✓ Valid label standard version: {}",
                label_std_version as char
            );
        }
    }

    // Check 4: Overall ASCII compliance
    let ascii_count = vol1_label
        .iter()
        .filter(|&&b| (b >= 0x20 && b <= 0x7E) || b == 0x00)
        .count();
    let ascii_ratio = ascii_count as f64 / vol1_label.len() as f64;
    if ascii_ratio >= 0.8 {
        validation_score += 2;
        debug!("✓ Good ASCII compliance: {:.1}%", ascii_ratio * 100.0);
    }

    // Check 5: Reasonable data distribution (not too repetitive)
    let unique_bytes = vol1_label
        .iter()
        .collect::<std::collections::HashSet<_>>()
        .len();
    if unique_bytes >= 10 {
        validation_score += 2;
        debug!("✓ Good data diversity: {} unique bytes", unique_bytes);
    }

    // Check 6: LTFS-specific structural validation
    if validate_ltfs_vol1_structure(vol1_label) {
        validation_score += 1;
        debug!("✓ LTFS VOL1 structure validation passed");
    }

    let validation_percentage = (validation_score as f64 / max_score as f64) * 100.0;
    info!(
        "Extended LTFS validation score: {}/{} ({:.1}%)",
        validation_score, max_score, validation_percentage
    );

    if validation_score >= 6 {
        info!("Extended LTFS properties validation passed with high confidence");
        Ok(true)
    } else if validation_score >= 4 {
        info!("Extended LTFS properties validation passed with medium confidence");
        Ok(true) // Allow with warnings
    } else {
        warn!("❌ Extended LTFS properties validation failed - score too low");
        Ok(false)
    }
}

/// Validate LTFS-specific VOL1 label structure
fn validate_ltfs_vol1_structure(vol1_label: &[u8]) -> bool {
    // LTFS VOL1 should have specific structural characteristics

    // Check for proper field separators and lengths
    let mut structure_score = 0u32;

    // Field 1: Volume identifier (4 bytes "VOL1")
    if vol1_label.starts_with(b"VOL1") {
        structure_score += 1;
    }

    // Field 2: Volume serial (6 bytes, typically alphanumeric)
    if vol1_label.len() >= 10 {
        let vol_serial = &vol1_label[4..10];
        if vol_serial
            .iter()
            .all(|&b| b.is_ascii_alphanumeric() || b == 0x20)
        {
            structure_score += 1;
        }
    }

    // Field 3: Security byte (should be space or ASCII)
    if vol1_label.len() >= 11 && (vol1_label[10] == 0x20 || vol1_label[10].is_ascii()) {
        structure_score += 1;
    }

    structure_score >= 2
}

/// Log detailed tape analysis for diagnostic purposes
fn log_detailed_tape_analysis(vol1_label: &[u8]) {
    info!("Detailed Tape Analysis Report");

    // Basic statistics
    let total_bytes = vol1_label.len();
    let non_zero_bytes = vol1_label.iter().filter(|&&b| b != 0).count();
    let ascii_bytes = vol1_label
        .iter()
        .filter(|&&b| b >= 0x20 && b <= 0x7E)
        .count();
    let control_bytes = vol1_label.iter().filter(|&&b| b < 0x20).count();

    info!(
        "Statistics: {} total bytes, {} non-zero, {} ASCII printable, {} control",
        total_bytes, non_zero_bytes, ascii_bytes, control_bytes
    );

    // Hex dump of first 40 bytes
    let preview_len = std::cmp::min(40, vol1_label.len());
    info!(
        "Hex dump (first {} bytes): {:02X?}",
        preview_len,
        &vol1_label[0..preview_len]
    );

    // ASCII representation
    let ascii_repr = vol1_label[0..preview_len]
        .iter()
        .map(|&b| {
            if b >= 0x20 && b <= 0x7E {
                b as char
            } else {
                '.'
            }
        })
        .collect::<String>();
    info!("ASCII representation: '{}'", ascii_repr);

    // Pattern analysis
    let unique_bytes = vol1_label
        .iter()
        .collect::<std::collections::HashSet<_>>()
        .len();
    info!("Data diversity: {} unique byte values", unique_bytes);

    // Look for any recognizable patterns
    if let Some(pattern) = identify_tape_patterns(vol1_label) {
        info!("Identified pattern: {}", pattern);
    }
}

/// Identify recognizable patterns in tape data
fn identify_tape_patterns(data: &[u8]) -> Option<String> {
    let text = String::from_utf8_lossy(data).to_lowercase();

    // Check for various tape-related patterns
    if text.contains("backup") || text.contains("archive") {
        return Some("Backup/Archive software signature".to_string());
    }

    if text.contains("tar") || text.contains("cpio") {
        return Some("Unix archive format signature".to_string());
    }

    if text.contains("ibm") || text.contains("tivoli") {
        return Some("IBM software signature".to_string());
    }

    if text.contains("hp") || text.contains("veritas") {
        return Some("Enterprise backup software signature".to_string());
    }

    // Check for filesystem signatures
    if data.windows(2).any(|window| window == [0x53, 0xEF]) {
        // ext2/3/4 magic
        return Some("Linux filesystem signature".to_string());
    }

    None
}
