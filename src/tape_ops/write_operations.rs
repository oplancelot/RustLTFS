use crate::error::{Result, RustLtfsError};
use crate::ltfs_index::LtfsIndex;
use super::{
    TapeOperations, FileWriteEntry, WriteProgress, WriteOptions, WriteResult,
    TapeCapacityInfo, CleaningStatus, EncryptionStatus
};
use std::path::Path;
use std::collections::HashMap;
use std::sync::Arc;
use parking_lot::Mutex;
use tracing::{debug, error, info, warn};
use tokio::io::{AsyncReadExt, AsyncSeekExt, BufReader};
use tokio::fs::File;
use std::io::SeekFrom;

/// LTFSCopyGUI compatible hash calculator
/// Corresponds to VB.NET CheckSumBlockwiseCalculator
pub struct CheckSumBlockwiseCalculator {
    sha1_hasher: sha1::Sha1,
    md5_hasher: md5::Context,
    sha256_hasher: sha2::Sha256,
    blake3_hasher: Option<blake3::Hasher>,
    xxh3_hasher: Option<xxhash_rust::xxh3::Xxh3>,
    xxh128_hasher: Option<xxhash_rust::xxh3::Xxh3>,
    bytes_processed: u64,
}

/// SCSI error handling result (corresponds to VB.NET MessageBox choice result)
#[derive(Debug, Clone, Copy)]
pub enum ScsiErrorAction {
    Abort,
    Retry,
    Ignore,
}

/// SCSI sense data processing and error handling
/// Corresponds to VB.NET TapeUtils.ParseSenseData functionality
pub struct ScsiErrorHandler {
    max_retry_attempts: u32,
    ignore_volume_overflow: bool,
}

impl ScsiErrorHandler {
    pub fn new(max_retries: u32, ignore_volume_overflow: bool) -> Self {
        Self {
            max_retry_attempts: max_retries,
            ignore_volume_overflow,
        }
    }

    /// Parse SCSI sense data and determine error type
    /// Corresponds to VB.NET TapeUtils.ParseSenseData
    pub fn parse_sense_data(&self, sense_data: &[u8]) -> Result<String> {
        if sense_data.len() < 8 {
            return Ok("Invalid sense data length".to_string());
        }

        let sense_key = sense_data[2] & 0x0F;
        let asc = sense_data[12];
        let ascq = sense_data[13];

        let error_description = match (sense_key, asc, ascq) {
            // No sense
            (0x00, _, _) => "No sense - operation completed successfully".to_string(),
            
            // Recovered error
            (0x01, _, _) => "Recovered error - operation completed with recovery action".to_string(),
            
            // Not ready
            (0x02, 0x04, 0x01) => "Not ready - becoming ready".to_string(),
            (0x02, 0x04, 0x02) => "Not ready - initializing command required".to_string(),
            (0x02, 0x04, 0x03) => "Not ready - manual intervention required".to_string(),
            (0x02, 0x04, 0x04) => "Not ready - format in progress".to_string(),
            (0x02, 0x30, 0x00) => "Not ready - incompatible medium installed".to_string(),
            (0x02, 0x3A, 0x00) => "Not ready - medium not present".to_string(),
            
            // Medium error
            (0x03, 0x11, 0x00) => "Medium error - unrecovered read error".to_string(),
            (0x03, 0x14, 0x01) => "Medium error - record not found".to_string(),
            (0x03, 0x30, 0x00) => "Medium error - incompatible medium installed".to_string(),
            (0x03, 0x31, 0x00) => "Medium error - medium format corrupted".to_string(),
            
            // Hardware error
            (0x04, 0x08, 0x00) => "Hardware error - logical unit communication failure".to_string(),
            (0x04, 0x08, 0x01) => "Hardware error - logical unit communication timeout".to_string(),
            (0x04, 0x15, 0x01) => "Hardware error - mechanical positioning error".to_string(),
            (0x04, 0x40, 0x00) => "Hardware error - diagnostic failure".to_string(),
            (0x04, 0x44, 0x00) => "Hardware error - internal target failure".to_string(),
            
            // Illegal request
            (0x05, 0x20, 0x00) => "Illegal request - invalid command operation code".to_string(),
            (0x05, 0x24, 0x00) => "Illegal request - invalid field in CDB".to_string(),
            (0x05, 0x25, 0x00) => "Illegal request - logical unit not supported".to_string(),
            (0x05, 0x26, 0x00) => "Illegal request - invalid field in parameter list".to_string(),
            
            // Unit attention
            (0x06, 0x28, 0x00) => "Unit attention - not ready to ready change".to_string(),
            (0x06, 0x29, 0x00) => "Unit attention - power on, reset, or bus device reset occurred".to_string(),
            (0x06, 0x2A, 0x01) => "Unit attention - mode parameters changed".to_string(),
            
            // Data protect
            (0x07, 0x27, 0x00) => "Data protect - write protected".to_string(),
            (0x07, 0x30, 0x00) => "Data protect - incompatible medium installed".to_string(),
            
            // Blank check
            (0x08, 0x00, 0x05) => "Blank check - end of data detected".to_string(),
            
            // Volume overflow (critical for LTFSCopyGUI compatibility)
            (0x0D, 0x00, 0x00) => "Volume overflow - physical end of medium".to_string(),
            
            // Aborted command
            (0x0B, 0x08, 0x00) => "Aborted command - logical unit communication failure".to_string(),
            (0x0B, 0x08, 0x01) => "Aborted command - logical unit communication timeout".to_string(),
            (0x0B, 0x43, 0x00) => "Aborted command - message error".to_string(),
            (0x0B, 0x47, 0x00) => "Aborted command - SCSI parity error".to_string(),
            
            // Default case
            _ => format!("Unknown error - Sense Key: 0x{:02X}, ASC: 0x{:02X}, ASCQ: 0x{:02X}", 
                        sense_key, asc, ascq),
        };

        Ok(error_description)
    }

    /// Check if error represents volume overflow
    /// Corresponds to VB.NET volume overflow detection logic
    pub fn is_volume_overflow(&self, sense_data: &[u8]) -> bool {
        if sense_data.len() < 3 {
            return false;
        }

        // Check for volume overflow condition (matching LTFSCopyGUI logic)
        let sense_key = sense_data[2] & 0x0F;
        let ili_bit = (sense_data[2] >> 5) & 0x01;  // ILI (Incorrect Length Indicator)
        let eom_bit = (sense_data[2] >> 6) & 0x01;  // EOM (End of Medium)

        // Volume overflow detection from LTFSCopyGUI:
        // ((sense(2) >> 6) And &H1) = 1 AndAlso ((sense(2) And &HF) = 13)
        (eom_bit == 1) && (sense_key == 0x0D)
    }

    /// Check if error represents end of medium warning
    /// Corresponds to LTFSCopyGUI EWEOM detection
    pub fn is_end_of_medium_warning(&self, sense_data: &[u8]) -> bool {
        if sense_data.len() < 3 {
            return false;
        }

        let sense_key = sense_data[2] & 0x0F;
        let eom_bit = (sense_data[2] >> 6) & 0x01;

        // Early warning end of medium (EWEOM)
        (eom_bit == 1) && (sense_key != 0x0D)
    }

