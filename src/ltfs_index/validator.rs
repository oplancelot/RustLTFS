//! LTFS Index Validation
//!
//! This module provides comprehensive validation for LTFS indexes.

use crate::error::Result;
use super::types::*;
use tracing::{debug, warn};

impl LtfsIndex {
    /// Validate XML structure before parsing
    pub(super) fn validate_xml_structure(xml_content: &str) -> Result<()> {
        debug!("Validating XML structure");

        // Basic structural checks
        if xml_content.len() < 50 {
            return Err(crate::error::RustLtfsError::parse("XML content too short"));
        }

        if !xml_content.contains("<ltfsindex") {
            return Err(crate::error::RustLtfsError::parse(
                "Missing LTFS index root element",
            ));
        }

        if !xml_content.contains("</ltfsindex>") {
            return Err(crate::error::RustLtfsError::parse(
                "XML appears incomplete - missing closing tag",
            ));
        }

        // Check for proper XML declaration
        if !xml_content.trim_start().starts_with("<?xml") {
            warn!("XML missing declaration, this may cause parsing issues");
        }

        // Count opening vs closing tags for basic balance check
        // Note: We need to account for self-closing tags like <name />
        let mut open_count = 0;
        let mut self_closing_count = 0;

        // Find all tags that start with '<' but are not XML declarations, comments, or closing tags
        for tag_match in xml_content.match_indices('<') {
            let start_pos = tag_match.0;
            if let Some(end_pos) = xml_content[start_pos..].find('>') {
                let tag = &xml_content[start_pos..start_pos + end_pos + 1];

                // Skip XML declarations and comments
                if tag.starts_with("<?xml") || tag.starts_with("<!--") {
                    continue;
                }

                // Skip closing tags
                if tag.starts_with("</") {
                    continue;
                }

                // Check if it's a self-closing tag
                if tag.ends_with("/>") {
                    self_closing_count += 1;
                } else {
                    open_count += 1;
                }
            }
        }

        let close_count = xml_content.matches("</").count();

        // For XML balance: open_count should equal close_count
        // Self-closing tags don't need closing tags
        if open_count != close_count {
            return Err(crate::error::RustLtfsError::parse(
                format!("XML structure imbalanced: {} opening tags vs {} closing tags (with {} self-closing tags)", 
                        open_count, close_count, self_closing_count)
            ));
        }

        debug!("XML structure validation passed");
        Ok(())
    }

    /// Validate parsed index for consistency - enhanced implementation
    pub(super) fn validate_parsed_index(index: &LtfsIndex) -> Result<()> {
        debug!("Validating parsed LTFS index");

        // Check version compatibility
        if !index.version.starts_with("2.") {
            warn!("LTFS version {} may not be fully supported", index.version);
        }

        // Check for required fields
        if index.volumeuuid.is_empty() {
            return Err(crate::error::RustLtfsError::parse("Missing volume UUID"));
        }

        if index.generationnumber == 0 {
            return Err(crate::error::RustLtfsError::parse(
                "Invalid generation number",
            ));
        }

        // Validate root directory
        if !index.root_directory.name.is_empty() {
            return Err(crate::error::RustLtfsError::parse(
                "Root directory must have empty name",
            ));
        }

        // Enhanced validations
        Self::validate_directory_structure(&index.root_directory)?;
        Self::validate_file_extents(&index.root_directory)?;
        Self::validate_uid_uniqueness(index)?;
        Self::validate_timestamps(&index.root_directory)?;

        debug!("Parsed index validation passed");
        Ok(())
    }

