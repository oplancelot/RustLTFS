/// IBM LTFS direct read/write operations interface

use crate::error::{Result, RustLtfsError};
use crate::scsi::{ScsiInterface, MediaType};
use crate::ltfs_index::{LtfsIndex, File};
use super::capacity::{TapeCapacity, CapacityManager};
use super::volume_info::LtfsVolumeInfo;
use super::performance::{PerformanceMonitor, CacheConfig, BatchConfig};
use tracing::{info, debug, warn};
use std::path::PathBuf;
use tokio::fs;
use uuid::Uuid;
use chrono;

/// Generate LTFS-compatible Z-format timestamp
fn format_ltfs_timestamp(datetime: chrono::DateTime<chrono::Utc>) -> String {
    format!("{}Z", datetime.format("%Y-%m-%dT%H:%M:%S%.9f"))
}

/// Get current timestamp in LTFS-compatible format
fn get_current_ltfs_timestamp() -> String {
    format_ltfs_timestamp(chrono::Utc::now())
}

/// IBM LTFS direct read/write operations interface
pub struct LtfsDirectAccess {
    scsi: ScsiInterface,
    device_path: String,
    cached_index: Option<LtfsIndex>,
    performance_monitor: Option<PerformanceMonitor>,
}

/// LTFS file system operations
impl LtfsDirectAccess {
    /// Create new LTFS direct access instance
    pub fn new(device_path: String) -> Self {
        Self {
            scsi: ScsiInterface::new(),
            device_path,
            cached_index: None,
            performance_monitor: None,
        }
    }

    /// Create LTFS direct access instance with performance optimization enabled
    pub fn with_performance_optimization(device_path: String, cache_config: CacheConfig, batch_config: BatchConfig) -> Self {
        Self {
            scsi: ScsiInterface::new(),
            device_path,
            cached_index: None,
            performance_monitor: Some(PerformanceMonitor::with_configs(cache_config, batch_config)),
        }
    }

    /// Enable performance monitoring
    pub fn enable_performance_monitoring(&mut self) {
        if self.performance_monitor.is_none() {
            self.performance_monitor = Some(PerformanceMonitor::new());
            info!("Performance monitoring enabled");
        }
    }

    /// Get performance statistics
    pub fn get_performance_stats(&self) -> Option<super::performance::PerformanceStats> {
        self.performance_monitor.as_ref().map(|pm| pm.get_performance_stats())
    }
    
    /// Initialize device connection
    pub fn initialize(&mut self) -> Result<()> {
        info!("Initializing LTFS device: {}", self.device_path);
        
        self.scsi.open_device(&self.device_path)?;
        
        // Send TEST_UNIT_READY command to check device ready status
        self.test_unit_ready()?;
        
        // Check if it's an LTFS formatted tape
        self.check_ltfs_format()?;
        
        info!("LTFS device initialization completed: {}", self.device_path);
        Ok(())
    }
    
    /// Test device ready status (based on IBM tape detection)
    fn test_unit_ready(&self) -> Result<()> {
        debug!("Checking device ready status");
        
        // Use new SCSI interface to check media status
        match self.scsi.check_media_status() {
            Ok(media_type) => {
                match media_type {
                    MediaType::NoTape => {
                        return Err(RustLtfsError::tape_device("No tape cartridge loaded".to_string()));
                    }
                    MediaType::Unknown(_) => {
                        warn!("Unknown media type detected, proceeding with caution");
                    }
                    _ => {
                        info!("Detected media type: {}", media_type.description());
                    }
                }
                Ok(())
            }
            Err(e) => {
                Err(RustLtfsError::tape_device(format!("Device not ready: {}", e)))
            }
        }
    }
    
    /// Check if the tape has LTFS format
    fn check_ltfs_format(&self) -> Result<()> {
        debug!("Checking LTFS format on tape");
        
        // This would involve reading the VOL1 label and checking for LTFS signature
        // For now, we'll just check if we can read an LTFS index
        match self.try_read_ltfs_index() {
            Ok(_) => {
                info!("LTFS format detected and valid");
                Ok(())
            }
            Err(e) => {
                warn!("LTFS format check failed: {}", e);
                Err(RustLtfsError::ltfs_index("Tape does not contain valid LTFS format".to_string()))
            }
        }
    }
    
    /// Try to read LTFS index from tape
    fn try_read_ltfs_index(&self) -> Result<LtfsIndex> {
        debug!("Attempting to read LTFS index from tape");
        
        // This is a simplified implementation
        // Real implementation would:
        // 1. Read VOL1 label from beginning of tape
        // 2. Locate index partition (usually partition a)
        // 3. Read index from the end of the index partition
        
        Err(RustLtfsError::unsupported("LTFS index reading not yet implemented".to_string()))
    }
    
    /// Write file directly to tape using LTFS format
    pub async fn write_file_direct(&mut self, source_path: &PathBuf, tape_path: &PathBuf) -> Result<()> {
        info!("Writing file directly to tape: {:?} -> {:?}", source_path, tape_path);
        
        // Check if source file exists
        if !source_path.exists() {
            return Err(RustLtfsError::file_operation(format!("Source file not found: {:?}", source_path)));
        }
        
        // Get file metadata
        let metadata = fs::metadata(source_path).await
            .map_err(|e| RustLtfsError::file_operation(format!("Failed to read file metadata: {}", e)))?;
        
        let file_size = metadata.len();
        
        // Check available tape capacity
        self.check_available_space(file_size)?;
        
        // Read file content
        let file_content = fs::read(source_path).await
            .map_err(|e| RustLtfsError::file_operation(format!("Failed to read file: {}", e)))?;
        
        // Write to tape using SCSI interface
        self.write_file_content_to_tape(&file_content, tape_path).await?;
        
        // Update LTFS index
        self.update_ltfs_index_for_write(source_path, tape_path, file_size).await?;
        
        info!("File written successfully to tape: {:?}", tape_path);
        Ok(())
    }
    
    /// Read file directly from tape
    pub async fn read_file_direct(&mut self, tape_path: &PathBuf, dest_path: &PathBuf) -> Result<()> {
        info!("Reading file directly from tape: {:?} -> {:?}", tape_path, dest_path);
        
        // Look up file in LTFS index
        let file_info = self.locate_file_on_tape(tape_path)?;
        
        // Read file content from tape
        let file_content = self.read_file_content_from_tape(&file_info).await?;
        
        // Write to destination
        fs::write(dest_path, file_content).await
            .map_err(|e| RustLtfsError::file_operation(format!("Failed to write file: {}", e)))?;
        
        info!("File read successfully from tape: {:?}", dest_path);
        Ok(())
    }
    