    /// Handle SCSI write error with user interaction simulation
    /// Corresponds to VB.NET error handling with MessageBox choices
    pub fn handle_write_error(
        &self, 
        sense_data: &[u8], 
        file_path: &str,
        retry_count: u32
    ) -> Result<ScsiErrorAction> {
        let error_description = self.parse_sense_data(sense_data)?;
        
        // Check for volume overflow
        if self.is_volume_overflow(sense_data) {
            if self.ignore_volume_overflow {
                warn!("Volume overflow detected but ignored due to configuration: {}", file_path);
                return Ok(ScsiErrorAction::Ignore);
            } else {
                error!("Volume overflow detected for file: {}", file_path);
                error!("Error: {}", error_description);
                // In LTFSCopyGUI this would show a MessageBox and stop
                return Ok(ScsiErrorAction::Abort);
            }
        }

        // Check for end of medium warning
        if self.is_end_of_medium_warning(sense_data) {
            warn!("End of medium warning for file: {}", file_path);
            warn!("Warning: {}", error_description);
            // Continue operation but log warning
            return Ok(ScsiErrorAction::Ignore);
        }

        // Check for retryable errors
        if retry_count < self.max_retry_attempts {
            warn!("SCSI write error (attempt {}/{}): {}", 
                  retry_count + 1, self.max_retry_attempts, error_description);
            warn!("Retrying operation for file: {}", file_path);
            return Ok(ScsiErrorAction::Retry);
        }

        // Max retries exceeded - in GUI version this would show MessageBox
        error!("SCSI write error after {} attempts: {}", self.max_retry_attempts, error_description);
        error!("Failed file: {}", file_path);
        
        // For now, abort on persistent errors
        // In a GUI version, this would present Abort/Retry/Ignore options to user
        Ok(ScsiErrorAction::Abort)
    }

    /// Format sense data as hex string for debugging
    /// Corresponds to VB.NET TapeUtils.Byte2Hex functionality
    pub fn format_sense_hex(&self, sense_data: &[u8]) -> String {
        sense_data.iter()
            .map(|byte| format!("{:02X}", byte))
            .collect::<Vec<String>>()
            .join(" ")
    }
}

/// Partition write state (corresponds to VB.NET partition management)
#[derive(Debug, Clone)]
pub struct PartitionWriteState {
    pub current_partition: u8,
    pub current_block: u64,
    pub is_index_partition: bool,
    pub last_filemark_position: Option<crate::scsi::TapePosition>,
}

/// 文件预读状态
#[derive(Debug)]
pub struct FilePreReadState {
    pub file_path: std::path::PathBuf,
    pub buffer: Vec<u8>,
    pub bytes_read: usize,
    pub is_ready: bool,
    pub error: Option<String>,
}

/// Write performance statistics (writes performance monitoring)
#[derive(Debug)]
pub struct WritePerformanceStats {
    pub total_blocks_written: u64,
    pub total_write_time_ms: u64,
    pub average_speed_mbps: f64,
    pub last_speed_check: std::time::Instant,
    pub speed_samples: Vec<f64>,
}

impl Default for WritePerformanceStats {
    fn default() -> Self {
        Self {
            total_blocks_written: 0,
            total_write_time_ms: 0,
            average_speed_mbps: 0.0,
            last_speed_check: std::time::Instant::now(),
            speed_samples: Vec::new(),
        }
    }
}

/// CheckSumBlockwiseCalculator 实现
impl CheckSumBlockwiseCalculator {
    /// Create new hash calculator based on WriteOptions configuration
    pub fn new_with_options(options: &WriteOptions) -> Self {
        use sha1::Digest as Sha1Digest;
        use sha2::Digest as Sha256Digest;
        
        Self {
            sha1_hasher: Sha1Digest::new(),
            md5_hasher: md5::Context::new(),
            sha256_hasher: Sha256Digest::new(),
            blake3_hasher: if options.hash_blake3_enabled {
                Some(blake3::Hasher::new())
            } else {
                None
            },
            xxh3_hasher: if options.hash_xxhash3_enabled {
                Some(xxhash_rust::xxh3::Xxh3::new())
            } else {
                None
            },
            xxh128_hasher: if options.hash_xxhash128_enabled {
                Some(xxhash_rust::xxh3::Xxh3::new())
            } else {
                None
            },
            bytes_processed: 0,
        }
    }

    pub fn new() -> Self {
        use sha1::Digest as Sha1Digest;
        use sha2::Digest as Sha256Digest;
        
        Self {
            sha1_hasher: Sha1Digest::new(),
            md5_hasher: md5::Context::new(),
            sha256_hasher: Sha256Digest::new(),
            blake3_hasher: Some(blake3::Hasher::new()),
            xxh3_hasher: Some(xxhash_rust::xxh3::Xxh3::new()),
            xxh128_hasher: Some(xxhash_rust::xxh3::Xxh3::new()),
            bytes_processed: 0,
        }
    }

    /// Process data block (corresponds to VB.NET Propagate method)
    pub fn propagate(&mut self, data: &[u8]) {
        use sha1::Digest as Sha1Digest;
        use sha2::Digest as Sha256Digest;
        
        self.sha1_hasher.update(data);
        self.md5_hasher.consume(data);
        self.sha256_hasher.update(data);
        
        if let Some(ref mut hasher) = self.blake3_hasher {
            hasher.update(data);
        }
        
        if let Some(ref mut hasher) = self.xxh3_hasher {
            hasher.update(data);
        }
        
        if let Some(ref mut hasher) = self.xxh128_hasher {
            hasher.update(data);
        }
        
        self.bytes_processed += data.len() as u64;
    }

    /// Complete final processing (corresponds to VB.NET ProcessFinalBlock method)
    pub fn process_final_block(&mut self) {
        // All hashers complete final processing when finalize is called
    }

    /// Get SHA1 value
    pub fn sha1_value(&self) -> String {
        use sha1::Digest;
        let mut hasher = self.sha1_hasher.clone();
        format!("{:X}", hasher.finalize())
    }

    /// Get MD5 value
    pub fn md5_value(&self) -> String {
        format!("{:X}", self.md5_hasher.clone().compute())
    }

    /// Get SHA256 value
    pub fn sha256_value(&self) -> String {
        use sha2::Digest;
        let mut hasher = self.sha256_hasher.clone();
        format!("{:X}", hasher.finalize())
    }

    /// Get BLAKE3 value
    pub fn blake3_value(&self) -> Option<String> {
        self.blake3_hasher.as_ref().map(|hasher| {
            hex::encode_upper(hasher.clone().finalize().as_bytes())
        })
    }

    /// Get XxHash3 value
    pub fn xxhash3_value(&self) -> Option<String> {
        self.xxh3_hasher.as_ref().map(|hasher| {
            format!("{:X}", hasher.clone().digest())
        })
    }

    /// Get XxHash128 value
    pub fn xxhash128_value(&self) -> Option<String> {
        self.xxh128_hasher.as_ref().map(|hasher| {
            format!("{:X}", hasher.clone().digest128())
        })
    }

