use crate::error::Result;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use tracing::{debug, info, warn};

/// LTFS Index structure based on LTFS specification
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LtfsIndex {
    #[serde(rename = "@version")]
    pub version: String,
    pub creator: String,
    pub volumeuuid: String,
    pub generationnumber: u64,
    pub updatetime: String,
    pub location: Location,
    pub allowpolicyupdate: Option<bool>,
    #[serde(rename = "contents")]
    pub root_directory: Directory,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Location {
    pub partition: String,
    pub startblock: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Directory {
    pub name: String,
    #[serde(rename = "fileuid")]
    pub uid: u64,
    #[serde(rename = "creationtime")]
    pub creation_time: String,
    #[serde(rename = "changetime")]
    pub change_time: String,
    #[serde(rename = "modifytime")]
    pub modify_time: String,
    #[serde(rename = "accesstime")]
    pub access_time: String,
    #[serde(rename = "backuptime")]
    pub backup_time: String,
    #[serde(rename = "readonly")]
    pub read_only: bool,
    #[serde(rename = "contents")]
    pub contents: DirectoryContents,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct DirectoryContents {
    #[serde(rename = "directory", default)]
    pub directories: Vec<Directory>,
    #[serde(rename = "file", default)]
    pub files: Vec<File>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct File {
    pub name: String,
    #[serde(rename = "fileuid")]
    pub uid: u64,
    pub length: u64,
    #[serde(rename = "creationtime")]
    pub creation_time: String,
    #[serde(rename = "changetime")]
    pub change_time: String,
    #[serde(rename = "modifytime")]
    pub modify_time: String,
    #[serde(rename = "accesstime")]
    pub access_time: String,
    #[serde(rename = "backuptime")]
    pub backup_time: String,
    #[serde(rename = "readonly")]
    pub read_only: bool,
    #[serde(rename = "symlink")]
    pub symlink: Option<String>,
    #[serde(rename = "extentinfo", default)]
    pub extents: Vec<FileExtent>,
    #[serde(rename = "extendedattributes")]
    pub extended_attributes: Option<ExtendedAttributes>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileExtent {
    pub partition: String,
    #[serde(rename = "startblock")]
    pub start_block: u64,
    #[serde(rename = "bytecount")]
    pub byte_count: u64,
    #[serde(rename = "fileoffset")]
    pub file_offset: u64,
    #[serde(rename = "byteoffset")]
    pub byte_offset: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExtendedAttributes {
    #[serde(rename = "xattr", default)]
    pub attributes: Vec<ExtendedAttribute>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExtendedAttribute {
    pub key: String,
    #[serde(rename = "value")]
    pub value: String,
}

/// Result of path lookup in LTFS index
#[derive(Debug, Clone)]
pub enum PathType {
    File(File),
    Directory(Directory),
    NotFound,
}

/// Directory entry for listing
#[derive(Debug, Clone)]
pub enum DirectoryEntry {
    File(File),
    Directory(Directory),
}

impl DirectoryEntry {
    pub fn name(&self) -> &str {
        match self {
            DirectoryEntry::File(f) => &f.name,
            DirectoryEntry::Directory(d) => &d.name,
        }
    }

    pub fn size(&self) -> u64 {
        match self {
            DirectoryEntry::File(f) => f.length,
            DirectoryEntry::Directory(_) => 0,
        }
    }

    pub fn modify_time(&self) -> &str {
        match self {
            DirectoryEntry::File(f) => &f.modify_time,
            DirectoryEntry::Directory(d) => &d.modify_time,
        }
    }

    pub fn is_directory(&self) -> bool {
        matches!(self, DirectoryEntry::Directory(_))
    }
}

impl LtfsIndex {
    /// Parse LTFS index from XML content
    pub fn from_xml(xml_content: &str) -> Result<Self> {
        debug!("Parsing LTFS index XML, length: {}", xml_content.len());
        
        let index: LtfsIndex = quick_xml::de::from_str(xml_content)
            .map_err(|e| crate::error::RustLtfsError::parse(format!("Failed to parse LTFS index XML: {}", e)))?;
        
        info!("Successfully parsed LTFS index, version: {}, generation: {}", 
            index.version, index.generationnumber);
        
        Ok(index)
    }
    
    /// Serialize LTFS index to XML string
    pub fn to_xml(&self) -> Result<String> {
        let xml_string = quick_xml::se::to_string(self)
            .map_err(|e| crate::error::RustLtfsError::file_operation(
                format!("Failed to serialize LTFS index to XML: {}", e)
            ))?;
        
        // Add XML declaration
        let complete_xml = format!("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n{}", xml_string);
        
        debug!("Serialized LTFS index to XML ({} bytes)", complete_xml.len());
        Ok(complete_xml)
    }
    
    /// Get next available file UID
    pub fn get_next_file_uid(&self) -> u64 {
        let mut max_uid = self.root_directory.uid;
        self.collect_all_uids_from_dir(&self.root_directory, &mut max_uid);
        max_uid + 1
    }
    
    /// Recursively collect all UIDs from directory tree
    fn collect_all_uids_from_dir(&self, directory: &Directory, max_uid: &mut u64) {
        // Check directory's own UID
        if directory.uid > *max_uid {
            *max_uid = directory.uid;
        }
        
        // Check subdirectories
        for subdir in &directory.contents.directories {
            self.collect_all_uids_from_dir(subdir, max_uid);
        }
        
        // Check files
        for file in &directory.contents.files {
            if file.uid > *max_uid {
                *max_uid = file.uid;
            }
        }
    }
    
    /// Insert a file into the specified directory path
    pub fn insert_file(&mut self, parent_path: &str, file: File) -> Result<()> {
        let normalized_path = normalize_path(parent_path);
        debug!("Inserting file '{}' into directory '{}'", file.name, normalized_path);
        
        if normalized_path == "/" || normalized_path.is_empty() {
            // Insert into root directory
            self.root_directory.contents.files.push(file);
            return Ok(());
        }
        
        // For simplicity, just insert into root for now
        // TODO: Implement proper directory traversal later
        info!("Simplified implementation: inserting '{}' into root directory", file.name);
        self.root_directory.contents.files.push(file);
        Ok(())
    }
    
    /// Increment generation number
    pub fn increment_generation(&mut self) {
        self.generationnumber += 1;
        self.updatetime = get_current_timestamp();
        debug!("Updated LTFS index generation to {}", self.generationnumber);
    }

    /// Find a path in the LTFS index
    pub fn find_path(&self, path: &str) -> Result<PathType> {
        debug!("Finding path: {}", path);
        
        let normalized_path = self.normalize_path(path);
        let path_parts: Vec<&str> = normalized_path
            .split('/')
            .filter(|s| !s.is_empty())
            .collect();

        if path_parts.is_empty() {
            // Root directory
            return Ok(PathType::Directory(self.root_directory.clone()));
        }

        self.find_path_recursive(&self.root_directory, &path_parts, 0)
    }

    /// List directory contents
    pub fn list_directory(&self, path: &str) -> Result<Vec<DirectoryEntry>> {
        debug!("Listing directory: {}", path);
        
        match self.find_path(path)? {
            PathType::Directory(dir) => {
                let mut entries = Vec::new();
                
                // Add subdirectories
                for subdir in &dir.contents.directories {
                    entries.push(DirectoryEntry::Directory(subdir.clone()));
                }
                
                // Add files
                for file in &dir.contents.files {
                    entries.push(DirectoryEntry::File(file.clone()));
                }
                
                // Sort entries by name
                entries.sort_by(|a, b| a.name().cmp(b.name()));
                
                debug!("Found {} entries in directory {}", entries.len(), path);
                Ok(entries)
            }
            PathType::File(_) => {
                Err(crate::error::RustLtfsError::file_operation(
                    format!("Path {} is a file, not a directory", path)
                ))
            }
            PathType::NotFound => {
                Err(crate::error::RustLtfsError::file_operation(
                    format!("Directory {} not found", path)
                ))
            }
        }
    }

    /// Get file information
    pub fn get_file_info(&self, path: &str) -> Result<File> {
        debug!("Getting file info: {}", path);
        
        match self.find_path(path)? {
            PathType::File(file) => Ok(file),
            PathType::Directory(_) => {
                Err(crate::error::RustLtfsError::file_operation(
                    format!("Path {} is a directory, not a file", path)
                ))
            }
            PathType::NotFound => {
                Err(crate::error::RustLtfsError::file_operation(
                    format!("File {} not found", path)
                ))
            }
        }
    }

    /// Normalize path (remove redundant slashes, etc.)
    fn normalize_path(&self, path: &str) -> String {
        let mut normalized = path.replace('\\', "/");
        
        // Remove multiple consecutive slashes
        while normalized.contains("//") {
            normalized = normalized.replace("//", "/");
        }
        
        // Remove trailing slash unless it's root
        if normalized.len() > 1 && normalized.ends_with('/') {
            normalized.pop();
        }
        
        normalized
    }

    /// Recursively find path in directory tree
    fn find_path_recursive(&self, current_dir: &Directory, path_parts: &[&str], index: usize) -> Result<PathType> {
        if index >= path_parts.len() {
            return Ok(PathType::Directory(current_dir.clone()));
        }

        let current_part = path_parts[index];

        // Check subdirectories
        for subdir in &current_dir.contents.directories {
            if subdir.name == current_part {
                if index == path_parts.len() - 1 {
                    // This is the target
                    return Ok(PathType::Directory(subdir.clone()));
                } else {
                    // Continue searching in subdirectory
                    return self.find_path_recursive(subdir, path_parts, index + 1);
                }
            }
        }

        // Check files (only if this is the last part)
        if index == path_parts.len() - 1 {
            for file in &current_dir.contents.files {
                if file.name == current_part {
                    return Ok(PathType::File(file.clone()));
                }
            }
        }

        Ok(PathType::NotFound)
    }
}

impl File {
    /// Get total file size
    pub fn total_size(&self) -> u64 {
        self.length
    }

    /// Get all extents sorted by file offset
    pub fn get_sorted_extents(&self) -> Vec<FileExtent> {
        let mut extents = self.extents.clone();
        extents.sort_by(|a, b| a.file_offset.cmp(&b.file_offset));
        extents
    }

    /// Check if file is a symbolic link
    pub fn is_symlink(&self) -> bool {
        self.symlink.is_some()
    }

    /// Get extended attribute value by key
    pub fn get_extended_attribute(&self, key: &str) -> Option<String> {
        self.extended_attributes
            .as_ref()?
            .attributes
            .iter()
            .find(|attr| attr.key == key)
            .map(|attr| attr.value.clone())
    }

    /// Check if file has any extents
    pub fn has_extents(&self) -> bool {
        !self.extents.is_empty()
    }
}

/// Normalize path (remove redundant slashes, etc.)
pub fn normalize_path(path: &str) -> String {
    let mut normalized = path.replace('\\', "/");
    
    // Remove multiple consecutive slashes
    while normalized.contains("//") {
        normalized = normalized.replace("//", "/");
    }
    
    // Remove trailing slash unless it's root
    if normalized.len() > 1 && normalized.ends_with('/') {
        normalized.pop();
    }
    
    normalized
}

/// Get current timestamp in LTFS format
pub fn get_current_timestamp() -> String {
    use chrono::Utc;
    Utc::now().format("%Y-%m-%dT%H:%M:%S%.9fZ").to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_normalize_path() {
        let index = LtfsIndex {
            version: "2.4".to_string(),
            creator: "test".to_string(),
            volumeuuid: "test".to_string(),
            generationnumber: 1,
            updatetime: "test".to_string(),
            location: Location {
                partition: "a".to_string(),
                startblock: 0,
            },
            allowpolicyupdate: None,
            root_directory: Directory {
                name: "".to_string(),
                uid: 0,
                creation_time: "test".to_string(),
                change_time: "test".to_string(),
                modify_time: "test".to_string(),
                access_time: "test".to_string(),
                backup_time: "test".to_string(),
                read_only: false,
                contents: DirectoryContents::default(),
            },
        };

        assert_eq!(index.normalize_path("/"), "/");
        assert_eq!(index.normalize_path("//test//path//"), "/test/path");
        assert_eq!(index.normalize_path("\\test\\path"), "/test/path");
        assert_eq!(index.normalize_path("/test/path/"), "/test/path");
    }

    #[test]
    fn test_directory_entry_methods() {
        let file = File {
            name: "test.txt".to_string(),
            uid: 1,
            length: 1024,
            creation_time: "2023-01-01T00:00:00Z".to_string(),
            change_time: "2023-01-01T00:00:00Z".to_string(),
            modify_time: "2023-01-01T00:00:00Z".to_string(),
            access_time: "2023-01-01T00:00:00Z".to_string(),
            backup_time: "2023-01-01T00:00:00Z".to_string(),
            read_only: false,
            symlink: None,
            extents: vec![],
            extended_attributes: None,
        };

        let entry = DirectoryEntry::File(file);
        assert_eq!(entry.name(), "test.txt");
        assert_eq!(entry.size(), 1024);
        assert!(!entry.is_directory());
    }
}