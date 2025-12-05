//! LTFS Index Writing and Management Operations
//!
//! This module handles LTFS index creation, update, and management.

use super::super::TapeOperations;
use super::super::utils::{get_current_ltfs_timestamp, system_time_to_ltfs_timestamp};
use crate::error::{Result, RustLtfsError};
use crate::ltfs_index::LtfsIndex;
use std::collections::HashMap;
use std::path::Path;
use tracing::debug;

/// Index management operations for TapeOperations
impl TapeOperations {
    /// Create new empty LTFS index
    fn create_new_ltfs_index(&self) -> LtfsIndex {
        use uuid::Uuid;

        let now = get_current_ltfs_timestamp();
        let volume_uuid = Uuid::new_v4();

        LtfsIndex {
            version: "2.4.0".to_string(),
            creator: "RustLTFS".to_string(),
            volumeuuid: volume_uuid.to_string(),
            generationnumber: 1,
            updatetime: now.clone(),
            location: crate::ltfs_index::Location {
                partition: "b".to_string(), // Data partition
                startblock: 0,
            },
            previousgenerationlocation: None,
            allowpolicyupdate: Some(true),
            volumelockstate: None,
            highestfileuid: Some(1),
            root_directory: crate::ltfs_index::Directory {
                name: "".to_string(),
                uid: 1,
                creation_time: now.clone(),
                change_time: now.clone(),
                modify_time: now.clone(),
                access_time: now.clone(),
                backup_time: now,
                read_only: false,
                contents: crate::ltfs_index::DirectoryContents {
                    files: Vec::new(),
                    directories: Vec::new(),
                },
            },
        }
    }

    /// Add file to target directory, creating directories as needed
    /// This function handles UID allocation AFTER directory creation to prevent conflicts
    pub fn add_file_to_target_directory(
        &self,
        index: &mut LtfsIndex,
        file: crate::ltfs_index::File,
        target_path: &str,
    ) -> Result<()> {
        debug!(
            "Adding file '{}' to target path '{}'",
            file.name, target_path
        );

        // Normalize target path
        let normalized_path = target_path.trim_start_matches('/').trim_end_matches('/');
        debug!("Normalized path: '{}'", normalized_path);

        if normalized_path.is_empty() {
            // Add to root directory - allocate UID here
            let file_name = file.name.clone();
            let mut file_to_add = file;
            let new_file_uid = index.highestfileuid.unwrap_or(0) + 1;
            file_to_add.uid = new_file_uid;
            index.highestfileuid = Some(new_file_uid);

            debug!(
                "Adding file '{}' to root directory with UID {}",
                file_name, new_file_uid
            );
            index.root_directory.contents.files.push(file_to_add);
            debug!(
                "Root directory now has {} files",
                index.root_directory.contents.files.len()
            );
            return Ok(());
        }

        // Split path into components
        let path_parts: Vec<&str> = normalized_path.split('/').collect();
        debug!("Target path components: {:?}", path_parts);

        // Navigate to target directory, creating directories as needed
        debug!("Finding/creating target directory path...");
        // First ensure directory path exists (this may update highestfileuid)
        {
            self.ensure_directory_path_exists(index, &path_parts)?;
        }
        debug!("Target directory found/created, adding file...");

        // CRITICAL: Allocate file UID AFTER directory creation to avoid conflicts
        // Directory creation may have updated highestfileuid, so we get fresh value
        let file_name = file.name.clone();
        let mut file_to_add = file;
        let new_file_uid = index.highestfileuid.unwrap_or(0) + 1;
        file_to_add.uid = new_file_uid;
        index.highestfileuid = Some(new_file_uid);

        debug!(
            "Allocated UID {} for file '{}' after directory creation",
            new_file_uid, file_name
        );

        // Now get a fresh reference to the target directory to add the file
        let target_dir = self.get_directory_by_path_mut(index, &path_parts)?;
        target_dir.contents.files.push(file_to_add);
        debug!(
            "File '{}' added to directory '{}', directory now has {} files",
            file_name,
            normalized_path,
            target_dir.contents.files.len()
        );

        Ok(())
    }