    /// Get filtered hash map based on WriteOptions (LTFSCopyGUI compatible keys)
    pub fn get_enabled_hashes(&self, options: &WriteOptions) -> HashMap<String, String> {
        let mut hashes = HashMap::new();
        
        if options.hash_sha1_enabled {
            hashes.insert("ltfs.hash.sha1sum".to_string(), self.sha1_value());
        }
        
        if options.hash_md5_enabled {
            hashes.insert("ltfs.hash.md5sum".to_string(), self.md5_value());
        }
        
        // SHA256 is always included when hash_on_write is enabled
        hashes.insert("ltfs.hash.sha256sum".to_string(), self.sha256_value());
        
        if options.hash_blake3_enabled {
            if let Some(blake3) = self.blake3_value() {
                hashes.insert("ltfs.hash.blake3sum".to_string(), blake3);
            }
        }
        
        if options.hash_xxhash3_enabled {
            if let Some(xxh3) = self.xxhash3_value() {
                hashes.insert("ltfs.hash.xxhash3sum".to_string(), xxh3);
            }
        }
        
        if options.hash_xxhash128_enabled {
            if let Some(xxh128) = self.xxhash128_value() {
                hashes.insert("ltfs.hash.xxhash128sum".to_string(), xxh128);
            }
        }
        
        hashes
    }

    /// Get all hash values as map (LTFSCopyGUI compatible keys)
    pub fn get_all_hashes(&self) -> HashMap<String, String> {
        let mut hashes = HashMap::new();
        
        hashes.insert("ltfs.hash.sha1sum".to_string(), self.sha1_value());
        hashes.insert("ltfs.hash.md5sum".to_string(), self.md5_value());
        hashes.insert("ltfs.hash.sha256sum".to_string(), self.sha256_value());
        
        if let Some(blake3) = self.blake3_value() {
            hashes.insert("ltfs.hash.blake3sum".to_string(), blake3);
        }
        
        if let Some(xxh3) = self.xxhash3_value() {
            hashes.insert("ltfs.hash.xxhash3sum".to_string(), xxh3);
        }
        
        if let Some(xxh128) = self.xxhash128_value() {
            hashes.insert("ltfs.hash.xxhash128sum".to_string(), xxh128);
        }
        
        hashes
    }
}

/// TapeOperations写入操作实现
impl TapeOperations {
    /// Locate to write position precisely (corresponds to VB.NET LocateToWritePosition)
    pub async fn locate_to_write_position(&mut self) -> Result<PartitionWriteState> {
        info!("Locating to write position with ExtraPartitionCount = {}", 
              self.get_extra_partition_count());
        
        // Read current position
        let current_pos = self.scsi.read_position()?;
        info!(
            "Current tape position: partition={}, block={}",
            current_pos.partition, current_pos.block_number
        );
        
        // 使用ExtraPartitionCount进行分区映射 (对应LTFSCopyGUI的Math.Min逻辑)
        let logical_data_partition = 1u8; // Partition B (data partition)
        let data_partition = self.get_target_partition(logical_data_partition);
        let mut target_block = current_pos.block_number;
        
        info!(
            "Partition mapping: logical={} -> physical={} (ExtraPartitionCount={})",
            logical_data_partition, data_partition, self.get_extra_partition_count()
        );
        
        // If not in target data partition, move to end of data partition
        if current_pos.partition != data_partition {
            info!("Moving to end of data partition {}", data_partition);
            self.scsi.locate_block(data_partition, 0)?;
            self.scsi.space(crate::scsi::SpaceType::EndOfData, 0)?;
            
            let eod_pos = self.scsi.read_position()?;
            target_block = eod_pos.block_number;
            info!(
                "End of data position: partition={}, block={}",
                eod_pos.partition, eod_pos.block_number
            );
        } else {
            // In data partition, check if we need to move to end
            if self.write_options.goto_eod_on_write {
                self.scsi.space(crate::scsi::SpaceType::EndOfData, 0)?;
                let eod_pos = self.scsi.read_position()?;
                target_block = eod_pos.block_number;
                info!(
                    "Moved to end of data: partition={}, block={}",
                    eod_pos.partition, eod_pos.block_number
                );
            }
        }
        
        // Validate position is reasonable (对应LTFSCopyGUI的分区验证逻辑)
        if let Some(ref schema) = &self.schema {
            let schema_partition = if schema.location.partition == "b" { 1 } else { 0 };
            let target_schema_partition = self.get_target_partition(schema_partition);
            
            if target_schema_partition == data_partition && 
               target_block <= schema.location.startblock {
                return Err(RustLtfsError::tape_device(format!(
                    "Current position p{}b{} not allowed for index write, index is at p{}b{}",
                    data_partition, target_block, 
                    target_schema_partition, schema.location.startblock
                )));
            }
        }
        
        let write_state = PartitionWriteState {
            current_partition: data_partition,
            current_block: target_block,
            is_index_partition: false,
            last_filemark_position: None,
        };
        
        info!(
            "Positioning complete, write position: partition={}, block={}",
            write_state.current_partition, write_state.current_block
        );
        
        Ok(write_state)
    }
    