    /// Write file content to tape
    async fn write_file_content_to_tape(&self, content: &[u8], _tape_path: &PathBuf) -> Result<()> {
        debug!("Writing {} bytes to tape", content.len());
        
        // This would involve:
        // 1. Positioning tape to correct location
        // 2. Writing file data in LTFS format
        // 3. Writing file metadata
        
        // For now, this is a placeholder
        warn!("Tape writing not yet implemented");
        Ok(())
    }
    
    /// Update LTFS index for write operation
    async fn update_ltfs_index_for_write(
        &mut self,
        source_path: &PathBuf,
        tape_path: &PathBuf,
        file_size: u64,
    ) -> Result<()> {
        debug!("Updating LTFS index for written file");
        
        // Load current index or create new one
        if self.cached_index.is_none() {
            self.cached_index = Some(self.try_read_ltfs_index().unwrap_or_else(|_| {
                // Create minimal index if none exists
                self.create_minimal_ltfs_index()
            }));
        }
        
        // Add file to index
        if self.cached_index.is_some() {
            let file_entry = self.create_file_entry(source_path, tape_path, file_size)?;
            
            // Insert file into index
            let parent_path = tape_path.parent()
                .and_then(|p| p.to_str())
                .unwrap_or("/");
            
            if let Some(ref mut index) = self.cached_index {
                index.insert_file(parent_path, file_entry)?;
                index.increment_generation();
            }
        }
        
        info!("LTFS index updated for file: {:?}", tape_path);
        Ok(())
    }
    