    /// Validate directory structure recursively
    fn validate_directory_structure(directory: &Directory) -> Result<()> {
        debug!("Validating directory structure: {}", directory.name);

        // Check directory UID
        if directory.uid == 0 {
            return Err(crate::error::RustLtfsError::parse(format!(
                "Directory '{}' has invalid UID 0",
                directory.name
            )));
        }

        // Check for duplicate names in contents
        let mut file_names = std::collections::HashSet::new();
        let mut dir_names = std::collections::HashSet::new();

        for file in &directory.contents.files {
            if file.name.is_empty() {
                return Err(crate::error::RustLtfsError::parse(
                    "File with empty name found",
                ));
            }

            if !file_names.insert(&file.name) {
                return Err(crate::error::RustLtfsError::parse(format!(
                    "Duplicate file name '{}' in directory '{}'",
                    file.name, directory.name
                )));
            }
        }

        for subdir in &directory.contents.directories {
            if subdir.name.is_empty() {
                return Err(crate::error::RustLtfsError::parse(
                    "Directory with empty name found",
                ));
            }

            if !dir_names.insert(&subdir.name) {
                return Err(crate::error::RustLtfsError::parse(format!(
                    "Duplicate directory name '{}' in directory '{}'",
                    subdir.name, directory.name
                )));
            }

            // Check for name conflicts between files and directories
            if file_names.contains(&subdir.name) {
                return Err(crate::error::RustLtfsError::parse(format!(
                    "Name conflict: '{}' exists as both file and directory",
                    subdir.name
                )));
            }

            // Recursively validate subdirectories
            Self::validate_directory_structure(subdir)?;
        }

        Ok(())
    }

    /// Validate file extents for consistency
    fn validate_file_extents(directory: &Directory) -> Result<()> {
        debug!("Validating file extents in directory: {}", directory.name);

        for file in &directory.contents.files {
            // Skip symlinks
            if file.symlink.is_some() {
                continue;
            }

            // Check if file has extents when it should
            if file.length > 0 && file.extent_info.extents.is_empty() {
                return Err(crate::error::RustLtfsError::parse(format!(
                    "File '{}' has size {} but no extents",
                    file.name, file.length
                )));
            }

            if file.length == 0 && !file.extent_info.extents.is_empty() {
                warn!("File '{}' has zero size but contains extents", file.name);
            }

            // Validate each extent
            let mut total_extent_size = 0u64;
            let mut last_file_offset = 0u64;

            for extent in &file.extent_info.extents {
                // Check partition validity
                if extent.partition != "a"
                    && extent.partition != "b"
                    && extent.partition != "A"
                    && extent.partition != "B"
                {
                    return Err(crate::error::RustLtfsError::parse(format!(
                        "Invalid partition '{}' in file '{}'",
                        extent.partition, file.name
                    )));
                }

                // Check extent size consistency
                if extent.byte_count == 0 {
                    return Err(crate::error::RustLtfsError::parse(format!(
                        "Zero-size extent in file '{}'",
                        file.name
                    )));
                }

                // Check file offset ordering
                if extent.file_offset < last_file_offset {
                    return Err(crate::error::RustLtfsError::parse(format!(
                        "Extents not ordered by file offset in file '{}'",
                        file.name
                    )));
                }

                last_file_offset = extent.file_offset + extent.byte_count;
                total_extent_size += extent.byte_count;
            }

            // Check total size consistency
            if total_extent_size != file.length {
                return Err(crate::error::RustLtfsError::parse(format!(
                    "File '{}': declared size {} doesn't match extent total {}",
                    file.name, file.length, total_extent_size
                )));
            }
        }

        // Recursively validate subdirectories
        for subdir in &directory.contents.directories {
            Self::validate_file_extents(subdir)?;
        }

        Ok(())
    }

    /// Validate UID uniqueness across the entire index
    fn validate_uid_uniqueness(index: &LtfsIndex) -> Result<()> {
        debug!("Validating UID uniqueness");

        let mut used_uids = std::collections::HashSet::new();

        // Add root directory UID
        if !used_uids.insert(index.root_directory.uid) {
            return Err(crate::error::RustLtfsError::parse(format!(
                "Duplicate UID {} found",
                index.root_directory.uid
            )));
        }

        Self::collect_and_validate_uids(&index.root_directory, &mut used_uids)?;

        debug!(
            "UID validation passed, found {} unique UIDs",
            used_uids.len()
        );
        Ok(())
    }