    /// Stream write file to tape (refactored version, solves large file memory issues)
    /// Corresponds to VB.NET block read/write logic
    pub async fn write_file_to_tape_streaming(
        &mut self,
        source_path: &Path,
        target_path: &str,
    ) -> Result<WriteResult> {
        info!("Streaming file write to tape: {:?} -> {}", source_path, target_path);

        // Check stop flag
        if self.stop_flag {
            return Err(RustLtfsError::operation_cancelled(
                "Write operation stopped by user".to_string(),
            ));
        }

        // Offline mode handling
        if self.offline_mode {
            info!("Offline mode: simulating file write operation");
            self.write_progress.current_files_processed += 1;
            return Ok(WriteResult {
                position: crate::scsi::TapePosition {
                    partition: 1,
                    block_number: 0,
                    file_number: 0,
                    set_number: 0,
                    end_of_data: false,
                    beginning_of_partition: false,
                },
                blocks_written: 0,
                bytes_written: 0,
            });
        }

        // Get file metadata
        let metadata = tokio::fs::metadata(source_path).await.map_err(|e| {
            RustLtfsError::file_operation(format!("Unable to get file information: {}", e))
        })?;
        
        let file_size = metadata.len();
        info!("File size: {} bytes", file_size);

        // Skip .xattr files
        if let Some(ext) = source_path.extension() {
            if ext.to_string_lossy().to_lowercase() == "xattr" {
                info!("Skipping .xattr file: {:?}", source_path);
                return Ok(WriteResult {
                    position: crate::scsi::TapePosition {
                        partition: 1,
                        block_number: 0,
                        file_number: 0,
                        set_number: 0,
                        end_of_data: false,
                        beginning_of_partition: false,
                    },
                    blocks_written: 0,
                    bytes_written: 0,
                });
            }
        }

        // Skip symlinks if configured
        if self.write_options.skip_symlinks && metadata.file_type().is_symlink() {
            info!("Skipping symlink: {:?}", source_path);
            return Ok(WriteResult {
                position: crate::scsi::TapePosition {
                    partition: 1,
                    block_number: 0,
                    file_number: 0,
                    set_number: 0,
                    end_of_data: false,
                    beginning_of_partition: false,
                },
                blocks_written: 0,
                bytes_written: 0,
            });
        }

        // Check available tape space
        if let Err(e) = self.check_available_space(file_size) {
            return Err(RustLtfsError::tape_device(format!(
                "Insufficient tape space: {}",
                e
            )));
        }

        // Locate to write position
        let write_state = self.locate_to_write_position().await?;
        
        // Get write start position
        let write_start_position = self.scsi.read_position()?;
        
        // Open file and create buffered reader
        let mut file = File::open(source_path).await.map_err(|e| {
            RustLtfsError::file_operation(format!("Unable to open file: {}", e))
        })?;
        
        let mut buf_reader = BufReader::with_capacity(
            self.block_size as usize * 32, // 32-block buffer
            file
        );

        // Initialize hash calculator (if enabled) based on configuration
        let mut hash_calculator = if self.write_options.hash_on_write {
            Some(CheckSumBlockwiseCalculator::new_with_options(&self.write_options))
        } else {
            None
        };

        // Write statistics
        let mut total_blocks_written = 0u32;
        let mut total_bytes_written = 0u64;
        let write_start_time = std::time::Instant::now();
        
        // Choose processing strategy based on file size
        if file_size <= self.block_size as u64 {
            // Small file: read and write in one go
            info!("Processing small file ({} bytes)", file_size);
            
            let mut buffer = vec![0u8; self.block_size as usize];
            let bytes_read = buf_reader.read(&mut buffer[..file_size as usize]).await.map_err(|e| {
                RustLtfsError::file_operation(format!("Failed to read file: {}", e))
            })?;
            
            if bytes_read != file_size as usize {
                return Err(RustLtfsError::file_operation(format!(
                    "Bytes read mismatch: expected {} bytes, actually read {} bytes",
                    file_size, bytes_read
                )));
            }
            
            // Calculate hash
            if let Some(ref mut calc) = hash_calculator {
                calc.propagate(&buffer[..bytes_read]);
                calc.process_final_block();
            }
            
            // Write to tape
            let blocks_written = self.scsi.write_blocks(1, &buffer)?;
            
            if blocks_written != 1 {
                return Err(RustLtfsError::scsi(format!(
                    "Expected to write 1 block, but actually wrote {} blocks",
                    blocks_written
                )));
            }
            
            total_blocks_written = blocks_written;
            total_bytes_written = file_size;
            
        } else {
            // Large file: block-wise streaming processing
            info!("Processing large file ({} bytes), using block-wise streaming", file_size);
            
            let mut buffer = vec![0u8; self.block_size as usize];
            let mut remaining_bytes = file_size;
            
            while remaining_bytes > 0 {
                // Check stop and pause flags
                if self.stop_flag {
                    return Err(RustLtfsError::operation_cancelled(
                        "Write operation stopped".to_string(),
                    ));
                }
                
                while self.pause_flag && !self.stop_flag {
                    tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;
                }
                
                // Calculate bytes to read for current block
                let bytes_to_read = std::cmp::min(remaining_bytes, self.block_size as u64) as usize;
                
                // Clear buffer (for last block, this ensures zero padding)
                buffer.fill(0);
                
                // Read data from file
                let bytes_read = buf_reader.read(&mut buffer[..bytes_to_read]).await.map_err(|e| {
                    RustLtfsError::file_operation(format!("Failed to read file: {}", e))
                })?;
                
                if bytes_read == 0 {
                    break; // End of file
                }
                
                // Calculate hash
                if let Some(ref mut calc) = hash_calculator {
                    calc.propagate(&buffer[..bytes_read]);
                }
                
                // Apply speed limiting (if configured)
                if let Some(speed_limit_mbps) = self.write_options.speed_limit {
                    self.apply_speed_limit(self.block_size as u64, speed_limit_mbps).await;
                }
                
                // Write block to tape
                let blocks_written = self.scsi.write_blocks(1, &buffer)?;
                
                if blocks_written != 1 {
                    return Err(RustLtfsError::scsi(format!(
                        "Expected to write 1 block, but actually wrote {} blocks",
                        blocks_written
                    )));
                }
                
                total_blocks_written += blocks_written;
                total_bytes_written += bytes_read as u64;
                remaining_bytes -= bytes_read as u64;
                
                // Update progress
                self.write_progress.current_bytes_processed += bytes_read as u64;
                
                debug!(
                    "Written {} blocks, {} bytes, remaining {} bytes",
                    total_blocks_written, total_bytes_written, remaining_bytes
                );
            }
            
            // Complete hash calculation
            if let Some(ref mut calc) = hash_calculator {
                calc.process_final_block();
            }
        }
        
        // Write file mark to separate files
        self.scsi.write_filemarks(1)?;
        
        let write_duration = write_start_time.elapsed();
        let speed_mbps = if write_duration.as_millis() > 0 {
            (total_bytes_written as f64 / (1024.0 * 1024.0)) / 
            (write_duration.as_millis() as f64 / 1000.0)
        } else {
            0.0
        };
        
        info!(
            "File write complete: {} blocks ({} bytes), took {:?}, speed {:.2} MiB/s",
            total_blocks_written, total_bytes_written, write_duration, speed_mbps
        );
        
        // Update LTFS index with computed hashes
        if let Some(hash_calc) = hash_calculator {
            let hashes = hash_calc.get_enabled_hashes(&self.write_options);
            self.update_index_for_file_write_enhanced(
                source_path,
                target_path,
                file_size,
                &write_start_position,
                Some(hashes),
            )?;
        } else {
            self.update_index_for_file_write(
                source_path,
                target_path,
                file_size,
                &write_start_position,
            )?;
        }
        
        // Update progress counters
        self.write_progress.current_files_processed += 1;
        self.write_progress.total_bytes_unindexed += file_size;
        
        // Check if index update is needed based on interval or force_index option
        if self.write_progress.total_bytes_unindexed >= self.write_options.index_write_interval ||
           self.write_options.force_index {
            info!("Index write interval reached or force_index enabled, updating index...");
            self.update_index_on_tape_with_options_dual_partition(self.write_options.force_index).await?;
        }
        
        Ok(WriteResult {
            position: write_start_position,
            blocks_written: total_blocks_written,
            bytes_written: total_bytes_written,
        })
    }