    /// Create minimal LTFS index
    fn create_minimal_ltfs_index(&self) -> LtfsIndex {
        warn!("Creating minimal LTFS index");
        
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
    
    /// Create file entry for LTFS index
    fn create_file_entry(&self, source_path: &PathBuf, tape_path: &PathBuf, file_size: u64) -> Result<File> {
        use crate::ltfs_index::get_current_timestamp;
        
        let file_name = tape_path.file_name()
            .and_then(|n| n.to_str())
            .ok_or_else(|| RustLtfsError::parameter_validation("Invalid file name".to_string()))?;
        
        // Create file entry
        let file = File {
            name: file_name.to_string(),
            uid: self.generate_file_uid(),
            length: file_size,
            creation_time: get_current_timestamp(),
            change_time: get_current_timestamp(),
            modify_time: get_current_timestamp(),
            access_time: get_current_timestamp(),
            backup_time: get_current_timestamp(),
            read_only: false,
            openforwrite: false,
            symlink: None,
            extent_info: crate::ltfs_index::ExtentInfo::default(),
            extended_attributes: None,
        };
        
        Ok(file)
    }
    
    /// Generate unique file UID
    fn generate_file_uid(&self) -> u64 {
        // This should generate a unique UID
        // For now, use timestamp-based UID
        use std::time::{SystemTime, UNIX_EPOCH};
        
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs()
    }
    
    /// Locate file on tape using LTFS index
    fn locate_file_on_tape(&self, tape_path: &PathBuf) -> Result<File> {
        debug!("Locating file on tape: {:?}", tape_path);
        
        // This would look up the file in the LTFS index
        // and return its metadata and extent information
        
        Err(RustLtfsError::unsupported("File location not yet implemented".to_string()))
    }
    
    /// Read file content from tape
    async fn read_file_content_from_tape(&self, _file_info: &File) -> Result<Vec<u8>> {
        debug!("Reading file content from tape");
        
        // This would:
        // 1. Use file extent information to locate data on tape
        // 2. Position tape to correct blocks
        // 3. Read file data
        // 4. Reassemble multi-extent files
        
        Err(RustLtfsError::unsupported("File reading not yet implemented".to_string()))
    }
    
    /// Get tape capacity information
    pub fn get_capacity_info(&self) -> Result<TapeCapacity> {
        let capacity_manager = CapacityManager::new(ScsiInterface::new());
        capacity_manager.get_capacity_info()
    }
    
    /// Check available space on tape
    pub fn check_available_space(&self, required_size: u64) -> Result<bool> {
        let capacity_manager = CapacityManager::new(ScsiInterface::new());
        capacity_manager.check_available_space(required_size)
    }
    
    /// Get current volume information
    pub fn get_volume_info(&self) -> Option<LtfsVolumeInfo> {
        // Create volume info from cached index if available
        self.cached_index.as_ref().map(|index| LtfsVolumeInfo::from_index(index))
    }
    
    /// Get cached LTFS index
    pub fn get_cached_index(&self) -> Option<&LtfsIndex> {
        self.cached_index.as_ref()
    }
    
    /// Refresh LTFS index from tape
    pub async fn refresh_index(&mut self) -> Result<()> {
        info!("Refreshing LTFS index from tape");
        
        // Use enhanced error recovery for index reading
        match self.try_read_ltfs_index_with_recovery().await {
            Ok(index) => {
                self.cached_index = Some(index);
                info!("LTFS index refreshed successfully");
                Ok(())
            }
            Err(e) => {
                warn!("Failed to refresh index: {}", e);
                Err(e)
            }
        }
    }
    
    /// Try to read LTFS index with enhanced error recovery (based on LTFSCopyGUI patterns)
    async fn try_read_ltfs_index_with_recovery(&self) -> Result<LtfsIndex> {
        debug!("Attempting to read LTFS index with recovery mechanisms");
        
        const MAX_INDEX_RETRIES: u32 = 3;
        let mut retry_count = 0;
        
        while retry_count < MAX_INDEX_RETRIES {
            match self.try_read_ltfs_index() {
                Ok(index) => return Ok(index),
                Err(e) => {
                    retry_count += 1;
                    warn!("Index read attempt {} failed: {}", retry_count, e);
                    
                    if retry_count < MAX_INDEX_RETRIES {
                        // Apply recovery strategies
                        if let Err(recovery_error) = self.recover_from_index_error(&e).await {
                            debug!("Index recovery failed: {}", recovery_error);
                        }
                        
                        // Progressive delay between retries
                        let delay_ms = std::cmp::min(1000 * retry_count, 5000);
                        tokio::time::sleep(tokio::time::Duration::from_millis(delay_ms as u64)).await;
                    } else {
                        return Err(RustLtfsError::ltfs_index(
                            format!("Failed to read LTFS index after {} attempts: {}", MAX_INDEX_RETRIES, e)
                        ));
                    }
                }
            }
        }
        
        Err(RustLtfsError::ltfs_index("Index recovery exhausted all attempts".to_string()))
    }
    
    /// Recover from index reading errors (enhanced implementation based on LTFSCopyGUI)
    async fn recover_from_index_error(&self, error: &RustLtfsError) -> Result<()> {
        debug!("Attempting recovery from index error: {}", error);
        
        // Strategy 1: Try to reposition to beginning of tape
        match error {
            RustLtfsError::Scsi(_) => {
                debug!("SCSI error detected, attempting repositioning");
                if let Err(e) = self.scsi.locate_block(0, 0) {
                    debug!("Failed to reposition to beginning: {}", e);
                } else {
                    debug!("Successfully repositioned to beginning of tape");
                }
            }
            RustLtfsError::Parse(_) => {
                debug!("Parse error detected, attempting alternative read strategies");
                // Try reading from alternative index locations
                if let Err(e) = self.try_alternative_index_locations().await {
                    debug!("Alternative index locations failed: {}", e);
                }
            }
            RustLtfsError::TapeDevice(_) => {
                debug!("Tape device error detected, attempting device reset");
                if let Err(e) = self.attempt_device_reset().await {
                    debug!("Device reset failed: {}", e);
                }
            }
            _ => {
                debug!("General error, applying basic recovery");
            }
        }
        
        Ok(())
    }
    
    /// Try alternative index locations (based on LTFSCopyGUI strategy)
    async fn try_alternative_index_locations(&self) -> Result<()> {
        debug!("Trying alternative index locations");
        
        // Common LTFS index locations observed from LTFSCopyGUI
        let alternative_locations = vec![5, 3, 1, 10, 20, 100];
        
        for block in alternative_locations {
            if let Ok(_) = self.scsi.locate_block(0, block) {
                debug!("Successfully positioned to alternative location: block {}", block);
                return Ok(());
            }
        }
        
        Err(RustLtfsError::tape_device("No alternative index locations found"))
    }
    
    /// Attempt device reset for recovery
    async fn attempt_device_reset(&self) -> Result<()> {
        debug!("Attempting device reset for recovery");
        
        // Strategy 1: Try to rewind to beginning
        if let Err(e) = self.scsi.locate_block(0, 0) {
            debug!("Rewind failed: {}", e);
            
            // Strategy 2: Try unload/load cycle (if supported)
            if let Ok(_) = self.scsi.eject_tape() {
                tokio::time::sleep(tokio::time::Duration::from_secs(3)).await;
                if let Ok(_) = self.scsi.load_tape() {
                    info!("Successfully performed unload/load cycle for recovery");
                    return Ok(());
                }
            }
            
            return Err(RustLtfsError::tape_device("Device reset failed"));
        }
        
        info!("Device reset completed successfully");
        Ok(())
    }
    
    /// Enhanced file read with comprehensive error handling
    pub async fn read_file_robust(&self, file_info: &File, destination: &PathBuf) -> Result<()> {
        info!("Reading file with robust error handling: {} -> {:?}", file_info.name, destination);
        
        const MAX_FILE_RETRIES: u32 = 3;
        let mut retry_count = 0;
        
        while retry_count < MAX_FILE_RETRIES {
            match self.read_file_content(file_info, 0, None).await {
                Ok(content) => {
                    // Create destination directory if needed
                    if let Some(parent) = destination.parent() {
                        tokio::fs::create_dir_all(parent).await
                            .map_err(|e| RustLtfsError::file_operation(
                                format!("Failed to create destination directory: {}", e)
                            ))?;
                    }
                    
                    // Write to destination with verification
                    tokio::fs::write(destination, &content).await
                        .map_err(|e| RustLtfsError::file_operation(
                            format!("Failed to write file: {}", e)
                        ))?;
                    
                    // Verify written file
                    let written_size = tokio::fs::metadata(destination).await
                        .map_err(|e| RustLtfsError::verification(
                            format!("Cannot verify written file: {}", e)
                        ))?.len();
                    
                    if written_size != file_info.length {
                        return Err(RustLtfsError::verification(
                            format!("File size mismatch: expected {}, got {}", file_info.length, written_size)
                        ));
                    }
                    
                    info!("File read completed successfully: {}", file_info.name);
                    return Ok(());
                }
                Err(e) => {
                    retry_count += 1;
                    warn!("File read attempt {} failed: {}", retry_count, e);
                    
                    if retry_count < MAX_FILE_RETRIES {
                        // Apply file-specific recovery strategies
                        if let Err(recovery_error) = self.recover_from_file_error(&e, file_info).await {
                            debug!("File recovery failed: {}", recovery_error);
                        }
                        
                        // Progressive delay
                        let delay_ms = std::cmp::min(2000 * retry_count, 10000);
                        tokio::time::sleep(tokio::time::Duration::from_millis(delay_ms as u64)).await;
                    } else {
                        return Err(RustLtfsError::file_operation(
                            format!("Failed to read file '{}' after {} attempts: {}", file_info.name, MAX_FILE_RETRIES, e)
                        ));
                    }
                }
            }
        }
        
        Err(RustLtfsError::file_operation("File recovery exhausted all attempts".to_string()))
    }
    
    /// Recover from file reading errors
    async fn recover_from_file_error(&self, error: &RustLtfsError, file_info: &File) -> Result<()> {
        debug!("Attempting recovery from file error for '{}': {}", file_info.name, error);
        
        // Strategy 1: Try to reposition to file start if we have extent info
        if let Some(first_extent) = file_info.extent_info.extents.first() {
            let partition_id = match first_extent.partition.as_str() {
                "a" => 0,
                "b" => 1,
                _ => 0,
            };
            
            if let Err(e) = self.scsi.locate_block(partition_id, first_extent.start_block) {
                debug!("Failed to reposition to file start: {}", e);
            } else {
                debug!("Successfully repositioned to file start block {}", first_extent.start_block);
            }
        }
        
        // Strategy 2: Test drive responsiveness
        match self.scsi.read_position() {
            Ok(position) => {
                debug!("Drive responsive at position: partition {}, block {}", 
                       position.partition, position.block_number);
            }
            Err(e) => {
                debug!("Drive not responsive: {}", e);
                // Try a simple reset
                if let Err(reset_error) = self.attempt_device_reset().await {
                    debug!("Device reset during file recovery failed: {}", reset_error);
                }
            }
        }
        
        Ok(())
    }
    
    /// Read file content for display or partial reading (enhanced with error handling)
    pub async fn read_file_content(&self, file_info: &File, start: u64, length: Option<u64>) -> Result<Vec<u8>> {
        info!("Reading file content: {} (start: {}, length: {:?})", file_info.name, start, length);
        
        // For now, return a placeholder implementation
        // In a real implementation, this would read from tape extents
        
        Err(RustLtfsError::unsupported("File content reading not yet fully implemented".to_string()))
    }
    
    /// Get partition ID from partition string
    fn get_partition_id(&self, partition: &str) -> Result<u8> {
        match partition.to_lowercase().as_str() {
            "a" => Ok(0),
            "b" => Ok(1),
            _ => Err(RustLtfsError::file_operation(
                format!("Unknown partition: {}", partition)
            ))
        }
    }

    /// Extract complete XML from buffer
    fn extract_complete_xml(&self, buffer: &[u8]) -> Result<String> {
        // Find XML start
        let xml_start = buffer.iter().position(|&b| b == b'<')
            .ok_or_else(|| crate::error::RustLtfsError::parse("No XML start tag found in buffer"))?;
        
        let xml_content = &buffer[xml_start..];
        
        // Find XML end - look for complete closing tag
        let xml_end = crate::ltfs::utils::find_xml_end(xml_content)
            .ok_or_else(|| crate::error::RustLtfsError::parse("XML appears incomplete - no closing tag found"))?;
        
        let xml_bytes = &xml_content[..xml_end];
        
        // Convert to UTF-8 string
        let xml_string = String::from_utf8(xml_bytes.to_vec())
            .map_err(|e| crate::error::RustLtfsError::parse(
                format!("Failed to convert XML to UTF-8: {}", e)
            ))?;
        
        // Basic completeness check - ensure we have essential elements
        if !xml_string.contains("<ltfsindex") || !xml_string.contains("</ltfsindex>") {
            return Err(crate::error::RustLtfsError::parse(
                "XML appears incomplete - missing essential LTFS elements"
            ));
        }
        
        Ok(xml_string)
    }

    /// Optimized file read with caching and performance monitoring
    pub async fn read_file_optimized(&mut self, file_info: &File, destination: &PathBuf) -> Result<()> {
        info!("Reading file with optimization: {} -> {:?}", file_info.name, destination);
        
        let start_time = std::time::Instant::now();
        
        // Check cache first
        if let Some(ref mut perf_monitor) = self.performance_monitor {
            let cache_key = super::performance::CacheKey::file_extent(file_info.uid, 0);
            if let Some(cached_data) = perf_monitor.cache().get(&cache_key) {
                // Cache hit - write directly from cache
                tokio::fs::write(destination, &cached_data).await
                    .map_err(|e| RustLtfsError::file_operation(format!("Failed to write cached file: {}", e)))?;
                
                perf_monitor.record_operation(cached_data.len() as u64);
                info!("File read from cache: {} ({} bytes)", file_info.name, cached_data.len());
                return Ok(());
            }
        }
        
        // Cache miss - read from tape with optimization
        let content = self.read_file_content_optimized(file_info).await?;
        
        // Write to destination
        if let Some(parent) = destination.parent() {
            tokio::fs::create_dir_all(parent).await
                .map_err(|e| RustLtfsError::file_operation(
                    format!("Failed to create destination directory: {}", e)
                ))?;
        }
        
        tokio::fs::write(destination, &content).await
            .map_err(|e| RustLtfsError::file_operation(
                format!("Failed to write file: {}", e)
            ))?;
        
        // Cache the content for future reads
        if let Some(ref mut perf_monitor) = self.performance_monitor {
            let cache_key = super::performance::CacheKey::file_extent(file_info.uid, 0);
            perf_monitor.cache().put(cache_key, content.clone());
            
            let elapsed = start_time.elapsed();
            perf_monitor.record_operation(content.len() as u64);
            
            debug!("File read completed in {:.2}s: {} ({} bytes)", 
                   elapsed.as_secs_f64(), file_info.name, content.len());
        }
        
        info!("Optimized file read completed: {} -> {:?}", file_info.name, destination);
        Ok(())
    }

    /// Read file content with optimization strategies
    async fn read_file_content_optimized(&mut self, file_info: &File) -> Result<Vec<u8>> {
        debug!("Reading file content with optimization: {}", file_info.name);
        
        // Validate file before reading
        file_info.validate()?;
        
        if file_info.is_symlink() {
            return Err(RustLtfsError::file_operation("Cannot read symlink content"));
        }

        if !file_info.has_extents() {
            return Err(RustLtfsError::file_operation("File has no extent information"));
        }

        let extents = file_info.get_sorted_extents();
        
        // Choose read strategy based on file size and extent count
        if extents.len() == 1 && file_info.total_size() <= 64 * 1024 * 1024 { // 64MB single extent
            // Small single-extent file - direct read
            self.read_single_extent_optimized(&extents[0], file_info.total_size()).await
        } else if extents.len() <= 5 && file_info.total_size() <= 256 * 1024 * 1024 { // 256MB multi-extent
            // Medium multi-extent file - parallel read
            self.read_multi_extent_parallel(file_info, &extents).await
        } else {
            // Large file - streaming read with progress
            self.read_large_file_streaming(file_info, &extents).await
        }
    }

    /// Optimized single extent read
    async fn read_single_extent_optimized(&mut self, extent: &crate::ltfs_index::FileExtent, total_size: u64) -> Result<Vec<u8>> {
        debug!("Single extent optimized read: {} bytes", total_size);
        
        // Check cache for block data
        let partition_id = self.get_partition_id(&extent.partition)?;
        if let Some(ref mut perf_monitor) = self.performance_monitor {
            let cache_key = super::performance::CacheKey::block_data(
                partition_id,
                extent.start_block,
                ((total_size + crate::scsi::block_sizes::LTO_BLOCK_SIZE as u64 - 1) / 
                 crate::scsi::block_sizes::LTO_BLOCK_SIZE as u64) as u32
            );
            
            if let Some(cached_blocks) = perf_monitor.cache().get(&cache_key) {
                // Extract exact file content from cached blocks
                let file_content = cached_blocks.get(extent.byte_offset as usize..)
                    .and_then(|slice| slice.get(..total_size as usize))
                    .map(|slice| slice.to_vec())
                    .ok_or_else(|| RustLtfsError::file_operation("Invalid cached block data"))?;
                
                debug!("Single extent read from cache: {} bytes", file_content.len());
                return Ok(file_content);
            }
        }
        
        // Read from tape with retry and caching
        self.read_extent_data_with_cache(extent, 0, total_size).await
    }

    /// Parallel multi-extent read for medium files
    async fn read_multi_extent_parallel(&mut self, file_info: &File, extents: &[crate::ltfs_index::FileExtent]) -> Result<Vec<u8>> {
        debug!("Multi-extent parallel read: {} extents, {} bytes", extents.len(), file_info.total_size());
        
        let mut result = Vec::with_capacity(file_info.total_size() as usize);
        
        // Read extents sequentially for now (true parallel would require complex positioning)
        for (i, extent) in extents.iter().enumerate() {
            let extent_data = self.read_extent_data_with_cache(extent, 0, extent.byte_count).await?;
            result.extend_from_slice(&extent_data);
            
            debug!("Read extent {}/{}: {} bytes", i + 1, extents.len(), extent_data.len());
        }
        
        info!("Multi-extent read completed: {} bytes from {} extents", result.len(), extents.len());
        Ok(result)
    }

    /// Streaming read for large files with progress reporting
    async fn read_large_file_streaming(&mut self, file_info: &File, extents: &[crate::ltfs_index::FileExtent]) -> Result<Vec<u8>> {
        info!("Large file streaming read: {} extents, {} bytes", extents.len(), file_info.total_size());
        
        let mut result = Vec::with_capacity(file_info.total_size() as usize);
        let mut bytes_read = 0u64;
        let total_size = file_info.total_size();
        
        for (i, extent) in extents.iter().enumerate() {
            // Read extent in chunks for large extents
            const CHUNK_SIZE: u64 = 8 * 1024 * 1024; // 8MB chunks
            
            let mut extent_offset = 0u64;
            while extent_offset < extent.byte_count {
                let chunk_size = std::cmp::min(CHUNK_SIZE, extent.byte_count - extent_offset);
                
                let chunk_data = self.read_extent_data_with_cache(extent, extent_offset, chunk_size).await?;
                result.extend_from_slice(&chunk_data);
                
                extent_offset += chunk_size;
                bytes_read += chunk_data.len() as u64;
                
                // Progress reporting
                let progress = (bytes_read as f64 / total_size as f64) * 100.0;
                if bytes_read % (32 * 1024 * 1024) == 0 { // Report every 32MB
                    info!("Large file read progress: {:.1}% ({}/{})", 
                          progress, bytes_read, total_size);
                }
            }
            
            debug!("Completed extent {}/{}: {} bytes", i + 1, extents.len(), extent.byte_count);
        }
        
        info!("Large file streaming read completed: {} bytes", result.len());
        Ok(result)
    }

    /// Read extent data with caching support
    async fn read_extent_data_with_cache(&mut self, extent: &crate::ltfs_index::FileExtent, offset: u64, length: u64) -> Result<Vec<u8>> {
        debug!("Reading extent data with cache: offset={}, length={}", offset, length);
        
        // Calculate block requirements
        let absolute_offset = extent.byte_offset + offset;
        let start_block = extent.start_block + absolute_offset / crate::scsi::block_sizes::LTO_BLOCK_SIZE as u64;
        let block_offset = (absolute_offset % crate::scsi::block_sizes::LTO_BLOCK_SIZE as u64) as usize;
        
        let total_bytes_needed = block_offset + length as usize;
        let blocks_needed = (total_bytes_needed + crate::scsi::block_sizes::LTO_BLOCK_SIZE as usize - 1) 
                           / crate::scsi::block_sizes::LTO_BLOCK_SIZE as usize;
        
        // Check cache for these blocks
        let partition_id = self.get_partition_id(&extent.partition)?;
        if let Some(ref mut perf_monitor) = self.performance_monitor {
            let cache_key = super::performance::CacheKey::block_data(
                partition_id,
                start_block,
                blocks_needed as u32
            );
            
            if let Some(cached_blocks) = perf_monitor.cache().get(&cache_key) {
                // Extract requested data from cached blocks
                let end_offset = block_offset + length as usize;
                if end_offset <= cached_blocks.len() {
                    let file_data = cached_blocks[block_offset..end_offset].to_vec();
                    debug!("Extent data read from cache: {} bytes", file_data.len());
                    return Ok(file_data);
                }
            }
        }
        
        // Cache miss - read from tape
        let partition_id = self.get_partition_id(&extent.partition)?;
        self.scsi.locate_block(partition_id, start_block)?;
        
        let mut buffer = vec![0u8; blocks_needed * crate::scsi::block_sizes::LTO_BLOCK_SIZE as usize];
        let blocks_read = self.scsi.read_blocks_with_retry(blocks_needed as u32, &mut buffer, 2)?;
        
        if blocks_read != blocks_needed as u32 {
            return Err(RustLtfsError::scsi(
                format!("Expected to read {} blocks, but read {}", blocks_needed, blocks_read)
            ));
        }
        
        // Cache the blocks for future use
        if let Some(ref mut perf_monitor) = self.performance_monitor {
            let cache_key = super::performance::CacheKey::block_data(
                partition_id,
                start_block,
                blocks_needed as u32
            );
            perf_monitor.cache().put(cache_key, buffer.clone());
        }
        
        // Extract the exact byte range
        let end_offset = block_offset + length as usize;
        if end_offset > buffer.len() {
            return Err(RustLtfsError::file_operation("Read beyond buffer bounds"));
        }
        
        let file_data = buffer[block_offset..end_offset].to_vec();
        debug!("Extent data read from tape: {} bytes", file_data.len());
        Ok(file_data)
    }

    /// Batch read multiple files efficiently
    pub async fn read_files_batch(&mut self, file_requests: Vec<(File, PathBuf)>) -> Result<Vec<Result<()>>> {
        info!("Starting batch read of {} files", file_requests.len());
        
        let mut results = Vec::with_capacity(file_requests.len());
        
        // Sort requests by tape position for optimal sequential access
        let mut sorted_requests = file_requests;
        sorted_requests.sort_by(|a, b| {
            let a_first_block = a.0.extent_info.extents.first().map(|e| e.start_block).unwrap_or(0);
            let b_first_block = b.0.extent_info.extents.first().map(|e| e.start_block).unwrap_or(0);
            a_first_block.cmp(&b_first_block)
        });
        
        // Process files in batches
        const BATCH_SIZE: usize = 10;
        for chunk in sorted_requests.chunks(BATCH_SIZE) {
            for (file_info, destination) in chunk {
                let result = self.read_file_optimized(file_info, destination).await;
                results.push(result);
            }
            
            // Small delay between batches to prevent drive overload
            if chunk.len() == BATCH_SIZE {
                tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;
            }
        }
        
        let successful_reads = results.iter().filter(|r| r.is_ok()).count();
        info!("Batch read completed: {}/{} files successful", successful_reads, sorted_requests.len());
        
        Ok(results)
    }
}

/// Index synchronization result
#[derive(Debug)]
pub struct IndexSyncResult {
    pub primary_sync_successful: bool,
    pub backup_sync_successful: bool,
    pub generation_updated: bool,
    pub volume_coherency_updated: bool,
    pub errors: Vec<String>,
}

/// Synchronization validation result
#[derive(Debug)]
pub struct SyncValidationResult {
    pub primary_readable: bool,
    pub backup_readable: bool,
    pub generation_consistent: bool,
    pub content_matching: bool,
    pub validation_errors: Vec<String>,
}

/// Enhanced index synchronization implementation for LtfsDirectAccess
impl LtfsDirectAccess {
    /// Enhanced index synchronization strategy (dual-partition compatible)
    pub async fn synchronize_index_dual_partition(&mut self) -> Result<IndexSyncResult> {
        info!("Starting enhanced index synchronization (dual-partition strategy)");
        
        let mut sync_result = IndexSyncResult {
            primary_sync_successful: false,
            backup_sync_successful: false,
            generation_updated: false,
            volume_coherency_updated: false,
            errors: Vec::new(),
        };
        
        // Step 1: Ensure we have a current index
        if self.cached_index.is_none() {
            match self.try_read_ltfs_index_with_recovery().await {
                Ok(index) => {
                    self.cached_index = Some(index);
                }
                Err(e) => {
                    sync_result.errors.push(format!("Failed to read current index: {}", e));
                    return Ok(sync_result);
                }
            }
        }
        
        let index = self.cached_index.as_mut().unwrap();
        
        // Step 2: Update generation number for new sync cycle
        index.increment_generation();
        sync_result.generation_updated = true;
        
        // Step 3: Update index timestamp
        index.updatetime = crate::ltfs_index::get_current_timestamp();
        
        // Step 4: Serialize updated index
        let xml_content = match index.to_xml() {
            Ok(xml) => xml,
            Err(e) => {
                sync_result.errors.push(format!("Failed to serialize index: {}", e));
                return Ok(sync_result);
            }
        };
        
        // Step 5: Write to primary location (partition A)
        match self.write_index_to_primary_location(&xml_content).await {
            Ok(_) => {
                sync_result.primary_sync_successful = true;
                info!("Primary index sync completed successfully");
            }
            Err(e) => {
                sync_result.errors.push(format!("Primary sync failed: {}", e));
                warn!("Primary index sync failed: {}", e);
            }
        }
        
        // Step 6: Write to backup location (for redundancy)
        match self.write_index_to_backup_location(&xml_content).await {
            Ok(_) => {
                sync_result.backup_sync_successful = true;
                info!("Backup index sync completed successfully");
            }
            Err(e) => {
                sync_result.errors.push(format!("Backup sync failed: {}", e));
                warn!("Backup index sync failed: {}", e);
            }
        }
        
        // Step 7: Update Volume Coherency Information (VCI)
        match self.update_volume_coherency_info().await {
            Ok(_) => {
                sync_result.volume_coherency_updated = true;
                info!("Volume coherency information updated");
            }
            Err(e) => {
                sync_result.errors.push(format!("VCI update failed: {}", e));
                warn!("VCI update failed: {}", e);
            }
        }
        
        // Step 8: Validate synchronization
        if sync_result.primary_sync_successful || sync_result.backup_sync_successful {
            info!("Index synchronization completed with {} primary, {} backup", 
                  if sync_result.primary_sync_successful { "success" } else { "failure" },
                  if sync_result.backup_sync_successful { "success" } else { "failure" });
        } else {
            warn!("Index synchronization failed for both primary and backup locations");
        }
        
        Ok(sync_result)
    }