    /// Recursively collect and validate UIDs
    fn collect_and_validate_uids(
        directory: &Directory,
        used_uids: &mut std::collections::HashSet<u64>,
    ) -> Result<()> {
        // Check subdirectories
        for subdir in &directory.contents.directories {
            if !used_uids.insert(subdir.uid) {
                return Err(crate::error::RustLtfsError::parse(format!(
                    "Duplicate UID {} found in directory '{}'",
                    subdir.uid, subdir.name
                )));
            }

            Self::collect_and_validate_uids(subdir, used_uids)?;
        }

        // Check files
        for file in &directory.contents.files {
            if !used_uids.insert(file.uid) {
                return Err(crate::error::RustLtfsError::parse(format!(
                    "Duplicate UID {} found in file '{}'",
                    file.uid, file.name
                )));
            }
        }

        Ok(())
    }

    /// Validate timestamps format and consistency
    fn validate_timestamps(directory: &Directory) -> Result<()> {
        debug!("Validating timestamps in directory: {}", directory.name);

        // Validate directory timestamps
        Self::validate_timestamp_format(&directory.creation_time, "creation_time")?;
        Self::validate_timestamp_format(&directory.change_time, "change_time")?;
        Self::validate_timestamp_format(&directory.modify_time, "modify_time")?;
        Self::validate_timestamp_format(&directory.access_time, "access_time")?;
        Self::validate_timestamp_format(&directory.backup_time, "backup_time")?;

        // Validate file timestamps
        for file in &directory.contents.files {
            Self::validate_timestamp_format(
                &file.creation_time,
                &format!("file '{}' creation_time", file.name),
            )?;
            Self::validate_timestamp_format(
                &file.change_time,
                &format!("file '{}' change_time", file.name),
            )?;
            Self::validate_timestamp_format(
                &file.modify_time,
                &format!("file '{}' modify_time", file.name),
            )?;
            Self::validate_timestamp_format(
                &file.access_time,
                &format!("file '{}' access_time", file.name),
            )?;
            Self::validate_timestamp_format(
                &file.backup_time,
                &format!("file '{}' backup_time", file.name),
            )?;
        }

        // Recursively validate subdirectories
        for subdir in &directory.contents.directories {
            Self::validate_timestamps(subdir)?;
        }

        Ok(())
    }

    /// Validate individual timestamp format
    fn validate_timestamp_format(timestamp: &str, field_name: &str) -> Result<()> {
        // LTFS timestamps should be in ISO 8601 format: YYYY-MM-DDTHH:MM:SS.nnnnnnnnnZ
        if timestamp.len() < 20 {
            // Minimum: "2023-01-01T00:00:00Z"
            return Err(crate::error::RustLtfsError::parse(format!(
                "Invalid timestamp format in {}: '{}' (too short)",
                field_name, timestamp
            )));
        }

        if !timestamp.ends_with('Z') {
            return Err(crate::error::RustLtfsError::parse(format!(
                "Invalid timestamp format in {}: '{}' (must end with Z)",
                field_name, timestamp
            )));
        }

        if !timestamp.contains('T') {
            return Err(crate::error::RustLtfsError::parse(format!(
                "Invalid timestamp format in {}: '{}' (missing T separator)",
                field_name, timestamp
            )));
        }

        // Try to parse with chrono for more thorough validation
        match chrono::DateTime::parse_from_rfc3339(timestamp) {
            Ok(_) => Ok(()),
            Err(_) => {
                // If RFC3339 parsing fails, try custom LTFS format
                match chrono::DateTime::parse_from_str(timestamp, "%Y-%m-%dT%H:%M:%S%.fZ") {
                    Ok(_) => Ok(()),
                    Err(_) => Err(crate::error::RustLtfsError::parse(format!(
                        "Invalid timestamp format in {}: '{}'",
                        field_name, timestamp
                    ))),
                }
            }
        }
    }

    /// Count total files in index for diagnostics
    pub(super) fn count_files_in_index(index: &LtfsIndex) -> usize {
        Self::count_files_in_directory(&index.root_directory)
    }

    /// Recursively count files in directory
    fn count_files_in_directory(directory: &Directory) -> usize {
        let mut count = directory.contents.files.len();

        for subdir in &directory.contents.directories {
            count += Self::count_files_in_directory(subdir);
        }

        count
    }
}