    /// Ensure directory path exists, creating directories as needed
    fn ensure_directory_path_exists<'a>(
        &self,
        index: &'a mut LtfsIndex,
        path_parts: &[&str],
    ) -> Result<&'a mut crate::ltfs_index::Directory> {
        debug!(
            "ensure_directory_path_exists called with path_parts: {:?}",
            path_parts
        );

        if path_parts.is_empty() {
            debug!("Path parts empty, returning root directory");
            return Ok(&mut index.root_directory);
        }

        let mut current_dir = &mut index.root_directory;
        debug!(
            "Starting at root directory with {} subdirectories",
            current_dir.contents.directories.len()
        );

        for (i, part) in path_parts.iter().enumerate() {
            debug!("Processing directory part: '{}' (level {})", part, i);
            debug!(
                "Current directory has {} subdirectories",
                current_dir.contents.directories.len()
            );

            // Find existing directory or create new one
            let dir_index = current_dir
                .contents
                .directories
                .iter()
                .position(|d| d.name == *part);

            match dir_index {
                Some(idx) => {
                    debug!("Found existing directory: '{}' at index {}", part, idx);
                    // Directory exists, continue navigation
                    current_dir = &mut current_dir.contents.directories[idx];
                }
                None => {
                    debug!("Creating new directory: '{}'", part);
                    // Create new directory
                    let now = get_current_ltfs_timestamp();
                    let new_uid = index.highestfileuid.unwrap_or(0) + 1;
                    debug!("New directory UID: {}", new_uid);

                    let new_directory = crate::ltfs_index::Directory {
                        name: part.to_string(),
                        uid: new_uid,
                        creation_time: now.clone(),
                        change_time: now.clone(),
                        modify_time: now.clone(),
                        access_time: now.clone(),
                        backup_time: now,
                        read_only: false,
                        contents: crate::ltfs_index::DirectoryContents {
                            files: Vec::new(),
                            directories: Vec::new(),
                        },
                    };

                    current_dir.contents.directories.push(new_directory);
                    index.highestfileuid = Some(new_uid);
                    debug!("Directory '{}' created and added, current directory now has {} subdirectories",
                           part, current_dir.contents.directories.len());

                    // Navigate to newly created directory
                    let last_index = current_dir.contents.directories.len() - 1;
                    current_dir = &mut current_dir.contents.directories[last_index];
                    debug!("Navigated to newly created directory '{}'", part);
                }
            }
        }

        debug!(
            "Final target directory reached, has {} files, {} subdirectories",
            current_dir.contents.files.len(),
            current_dir.contents.directories.len()
        );
        Ok(current_dir)
    }

    /// Get mutable reference to directory by path (helper function for add_file_to_target_directory)
    fn get_directory_by_path_mut<'a>(
        &self,
        index: &'a mut LtfsIndex,
        path_parts: &[&str],
    ) -> Result<&'a mut crate::ltfs_index::Directory> {
        if path_parts.is_empty() {
            return Ok(&mut index.root_directory);
        }

        let mut current_dir = &mut index.root_directory;

        for part in path_parts.iter() {
            let dir_index = current_dir
                .contents
                .directories
                .iter()
                .position(|d| d.name == *part)
                .ok_or_else(|| {
                    RustLtfsError::ltfs_index(format!("Directory '{}' not found in path", part))
                })?;

            current_dir = &mut current_dir.contents.directories[dir_index];
        }

        Ok(current_dir)
    }






    // ================== 索引更新相关 ==================

    /// Enhanced index update for file write (对应LTFSCopyGUI的索引更新逻辑)
    pub fn update_index_for_file_write_enhanced(
        &mut self,
        source_path: &Path,
        target_path: &str,
        file_size: u64,
        write_position: &crate::scsi::TapePosition,
        file_hashes: Option<HashMap<String, String>>,
    ) -> Result<()> {
        debug!(
            "Updating LTFS index for write: {:?} -> {} ({} bytes)",
            source_path, target_path, file_size
        );

        // Get or create current index
        let mut current_index = match &self.index {
            Some(index) => index.clone(),
            None => {
                // Create new index if none exists
                self.create_new_ltfs_index()
            }
        };

        // Create new file entry with enhanced metadata
        let file_name = source_path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("unknown")
            .to_string();

        let now = get_current_ltfs_timestamp();
        // NOTE: UID will be allocated in add_file_to_target_directory() after directories are created
        // This prevents UID conflicts when creating nested directories

        let extent = crate::ltfs_index::FileExtent {
            // 使用实际写入位置的分区信息，而不是硬编码
            partition: if write_position.partition == 0 {
                "a".to_string()
            } else {
                "b".to_string()
            },
            start_block: write_position.block_number,
            byte_count: file_size,
            file_offset: 0,
            byte_offset: 0,
        };

        // Get file metadata for timestamps
        let metadata = std::fs::metadata(source_path).map_err(|e| {
            RustLtfsError::file_operation(format!("Cannot get file metadata: {}", e))
        })?;

        let creation_time = metadata
            .created()
            .map(|t| system_time_to_ltfs_timestamp(t))
            .unwrap_or_else(|_| now.clone());

        let modify_time = metadata
            .modified()
            .map(|t| system_time_to_ltfs_timestamp(t))
            .unwrap_or_else(|_| now.clone());

        let access_time = metadata
            .accessed()
            .map(|t| system_time_to_ltfs_timestamp(t))
            .unwrap_or_else(|_| now.clone());

        let new_file = crate::ltfs_index::File {
            name: file_name,
            uid: 0, // Temporary placeholder - will be assigned in add_file_to_target_directory
            length: file_size,
            creation_time: creation_time,
            change_time: now.clone(),
            modify_time: modify_time,
            access_time: access_time,
            backup_time: now,
            read_only: false,
            openforwrite: false,
            symlink: None,
            extent_info: crate::ltfs_index::ExtentInfo {
                extents: vec![extent],
            },
            extended_attributes: if let Some(hashes) = file_hashes {
                // Create extended attributes following LTFSCopyGUI format
                let mut attributes = Vec::new();

                for (hash_key, hash_value) in hashes {
                    attributes.push(crate::ltfs_index::ExtendedAttribute {
                        key: hash_key, // Already contains full key name like "ltfs.hash.sha1sum"
                        value: hash_value,
                    });
                }

                // Add capacity remain attribute (placeholder)
                attributes.push(crate::ltfs_index::ExtendedAttribute {
                    key: "ltfscopygui.capacityremain".to_string(),
                    value: "12".to_string(), // Placeholder value
                });

                Some(crate::ltfs_index::ExtendedAttributes { attributes })
            } else {
                None
            },
        };

        // Parse target path and add file to appropriate directory
        debug!(
            "Before adding file: root directory has {} files, {} directories",
            current_index.root_directory.contents.files.len(),
            current_index.root_directory.contents.directories.len()
        );
        debug!(
            "Adding file '{}' to target path: '{}'",
            new_file.name, target_path
        );
        self.add_file_to_target_directory(&mut current_index, new_file, target_path)?;
        debug!(
            "After adding file: root directory has {} files, {} directories",
            current_index.root_directory.contents.files.len(),
            current_index.root_directory.contents.directories.len()
        );

        // Update index metadata
        current_index.generationnumber += 1;
        current_index.updatetime = get_current_ltfs_timestamp();
        // NOTE: highestfileuid is updated in add_file_to_target_directory

        // Update internal index
        self.index = Some(current_index.clone());
        self.schema = Some(current_index);
        self.modified = true; // Mark as modified for later index writing

        debug!("LTFS index updated with new file");
        Ok(())
    }

    /// Basic index update for file write operation
    pub fn update_index_for_file_write(
        &mut self,
        source_path: &Path,
        target_path: &str,
        file_size: u64,
        write_position: &crate::scsi::TapePosition,
    ) -> Result<()> {
        debug!(
            "Updating LTFS index for write: {:?} -> {} ({} bytes)",
            source_path, target_path, file_size
        );

        // Get or create current index
        let mut current_index = match &self.index {
            Some(index) => index.clone(),
            None => {
                // Create new index if none exists
                self.create_new_ltfs_index()
            }
        };

        // Create new file entry
        let file_name = source_path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("unknown")
            .to_string();

        let now = get_current_ltfs_timestamp();
        // NOTE: UID will be allocated in add_file_to_target_directory() after directories are created
        // This prevents UID conflicts when creating nested directories

        let extent = crate::ltfs_index::FileExtent {
            // 使用实际写入位置的分区信息，而不是硬编码
            partition: if write_position.partition == 0 {
                "a".to_string()
            } else {
                "b".to_string()
            },
            start_block: write_position.block_number,
            byte_count: file_size,
            file_offset: 0,
            byte_offset: 0,
        };

        let new_file = crate::ltfs_index::File {
            name: file_name,
            uid: 0, // Temporary placeholder - will be assigned in add_file_to_target_directory
            length: file_size,
            creation_time: now.clone(),
            change_time: now.clone(),
            modify_time: now.clone(),
            access_time: now.clone(),
            backup_time: now,
            read_only: false,
            openforwrite: false,
            symlink: None,
            extent_info: crate::ltfs_index::ExtentInfo {
                extents: vec![extent],
            },
            extended_attributes: None,
        };

        // Parse target path and add file to appropriate directory
        self.add_file_to_target_directory(&mut current_index, new_file, target_path)?;

        // Update index metadata
        current_index.generationnumber += 1;
        current_index.updatetime = get_current_ltfs_timestamp();
        // NOTE: highestfileuid is updated in add_file_to_target_directory

        // Update internal index
        self.index = Some(current_index.clone());

        debug!("LTFS index updated with new file");
        Ok(())
    }
}