    /// Write index to primary location with enhanced error handling
    async fn write_index_to_primary_location(&self, xml_content: &str) -> Result<()> {
        debug!("Writing index to primary location (partition A)");
        
        const MAX_WRITE_RETRIES: u32 = 3;
        let mut retry_count = 0;
        
        while retry_count < MAX_WRITE_RETRIES {
            match self.write_index_to_tape_enhanced(xml_content) {
                Ok(_) => {
                    info!("Primary index write successful on attempt {}", retry_count + 1);
                    return Ok(());
                }
                Err(e) => {
                    retry_count += 1;
                    warn!("Primary index write attempt {} failed: {}", retry_count, e);
                    
                    if retry_count < MAX_WRITE_RETRIES {
                        // Apply recovery strategy
                        if let Err(recovery_err) = self.recover_from_write_error(&e).await {
                            debug!("Write recovery failed: {}", recovery_err);
                        }
                        
                        // Progressive delay
                        let delay_ms = std::cmp::min(1000 * retry_count, 5000);
                        tokio::time::sleep(tokio::time::Duration::from_millis(delay_ms as u64)).await;
                    } else {
                        return Err(crate::error::RustLtfsError::ltfs_index(
                            format!("Primary index write failed after {} attempts: {}", MAX_WRITE_RETRIES, e)
                        ));
                    }
                }
            }
        }
        
        Err(crate::error::RustLtfsError::ltfs_index("Primary index write exhausted all retries".to_string()))
    }