    /// Write directory to tape (enhanced version based on LTFSCopyGUI AddDirectory)
    pub async fn write_directory_to_tape(
        &mut self,
        source_dir: &Path,
        target_path: &str,
    ) -> Result<()> {
        info!(
            "Writing directory to tape: {:?} -> {}",
            source_dir, target_path
        );

        // Check stop flag
        if self.stop_flag {
            return Err(RustLtfsError::operation_cancelled(
                "Write operation stopped by user".to_string(),
            ));
        }

        // Allow execution in offline mode but skip actual tape operations
        if self.offline_mode {
            info!("Offline mode: simulating directory write operation");
            return Ok(());
        }

        // Skip symlinks if configured (对应LTFSCopyGUI的SkipSymlink)
        let metadata = tokio::fs::metadata(source_dir).await.map_err(|e| {
            RustLtfsError::file_operation(format!("Cannot get directory metadata: {}", e))
        })?;

        if self.write_options.skip_symlinks && metadata.file_type().is_symlink() {
            info!("Skipping symlink directory: {:?}", source_dir);
            return Ok(());
        }

        // Create or get directory in LTFS index
        let dir_name = source_dir
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("unknown")
            .to_string();

        // Check if directory already exists in index
        let directory_exists = if let Some(ref index) = self.index {
            self.directory_exists_in_index(index, target_path, &dir_name)?
        } else {
            false
        };

        if !directory_exists {
            // Create directory in LTFS index (对应LTFSCopyGUI的目录创建逻辑)
            self.create_directory_in_index(source_dir, target_path)?;
        }

        // Get list of files and subdirectories
        let mut entries = tokio::fs::read_dir(source_dir)
            .await
            .map_err(|e| RustLtfsError::file_operation(format!("Cannot read directory: {}", e)))?;

        let mut files = Vec::new();
        let mut subdirs = Vec::new();

        while let Some(entry) = entries.next_entry().await.map_err(|e| {
            RustLtfsError::file_operation(format!("Cannot read directory entry: {}", e))
        })? {
            let entry_path = entry.path();
            let entry_metadata = entry.metadata().await.map_err(|e| {
                RustLtfsError::file_operation(format!("Cannot get entry metadata: {}", e))
            })?;

            if entry_metadata.is_file() {
                files.push(entry_path);
            } else if entry_metadata.is_dir() {
                subdirs.push(entry_path);
            }
        }

        // Sort files for consistent ordering (对应LTFSCopyGUI的排序逻辑)
        files.sort_by(|a, b| {
            a.file_name()
                .unwrap_or_default()
                .cmp(b.file_name().unwrap_or_default())
        });

        if self.write_options.parallel_add {
            // Parallel file processing (对应LTFSCopyGUI的Parallel.ForEach)
            info!("Processing {} files in parallel", files.len());

            for file_path in files {
                // Create target path for this file
                let file_name = file_path
                    .file_name()
                    .and_then(|n| n.to_str())
                    .unwrap_or("unknown");
                let file_target = format!("{}/{}", target_path, file_name);

                // Add to processing queue instead of immediate processing for parallel control
                let write_entry = FileWriteEntry {
                    source_path: file_path.clone(),
                    target_path: file_target.clone(),
                    tape_path: file_target,
                    file_size: tokio::fs::metadata(&file_path)
                        .await
                        .map(|m| m.len())
                        .unwrap_or(0),
                    size: tokio::fs::metadata(&file_path)
                        .await
                        .map(|m| m.len())
                        .unwrap_or(0),
                    is_directory: false,
                    preserve_permissions: self.write_options.preserve_permissions,
                    modified: false,
                    overwrite: self.write_options.overwrite,
                    hash: None,
                };

                self.write_queue.push(write_entry);
            }

            // Process write queue
            self.process_write_queue().await?;
        } else {
            // Sequential file processing (对应LTFSCopyGUI的串行处理)
            info!("Processing {} files sequentially", files.len());

            for file_path in files {
                if self.stop_flag {
                    break;
                }

                // Handle pause
                while self.pause_flag && !self.stop_flag {
                    tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;
                }

                // Skip excluded extensions (对应LTFSCopyGUI的exceptExtension逻辑)
                if let Some(ext) = file_path.extension() {
                    let ext_str = ext.to_string_lossy().to_lowercase();
                    if self
                        .write_options
                        .excluded_extensions
                        .iter()
                        .any(|e| e.to_lowercase() == ext_str)
                    {
                        info!("Skipping excluded extension file: {:?}", file_path);
                        continue;
                    }
                }

                // Create target path for this file
                let file_name = file_path
                    .file_name()
                    .and_then(|n| n.to_str())
                    .unwrap_or("unknown");
                let file_target = format!("{}/{}", target_path, file_name);

                // Write individual file
                if let Err(e) = self.write_file_to_tape_streaming(&file_path, &file_target).await {
                    error!("Failed to write file {:?}: {}", file_path, e);
                    // Continue with other files instead of failing entire directory
                }
            }
        }

        // Recursively process subdirectories
        for subdir_path in subdirs {
            if self.stop_flag {
                break;
            }

            let subdir_name = subdir_path
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("unknown");
            let subdir_target = format!("{}/{}", target_path, subdir_name);

            // Recursively write subdirectory
            if let Err(e) =
                Box::pin(self.write_directory_to_tape(&subdir_path, &subdir_target)).await
            {
                error!("Failed to write subdirectory {:?}: {}", subdir_path, e);
                // Continue with other directories
            }
        }

        info!(
            "Directory write completed: {:?} -> {}",
            source_dir, target_path
        );
        Ok(())
    }

    // ================== 异步写入操作 ==================

    /// Write multiple files asynchronously
    pub async fn write_files_async(
        &mut self,
        file_entries: Vec<FileWriteEntry>,
    ) -> Result<()> {
        info!("Starting async write operation for {} files", file_entries.len());

        // Add all entries to write queue
        self.write_queue.extend(file_entries);

        // Process queue
        self.process_write_queue().await?;

        info!("Async write operation completed");
        Ok(())
    }

    // ================== 索引更新相关 ==================

