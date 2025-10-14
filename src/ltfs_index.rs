use crate::error::Result;
use serde::{Deserialize, Serialize};
use tracing::{debug, info, warn};

/// LTFS Index structure based on LTFS specification
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename = "ltfsindex")]
pub struct LtfsIndex {
    #[serde(rename = "@version")]
    pub version: String,
    pub creator: String,
    pub volumeuuid: String,
    pub generationnumber: u64,
    pub updatetime: String,
    pub location: Location,
    #[serde(default)]
    pub previousgenerationlocation: Option<Location>,
    #[serde(default)]
    pub allowpolicyupdate: Option<bool>,
    #[serde(default)]
    pub volumelockstate: Option<String>,
    #[serde(default)]
    pub highestfileuid: Option<u64>,
    #[serde(rename = "directory")]
    pub root_directory: Directory,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Location {
    pub partition: String,
    #[serde(alias = "startBlock", alias = "start_block", alias = "block")]
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

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ExtentInfo {
    #[serde(rename = "extent", default)]
    pub extents: Vec<FileExtent>,
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
    pub openforwrite: bool,
    #[serde(rename = "symlink", default)]
    pub symlink: Option<String>,
    #[serde(rename = "extentinfo", default)]
    pub extent_info: ExtentInfo,
    #[serde(rename = "extendedattributes", default)]
    pub extended_attributes: Option<ExtendedAttributes>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileExtent {
    pub partition: String,
    #[serde(rename = "startblock", alias = "startBlock", alias = "start_block", alias = "block")]
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
    /// Parse LTFS index from XML content with enhanced error handling
    pub fn from_xml(xml_content: &str) -> Result<Self> {
        debug!("Parsing LTFS index XML, length: {}", xml_content.len());

        // 首先提取纯LTFS索引部分（跳过可能的ltfslabel部分）
        let index_xml = Self::extract_ltfs_index_section(xml_content)?;
        
        Self::validate_xml_structure(&index_xml)?;

        // 添加XML结构调试信息
        if tracing::enabled!(tracing::Level::DEBUG) {
            Self::debug_xml_structure(&index_xml);
        }

        let index: LtfsIndex = quick_xml::de::from_str(&index_xml).map_err(|e| {
            // 添加详细的解析错误信息
            let error_msg = format!(
                "Failed to parse LTFS index XML: {} (XML size: {} bytes)",
                e,
                index_xml.len()
            );
            
            // 输出XML的前1000个字符用于调试
            let preview = &index_xml[..std::cmp::min(1000, index_xml.len())];
            warn!("XML parsing failed. Content preview:\n{}", preview);
            
            crate::error::RustLtfsError::parse(error_msg)
        })?;

        // Post-validation of parsed index
        Self::validate_parsed_index(&index)?;

        info!(
            "Successfully parsed LTFS index, version: {}, generation: {}, files: {}",
            index.version,
            index.generationnumber,
            Self::count_files_in_index(&index)
        );

        Ok(index)
    }

    /// Parse LTFS index from XML content using streaming parser for very large files
    pub fn from_xml_streaming(xml_content: &str) -> Result<Self> {
        debug!(
            "Parsing large LTFS index using streaming parser, length: {}",
            xml_content.len()
        );

        // For very large indexes (>100MB), use streaming approach
        if xml_content.len() > 100_000_000 {
            warn!(
                "Large XML detected ({} bytes), parsing may take time",
                xml_content.len()
            );

            // Try chunked parsing approach
            match Self::parse_xml_in_chunks(xml_content) {
                Ok(index) => return Ok(index),
                Err(e) => {
                    warn!(
                        "Chunked parsing failed: {}, falling back to standard parsing",
                        e
                    );
                }
            }
        }

        // Fallback to standard parsing
        Self::from_xml(xml_content)
    }

    /// Extract LTFS index section from combined XML content 
    /// (separates ltfsindex from ltfslabel if both are present)
    fn extract_ltfs_index_section(xml_content: &str) -> Result<String> {
        debug!("Extracting LTFS index section from XML content");
        
        // 查找ltfsindex标签的开始和结束位置
        if let Some(index_start) = xml_content.find("<ltfsindex") {
            if let Some(index_end) = xml_content.find("</ltfsindex>") {
                let start_pos = index_start;
                let end_pos = index_end + "</ltfsindex>".len();
                
                // 提取ltfsindex部分
                let mut index_section = xml_content[start_pos..end_pos].to_string();
                
                // 确保有XML声明
                if !index_section.trim_start().starts_with("<?xml") {
                    index_section = format!("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n{}", index_section);
                }
                
                info!(
                    "Successfully extracted LTFS index section: {} bytes (from {} bytes total)",
                    index_section.len(),
                    xml_content.len()
                );
                
                return Ok(index_section);
            }
        }
        
        // 如果没有找到ltfsindex标签，可能整个内容就是索引
        if xml_content.contains("<ltfsindex") {
            debug!("XML content appears to be pure LTFS index format");
            return Ok(xml_content.to_string());
        }
        
        Err(crate::error::RustLtfsError::parse(
            "No LTFS index section found in XML content".to_string()
        ))
    }

    /// Debug XML structure to understand format issues
    fn debug_xml_structure(xml_content: &str) {
        debug!("Analyzing LTFS XML structure for compatibility debugging");
        
        // 查找location标签
        if let Some(location_start) = xml_content.find("<location") {
            let location_end = xml_content[location_start..]
                .find("</location>")
                .unwrap_or(200);
            let location_section = &xml_content[location_start..location_start + location_end + 11];
            debug!("Found location section: {}", location_section);
        }
        
        // 查找extent标签
        if let Some(extent_start) = xml_content.find("<extent") {
            let extent_end = xml_content[extent_start..]
                .find("/>")
                .or_else(|| xml_content[extent_start..].find("</extent>"))
                .unwrap_or(200);
            let extent_section = &xml_content[extent_start..extent_start + extent_end + 2];
            debug!("Found extent section: {}", extent_section);
        }
        
        // 查找所有可能的startblock字段变体
        let startblock_variants = ["startblock", "startBlock", "start_block", "block"];
        for variant in &startblock_variants {
            if xml_content.contains(variant) {
                debug!("Found field variant '{}' in XML", variant);
            }
        }
    }

    /// Validate XML structure before parsing
    fn validate_xml_structure(xml_content: &str) -> Result<()> {
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
    fn validate_parsed_index(index: &LtfsIndex) -> Result<()> {
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

        info!(
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
    fn count_files_in_index(index: &LtfsIndex) -> usize {
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

    /// Parse XML in chunks for very large files (experimental)
    fn parse_xml_in_chunks(xml_content: &str) -> Result<Self> {
        debug!("Attempting chunked XML parsing");

        // This is a simplified chunked approach
        // For production, would need more sophisticated streaming XML parser

        // For now, try to split the XML and reassemble if possible
        // This is mainly to test memory usage patterns with large indexes

        const CHUNK_SIZE: usize = 10_000_000; // 10MB chunks
        let mut processed_xml = String::new();

        for (i, chunk) in xml_content.as_bytes().chunks(CHUNK_SIZE).enumerate() {
            let chunk_str = String::from_utf8_lossy(chunk);
            debug!("Processing XML chunk {} ({} bytes)", i, chunk.len());
            processed_xml.push_str(&chunk_str);

            // Small delay to prevent memory pressure
            std::thread::sleep(std::time::Duration::from_millis(1));
        }

        // Parse the reassembled XML
        Self::from_xml(&processed_xml)
    }

    /// Serialize LTFS index to XML string
    pub fn to_xml(&self) -> Result<String> {
        let xml_string = quick_xml::se::to_string(self).map_err(|e| {
            crate::error::RustLtfsError::file_operation(format!(
                "Failed to serialize LTFS index to XML: {}",
                e
            ))
        })?;

        // Add XML declaration
        let complete_xml = format!("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n{}", xml_string);

        debug!(
            "Serialized LTFS index to XML ({} bytes)",
            complete_xml.len()
        );
        Ok(complete_xml)
    }

    /// Get next available file UID
    pub fn get_next_file_uid(&self) -> u64 {
        // 优先使用highestfileuid字段（符合LTFS规范）
        if let Some(highest_uid) = self.highestfileuid {
            return highest_uid + 1;
        }

        // 如果没有highestfileuid字段，则遍历所有文件查找最大UID
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
        debug!(
            "Inserting file '{}' into directory '{}'",
            file.name, normalized_path
        );

        if normalized_path == "/" || normalized_path.is_empty() {
            // Insert into root directory
            self.root_directory.contents.files.push(file);
            return Ok(());
        }

        // For simplicity, just insert into root for now
        // TODO: Implement proper directory traversal later
        info!(
            "Simplified implementation: inserting '{}' into root directory",
            file.name
        );
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
            PathType::File(_) => Err(crate::error::RustLtfsError::file_operation(format!(
                "Path {} is a file, not a directory",
                path
            ))),
            PathType::NotFound => Err(crate::error::RustLtfsError::file_operation(format!(
                "Directory {} not found",
                path
            ))),
        }
    }

    /// Get file information
    pub fn get_file_info(&self, path: &str) -> Result<File> {
        debug!("Getting file info: {}", path);

        match self.find_path(path)? {
            PathType::File(file) => Ok(file),
            PathType::Directory(_) => Err(crate::error::RustLtfsError::file_operation(format!(
                "Path {} is a directory, not a file",
                path
            ))),
            PathType::NotFound => Err(crate::error::RustLtfsError::file_operation(format!(
                "File {} not found",
                path
            ))),
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
    fn find_path_recursive(
        &self,
        current_dir: &Directory,
        path_parts: &[&str],
        index: usize,
    ) -> Result<PathType> {
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
        let mut extents = self.extent_info.extents.clone();
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
        !self.extent_info.extents.is_empty()
    }

    /// Validate file consistency - enhanced version
    pub fn validate(&self) -> Result<()> {
        // Check basic fields
        if self.name.is_empty() {
            return Err(crate::error::RustLtfsError::parse(
                "File name cannot be empty",
            ));
        }

        if self.uid == 0 {
            return Err(crate::error::RustLtfsError::parse(format!(
                "File '{}' has invalid UID 0",
                self.name
            )));
        }

        // Validate extents if not a symlink
        if !self.is_symlink() {
            if self.length > 0 && !self.has_extents() {
                return Err(crate::error::RustLtfsError::parse(format!(
                    "File '{}' has size {} but no extents",
                    self.name, self.length
                )));
            }

            // Validate extent consistency
            let mut total_size = 0u64;
            let mut last_offset = 0u64;

            for extent in &self.extent_info.extents {
                extent.validate()?;

                // Check extent ordering
                if extent.file_offset < last_offset {
                    return Err(crate::error::RustLtfsError::parse(format!(
                        "File '{}': extents not properly ordered",
                        self.name
                    )));
                }

                last_offset = extent.file_offset + extent.byte_count;
                total_size += extent.byte_count;
            }

            if total_size != self.length {
                return Err(crate::error::RustLtfsError::parse(format!(
                    "File '{}': size mismatch - declared: {}, extents total: {}",
                    self.name, self.length, total_size
                )));
            }
        }

        Ok(())
    }

    /// Check if file is read-only
    pub fn is_read_only(&self) -> bool {
        self.read_only
    }

    /// Check if file is open for write
    pub fn is_open_for_write(&self) -> bool {
        self.openforwrite
    }

    /// Get file's partition locations
    pub fn get_partitions(&self) -> Vec<String> {
        let mut partitions = Vec::new();
        for extent in &self.extent_info.extents {
            if !partitions.contains(&extent.partition) {
                partitions.push(extent.partition.clone());
            }
        }
        partitions
    }

    /// Calculate file fragmentation (number of extents)
    pub fn fragmentation_level(&self) -> usize {
        self.extent_info.extents.len()
    }
}

impl FileExtent {
    /// Validate extent parameters
    pub fn validate(&self) -> Result<()> {
        // Check partition
        match self.partition.to_lowercase().as_str() {
            "a" | "b" => {}
            _ => {
                return Err(crate::error::RustLtfsError::parse(format!(
                    "Invalid partition '{}'",
                    self.partition
                )))
            }
        }

        // Check byte count
        if self.byte_count == 0 {
            return Err(crate::error::RustLtfsError::parse(
                "Extent byte count cannot be zero",
            ));
        }

        // Check block alignment for byte_offset
        const BLOCK_SIZE: u64 = 65536; // 64KB LTO block size
        if self.byte_offset >= BLOCK_SIZE {
            return Err(crate::error::RustLtfsError::parse(format!(
                "Byte offset {} exceeds block size {}",
                self.byte_offset, BLOCK_SIZE
            )));
        }

        Ok(())
    }

    /// Get extent end position
    pub fn end_position(&self) -> u64 {
        self.file_offset + self.byte_count
    }

    /// Check if extent is in data partition (b)
    pub fn is_in_data_partition(&self) -> bool {
        self.partition.to_lowercase() == "b"
    }

    /// Check if extent is in index partition (a)
    pub fn is_in_index_partition(&self) -> bool {
        self.partition.to_lowercase() == "a"
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

/// 磁带文件读取定位信息 - 用于从磁带读取文件的关键结构
#[derive(Debug, Clone)]
pub struct TapeFileLocation {
    /// 文件名
    pub file_name: String,
    /// 文件UID
    pub file_uid: u64,
    /// 文件总大小
    pub file_size: u64,
    /// 数据块信息
    pub extents: Vec<TapeDataExtent>,
}

/// 磁带数据块信息 - 精确定位磁带上的数据
#[derive(Debug, Clone)]
pub struct TapeDataExtent {
    /// 分区 (通常文件数据在 'b' 分区)
    pub partition: String,
    /// 起始块号
    pub start_block: u64,
    /// 块内字节偏移
    pub byte_offset: u64,
    /// 数据字节数
    pub byte_count: u64,
    /// 文件内偏移
    pub file_offset: u64,
}

/// 磁带卷信息
#[derive(Debug, Clone)]
pub struct TapeVolumeInfo {
    pub volume_uuid: String,
    pub generation_number: u64,
    pub update_time: String,
    pub creator: String,
    pub index_partition: String,
    pub index_start_block: u64,
    pub total_files: usize,
}

impl LtfsIndex {
    /// 从Load格式索引中提取所有文件的磁带位置信息
    /// 这是实现磁带文件读取的关键方法
    pub fn extract_tape_file_locations(&self) -> Vec<TapeFileLocation> {
        let mut locations = Vec::new();

        // 递归遍历目录结构，提取所有文件
        self.extract_locations_from_directory(&self.root_directory, &mut locations);

        info!(
            "Extracted tape location information for {} files.",
            locations.len()
        );
        locations
    }

    /// 根据文件路径查找磁带位置信息
    pub fn find_file_location(&self, file_path: &str) -> Option<TapeFileLocation> {
        let normalized_path = self.normalize_path(file_path);
        debug!("查找文件位置: {}", normalized_path);

        // 在Load格式中，文件通常直接在根目录下
        if normalized_path.starts_with('/') {
            let file_name = normalized_path.trim_start_matches('/');

            // 在根目录查找文件
            for file in &self.root_directory.contents.files {
                if file.name == file_name {
                    return Some(self.create_tape_location(file));
                }
            }
        }

        // 递归搜索所有目录
        self.search_file_in_directory(&self.root_directory, &normalized_path)
    }

    /// 验证索引是否为可读取的Load格式
    pub fn validate_for_tape_reading(&self) -> Result<()> {
        // 验证是否为Load格式 (索引存储在partition a)
        if self.location.partition.to_lowercase() != "a" {
            return Err(crate::error::RustLtfsError::parse(format!(
                "索引必须存储在分区a，当前在分区{}",
                self.location.partition
            )));
        }

        // 验证文件数据完整性
        let locations = self.extract_tape_file_locations();
        if locations.is_empty() {
            warn!("索引中没有找到任何文件");
        }

        // 验证每个文件的extent信息
        for location in &locations {
            if location.extents.is_empty() {
                return Err(crate::error::RustLtfsError::parse(format!(
                    "文件 {} 缺少extent信息，无法从磁带读取",
                    location.file_name
                )));
            }

            // 验证extent总大小与文件大小一致
            let total_size: u64 = location.extents.iter().map(|e| e.byte_count).sum();
            if total_size != location.file_size {
                return Err(crate::error::RustLtfsError::parse(format!(
                    "文件 {} 的extent总大小({})与文件大小({})不匹配",
                    location.file_name, total_size, location.file_size
                )));
            }
        }

        info!(
            "Load格式索引验证通过，包含 {} 个可读取文件",
            locations.len()
        );
        Ok(())
    }

    /// 获取磁带卷信息
    pub fn get_tape_volume_info(&self) -> TapeVolumeInfo {
        TapeVolumeInfo {
            volume_uuid: self.volumeuuid.clone(),
            generation_number: self.generationnumber,
            update_time: self.updatetime.clone(),
            creator: self.creator.clone(),
            index_partition: self.location.partition.clone(),
            index_start_block: self.location.startblock,
            total_files: self.count_total_files(),
        }
    }

    /// 计算索引中的总文件数
    pub fn count_total_files(&self) -> usize {
        let mut count = 0;
        self.count_files_recursively(&self.root_directory, &mut count);
        count
    }

    // 私有辅助方法

    fn extract_locations_from_directory(
        &self,
        directory: &Directory,
        locations: &mut Vec<TapeFileLocation>,
    ) {
        // 处理当前目录的文件
        for file in &directory.contents.files {
            locations.push(self.create_tape_location(file));
        }

        // 递归处理子目录
        for subdir in &directory.contents.directories {
            self.extract_locations_from_directory(subdir, locations);
        }
    }

    fn create_tape_location(&self, file: &File) -> TapeFileLocation {
        let extents: Vec<TapeDataExtent> = file
            .extent_info
            .extents
            .iter()
            .map(|extent| TapeDataExtent {
                partition: extent.partition.clone(),
                start_block: extent.start_block,
                byte_offset: extent.byte_offset,
                byte_count: extent.byte_count,
                file_offset: extent.file_offset,
            })
            .collect();

        TapeFileLocation {
            file_name: file.name.clone(),
            file_uid: file.uid,
            file_size: file.length,
            extents,
        }
    }

    fn search_file_in_directory(
        &self,
        directory: &Directory,
        target_path: &str,
    ) -> Option<TapeFileLocation> {
        // 在当前目录查找文件
        for file in &directory.contents.files {
            if target_path.ends_with(&file.name) {
                return Some(self.create_tape_location(file));
            }
        }

        // 递归搜索子目录
        for subdir in &directory.contents.directories {
            if let Some(location) = self.search_file_in_directory(subdir, target_path) {
                return Some(location);
            }
        }

        None
    }

    fn count_files_recursively(&self, directory: &Directory, count: &mut usize) {
        *count += directory.contents.files.len();
        for subdir in &directory.contents.directories {
            self.count_files_recursively(subdir, count);
        }
    }
}

impl TapeDataExtent {
    /// 计算extent在磁带上的结束位置
    pub fn end_block(&self) -> u64 {
        // 假设每个块64KB
        self.start_block + ((self.byte_count + 65535) / 65536)
    }

    /// 检查是否在数据分区
    pub fn is_in_data_partition(&self) -> bool {
        self.partition.to_lowercase() == "b"
    }
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
            previousgenerationlocation: None,
            allowpolicyupdate: None,
            volumelockstate: None,
            highestfileuid: None,
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
            openforwrite: false,
            symlink: None,
            extent_info: ExtentInfo::default(),
            extended_attributes: None,
        };

        let entry = DirectoryEntry::File(file);
        assert_eq!(entry.name(), "test.txt");
        assert_eq!(entry.size(), 1024);
        assert!(!entry.is_directory());
    }
}