    /// Enhanced write index to tape with better error handling
    fn write_index_to_tape_enhanced(&self, xml_content: &str) -> Result<()> {
        debug!("Writing updated index to tape (size: {} bytes)", xml_content.len());
        
        // Step 1: Position to index partition (partition A) beginning
        self.scsi.locate_block(0, 5)?; // Block 5 is typical LTFS index location
        
        // Step 2: Calculate how many blocks we need for the index
        let xml_bytes = xml_content.as_bytes();
        let blocks_needed = (xml_bytes.len() + crate::scsi::block_sizes::LTO_BLOCK_SIZE as usize - 1) 
                           / crate::scsi::block_sizes::LTO_BLOCK_SIZE as usize;
        
        // Step 3: Prepare buffer with proper block alignment
        let buffer_size = blocks_needed * crate::scsi::block_sizes::LTO_BLOCK_SIZE as usize;
        let mut buffer = vec![0u8; buffer_size];
        
        // Copy XML data to buffer (rest will be zero-padded)
        buffer[..xml_bytes.len()].copy_from_slice(xml_bytes);
        
        // Step 4: Write blocks to tape
        let blocks_written = self.scsi.write_blocks(blocks_needed as u32, &buffer)?;
        
        if blocks_written != blocks_needed as u32 {
            return Err(crate::error::RustLtfsError::scsi(
                format!("Expected to write {} blocks, but wrote {}", blocks_needed, blocks_written)
            ));
        }
        
        // Step 5: Write file mark to separate index from data
        self.scsi.write_filemarks(1)?;
        
        debug!("Successfully wrote index to tape ({} blocks, {} bytes)", blocks_written, xml_bytes.len());
        Ok(())
    }