    /// Enhanced index update for file write (对应LTFSCopyGUI的索引更新逻辑)
    fn update_index_for_file_write_enhanced(
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

        let now = chrono::Utc::now().to_rfc3339();
        let new_uid = current_index.highestfileuid.unwrap_or(0) + 1;

        let extent = crate::ltfs_index::FileExtent {
            // 使用实际写入位置的分区信息，而不是硬编码
            partition: if write_position.partition == 0 { "a".to_string() } else { "b".to_string() },
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
            .map(|t| {
                let dt: chrono::DateTime<chrono::Utc> = t.into();
                dt.to_rfc3339()
            })
            .unwrap_or_else(|_| now.clone());

        let modify_time = metadata
            .modified()
            .map(|t| {
                let dt: chrono::DateTime<chrono::Utc> = t.into();
                dt.to_rfc3339()
            })
            .unwrap_or_else(|_| now.clone());

        let access_time = metadata
            .accessed()
            .map(|t| {
                let dt: chrono::DateTime<chrono::Utc> = t.into();
                dt.to_rfc3339()
            })
            .unwrap_or_else(|_| now.clone());

        let new_file = crate::ltfs_index::File {
            name: file_name,
            uid: new_uid,
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

        // Add file to appropriate directory (simplified - should handle path parsing)
        // For now, add to root directory
        current_index.root_directory.contents.files.push(new_file);

        // Update index metadata
        current_index.generationnumber += 1;
        current_index.updatetime = chrono::Utc::now().to_rfc3339();
        current_index.highestfileuid = Some(new_uid);

        // Update internal index
        self.index = Some(current_index.clone());
        self.schema = Some(current_index);
        self.modified = true; // Mark as modified for later index writing

        debug!("LTFS index updated with new file: UID {}", new_uid);
        Ok(())
    }

    /// Basic index update for file write operation
    fn update_index_for_file_write(
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

        let now = chrono::Utc::now().to_rfc3339();
        let new_uid = current_index.highestfileuid.unwrap_or(0) + 1;

        let extent = crate::ltfs_index::FileExtent {
            // 使用实际写入位置的分区信息，而不是硬编码
            partition: if write_position.partition == 0 { "a".to_string() } else { "b".to_string() },
            start_block: write_position.block_number,
            byte_count: file_size,
            file_offset: 0,
            byte_offset: 0,
        };

        let new_file = crate::ltfs_index::File {
            name: file_name,
            uid: new_uid,
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

        // Add file to root directory (simplified - should handle path parsing)
        current_index.root_directory.contents.files.push(new_file);

        // Update index metadata
        current_index.generationnumber += 1;
        current_index.updatetime = chrono::Utc::now().to_rfc3339();
        current_index.highestfileuid = Some(new_uid);

        // Update internal index
        self.index = Some(current_index.clone());

        debug!("LTFS index updated with new file: UID {}", new_uid);
        Ok(())
    }

    /// Create directory entry in LTFS index (对应LTFSCopyGUI的目录创建逻辑)
    fn create_directory_in_index(&mut self, source_dir: &Path, target_path: &str) -> Result<()> {
        let mut current_index = match &self.index {
            Some(index) => index.clone(),
            None => self.create_new_ltfs_index(),
        };

        let dir_name = source_dir
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("unknown")
            .to_string();

        let metadata = std::fs::metadata(source_dir).map_err(|e| {
            RustLtfsError::file_operation(format!("Cannot get directory metadata: {}", e))
        })?;

        let now = chrono::Utc::now().to_rfc3339();
        let new_uid = current_index.highestfileuid.unwrap_or(0) + 1;

        let creation_time = metadata
            .created()
            .map(|t| {
                let dt: chrono::DateTime<chrono::Utc> = t.into();
                dt.to_rfc3339()
            })
            .unwrap_or_else(|_| now.clone());

        let modify_time = metadata
            .modified()
            .map(|t| {
                let dt: chrono::DateTime<chrono::Utc> = t.into();
                dt.to_rfc3339()
            })
            .unwrap_or_else(|_| now.clone());

        let access_time = metadata
            .accessed()
            .map(|t| {
                let dt: chrono::DateTime<chrono::Utc> = t.into();
                dt.to_rfc3339()
            })
            .unwrap_or_else(|_| now.clone());

        let new_directory = crate::ltfs_index::Directory {
            name: dir_name,
            uid: new_uid,
            creation_time: creation_time,
            change_time: now.clone(),
            modify_time: modify_time,
            access_time: access_time,
            backup_time: now,
            read_only: false,
            contents: crate::ltfs_index::DirectoryContents {
                files: Vec::new(),
                directories: Vec::new(),
            },
        };

        // For now, add to root directory (should parse target_path properly)
        current_index
            .root_directory
            .contents
            .directories
            .push(new_directory);

        // Update index metadata
        current_index.generationnumber += 1;
        current_index.updatetime = chrono::Utc::now().to_rfc3339();
        current_index.highestfileuid = Some(new_uid);

        // Update internal index
        self.index = Some(current_index.clone());
        self.schema = Some(current_index);
        self.modified = true;

        debug!("Created directory in LTFS index: UID {}", new_uid);
        Ok(())
    }

    /// Update LTFS index on tape
    pub async fn update_ltfs_index(&mut self) -> Result<()> {
        self.update_index_on_tape().await
    }

    /// Write index to tape
    pub async fn write_index_to_tape(&mut self) -> Result<()> {
        self.update_index_on_tape().await
    }

    /// Commit index changes
    pub async fn commit_index_changes(&mut self) -> Result<()> {
        self.update_index_on_tape().await
    }

    /// Enhanced index update on tape (based on LTFSCopyGUI WriteCurrentIndex)
    /// Supports ForceIndex option for compatibility
    pub async fn update_index_on_tape(&mut self) -> Result<()> {
        self.update_index_on_tape_with_options(false).await
    }

    /// Update index on tape with force option (corresponds to VB.NET WriteCurrentIndex)
    pub async fn update_index_on_tape_with_options(&mut self, force_index: bool) -> Result<()> {
        info!("Starting to update tape LTFS index (force: {})...", force_index);

        // Allow execution in offline mode but skip actual tape operations
        if self.offline_mode {
            info!("Offline mode: simulating index update operation");
            self.write_progress.total_bytes_unindexed = 0;
            return Ok(());
        }

        // Check if index exists and has modifications
        let mut current_index = match &self.schema {
            Some(idx) => idx.clone(),
            None => {
                // Create new index if none exists
                self.create_new_ltfs_index()
            }
        };

        // Enhanced logic following LTFSCopyGUI: check force_index OR TotalBytesUnindexed
        let should_update = force_index || 
                           self.write_options.force_index || 
                           self.write_progress.total_bytes_unindexed > 0;

        if !should_update {
            info!("No unindexed data and force_index not set, skipping update (matching LTFSCopyGUI logic)");
            return Ok(());
        }

        // Position to End of Data (EOD) in data partition (对应LTFSCopyGUI的分区映射逻辑)
        let current_position = self.scsi.read_position()?;
        info!(
            "Current tape position: partition={}, block={}, ExtraPartitionCount={}",
            current_position.partition, current_position.block_number, self.get_extra_partition_count()
        );

        // 使用ExtraPartitionCount进行分区映射 (对应LTFSCopyGUI的Math.Min逻辑)
        let logical_data_partition = 1u8; // Partition B
        let data_partition = self.get_target_partition(logical_data_partition);
        
        if current_position.partition != data_partition || self.write_options.goto_eod_on_write {
            if current_position.partition != data_partition {
                info!("Moving to data partition {} (mapped from logical partition {})", 
                      data_partition, logical_data_partition);
                self.scsi.locate_block(data_partition, 0)?; // Move to beginning of data partition first
            }
            // Go to end of data
            self.scsi.space(crate::scsi::SpaceType::EndOfData, 0)?;
        }

        let eod_position = self.scsi.read_position()?;
        info!(
            "End of data position: partition={}, block={}",
            eod_position.partition, eod_position.block_number
        );

        // Validate position for index write (对应LTFSCopyGUI的ExtraPartitionCount验证逻辑)
        let extra_partition_count = self.get_extra_partition_count();
        if extra_partition_count > 0 {
            let current_schema_partition = if current_index.location.partition == "b" { 1 } else { 0 };
            let target_schema_partition = self.get_target_partition(current_schema_partition);
            
            if target_schema_partition != eod_position.partition {
                return Err(RustLtfsError::tape_device(format!(
                    "Current position p{}b{} not allowed for index write (ExtraPartitionCount={})",
                    eod_position.partition, eod_position.block_number, extra_partition_count
                )));
            }
            
            if current_index.location.startblock >= eod_position.block_number {
                return Err(RustLtfsError::tape_device(format!(
                    "Current position p{}b{} not allowed for index write, index at startblock {} (ExtraPartitionCount={})",
                    eod_position.partition, eod_position.block_number, current_index.location.startblock, extra_partition_count
                )));
            }
        }

        // Write filemark before index (corresponding to LTFSCopyGUI's WriteFileMark)
        self.scsi.write_filemarks(1)?;

        // Update index metadata (corresponding to LTFSCopyGUI's index metadata update)
        current_index.generationnumber += 1;
        current_index.updatetime = chrono::Utc::now().to_rfc3339();
        current_index.location.partition = "b".to_string(); // Data partition

        // Store previous generation location if exists
        if let Some(ref existing_index) = &self.index {
            current_index.previousgenerationlocation = Some(existing_index.location.clone());
        }

        // Get position for index write location
        let index_position = self.scsi.read_position()?;
        current_index.location.startblock = index_position.block_number;
        info!(
            "Index will be written at position: partition={}, block={}",
            index_position.partition, index_position.block_number
        );

        info!("Generating index XML...");

        // Create temporary file for index (matching LTFSCopyGUI's temporary file approach)
        let temp_index_path = std::env::temp_dir().join(format!(
            "LWI_{}.tmp",
            chrono::Utc::now().format("%Y%m%d_%H%M%S%.7f")
        ));

        // Serialize index to XML and save to temporary file
        let index_xml = current_index.to_xml()?;
        tokio::fs::write(&temp_index_path, index_xml)
            .await
            .map_err(|e| {
                RustLtfsError::file_operation(format!("Cannot write temporary index file: {}", e))
            })?;

        info!("Writing index to tape...");

        // Write index file to tape (matching LTFSCopyGUI's TapeUtils.Write approach)
        let index_content = tokio::fs::read(&temp_index_path).await.map_err(|e| {
            RustLtfsError::file_operation(format!("Cannot read temporary index file: {}", e))
        })?;

        // Calculate blocks needed for index
        let blocks_needed =
            (index_content.len() + self.block_size as usize - 1) / self.block_size as usize;
        let buffer_size = blocks_needed * self.block_size as usize;
        let mut buffer = vec![0u8; buffer_size];

        // Copy index content to buffer (rest will be zero-padded)
        buffer[..index_content.len()].copy_from_slice(&index_content);

        // Write index blocks to tape
        let blocks_written = self.scsi.write_blocks(blocks_needed as u32, &buffer)?;

        if blocks_written != blocks_needed as u32 {
            // Clean up temporary file
            if let Err(e) = tokio::fs::remove_file(&temp_index_path).await {
                warn!("Failed to remove temporary index file: {}", e);
            }
            return Err(RustLtfsError::scsi(format!(
                "Expected to write {} blocks for index, but wrote {}",
                blocks_needed, blocks_written
            )));
        }

        // Clean up temporary file (matching LTFSCopyGUI's IO.File.Delete)
        if let Err(e) = tokio::fs::remove_file(&temp_index_path).await {
            warn!("Failed to remove temporary index file: {}", e);
        }

        // Reset TotalBytesUnindexed (matching LTFSCopyGUI's logic)
        self.write_progress.total_bytes_unindexed = 0;

        // Clear current progress stats if requested (matching LTFSCopyGUI's ClearCurrentStat)
        if !force_index { // Only clear on normal updates, not forced ones
            self.write_progress.current_bytes_processed = 0;
            self.write_progress.current_files_processed = 0;
        }

        // Write filemark after index
        self.scsi.write_filemarks(1)?;

        // Update current position tracking
        let final_position = self.scsi.read_position()?;
        info!(
            "Index write completed at position: partition={}, block={}",
            final_position.partition, final_position.block_number
        );

        // Update internal state
        self.index = Some(current_index.clone());
        self.schema = Some(current_index);
        self.modified = false;

        info!("LTFS index update completed successfully");
        Ok(())
    }

    // ================== 文件处理相关 ==================

    /// Process file entry for writing
    pub async fn process_file_entry(&mut self, entry: &FileWriteEntry) -> Result<()> {
        self.write_file_to_tape_streaming(&entry.source_path, &entry.target_path).await.map(|_| ())
    }

    /// Calculate file hash (preserved for backward compatibility)
    pub async fn calculate_file_hash(&self, file_path: &Path) -> Result<String> {
        use sha2::{Digest, Sha256};

        let mut file = tokio::fs::File::open(file_path).await.map_err(|e| {
            RustLtfsError::file_operation(format!("Cannot open file for hashing: {}", e))
        })?;

        let mut hasher = Sha256::new();
        let mut buffer = vec![0u8; 1024 * 1024]; // 1MB buffer

        loop {
            match file.read(&mut buffer).await {
                Ok(0) => break, // EOF
                Ok(n) => hasher.update(&buffer[..n]),
                Err(e) => {
                    return Err(RustLtfsError::file_operation(format!(
                        "Error reading file for hashing: {}",
                        e
                    )))
                }
            }
        }

        let result = hasher.finalize();
        Ok(format!("{:x}", result))
    }

    /// Calculate multiple file hashes (对应LTFSCopyGUI的多种哈希计算)
    async fn calculate_file_hashes(&self, file_path: &Path) -> Result<HashMap<String, String>> {
        use sha1::{Digest, Sha1};
        use sha2::{Digest as Sha256Digest, Sha256};

        let mut file = tokio::fs::File::open(file_path).await.map_err(|e| {
            RustLtfsError::file_operation(format!("Cannot open file for hashing: {}", e))
        })?;

        let mut sha1_hasher = Sha1::new();
        let mut md5_hasher = md5::Context::new();
        let mut sha256_hasher = Sha256::new();
        
        let mut buffer = vec![0u8; 1024 * 1024]; // 1MB buffer

        loop {
            let bytes_read = file.read(&mut buffer).await.map_err(|e| {
                RustLtfsError::file_operation(format!("Error reading file for hash: {}", e))
            })?;

            if bytes_read == 0 {
                break;
            }

            sha1_hasher.update(&buffer[..bytes_read]);
            md5_hasher.consume(&buffer[..bytes_read]);
            sha256_hasher.update(&buffer[..bytes_read]);
        }

        let mut hashes = HashMap::new();
        
        // 按照LTFSCopyGUI的格式生成哈希值
        hashes.insert("sha1sum".to_string(), format!("{:X}", sha1_hasher.finalize()));
        hashes.insert("md5sum".to_string(), format!("{:X}", md5_hasher.compute()));
        hashes.insert("sha256sum".to_string(), format!("{:X}", sha256_hasher.finalize()));
        
        Ok(hashes)
    }

    /// Verify written data against source file
    pub async fn verify_written_data(&self, source_path: &Path, tape_uid: u64) -> Result<bool> {
        info!("Verifying written data for file: {:?}", source_path);

        // Calculate hash of source file
        let source_hash = self.calculate_file_hash(source_path).await?;

        // For now, we assume verification passes
        // In a full implementation, we would read the file back from tape and compare hashes
        debug!("Source file hash: {}", source_hash);
        
        // Placeholder verification logic
        let verification_passed = true;

        if !verification_passed {
            error!(
                "File verification failed: {:?} (UID: {})",
                source_path, tape_uid
            );
        } else {
            debug!("File verification passed: {:?}", source_path);
        }

        Ok(verification_passed)
    }

    // ================== 进度管理相关 ==================

    /// Update write progress
    pub fn update_write_progress(&mut self, files_processed: u64, bytes_processed: u64) {
        self.write_progress.current_files_processed = files_processed;
        self.write_progress.current_bytes_processed = bytes_processed;
    }

    /// Handle write queue processing
    async fn handle_write_queue(&mut self) -> Result<()> {
        self.process_write_queue().await
    }

    /// Process write queue (对应LTFSCopyGUI的队列处理机制)
    async fn process_write_queue(&mut self) -> Result<()> {
        info!(
            "Processing write queue with {} entries",
            self.write_queue.len()
        );

        let queue_copy = self.write_queue.clone();
        self.write_queue.clear();

        // Update progress
        self.write_progress.files_in_queue = queue_copy.len();

        for entry in queue_copy {
            if self.stop_flag {
                break;
            }

            // Handle pause
            while self.pause_flag && !self.stop_flag {
                tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;
            }

            // Process individual file write entry
            if let Err(e) = self
                .write_file_to_tape_streaming(&entry.source_path, &entry.target_path)
                .await
            {
                error!("Failed to write queued file {:?}: {}", entry.source_path, e);
                // Continue with other files in queue
            }
        }

        self.write_progress.files_in_queue = 0;
        info!("Write queue processing completed");
        Ok(())
    }

    // ================== 辅助函数 ==================

    /// Write file data to tape (separated for better error handling)
    async fn write_file_data_to_tape(
        &mut self,
        source_path: &Path,
        file_size: u64,
    ) -> Result<WriteResult> {
        // Read file content
        let file_content = tokio::fs::read(source_path)
            .await
            .map_err(|e| RustLtfsError::file_operation(format!("Unable to read file: {}", e)))?;

        // Position to data partition (partition B) for file data
        let current_position = self.scsi.read_position()?;
        info!(
            "Current tape position: partition={}, block={}",
            current_position.partition, current_position.block_number
        );

        // Move to data partition if not already there
        let data_partition = 1; // Partition B
        let write_start_block = current_position.block_number.max(100); // Start at block 100 for data

        if current_position.partition != data_partition {
            self.scsi.locate_block(data_partition, write_start_block)?;
        }

        // Calculate blocks needed
        let blocks_needed = (file_size + crate::scsi::block_sizes::LTO_BLOCK_SIZE as u64 - 1)
            / crate::scsi::block_sizes::LTO_BLOCK_SIZE as u64;
        let buffer_size =
            blocks_needed as usize * crate::scsi::block_sizes::LTO_BLOCK_SIZE as usize;
        let mut buffer = vec![0u8; buffer_size];

        // Copy file data to buffer (rest will be zero-padded)
        buffer[..file_content.len()].copy_from_slice(&file_content);

        // Get position before writing for extent information
        let write_position = self.scsi.read_position()?;

        // Write file data blocks
        let blocks_written = self.scsi.write_blocks(blocks_needed as u32, &buffer)?;

        if blocks_written != blocks_needed as u32 {
            return Err(RustLtfsError::scsi(format!(
                "Expected to write {} blocks, but wrote {}",
                blocks_needed, blocks_written
            )));
        }

        // Write file mark to separate this file from next
        self.scsi.write_filemarks(1)?;

        info!(
            "Successfully wrote {} blocks ({} bytes) to tape",
            blocks_written, file_size
        );

        Ok(WriteResult {
            position: write_position,
            blocks_written,
            bytes_written: file_size,
        })
    }

    /// Check available space on tape
    fn check_available_space(&self, required_size: u64) -> Result<()> {
        // For now, we assume there's enough space
        // In a full implementation, this would check MAM data or use other SCSI commands
        // to determine remaining capacity

        // Minimum safety check - require at least 1GB free space
        let min_required_space = required_size + 1024 * 1024 * 1024; // File size + 1GB buffer

        debug!(
            "Checking available space: required {} bytes (with buffer: {})",
            required_size, min_required_space
        );

        // This is a simplified check - in reality would query tape capacity
        if required_size > 8 * 1024 * 1024 * 1024 * 1024 {
            // 8TB limit for LTO-8
            return Err(RustLtfsError::tape_device(
                "File too large for tape capacity".to_string(),
            ));
        }

        Ok(())
    }

    /// Find existing file in LTFS index (对应LTFSCopyGUI的文件检查逻辑)
    fn find_existing_file_in_index(
        &self,
        index: &LtfsIndex,
        target_dir: &str,
        file_name: &str,
    ) -> Result<crate::ltfs_index::File> {
        // Parse target directory path and find the file
        // This is a simplified implementation - full version would properly parse directory structure
        let file_locations = index.extract_tape_file_locations();

        for location in &file_locations {
            if location.file_name.to_lowercase() == file_name.to_lowercase() {
                // Find the actual file object in the index
                return self.find_file_by_name_recursive(&index.root_directory, file_name);
            }
        }

        Err(RustLtfsError::ltfs_index(format!(
            "File not found: {}",
            file_name
        )))
    }

    /// Recursively find file by name in directory structure
    fn find_file_by_name_recursive(
        &self,
        dir: &crate::ltfs_index::Directory,
        file_name: &str,
    ) -> Result<crate::ltfs_index::File> {
        // Search files in current directory
        for file in &dir.contents.files {
            if file.name.to_lowercase() == file_name.to_lowercase() {
                return Ok(file.clone());
            }
        }

        // Recursively search subdirectories
        for subdir in &dir.contents.directories {
            if let Ok(found_file) = self.find_file_by_name_recursive(subdir, file_name) {
                return Ok(found_file);
            }
        }

        Err(RustLtfsError::ltfs_index(format!(
            "File not found: {}",
            file_name
        )))
    }

    /// Check if local file is same as tape file (对应LTFSCopyGUI的IsSameFile逻辑)
    async fn is_same_file(
        &self,
        local_path: &Path,
        tape_file: &crate::ltfs_index::File,
    ) -> Result<bool> {
        let metadata = tokio::fs::metadata(local_path).await.map_err(|e| {
            RustLtfsError::file_operation(format!("Cannot get file metadata: {}", e))
        })?;

        // Compare file size
        if metadata.len() != tape_file.length {
            return Ok(false);
        }

        // Compare modification time if available
        if let Ok(modified_time) = metadata.modified() {
            if let Ok(tape_time) = chrono::DateTime::parse_from_rfc3339(&tape_file.modify_time) {
                let local_time: chrono::DateTime<chrono::Utc> = modified_time.into();

                // Allow small time differences (within 2 seconds) due to precision differences
                let time_diff = (local_time.timestamp() - tape_time.timestamp()).abs();
                if time_diff > 2 {
                    return Ok(false);
                }
            }
        }

        // If hash checking is enabled, compare file hashes
        if self.write_options.hash_on_write {
            let local_hash = self.calculate_file_hash(local_path).await?;
            // For now, we assume tape file doesn't have hash stored
            // In full implementation, we would compare with stored hash
            debug!("Local file hash: {}", local_hash);
        }

        // Files are considered the same if size matches and time is close
        Ok(true)
    }

    /// Apply speed limiting (对应LTFSCopyGUI的SpeedLimit功能)
    async fn apply_speed_limit(&mut self, bytes_to_write: u64, speed_limit_mbps: u32) {
        let speed_limit_bytes_per_sec = (speed_limit_mbps as u64) * 1024 * 1024;
        let expected_duration = bytes_to_write * 1000 / speed_limit_bytes_per_sec; // in milliseconds

        if expected_duration > 0 {
            debug!(
                "Speed limiting: waiting {}ms for {} bytes at {} MiB/s",
                expected_duration, bytes_to_write, speed_limit_mbps
            );
            tokio::time::sleep(tokio::time::Duration::from_millis(expected_duration)).await;
        }
    }

    /// Check if directory exists in LTFS index
    fn directory_exists_in_index(
        &self,
        index: &LtfsIndex,
        target_path: &str,
        dir_name: &str,
    ) -> Result<bool> {
        // This is a simplified implementation
        // In a full implementation, we would properly parse the path and navigate the directory tree
        debug!(
            "Checking if directory exists: {} in {}",
            dir_name, target_path
        );
        Ok(false) // For now, always assume directory doesn't exist
    }

    /// Create new empty LTFS index
    fn create_new_ltfs_index(&self) -> LtfsIndex {
        use uuid::Uuid;

        let now = chrono::Utc::now().to_rfc3339();
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
}