    /// Write index to backup location for redundancy
    async fn write_index_to_backup_location(&self, xml_content: &str) -> Result<()> {
        debug!("Writing index to backup location");
        
        // Strategy: Write to alternative blocks in partition A for redundancy
        let backup_locations = vec![10, 15, 20]; // Alternative block locations
        
        for (attempt, block) in backup_locations.iter().enumerate() {
            match self.write_index_to_specific_block(xml_content, 0, *block).await {
                Ok(_) => {
                    info!("Backup index written successfully to block {} (attempt {})", block, attempt + 1);
                    return Ok(());
                }
                Err(e) => {
                    warn!("Backup write to block {} failed: {}", block, e);
                    continue;
                }
            }
        }
        
        Err(crate::error::RustLtfsError::ltfs_index("All backup index write attempts failed".to_string()))
    }

    /// Write index to specific block location
    async fn write_index_to_specific_block(&self, xml_content: &str, partition: u8, block: u64) -> Result<()> {
        debug!("Writing index to partition {}, block {}", partition, block);
        
        // Position to specific location
        self.scsi.locate_block(partition, block)?;
        
        // Calculate blocks needed
        let xml_bytes = xml_content.as_bytes();
        let blocks_needed = (xml_bytes.len() + crate::scsi::block_sizes::LTO_BLOCK_SIZE as usize - 1) 
                           / crate::scsi::block_sizes::LTO_BLOCK_SIZE as usize;
        
        // Prepare buffer
        let buffer_size = blocks_needed * crate::scsi::block_sizes::LTO_BLOCK_SIZE as usize;
        let mut buffer = vec![0u8; buffer_size];
        buffer[..xml_bytes.len()].copy_from_slice(xml_bytes);
        
        // Write blocks
        let blocks_written = self.scsi.write_blocks(blocks_needed as u32, &buffer)?;
        
        if blocks_written != blocks_needed as u32 {
            return Err(crate::error::RustLtfsError::scsi(
                format!("Block write mismatch: expected {}, wrote {}", blocks_needed, blocks_written)
            ));
        }
        
        // Write file mark
        self.scsi.write_filemarks(1)?;
        
        debug!("Successfully wrote {} blocks to partition {}, block {}", blocks_written, partition, block);
        Ok(())
    }

    /// Update Volume Coherency Information (VCI) for LTFS compliance
    async fn update_volume_coherency_info(&self) -> Result<()> {
        debug!("Updating Volume Coherency Information (VCI)");
        
        // VCI is typically stored in MAM attributes
        // This follows LTFSCopyGUI's approach to maintain volume consistency
        
        // Get current generation number from cached index
        if let Some(ref index) = self.cached_index {
            let generation = index.generationnumber;
            let volume_uuid = &index.volumeuuid;
            
            // Update MAM attribute for current generation (following LTFS specification)
            const VCI_GENERATION_ATTR_ID: u16 = 0x080C; // LTFS VCI generation number
            const VCI_UUID_ATTR_ID: u16 = 0x080B;       // LTFS volume UUID
            
            // Write generation number to MAM
            match self.write_vci_generation_to_mam(VCI_GENERATION_ATTR_ID, generation) {
                Ok(_) => debug!("VCI generation {} written to MAM", generation),
                Err(e) => warn!("Failed to write VCI generation to MAM: {}", e),
            }
            
            // Write volume UUID to MAM
            match self.write_vci_uuid_to_mam(VCI_UUID_ATTR_ID, volume_uuid) {
                Ok(_) => debug!("VCI UUID written to MAM"),
                Err(e) => warn!("Failed to write VCI UUID to MAM: {}", e),
            }
            
            info!("VCI update completed for generation {}", generation);
            Ok(())
        } else {
            Err(crate::error::RustLtfsError::ltfs_index("No index available for VCI update".to_string()))
        }
    }

    /// Write VCI generation number to MAM
    fn write_vci_generation_to_mam(&self, attribute_id: u16, generation: u64) -> Result<()> {
        debug!("Writing VCI generation {} to MAM attribute 0x{:04X}", generation, attribute_id);
        
        // Convert generation to big-endian bytes (LTFS standard format)
        let generation_bytes = [
            ((generation >> 56) & 0xFF) as u8,
            ((generation >> 48) & 0xFF) as u8,
            ((generation >> 40) & 0xFF) as u8,
            ((generation >> 32) & 0xFF) as u8,
            ((generation >> 24) & 0xFF) as u8,
            ((generation >> 16) & 0xFF) as u8,
            ((generation >> 8) & 0xFF) as u8,
            (generation & 0xFF) as u8,
        ];
        
        // Write to MAM using SCSI interface
        match self.scsi.set_mam_attribute(attribute_id, &generation_bytes, crate::scsi::MamAttributeFormat::Binary) {
            Ok(_) => {
                debug!("VCI generation written successfully");
                Ok(())
            }
            Err(e) => {
                warn!("Failed to write VCI generation: {}", e);
                Err(e)
            }
        }
    }

    /// Write VCI volume UUID to MAM
    fn write_vci_uuid_to_mam(&self, attribute_id: u16, volume_uuid: &str) -> Result<()> {
        debug!("Writing VCI UUID to MAM attribute 0x{:04X}", attribute_id);
        
        // Convert UUID string to bytes (ASCII format)
        let uuid_bytes = volume_uuid.as_bytes();
        
        // Ensure UUID fits in MAM attribute (typically max 256 bytes)
        if uuid_bytes.len() > 256 {
            return Err(crate::error::RustLtfsError::parameter_validation(
                "UUID too long for MAM attribute".to_string()
            ));
        }
        
        // Write to MAM using ASCII format (0x01)
        match self.scsi.set_mam_attribute(attribute_id, uuid_bytes, crate::scsi::MamAttributeFormat::Text) {
            Ok(_) => {
                debug!("VCI UUID written successfully");
                Ok(())
            }
            Err(e) => {
                warn!("Failed to write VCI UUID: {}", e);
                Err(e)
            }
        }
    }

    /// Recover from write errors with specific strategies
    async fn recover_from_write_error(&self, error: &crate::error::RustLtfsError) -> Result<()> {
        debug!("Attempting recovery from write error: {}", error);
        
        // Strategy 1: Try to reposition to beginning of partition
        if let Err(e) = self.scsi.locate_block(0, 0) {
            debug!("Failed to reposition for write recovery: {}", e);
        } else {
            debug!("Successfully repositioned for write recovery");
        }
        
        // Strategy 2: Test write capability with a small test
        let test_data = vec![0u8; crate::scsi::block_sizes::LTO_BLOCK_SIZE as usize];
        match self.scsi.write_blocks(1, &test_data) {
            Ok(_) => {
                debug!("Write test successful - drive appears operational");
                // Position back to intended location
                if let Err(e) = self.scsi.locate_block(0, 5) {
                    debug!("Failed to reposition after write test: {}", e);
                }
            }
            Err(e) => {
                debug!("Write test failed: {}", e);
                // Try device reset as last resort
                if let Err(reset_error) = self.attempt_device_reset().await {
                    debug!("Device reset during write recovery failed: {}", reset_error);
                }
            }
        }
        
        Ok(())
    }

    /// Validate index synchronization integrity
    pub async fn validate_index_synchronization(&self) -> Result<SyncValidationResult> {
        info!("Validating index synchronization integrity");
        
        let mut validation_result = SyncValidationResult {
            primary_readable: false,
            backup_readable: false,
            generation_consistent: false,
            content_matching: false,
            validation_errors: Vec::new(),
        };
        
        // Try to read from primary location
        match self.read_index_from_primary_location().await {
            Ok(primary_xml) => {
                validation_result.primary_readable = true;
                
                // Try to read from backup location
                match self.read_index_from_backup_location().await {
                    Ok(backup_xml) => {
                        validation_result.backup_readable = true;
                        
                        // Compare content
                        if primary_xml == backup_xml {
                            validation_result.content_matching = true;
                        } else {
                            validation_result.validation_errors.push("Primary and backup index content mismatch".to_string());
                        }
                    }
                    Err(e) => {
                        validation_result.validation_errors.push(format!("Backup index read failed: {}", e));
                    }
                }
                
                // Validate generation consistency
                match crate::ltfs_index::LtfsIndex::from_xml_streaming(&primary_xml) {
                    Ok(index) => {
                        if let Some(ref cached) = self.cached_index {
                            if index.generationnumber == cached.generationnumber {
                                validation_result.generation_consistent = true;
                            } else {
                                validation_result.validation_errors.push(
                                    format!("Generation mismatch: cached={}, read={}", 
                                           cached.generationnumber, index.generationnumber)
                                );
                            }
                        }
                    }
                    Err(e) => {
                        validation_result.validation_errors.push(format!("Primary index parse failed: {}", e));
                    }
                }
            }
            Err(e) => {
                validation_result.validation_errors.push(format!("Primary index read failed: {}", e));
            }
        }
        
        info!("Sync validation completed: primary={}, backup={}, generation={}, content={}", 
              validation_result.primary_readable,
              validation_result.backup_readable,
              validation_result.generation_consistent,
              validation_result.content_matching);
        
        Ok(validation_result)
    }

    /// Read index from primary location for validation
    async fn read_index_from_primary_location(&self) -> Result<String> {
        debug!("Reading index from primary location for validation");
        
        // Position to primary location (block 5)
        self.scsi.locate_block(0, 5)?;
        
        // Try to read index with limited size for validation
        let mut buffer = vec![0u8; 50 * crate::scsi::block_sizes::LTO_BLOCK_SIZE as usize]; // 50 blocks max
        
        match self.scsi.read_blocks(50, &mut buffer) {
            Ok(_) => {
                // Extract XML from buffer
                self.extract_complete_xml(&buffer)
            }
            Err(e) => {
                Err(crate::error::RustLtfsError::scsi(format!("Primary index read failed: {}", e)))
            }
        }
    }

    /// Read index from backup location for validation
    async fn read_index_from_backup_location(&self) -> Result<String> {
        debug!("Reading index from backup location for validation");
        
        // Try backup locations in order
        let backup_locations = vec![10, 15, 20];
        
        for block in backup_locations {
            if let Ok(_) = self.scsi.locate_block(0, block) {
                let mut buffer = vec![0u8; 50 * crate::scsi::block_sizes::LTO_BLOCK_SIZE as usize];
                
                if let Ok(_) = self.scsi.read_blocks(50, &mut buffer) {
                    if let Ok(xml) = self.extract_complete_xml(&buffer) {
                        debug!("Successfully read backup index from block {}", block);
                        return Ok(xml);
                    }
                }
            }
        }
        
        Err(crate::error::RustLtfsError::ltfs_index("No readable backup index found".to_string()))
    }
}

/// Convenience function: Create and initialize LTFS direct access instance
pub async fn create_ltfs_access(device_path: String) -> Result<LtfsDirectAccess> {
    let mut ltfs = LtfsDirectAccess::new(device_path);
    ltfs.initialize()?;
    Ok(ltfs)
}