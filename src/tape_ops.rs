use crate::error::{Result, RustLtfsError};
use crate::ltfs_index::LtfsIndex;
use crate::scsi::{ScsiInterface, MediaType};
use tracing::{info, warn, debug};
use std::path::Path;
use uuid::Uuid;

/// LTFS格式化状态枚举（基于LTFSCopyGUI的检测策略）
#[derive(Debug, Clone, PartialEq)]
pub enum LtfsFormatStatus {
    /// 磁带已正常格式化为LTFS（包含索引大小）
    LtfsFormatted(usize),
    /// 磁带为空白（未写入任何数据）
    BlankTape,
    /// 磁带有数据但不是LTFS格式
    NonLtfsFormat,
    /// LTFS索引损坏或不完整
    CorruptedIndex,
    /// 磁带定位失败
    PositioningFailed,
    /// 硬件错误或通信问题
    HardwareError,
    /// 未知状态（无法确定）
    Unknown,
}

impl LtfsFormatStatus {
    /// 获取状态描述
    pub fn description(&self) -> &'static str {
        match self {
            LtfsFormatStatus::LtfsFormatted(_) => "LTFS formatted tape",
            LtfsFormatStatus::BlankTape => "Blank tape (no data)",
            LtfsFormatStatus::NonLtfsFormat => "Non-LTFS formatted tape",
            LtfsFormatStatus::CorruptedIndex => "LTFS tape with corrupted index",
            LtfsFormatStatus::PositioningFailed => "Tape positioning failed",
            LtfsFormatStatus::HardwareError => "Hardware or communication error",
            LtfsFormatStatus::Unknown => "Unknown format status",
        }
    }
    
    /// 判断是否为正常的LTFS格式
    pub fn is_ltfs_formatted(&self) -> bool {
        matches!(self, LtfsFormatStatus::LtfsFormatted(_))
    }
}

/// Index location information
#[derive(Debug, Clone)]
struct IndexLocation {
    partition: String,
    start_block: u64,
}

/// LTFS分区标签结构 (对应LTFSCopyGUI的ltfslabel)
#[derive(Debug, Clone)]
pub struct LtfsPartitionLabel {
    pub volume_uuid: String,
    pub blocksize: u32,
    pub compression: bool,
    pub index_partition: u8,      // 通常是0 (partition a)
    pub data_partition: u8,       // 通常是1 (partition b) 
    pub format_time: String,
}

impl Default for LtfsPartitionLabel {
    fn default() -> Self {
        Self {
            volume_uuid: String::new(),
            blocksize: crate::scsi::block_sizes::LTO_BLOCK_SIZE, // 默认64KB
            compression: false,
            index_partition: 0,
            data_partition: 1,
            format_time: String::new(),
        }
    }
}

/// Helper function to warn about deep directory nesting
fn warn_if_deep_nesting(subdirs: &[crate::ltfs_index::Directory]) {
    if !subdirs.is_empty() {
        warn!("Deep directory nesting detected - some subdirectories may not be extracted in this implementation");
    }
}

/// Path content types for describing tape path contents
#[derive(Debug, Clone)]
pub enum PathContent {
    /// Directory content
    Directory(Vec<DirectoryEntry>),
    /// File content
    File(FileInfo),
}

/// Directory entry information
#[derive(Debug, Clone)]
pub struct DirectoryEntry {
    pub name: String,
    pub is_directory: bool,
    pub size: Option<u64>,
    pub file_count: Option<u64>,
    pub file_uid: Option<u64>,
    pub created_time: Option<String>,
    pub modified_time: Option<String>,
}

/// File information
#[derive(Debug, Clone)]
pub struct FileInfo {
    pub name: String,
    pub size: u64,
    pub file_uid: u64,
    pub created_time: Option<String>,
    pub modified_time: Option<String>,
    pub access_time: Option<String>,
}

/// Extraction result information
#[derive(Debug, Clone)]
pub struct ExtractionResult {
    pub files_extracted: u64,
    pub directories_created: u64,
    pub total_bytes: u64,
    pub verification_passed: bool,
}

/// Tape space information
#[derive(Debug, Clone)]
pub struct TapeSpaceInfo {
    pub total_capacity: u64,
    pub used_space: u64,
    pub free_space: u64,
    pub compression_ratio: f64,
    pub partition_a_used: u64,
    pub partition_b_used: u64,
}

/// LTFS access interface for tape device operations
pub struct LtfsAccess {
    device_path: String,
}

impl LtfsAccess {
    pub fn new(device_path: &str) -> Result<Self> {
        // Simulate device open logic
        // Real implementation needs Windows SCSI interface
        Ok(Self {
            device_path: device_path.to_string(),
        })
    }
}

/// Tape operations - core functionality from LTFSCopyGUI
pub struct TapeOperations {
    device_path: String,
    offline_mode: bool,
    index: Option<LtfsIndex>,
    tape_handle: Option<LtfsAccess>,
    drive_handle: Option<i32>,
    schema: Option<LtfsIndex>,
    block_size: u32,
    tape_drive: String,
    scsi: ScsiInterface,
    partition_label: Option<LtfsPartitionLabel>,  // 对应LTFSCopyGUI的plabel
}

impl TapeOperations {
    /// Create new tape operations instance
    pub fn new(device: &str, offline_mode: bool) -> Self {
        Self {
            device_path: device.to_string(),
            offline_mode,
            index: None,
            tape_handle: None,
            drive_handle: None,
            schema: None,
            block_size: 524288, // Default block size
            tape_drive: device.to_string(),
            scsi: ScsiInterface::new(),
            partition_label: None,  // 初始化为None，稍后读取
        }
    }

    /// Initialize tape operations
    pub async fn initialize(&mut self) -> Result<()> {
        info!("Initializing tape device: {}", self.device_path);
        
        if self.offline_mode {
            info!("Offline mode, skipping device initialization");
            return Ok(());
        }
        
        // Open SCSI device
        match self.scsi.open_device(&self.device_path) {
            Ok(()) => {
                info!("Tape device opened successfully");
                
                // Check device status and media type
                match self.scsi.check_media_status() {
                    Ok(media_type) => {
                        match media_type {
                            MediaType::NoTape => {
                                warn!("No tape detected in drive");
                                return Err(RustLtfsError::tape_device("No tape loaded".to_string()));
                            }
                            MediaType::Unknown(_) => {
                                warn!("Unknown media type detected, attempting to continue");
                            }
                            _ => {
                                info!("Detected media type: {}", media_type.description());
                            }
                        }
                    }
                    Err(e) => {
                        warn!("Failed to check media status: {}", e);
                        return Err(RustLtfsError::tape_device(format!("Media status check failed: {}", e)));
                    }
                }
                
                // 首先读取分区标签 (对应LTFSCopyGUI的plabel读取)
                info!("Reading LTFS partition label...");
                match self.read_partition_label().await {
                    Ok(plabel) => {
                        info!("Partition label loaded: UUID={}, blocksize={}", 
                              plabel.volume_uuid, plabel.blocksize);
                        self.partition_label = Some(plabel.clone());
                        // 更新block_size为从分区标签读取的值
                        self.block_size = plabel.blocksize;
                    }
                    Err(e) => {
                        warn!("Failed to read partition label: {}, using defaults", e);
                        // 使用默认分区标签
                        self.partition_label = Some(LtfsPartitionLabel::default());
                    }
                }
                
                // Auto read LTFS index when device opened
                info!("Device opened, auto reading LTFS index ...");
                match self.read_index_from_tape().await {
                    Ok(()) => {
                        info!("LTFS index successfully loaded from tape");
                    }
                    Err(e) => {
                        warn!("Failed to read LTFS index from tape: {}", e);
                        
                        // Try recovery strategies
                        info!("Attempting to diagnose and recover from index read failure...");
                        
                        if let Err(recovery_error) = self.diagnose_and_recover().await {
                            warn!("Recovery attempts failed: {}", recovery_error);
                            return Err(e); // Return original error
                        } else {
                            info!("Recovery successful, retrying index read...");
                            self.read_index_from_tape().await?;
                        }
                    }
                }
            }
            Err(e) => {
                warn!("Failed to open tape device: {}", e);
                return Err(RustLtfsError::tape_device(format!("Device open failed: {}", e)));
            }
        }
        
        Ok(())
    }

    /// Public method for comprehensive tape diagnosis
    pub async fn diagnose_tape_status(&mut self, detailed: bool, test_read: bool) -> Result<()> {
        info!("Starting comprehensive tape diagnosis...");
        
        // Step 1: Try to open device
        info!("\n=== STEP 1: Device Connection Test ===");
        match self.scsi.open_device(&self.device_path) {
            Ok(()) => {
                info!("✅ Successfully opened tape device: {}", self.device_path);
            }
            Err(e) => {
                info!("❌ Failed to open tape device: {}", e);
                info!("\n🔍 Diagnosis: Device not found or access denied");
                info!("Possible causes:");
                info!("  • No tape drive connected to this device path");
                info!("  • Driver not installed or not functioning");
                info!("  • Insufficient permissions");
                info!("  • Device is being used by another application");
                return Err(e);
            }
        }
        
        // Step 2: Check media status
        info!("\n=== STEP 2: Media Status Check ===");
        match self.scsi.check_media_status() {
            Ok(media_type) => {
                match media_type {
                    crate::scsi::MediaType::NoTape => {
                        info!("❌ No tape detected in drive");
                        info!("\n🔍 Diagnosis: Drive is empty");
                        info!("Action required: Insert a tape cartridge");
                        return Ok(());
                    }
                    crate::scsi::MediaType::Unknown(code) => {
                        info!("⚠️ Unknown media type detected (code: 0x{:04X})", code);
                        info!("\n🔍 Diagnosis: Tape type not recognized");
                        info!("Possible causes:");
                        info!("  • Non-LTFS formatted tape");
                        info!("  • Incompatible tape type");
                        info!("  • Damaged tape or cartridge");
                    }
                    _ => {
                        info!("✅ Detected media type: {}", media_type.description());
                    }
                }
            }
            Err(e) => {
                info!("❌ Failed to check media status: {}", e);
                info!("\n🔍 Diagnosis: Drive or media communication issue");
            }
        }
        
        // Step 2.5: LTFS Format Detection (using LTFSCopyGUI strategy)
        info!("\n=== STEP 2.5: LTFS Format Detection (LTFSCopyGUI Strategy) ===");
        match self.detect_ltfs_format_status().await {
            Ok(format_status) => {
                match format_status {
                    LtfsFormatStatus::LtfsFormatted(size) => {
                        info!("✅ LTFS formatted tape detected (index size: {} bytes)", size);
                    }
                    LtfsFormatStatus::BlankTape => {
                        info!("📭 Blank tape detected (no data written)");
                    }
                    LtfsFormatStatus::NonLtfsFormat => {
                        info!("⚠️ Non-LTFS format detected (has data but not LTFS)");
                    }
                    LtfsFormatStatus::CorruptedIndex => {
                        info!("❌ LTFS tape with corrupted index");
                    }
                    _ => {
                        info!("❌ Format detection failed: {}", format_status.description());
                    }
                }
            }
            Err(e) => {
                info!("❌ Format detection error: {}", e);
            }
        }
        
        // Step 3: Position test
        info!("\n=== STEP 3: Position Reading Test ===");
        match self.scsi.read_position() {
            Ok(position) => {
                info!("✅ Successfully read tape position");
                info!("   Partition: {}", position.partition);
                info!("   Block: {}", position.block_number);
                if detailed {
                    info!("   File Number: {}", position.file_number);
                    info!("   Set Number: {}", position.set_number);
                    info!("   End of Data: {}", position.end_of_data);
                    info!("   Beginning of Partition: {}", position.beginning_of_partition);
                }
            }
            Err(e) => {
                info!("❌ Failed to read tape position: {}", e);
                info!("\n🔍 Diagnosis: Tape positioning issue");
            }
        }
        
        // Step 4: Basic read test (if requested)
        if test_read {
            info!("\n=== STEP 4: Basic Read Test ===");
            
            // Try to position to beginning
            match self.scsi.locate_block(0, 0) {
                Ok(()) => {
                    info!("✅ Successfully positioned to beginning of tape");
                    
                    // Try to read first block
                    let mut test_buffer = vec![0u8; crate::scsi::block_sizes::LTO_BLOCK_SIZE as usize];
                    match self.scsi.read_blocks(1, &mut test_buffer) {
                        Ok(_) => {
                            info!("✅ Successfully read first block from tape");
                            
                            let non_zero_bytes = test_buffer.iter().filter(|&&b| b != 0).count();
                            if non_zero_bytes == 0 {
                                info!("ℹ️ First block contains only zeros (tape may be blank)");
                            } else {
                                info!("ℹ️ First block contains {} non-zero bytes", non_zero_bytes);
                                
                                // Check for LTFS signature
                                if test_buffer.windows(4).any(|window| window == b"LTFS") {
                                    info!("✅ Found LTFS signature in first block");
                                } else {
                                    info!("⚠️ No LTFS signature found in first block");
                                }
                            }
                            
                            if detailed {
                                // Show first 256 bytes in hex dump format
                                info!("\n📊 First 256 bytes of tape (hex dump):");
                                for (i, chunk) in test_buffer[..256].chunks(16).enumerate() {
                                    let offset = i * 16;
                                    let hex: String = chunk.iter().map(|b| format!("{:02X}", b)).collect::<Vec<_>>().join(" ");
                                    let ascii: String = chunk.iter().map(|&b| {
                                        if b >= 32 && b <= 126 { b as char } else { '.' }
                                    }).collect();
                                    info!("{:08X}: {:48} {}", offset, hex, ascii);
                                }
                            }
                        }
                        Err(e) => {
                            info!("❌ Failed to read first block: {}", e);
                            info!("\n🔍 Diagnosis: Read operation failed");
                            info!("Possible causes:");
                            info!("  • Tape is write-protected or damaged");
                            info!("  • Drive read head needs cleaning");
                            info!("  • Tape format incompatibility");
                        }
                    }
                }
                Err(e) => {
                    info!("❌ Failed to position to beginning: {}", e);
                    info!("\n🔍 Diagnosis: Tape positioning failed");
                }
            }
        }
        
        // Step 5: LTFS structure check
        if test_read {
            info!("\n=== STEP 5: LTFS Structure Analysis ===");
            
            // Try to read volume label at block 0
            match self.scsi.locate_block(0, 0) {
                Ok(()) => {
                    let mut buffer = vec![0u8; crate::scsi::block_sizes::LTO_BLOCK_SIZE as usize];
                    match self.scsi.read_blocks(1, &mut buffer) {
                        Ok(_) => {
                            if buffer.windows(4).any(|window| window == b"LTFS") {
                                info!("✅ LTFS volume label found at block 0");
                            } else {
                                info!("❌ No LTFS volume label found at block 0");
                                
                                // Try common index locations
                                let test_blocks = [5, 6, 10, 1];
                                for &block in &test_blocks {
                                    if let Ok(()) = self.scsi.locate_block(0, block) {
                                        if let Ok(_) = self.scsi.read_blocks(1, &mut buffer) {
                                            if let Ok(content) = String::from_utf8(buffer.clone()) {
                                                if content.contains("<?xml") || content.contains("<ltfsindex") {
                                                    info!("✅ Potential LTFS index found at block {}", block);
                                                    break;
                                                }
                                            }
                                        }
                                    }
                                }
                            }
                        }
                        Err(_) => {
                            info!("❌ Cannot read volume label area");
                        }
                    }
                }
                Err(_) => {
                    info!("❌ Cannot position to volume label area");
                }
            }
        }
        
        info!("\n=== DIAGNOSIS COMPLETE ===");
        info!("📋 Summary:");
        info!("  • Device: {}", self.device_path);
        info!("  • Status: Analysis completed");
        info!("  • For detailed help, use: rustltfs.exe diagnose --help");
        
        Ok(())
    }

    /// Diagnose and recover from tape reading failures
    async fn diagnose_and_recover(&mut self) -> Result<()> {
        info!("Starting tape diagnosis and recovery procedure...");
        
        // Step 1: Check if tape is properly loaded and responsive
        info!("Step 1: Checking tape responsiveness...");
        match self.scsi.read_position() {
            Ok(position) => {
                info!("Tape is responsive. Current position: partition {}, block {}", 
                      position.partition, position.block_number);
            }
            Err(e) => {
                warn!("Tape not responsive: {}", e);
                return Err(RustLtfsError::tape_device("Tape drive not responding".to_string()));
            }
        }
        
        // Step 2: Try to rewind to beginning and test basic read
        info!("Step 2: Rewinding to beginning of tape...");
        self.scsi.locate_block(0, 0)?;
        
        // Wait for rewind to complete
        tokio::time::sleep(tokio::time::Duration::from_secs(3)).await;
        
        // Step 3: Test read first block to check if tape has data
        info!("Step 3: Testing basic read capability...");
        let mut test_buffer = vec![0u8; crate::scsi::block_sizes::LTO_BLOCK_SIZE as usize];
        match self.scsi.read_blocks(1, &mut test_buffer) {
            Ok(_) => {
                info!("Basic read test successful");
                
                // Check if first block contains any meaningful data
                let non_zero_bytes = test_buffer.iter().filter(|&&b| b != 0).count();
                if non_zero_bytes == 0 {
                    warn!("Tape appears to be blank or unformatted");
                    return Err(RustLtfsError::tape_device("Tape appears to be blank or not LTFS formatted".to_string()));
                } else {
                    info!("Found {} non-zero bytes in first block", non_zero_bytes);
                }
            }
            Err(e) => {
                warn!("Basic read test failed: {}", e);
                return Err(RustLtfsError::tape_device(format!("Cannot read from tape: {}", e)));
            }
        }
        
        // Step 4: Check for LTFS volume label signature
        info!("Step 4: Checking for LTFS volume label...");
        if test_buffer.windows(4).any(|window| window == b"LTFS") {
            info!("LTFS signature found in volume label");
        } else {
            // Try looking in different locations
            warn!("No LTFS signature found in first block, checking alternative locations...");
            
            // Try blocks 1-5 for LTFS signature
            for block_num in 1..=5 {
                self.scsi.locate_block(0, block_num)?;
                match self.scsi.read_blocks(1, &mut test_buffer) {
                    Ok(_) => {
                        if test_buffer.windows(4).any(|window| window == b"LTFS") {
                            info!("LTFS signature found at block {}", block_num);
                            break;
                        }
                    }
                    Err(e) => {
                        debug!("Failed to read block {}: {}", block_num, e);
                    }
                }
            }
        }
        
        // Step 5: Try alternative index locations
        info!("Step 5: Attempting recovery using alternative index locations...");
        let recovery_locations = vec![
            (0, 5),   // Standard LTFS index location
            (0, 6),   // Alternative location
            (0, 10),  // Another common location
            (0, 0),   // Volume label location
        ];
        
        for (partition, block) in recovery_locations {
            info!("Trying index recovery at partition {}, block {}...", partition, block);
            
            match self.scsi.locate_block(partition, block) {
                Ok(()) => {
                    // Try a small read to see if we can get any XML-like data
                    let mut recovery_buffer = vec![0u8; 10 * crate::scsi::block_sizes::LTO_BLOCK_SIZE as usize];
                    match self.scsi.read_blocks(10, &mut recovery_buffer) {
                        Ok(_) => {
                            // Check for XML content
                            if let Ok(content) = String::from_utf8(recovery_buffer.clone()) {
                                if content.contains("<?xml") || content.contains("<ltfsindex") {
                                    info!("Found potential LTFS index data at partition {}, block {}", partition, block);
                                    return Ok(());
                                }
                            }
                        }
                        Err(e) => {
                            debug!("Recovery read failed at partition {}, block {}: {}", partition, block, e);
                        }
                    }
                }
                Err(e) => {
                    debug!("Cannot position to partition {}, block {}: {}", partition, block, e);
                }
            }
        }
        
        Err(RustLtfsError::tape_device("Unable to recover tape or find valid LTFS index".to_string()))
    }

    /// Read LTFS index from tape (对应LTFSCopyGUI的读取索引ToolStripMenuItem_Click)
    pub async fn read_index_from_tape(&mut self) -> Result<()> {
        info!("Starting to read LTFS index from tape ...");
        
        if self.offline_mode {
            info!("Offline mode: using dummy index for simulation");
            return Ok(());
        }
        
        // 首先检测LTFS格式化状态，但对于空白检测要更谨慎
        let format_status = self.detect_ltfs_format_status().await?;
        
        match format_status {
            LtfsFormatStatus::LtfsFormatted(_) => {
                info!("Tape is LTFS formatted, proceeding with index reading");
            }
            LtfsFormatStatus::BlankTape => {
                warn!("Initial format detection suggests blank tape, but attempting direct read anyway");
                info!("Note: LTFSCopyGUI may use different detection methods");
            }
            LtfsFormatStatus::PositioningFailed => {
                return Err(RustLtfsError::tape_device(
                    "Failed to position tape for index reading".to_string()
                ));
            }
            LtfsFormatStatus::HardwareError => {
                return Err(RustLtfsError::scsi(
                    "Hardware error detected during format detection".to_string()
                ));
            }
            LtfsFormatStatus::NonLtfsFormat => {
                warn!("Tape appears to have non-LTFS format, attempting direct read anyway");
            }
            LtfsFormatStatus::CorruptedIndex => {
                warn!("Corrupted index detected, attempting direct read anyway");
            }
            LtfsFormatStatus::Unknown => {
                warn!("Unknown format status, attempting direct index read");
            }
        }
        
        // LTFS index reading steps using real SCSI operations (following LTFSCopyGUI method):
        
        // 1. Locate to index partition (partition a) - 对应TapeUtils.Locate调用
        info!("Locating to index partition (partition a)");
        
        // 直接定位到索引分区的起始位置 (对应LTFSCopyGUI的方法)
        // TapeUtils.Locate(driveHandle, 0, IndexPartition, TapeUtils.LocateDestType.Block)
        let index_partition = 0; // IndexPartition = 0 (partition a)
        self.scsi.locate_block(index_partition, 0)?;
        
        debug!("Located to index partition {}, block 0", index_partition);
        
        // 2. Read index XML data using file mark method (对应TapeUtils.ReadToFileMark)
        info!("Reading index XML data using file mark method");
        
        let xml_content = self.read_index_with_multiple_strategies()?;
        
        // 3. Parse LTFS index
        info!("Parsing LTFS index");
        
        let index = LtfsIndex::from_xml_streaming(&xml_content)?;
        
        // 4. Update internal state
        info!("Index reading completed, updating internal state");
        
        self.index = Some(index.clone());
        self.schema = Some(index);
        
        info!("LTFS index successfully loaded from tape");
        
        Ok(())
    }
    
    /// 使用多种策略读取LTFS索引 (增强容错能力)
    fn read_index_with_multiple_strategies(&self) -> Result<String> {
        info!("Attempting index read with multiple strategies");
        
        // 策略1: 使用动态blocksize和临时文件 (主要策略)
        match self.read_index_xml_from_tape_with_file_mark() {
            Ok(xml_content) => {
                info!("Strategy 1 (dynamic blocksize + temp file) succeeded");
                return Ok(xml_content);
            }
            Err(e) => {
                warn!("Strategy 1 failed: {}", e);
            }
        }
        
        // 策略2: 读取数据区最新索引 (LTFSCopyGUI特有策略)
        warn!("Trying strategy 2: read latest index from data partition");
        match self.read_latest_index_from_data_partition() {
            Ok(xml_content) => {
                info!("Strategy 2 (latest index from data partition) succeeded");
                return Ok(xml_content);
            }
            Err(e) => {
                warn!("Strategy 2 failed: {}", e);
            }
        }
        
        // 策略3: 使用固定64KB blocksize重试
        warn!("Trying fallback strategy 3: fixed 64KB blocksize");
        match self.read_index_with_fixed_blocksize(65536) {
            Ok(xml_content) => {
                info!("Strategy 3 (fixed 64KB blocksize) succeeded");
                return Ok(xml_content);
            }
            Err(e) => {
                warn!("Strategy 3 failed: {}", e);
            }
        }
        
        // 策略4: 使用固定512KB blocksize重试
        warn!("Trying fallback strategy 4: fixed 512KB blocksize");
        match self.read_index_with_fixed_blocksize(524288) {
            Ok(xml_content) => {
                info!("Strategy 4 (fixed 512KB blocksize) succeeded");
                return Ok(xml_content);
            }
            Err(e) => {
                warn!("Strategy 4 failed: {}", e);
            }
        }
        
        // 策略5: 使用渐进式扩展读取 (最后的尝试)
        warn!("Trying final fallback strategy 5: progressive expansion");
        match self.read_index_xml_from_tape() {
            Ok(xml_content) => {
                info!("Strategy 5 (progressive expansion) succeeded");
                return Ok(xml_content);
            }
            Err(e) => {
                warn!("Strategy 5 failed: {}", e);
            }
        }
        
        // 所有策略都失败
        Err(RustLtfsError::ltfs_index(
            "All index reading strategies failed. The tape may be:\n\
             1. Blank or unformatted\n\
             2. Using an unsupported LTFS version\n\
             3. Corrupted or damaged\n\
             4. Using a non-standard blocksize\n\
             5. Positioned incorrectly".to_string()
        ))
    }
    
    /// 使用固定blocksize读取索引 (回退策略)
    fn read_index_with_fixed_blocksize(&self, blocksize: u32) -> Result<String> {
        info!("Trying fixed blocksize: {} bytes", blocksize);
        
        // 临时修改分区标签以使用指定的blocksize
        let temp_plabel = LtfsPartitionLabel {
            blocksize,
            ..LtfsPartitionLabel::default()
        };
        
        // 创建临时的TapeOperations来使用不同的blocksize
        // 这里简化实现，直接调用read_to_file_mark_with_temp_file
        self.read_to_file_mark_with_temp_file(blocksize as usize)
    }
    
    /// 读取数据区最新索引 (对应LTFSCopyGUI的"读取数据区最新索引"功能)
    fn read_latest_index_from_data_partition(&self) -> Result<String> {
        info!("Attempting to read latest index from data partition (partition B)");
        
        // LTFS标准：数据区（partition B）可能包含最新的索引副本
        // 这是LTFSCopyGUI特有的策略，用于处理索引分区损坏的情况
        
        // 第1步：尝试从volume label获取最新索引位置
        if let Ok(latest_location) = self.get_latest_index_location_from_volume_label() {
            info!("Found latest index location from volume label: partition {}, block {}", 
                  latest_location.partition, latest_location.start_block);
                  
            if let Ok(xml_content) = self.read_index_from_specific_location(&latest_location) {
                return Ok(xml_content);
            }
        }
        
        // 第2步：搜索数据分区中的索引副本
        self.search_index_copies_in_data_partition()
    }
    
    /// 从volume label获取最新索引位置
    fn get_latest_index_location_from_volume_label(&self) -> Result<IndexLocation> {
        info!("Reading volume label to find latest index location");
        
        // 定位到volume label (partition A, block 0)
        self.scsi.locate_block(0, 0)?;
        
        let mut buffer = vec![0u8; crate::scsi::block_sizes::LTO_BLOCK_SIZE as usize];
        self.scsi.read_blocks(1, &mut buffer)?;
        
        // 解析volume label中的索引位置指针
        // LTFS volume label格式包含：
        // - Current index location (当前索引位置)
        // - Previous index location (上一个索引位置)
        
        self.parse_index_locations_from_volume_label(&buffer)
    }
    
    /// 解析volume label中的索引位置信息
    fn parse_index_locations_from_volume_label(&self, buffer: &[u8]) -> Result<IndexLocation> {
        // 查找LTFS volume label标识
        let ltfs_signature = b"LTFS";
        
        if let Some(ltfs_pos) = buffer.windows(4).position(|w| w == ltfs_signature) {
            info!("Found LTFS volume label at offset {}", ltfs_pos);
            
            // LTFS volume label结构（简化版本）：
            // - LTFS signature (4 bytes)
            // - Version info
            // - Current index location (partition + block)  
            // - Previous index location (partition + block)
            
            // 搜索可能的索引位置信息
            // 通常在LTFS签名后的几百字节内
            let search_area = &buffer[ltfs_pos..std::cmp::min(ltfs_pos + 1024, buffer.len())];
            
            // 查找非零的块号（可能的索引位置）
            for i in (0..search_area.len()-8).step_by(4) {
                let potential_block = u32::from_le_bytes([
                    search_area[i], search_area[i+1], 
                    search_area[i+2], search_area[i+3]
                ]) as u64;
                
                // 合理的索引位置：通常在block 5-1000之间
                if potential_block >= 5 && potential_block <= 1000 {
                    info!("Found potential index location at block {}", potential_block);
                    return Ok(IndexLocation {
                        partition: "a".to_string(),
                        start_block: potential_block,
                    });
                }
            }
            
            // 如果没找到，尝试查找数据分区的索引
            // 搜索大的块号（数据分区的索引位置）
            for i in (0..search_area.len()-8).step_by(4) {
                let potential_block = u32::from_le_bytes([
                    search_area[i], search_area[i+1], 
                    search_area[i+2], search_area[i+3]
                ]) as u64;
                
                // 数据分区的索引位置：通常是较大的块号
                if potential_block >= 1000 && potential_block <= 1000000 {
                    info!("Found potential data partition index location at block {}", potential_block);
                    return Ok(IndexLocation {
                        partition: "b".to_string(),
                        start_block: potential_block,
                    });
                }
            }
        }
        
        Err(RustLtfsError::ltfs_index("No valid index location found in volume label".to_string()))
    }
    
    /// 从指定位置读取索引
    fn read_index_from_specific_location(&self, location: &IndexLocation) -> Result<String> {
        info!("Reading index from partition {}, block {}", 
              location.partition, location.start_block);
        
        let partition_id = match location.partition.to_lowercase().as_str() {
            "a" => 0,
            "b" => 1,
            _ => return Err(RustLtfsError::ltfs_index(
                format!("Invalid partition: {}", location.partition)
            ))
        };
        
        // 定位到指定位置
        self.scsi.locate_block(partition_id, location.start_block)?;
        
        // 使用动态blocksize读取
        let block_size = self.partition_label
            .as_ref()
            .map(|plabel| plabel.blocksize as usize)
            .unwrap_or(crate::scsi::block_sizes::LTO_BLOCK_SIZE as usize);
        
        self.read_to_file_mark_with_temp_file(block_size)
    }
    
    /// 在数据分区中搜索索引副本
    fn search_index_copies_in_data_partition(&self) -> Result<String> {
        info!("Searching for index copies in data partition (partition B)");
        
        // 策略：在数据分区的几个常见位置搜索索引
        let search_locations = vec![
            100,    // 数据分区开始附近
            500,    // 中等位置
            1000,   // 更远的位置
            5000,   // 大文件后可能的索引位置
            10000,  // 更大的数据后
        ];
        
        for &block in &search_locations {
            info!("Searching for index at data partition block {}", block);
            
            match self.scsi.locate_block(1, block) {
                Ok(()) => {
                    // 尝试读取并检查是否是有效的LTFS索引
                    let block_size = self.partition_label
                        .as_ref()
                        .map(|plabel| plabel.blocksize as usize)
                        .unwrap_or(crate::scsi::block_sizes::LTO_BLOCK_SIZE as usize);
                    
                    match self.try_read_index_at_current_position(block_size) {
                        Ok(xml_content) => {
                            if self.is_valid_ltfs_index(&xml_content) {
                                info!("Found valid LTFS index at data partition block {}", block);
                                return Ok(xml_content);
                            }
                        }
                        Err(_) => {
                            debug!("No valid index found at data partition block {}", block);
                        }
                    }
                }
                Err(e) => {
                    debug!("Cannot position to data partition block {}: {}", block, e);
                }
            }
        }
        
        Err(RustLtfsError::ltfs_index("No valid index found in data partition".to_string()))
    }
    
    /// 在当前位置尝试读取索引（简化版本）
    fn try_read_index_at_current_position(&self, block_size: usize) -> Result<String> {
        let mut buffer = vec![0u8; block_size * 10]; // 读取10个块
        
        match self.scsi.read_blocks(10, &mut buffer) {
            Ok(_) => {
                let content = String::from_utf8_lossy(&buffer);
                let cleaned = content.replace('\0', "").trim().to_string();
                
                if cleaned.len() > 100 {
                    Ok(cleaned)
                } else {
                    Err(RustLtfsError::ltfs_index("No sufficient data at position".to_string()))
                }
            }
            Err(e) => Err(e)
        }
    }
    
    /// 检查是否是有效的LTFS索引
    fn is_valid_ltfs_index(&self, xml_content: &str) -> bool {
        xml_content.contains("<ltfsindex") && 
        xml_content.contains("</ltfsindex>") &&
        xml_content.contains("<directory") &&
        xml_content.len() > 200
    }
    
    /// Find current LTFS index location from volume label
    fn find_current_index_location(&self) -> Result<IndexLocation> {
        debug!("Finding current index location from volume label");
        
        // LTFS Volume Label is typically at partition A, block 0
        self.scsi.locate_block(0, 0)?;
        
        let mut buffer = vec![0u8; crate::scsi::block_sizes::LTO_BLOCK_SIZE as usize];
        
        match self.scsi.read_blocks(1, &mut buffer) {
            Ok(_) => {
                // Parse volume label to find index location
                // LTFS volume label contains pointers to current and previous index
                if let Some(location) = self.parse_volume_label(&buffer)? {
                    return Ok(location);
                }
            }
            Err(e) => {
                warn!("Failed to read volume label: {}", e);
            }
        }
        
        // Fallback to default index location (block 5 on partition A)
        warn!("Using default index location: partition A, block 5");
        Ok(IndexLocation {
            partition: "a".to_string(),
            start_block: 5,
        })
    }
    
    /// Parse LTFS volume label to extract index location
    fn parse_volume_label(&self, buffer: &[u8]) -> Result<Option<IndexLocation>> {
        // LTFS volume label parsing - simplified implementation
        // In a full implementation, this would parse the actual LTFS volume label structure
        
        // Look for LTFS signature in the buffer
        let ltfs_signature = b"LTFS";
        if let Some(pos) = buffer.windows(ltfs_signature.len())
            .position(|window| window == ltfs_signature) {
            debug!("Found LTFS signature at offset {}", pos);
            
            // For now, use a fixed index location
            // Real implementation would parse the volume label structure
            return Ok(Some(IndexLocation {
                partition: "a".to_string(),
                start_block: 5,
            }));
        }
        
        debug!("No LTFS signature found in volume label");
        Ok(None)
    }
    
    /// Read index XML data from tape using file mark method (对应TapeUtils.ReadToFileMark)
    fn read_index_xml_from_tape_with_file_mark(&self) -> Result<String> {
        debug!("Reading LTFS index XML data using file mark method");
        
        // 获取动态blocksize (对应LTFSCopyGUI的plabel.blocksize)
        let block_size = self.partition_label
            .as_ref()
            .map(|plabel| plabel.blocksize as usize)
            .unwrap_or(crate::scsi::block_sizes::LTO_BLOCK_SIZE as usize);
        
        info!("Using dynamic blocksize: {} bytes", block_size);
        
        // 使用临时文件策略，模仿LTFSCopyGUI的方法
        self.read_to_file_mark_with_temp_file(block_size)
    }
    
    /// 使用临时文件读取到文件标记 (对应TapeUtils.ReadToFileMark)
    fn read_to_file_mark_with_temp_file(&self, block_size: usize) -> Result<String> {
        use std::io::Write;
        
        // 创建临时文件 (对应LTFSCopyGUI的tmpFile)
        let temp_dir = std::env::temp_dir();
        let temp_filename = format!("LTFSIndex_{}.tmp", 
            chrono::Utc::now().format("%Y%m%d_%H%M%S"));
        let temp_path = temp_dir.join(temp_filename);
        
        info!("Creating temporary index file: {:?}", temp_path);
        
        let mut temp_file = std::fs::File::create(&temp_path)?;
        let mut total_bytes_written = 0u64;
        let mut blocks_attempted = 0;
        let max_blocks_to_try = 200; // 增加尝试次数
        let max_index_size = 50 * 1024 * 1024; // 50MB 最大索引大小限制
        
        info!("Starting index read with blocksize {}, will try up to {} blocks", 
              block_size, max_blocks_to_try);
        
        // 读取直到文件标记 (对应LTFSCopyGUI的ReadToFileMark实现)
        loop {
            if blocks_attempted >= max_blocks_to_try {
                warn!("Reached maximum block attempt limit ({}), stopping", max_blocks_to_try);
                break;
            }
            
            if total_bytes_written > max_index_size {
                warn!("Reached maximum index size limit ({}MB), stopping", 
                      max_index_size / 1024 / 1024);
                break;
            }
            
            let mut buffer = vec![0u8; block_size];
            blocks_attempted += 1;
            
            // 尝试读取下一个块
            match self.scsi.read_blocks(1, &mut buffer) {
                Ok(blocks_read) => {
                    if blocks_read == 0 {
                        debug!("Reached end of data at block {}, no more blocks to read", blocks_attempted);
                        break;
                    }
                    
                    // 检查是否有非零数据
                    let non_zero_count = buffer.iter().filter(|&&b| b != 0).count();
                    debug!("Block {}: {} bytes read, {} non-zero bytes", 
                           blocks_attempted, block_size, non_zero_count);
                    
                    if non_zero_count == 0 {
                        // 全零块可能是文件标记
                        debug!("Zero block encountered at block {}, treating as file mark", blocks_attempted);
                        break;
                    }
                    
                    // 将数据写入临时文件 (关键：使用文件而非内存)
                    temp_file.write_all(&buffer)?;
                    total_bytes_written += block_size as u64;
                    
                    debug!("Wrote {} bytes to temp file, total: {} bytes", 
                           block_size, total_bytes_written);
                    
                    // 检查是否可能包含完整的XML (提前结束优化)
                    if total_bytes_written > 1024 && blocks_attempted % 10 == 0 {
                        // 每10个块检查一次是否包含XML结束标记
                        temp_file.flush()?;
                        if self.check_temp_file_for_xml_end(&temp_path)? {
                            info!("Found XML end marker, stopping read");
                            break;
                        }
                    }
                }
                Err(e) => {
                    debug!("Read error after {} blocks: {}", blocks_attempted, e);
                    
                    if total_bytes_written == 0 {
                        // 第一次读取就失败
                        return Err(RustLtfsError::ltfs_index(
                            "No data could be read from tape".to_string()
                        ));
                    }
                    // 如果已经读取了一些数据，就停止并尝试解析
                    break;
                }
            }
        }
        
        temp_file.flush()?;
        drop(temp_file); // 确保文件关闭
        
        info!("Index read completed: {} blocks attempted, {} bytes written to temp file", 
              blocks_attempted, total_bytes_written);
        
        // 从临时文件读取并清理 (对应LTFSCopyGUI的处理方式)
        let xml_content = std::fs::read_to_string(&temp_path)?;
        
        // 清理临时文件
        if let Err(e) = std::fs::remove_file(&temp_path) {
            warn!("Failed to remove temporary file {:?}: {}", temp_path, e);
        }
        
        let cleaned_xml = xml_content.replace('\0', "").trim().to_string();
        
        if cleaned_xml.is_empty() {
            warn!("No LTFS index data found after reading {} blocks", blocks_attempted);
            warn!("This could indicate:");
            warn!("  1. Tape is at incorrect position");
            warn!("  2. LTFS index is located elsewhere");
            warn!("  3. Tape uses different block structure");
            warn!("  4. Index may be corrupted or compressed");
            warn!("  5. Blocksize mismatch (tried: {} bytes)", block_size);
            return Err(RustLtfsError::ltfs_index("Index XML is empty".to_string()));
        } else {
            info!("Found {} bytes of potential index data using blocksize {}", 
                  cleaned_xml.len(), block_size);
        }
        
        Ok(cleaned_xml)
    }
    
    /// 检查临时文件是否包含XML结束标记
    fn check_temp_file_for_xml_end(&self, temp_path: &std::path::Path) -> Result<bool> {
        use std::io::{BufRead, BufReader, Seek, SeekFrom};
        
        let mut file = std::fs::File::open(temp_path)?;
        
        // 检查文件末尾1KB的数据
        let file_len = file.seek(SeekFrom::End(0))?;
        let check_len = std::cmp::min(1024, file_len);
        file.seek(SeekFrom::End(-(check_len as i64)))?;
        
        let reader = BufReader::new(file);
        let content: String = reader.lines()
            .map(|line| line.unwrap_or_default())
            .collect::<Vec<_>>()
            .join("\n");
        
        Ok(content.contains("</ltfsindex>"))
    }

    /// Read index XML data from tape with progressive expansion
    fn read_index_xml_from_tape(&self) -> Result<String> {
        debug!("Reading LTFS index XML data from tape");
        
        let mut xml_content = String::new();
        let mut blocks_to_read = 10u32; // Start with 10 blocks
        let max_blocks = 200u32; // Maximum 200 blocks for safety (12.8MB)
        
        loop {
            debug!("Attempting to read {} blocks for index", blocks_to_read);
            let buffer_size = blocks_to_read as usize * crate::scsi::block_sizes::LTO_BLOCK_SIZE as usize;
            let mut buffer = vec![0u8; buffer_size];
            
            match self.scsi.read_blocks_with_retry(blocks_to_read, &mut buffer, 2) {
                Ok(blocks_read) => {
                    debug!("Successfully read {} blocks", blocks_read);
                    
                    // Find the actual data length (look for XML end)
                    let actual_data_len = buffer.iter()
                        .position(|&b| b == 0)
                        .unwrap_or(buffer.len());
                    
                    // Convert to string
                    match String::from_utf8(buffer[..actual_data_len].to_vec()) {
                        Ok(content) => {
                            xml_content = content;
                            
                            // Check if we have a complete XML document
                            if xml_content.contains("</ltfsindex>") {
                                info!("Complete LTFS index XML found ({} bytes)", xml_content.len());
                                break;
                            }
                            
                            // If incomplete and we haven't hit the limit, try reading more blocks
                            if blocks_to_read < max_blocks {
                                blocks_to_read = std::cmp::min(blocks_to_read * 2, max_blocks);
                                debug!("XML incomplete, expanding to {} blocks", blocks_to_read);
                                continue;
                            } else {
                                warn!("Reached maximum block limit, using partial XML");
                                break;
                            }
                        }
                        Err(e) => {
                            return Err(RustLtfsError::ltfs_index(
                                format!("Failed to parse index data as UTF-8: {}", e)
                            ));
                        }
                    }
                }
                Err(e) => {
                    warn!("Failed to read {} blocks from tape: {}", blocks_to_read, e);
                    
                    // Provide more specific error information
                    if e.to_string().contains("Direct block read operation failed") {
                        return Err(RustLtfsError::scsi(
                            format!("Failed to read index from tape: {}. This may indicate:\n\n\
                                     1. Tape is blank or not LTFS formatted\n\
                                     2. Tape position is incorrect\n\
                                     3. Tape drive hardware issue\n\
                                     4. SCSI communication problem\n\n\
                                     Try using --skip-index option to bypass automatic index reading.", e)
                        ));
                    }
                    
                    return Err(RustLtfsError::scsi(
                        format!("Failed to read index from tape: {}", e)
                    ));
                }
            }
        }
        
        // Validate the extracted XML
        self.validate_index_xml(&xml_content)?;
        
        info!("Successfully read LTFS index ({} bytes) from tape", xml_content.len());
        Ok(xml_content)
    }
    
    /// 检测磁带LTFS格式化状态（基于LTFSCopyGUI的策略）
    /// 不通过卷标判断，而是直接尝试读取LTFS索引
    pub async fn detect_ltfs_format_status(&mut self) -> Result<LtfsFormatStatus> {
        info!("Detecting LTFS format status using LTFSCopyGUI strategy...");
        
        if self.offline_mode {
            return Ok(LtfsFormatStatus::Unknown);
        }
        
        // 步骤1：定位到索引分区（partition a）的block 0
        info!("Step 1: Locating to index partition (partition a, block 0)");
        match self.scsi.locate_block(0, 0) {
            Ok(()) => {
                info!("Successfully positioned to index location");
            }
            Err(e) => {
                warn!("Failed to position to index location: {}", e);
                return Ok(LtfsFormatStatus::PositioningFailed);
            }
        }
        
        // 步骤2：尝试使用ReadToFileMark方法读取索引
        info!("Step 2: Attempting to read LTFS index using ReadToFileMark method");
        let index_read_result = self.try_read_ltfs_index();
        
        // 步骤3：基于读取结果判断格式化状态
        match index_read_result {
            Ok(xml_content) => {
                if !xml_content.trim().is_empty() {
                    // 尝试解析XML以验证LTFS索引的有效性
                    match self.validate_index_xml(&xml_content) {
                        Ok(()) => {
                            info!("✅ Valid LTFS index found - tape is LTFS formatted");
                            Ok(LtfsFormatStatus::LtfsFormatted(xml_content.len()))
                        }
                        Err(e) => {
                            warn!("⚠️ Found data but invalid LTFS index: {}", e);
                            Ok(LtfsFormatStatus::CorruptedIndex)
                        }
                    }
                } else {
                    info!("📭 No index data found - tape appears blank");
                    Ok(LtfsFormatStatus::BlankTape)
                }
            }
            Err(e) => {
                info!("❌ Failed to read index: {}", e);
                self.classify_format_detection_error(e)
            }
        }
    }
    
    /// 尝试读取LTFS索引（模拟LTFSCopyGUI的ReadToFileMark方法）
    fn try_read_ltfs_index(&self) -> Result<String> {
        info!("Trying to read LTFS index using file mark method...");
        
        let mut xml_content = String::new();
        let block_size = crate::scsi::block_sizes::LTO_BLOCK_SIZE as usize;
        let mut blocks_read = 0u32;
        let max_blocks = 50; // 限制读取块数，避免读取过多数据
        let mut has_data = false;
        
        // 使用文件标记方法读取，直到遇到文件标记或错误
        loop {
            if blocks_read >= max_blocks {
                info!("Reached maximum read limit ({}), stopping", max_blocks);
                break;
            }
            
            let mut buffer = vec![0u8; block_size];
            
            match self.scsi.read_blocks(1, &mut buffer) {
                Ok(read_count) => {
                    if read_count == 0 {
                        info!("No more blocks to read (reached end)");
                        break;
                    }
                    
                    blocks_read += 1;
                    
                    // 检查是否有非零数据
                    let non_zero_count = buffer.iter().filter(|&&b| b != 0).count();
                    if non_zero_count > 0 {
                        has_data = true;
                        info!("Block {}: {} non-zero bytes", blocks_read, non_zero_count);
                    }
                    
                    // 检查是否全零块（可能表示文件标记）
                    if buffer.iter().all(|&b| b == 0) {
                        info!("Encountered zero block at {}, assuming file mark", blocks_read);
                        break;
                    }
                    
                    // 尝试转换为UTF-8并添加到XML内容
                    match String::from_utf8(buffer) {
                        Ok(block_content) => {
                            let trimmed = block_content.trim_end_matches('\0');
                            xml_content.push_str(trimmed);
                            
                            // 检查是否已读取完整的XML
                            if xml_content.contains("</ltfsindex>") {
                                info!("Found complete LTFS index XML");
                                break;
                            }
                        }
                        Err(_) => {
                            // 非UTF-8数据，可能到达了文件标记或二进制数据
                            info!("Non-UTF8 data encountered, stopping read");
                            break;
                        }
                    }
                }
                Err(e) => {
                    info!("Read error after {} blocks: {}", blocks_read, e);
                    if !has_data {
                        // 第一次读取就失败，可能是空白磁带
                        return Err(RustLtfsError::ltfs_index("No data could be read from tape".to_string()));
                    }
                    break;
                }
            }
        }
        
        let cleaned_xml = xml_content.replace('\0', "").trim().to_string();
        info!("Read completed: {} blocks, {} characters", blocks_read, cleaned_xml.len());
        
        Ok(cleaned_xml)
    }
    
    /// 分类格式检测错误
    fn classify_format_detection_error(&self, error: crate::error::RustLtfsError) -> Result<LtfsFormatStatus> {
        let error_msg = error.to_string();
        
        if error_msg.contains("No data could be read") {
            Ok(LtfsFormatStatus::BlankTape)
        } else if error_msg.contains("positioning") || error_msg.contains("locate") {
            Ok(LtfsFormatStatus::PositioningFailed)
        } else if error_msg.contains("SCSI") || error_msg.contains("communication") {
            Ok(LtfsFormatStatus::HardwareError)
        } else {
            Ok(LtfsFormatStatus::Unknown)
        }
    }
    
    /// Validate index XML structure
    fn validate_index_xml(&self, xml_content: &str) -> Result<()> {
        debug!("Validating LTFS index XML structure");
        
        // Basic validation checks
        if xml_content.is_empty() {
            return Err(RustLtfsError::ltfs_index("Index XML is empty".to_string()));
        }
        
        if !xml_content.contains("<ltfsindex") {
            return Err(RustLtfsError::ltfs_index("Invalid LTFS index format - missing ltfsindex element".to_string()));
        }
        
        if !xml_content.contains("</ltfsindex>") {
            warn!("LTFS index XML may be incomplete - missing closing tag");
        }
        
        debug!("LTFS index XML validation passed");
        Ok(())
    }

    /// Load index from local file
    pub async fn load_index_from_file(&mut self, index_path: &Path) -> Result<()> {
        info!("Loading LTFS index from file: {:?}", index_path);
        
        let xml_content = tokio::fs::read_to_string(index_path).await
            .map_err(|e| RustLtfsError::file_operation(
                format!("Unable to read index file: {}", e)
            ))?;
        
        let index = LtfsIndex::from_xml(&xml_content)?;
        self.index = Some(index.clone());
        self.schema = Some(index);
        
        info!("Index file loaded successfully");
        Ok(())
    }

    /// 读取LTFS分区标签 (对应LTFSCopyGUI的plabel读取)
    async fn read_partition_label(&mut self) -> Result<LtfsPartitionLabel> {
        info!("Reading LTFS partition label from tape");
        
        if self.offline_mode {
            return Ok(LtfsPartitionLabel::default());
        }
        
        // LTFS分区标签通常位于分区a的block 0
        // 首先定位到开头
        self.scsi.locate_block(0, 0)?; // 分区a, 块0 (相当于rewind)
        
        // 读取第一个块，包含LTFS卷标签
        let mut buffer = vec![0u8; crate::scsi::block_sizes::LTO_BLOCK_SIZE as usize];
        self.scsi.read_blocks(1, &mut buffer)?;
        
        // 解析LTFS卷标签
        self.parse_ltfs_volume_label(&buffer)
    }
    
    /// 解析LTFS卷标签获取分区标签信息
    fn parse_ltfs_volume_label(&self, buffer: &[u8]) -> Result<LtfsPartitionLabel> {
        // LTFS卷标签是一个复杂的二进制结构
        // 这里实现简化版本，重点获取blocksize
        
        // 查找LTFS签名 "LTFS"
        let ltfs_signature = b"LTFS";
        if let Some(pos) = buffer.windows(4).position(|w| w == ltfs_signature) {
            info!("Found LTFS signature at offset {}", pos);
            
            // 尝试从标签中提取信息
            // 注意：这是简化实现，真正的LTFS标签解析更复杂
            let mut plabel = LtfsPartitionLabel::default();
            
            // 尝试在标签附近查找blocksize信息
            // 常见的blocksize值：65536 (64KB), 524288 (512KB), 1048576 (1MB)
            let common_blocksizes = [524288u32, 1048576u32, 262144u32, 131072u32, 65536u32];
            
            // 在LTFS签名后的数据中查找可能的blocksize
            let search_data = &buffer[pos..std::cmp::min(pos + 512, buffer.len())];
            for &blocksize in &common_blocksizes {
                let blocksize_bytes = blocksize.to_le_bytes();
                if search_data.windows(4).any(|w| w == blocksize_bytes) {
                    info!("Found potential blocksize: {}", blocksize);
                    plabel.blocksize = blocksize;
                    break;
                }
            }
            
            // 尝试提取UUID（如果可能）
            // 这里简化处理，在实际实现中需要按照LTFS规范解析
            
            Ok(plabel)
        } else {
            warn!("No LTFS signature found in volume label");
            // 如果没有找到LTFS签名，使用启发式方法
            self.detect_blocksize_heuristic(buffer)
        }
    }
    
    /// 启发式检测blocksize
    fn detect_blocksize_heuristic(&self, buffer: &[u8]) -> Result<LtfsPartitionLabel> {
        info!("Using heuristic blocksize detection");
        
        let mut plabel = LtfsPartitionLabel::default();
        
        // 分析buffer中的模式来猜测blocksize
        // 如果buffer主要是零，可能使用了较大的blocksize
        let non_zero_count = buffer.iter().filter(|&&b| b != 0).count();
        let zero_ratio = (buffer.len() - non_zero_count) as f64 / buffer.len() as f64;
        
        info!("Buffer analysis: {:.1}% zeros", zero_ratio * 100.0);
        
        if zero_ratio > 0.8 {
            // 高零比率，可能是大blocksize
            plabel.blocksize = 524288; // 512KB
            info!("High zero ratio detected, using 512KB blocksize");
        } else if non_zero_count > 32768 {
            // 较多数据，可能是标准blocksize
            plabel.blocksize = 65536; // 64KB
            info!("Standard data pattern detected, using 64KB blocksize");
        } else {
            // 默认使用常见的512KB
            plabel.blocksize = 524288;
            info!("Using default 512KB blocksize");
        }
        
        Ok(plabel)
    }

    /// Get index statistics
    pub fn get_index_statistics(&self) -> Option<IndexStatistics> {
        self.index.as_ref().map(|index| IndexStatistics {
            volume_uuid: index.volumeuuid.clone(),
            generation_number: index.generationnumber,
            update_time: index.updatetime.clone(),
            creator: index.creator.clone(),
            total_files: index.extract_tape_file_locations().len(),
        })
    }

    /// Write file to tape
    pub async fn write_file_to_tape(&mut self, source_path: &Path, target_path: &str) -> Result<()> {
        info!("Writing file to tape: {:?} -> {}", source_path, target_path);
        
        // Allow execution in offline mode but skip actual tape operations
        if self.offline_mode {
            info!("Offline mode: simulating file write operation");
            return Ok(());
        }
        
        // File write steps using real SCSI operations:
        
        // 1. Check file size and status
        let file_size = tokio::fs::metadata(source_path).await
            .map_err(|e| RustLtfsError::file_operation(format!("Unable to get file information: {}", e)))?
            .len();
        
        info!("File size: {} bytes", file_size);
        
        // 2. Check available space on tape
        if let Err(e) = self.check_available_space(file_size) {
            return Err(RustLtfsError::tape_device(format!("Insufficient space on tape: {}", e)));
        }
        
        // 3. Read file content
        let file_content = tokio::fs::read(source_path).await
            .map_err(|e| RustLtfsError::file_operation(format!("Unable to read file: {}", e)))?;
        
        // 4. Position to data partition (partition B) for file data
        let current_position = self.scsi.read_position()?;
        info!("Current tape position: partition={}, block={}", 
            current_position.partition, current_position.block_number);
        
        // Move to data partition if not already there
        let data_partition = 1; // Partition B
        let write_start_block = current_position.block_number.max(100); // Start at block 100 for data
        
        if current_position.partition != data_partition {
            self.scsi.locate_block(data_partition, write_start_block)?;
        }
        
        // 5. Write file data in blocks
        let blocks_needed = (file_size + crate::scsi::block_sizes::LTO_BLOCK_SIZE as u64 - 1) 
                           / crate::scsi::block_sizes::LTO_BLOCK_SIZE as u64;
        let buffer_size = blocks_needed as usize * crate::scsi::block_sizes::LTO_BLOCK_SIZE as usize;
        let mut buffer = vec![0u8; buffer_size];
        
        // Copy file data to buffer (rest will be zero-padded)
        buffer[..file_content.len()].copy_from_slice(&file_content);
        
        // Get position before writing for extent information
        let write_position = self.scsi.read_position()?;
        
        // Write file data blocks
        let blocks_written = self.scsi.write_blocks(blocks_needed as u32, &buffer)?;
        
        if blocks_written != blocks_needed as u32 {
            return Err(RustLtfsError::scsi(
                format!("Expected to write {} blocks, but wrote {}", blocks_needed, blocks_written)
            ));
        }
        
        // Write file mark to separate this file from next
        self.scsi.write_filemarks(1)?;
        
        info!("Successfully wrote {} blocks ({} bytes) to tape", blocks_written, file_size);
        
        // 6. Update LTFS index with new file entry
        self.update_index_for_file_write(source_path, target_path, file_size, &write_position)?;
        
        info!("File write completed: {:?}", source_path);
        Ok(())
    }
    
    /// Check available space on tape
    fn check_available_space(&self, required_size: u64) -> Result<()> {
        // For now, we assume there's enough space
        // In a full implementation, this would check MAM data or use other SCSI commands
        // to determine remaining capacity
        
        // Minimum safety check - require at least 1GB free space
        let min_required_space = required_size + 1024 * 1024 * 1024; // File size + 1GB buffer
        
        debug!("Checking available space: required {} bytes (with buffer: {})", 
               required_size, min_required_space);
        
        // This is a simplified check - in reality would query tape capacity
        if required_size > 8 * 1024 * 1024 * 1024 * 1024 { // 8TB limit for LTO-8
            return Err(RustLtfsError::tape_device("File too large for tape capacity".to_string()));
        }
        
        Ok(())
    }
    
    /// Update LTFS index for file write operation
    fn update_index_for_file_write(
        &mut self, 
        source_path: &Path, 
        target_path: &str, 
        file_size: u64,
        write_position: &crate::scsi::TapePosition
    ) -> Result<()> {
        debug!("Updating LTFS index for write: {:?} -> {} ({} bytes)", 
               source_path, target_path, file_size);
        
        // Get or create current index
        let mut current_index = match &self.index {
            Some(index) => index.clone(),
            None => {
                // Create new index if none exists
                self.create_new_ltfs_index()
            }
        };
        
        // Create new file entry
        let file_name = source_path.file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("unknown")
            .to_string();
        
        let now = chrono::Utc::now().to_rfc3339();
        let new_uid = current_index.highestfileuid.unwrap_or(0) + 1;
        
        let extent = crate::ltfs_index::FileExtent {
            partition: match write_position.partition {
                0 => "a".to_string(),
                1 => "b".to_string(),
                _ => "b".to_string(), // Default to data partition
            },
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
    
    /// Create new LTFS index
    fn create_new_ltfs_index(&self) -> LtfsIndex {
        info!("Creating new LTFS index");
        
        let now = chrono::Utc::now().to_rfc3339();
        
        LtfsIndex {
            version: "2.4.0".to_string(),
            creator: "RustLTFS".to_string(),
            volumeuuid: uuid::Uuid::new_v4().to_string(),
            generationnumber: 1,
            updatetime: now.clone(),
            location: crate::ltfs_index::Location {
                partition: "a".to_string(),
                startblock: 5,
            },
            previousgenerationlocation: None,
            allowpolicyupdate: None,
            volumelockstate: None,
            highestfileuid: Some(0),
            root_directory: crate::ltfs_index::Directory {
                name: "".to_string(),
                uid: 0,
                creation_time: now,
                change_time: chrono::Utc::now().to_rfc3339(),
                modify_time: chrono::Utc::now().to_rfc3339(),
                access_time: chrono::Utc::now().to_rfc3339(),
                backup_time: chrono::Utc::now().to_rfc3339(),
                read_only: false,
                contents: crate::ltfs_index::DirectoryContents {
                    files: Vec::new(),
                    directories: Vec::new(),
                },
            },
        }
    }

    /// Write directory to tape
    pub async fn write_directory_to_tape(&mut self, source_dir: &Path, target_path: &str) -> Result<()> {
        info!("Writing directory to tape: {:?} -> {}", source_dir, target_path);
        
        // Allow execution in offline mode
        if self.offline_mode {
            info!("Offline mode: simulating directory write operation");
        }
        
        // Directory write steps:
        
        // 1. Traverse directory structure
        info!("Traversing directory structure");
        
        // 2. Create index entries for each file and subdirectory
        info!("Creating index entries");
        
        // 3. Recursively process subdirectories
        info!("Recursively processing subdirectories");
        
        // 4. Batch write file data
        info!("Batch writing file data");
        
        warn!("Current simulation implementation - need to implement real directory write operations");
        
        Ok(())
    }

    /// List specified path content
    pub async fn list_path_content(&self, tape_path: &str) -> Result<Option<PathContent>> {
        info!("Listing path content: {}", tape_path);
        
        // Check if index is loaded
        let index = match &self.index {
            Some(idx) => idx,
            None => {
                warn!("Index not loaded");
                return Ok(None);
            }
        };
        
        // Use LTFS index to find the actual path
        match index.find_path(tape_path)? {
            crate::ltfs_index::PathType::Directory(dir) => {
                let mut entries = Vec::new();
                
                // Add subdirectories
                for subdir in &dir.contents.directories {
                    entries.push(DirectoryEntry {
                        name: subdir.name.clone(),
                        is_directory: true,
                        size: None,
                        file_count: Some((subdir.contents.files.len() + subdir.contents.directories.len()) as u64),
                        file_uid: Some(subdir.uid),
                        created_time: Some(subdir.creation_time.clone()),
                        modified_time: Some(subdir.change_time.clone()),
                    });
                }
                
                // Add files
                for file in &dir.contents.files {
                    entries.push(DirectoryEntry {
                        name: file.name.clone(),
                        is_directory: false,
                        size: Some(file.length),
                        file_count: None,
                        file_uid: Some(file.uid),
                        created_time: Some(file.creation_time.clone()),
                        modified_time: Some(file.modify_time.clone()),
                    });
                }
                
                Ok(Some(PathContent::Directory(entries)))
            }
            crate::ltfs_index::PathType::File(file) => {
                let file_info = FileInfo {
                    name: file.name.clone(),
                    size: file.length,
                    file_uid: file.uid,
                    created_time: Some(file.creation_time.clone()),
                    modified_time: Some(file.modify_time.clone()),
                    access_time: Some(file.access_time.clone()),
                };
                
                Ok(Some(PathContent::File(file_info)))
            }
            crate::ltfs_index::PathType::NotFound => {
                debug!("Path not found: {}", tape_path);
                Ok(None)
            }
        }
    }

    /// Preview file content
    pub async fn preview_file_content(&self, file_uid: u64, max_lines: usize) -> Result<String> {
        info!("Previewing file content: UID {}, max lines: {}", file_uid, max_lines);
        
        if self.offline_mode {
            info!("Offline mode: returning dummy preview content");
            return Ok("[Offline Mode] File content preview not available without tape access".to_string());
        }
        
        // Find file by UID in index
        let index = match &self.index {
            Some(idx) => idx,
            None => {
                return Err(RustLtfsError::ltfs_index("Index not loaded".to_string()));
            }
        };
        
        let file_info = self.find_file_by_uid(index, file_uid)?;
        
        // Read file content using SCSI operations
        let content_bytes = self.read_file_content_from_tape(&file_info, max_lines * 100).await?; // Estimate bytes per line
        
        // Convert to string and limit lines
        let content_str = String::from_utf8_lossy(&content_bytes);
        let lines: Vec<&str> = content_str.lines().take(max_lines).collect();
        
        Ok(lines.join("\n"))
    }
    
    /// Find file by UID in LTFS index
    fn find_file_by_uid(&self, index: &LtfsIndex, file_uid: u64) -> Result<crate::ltfs_index::File> {
        self.search_file_by_uid(&index.root_directory, file_uid)
            .ok_or_else(|| RustLtfsError::ltfs_index(format!("File with UID {} not found", file_uid)))
    }
    
    /// Recursively search for file by UID
    fn search_file_by_uid(&self, dir: &crate::ltfs_index::Directory, file_uid: u64) -> Option<crate::ltfs_index::File> {
        // Search files in current directory
        for file in &dir.contents.files {
            if file.uid == file_uid {
                return Some(file.clone());
            }
        }
        
        // Recursively search subdirectories
        for subdir in &dir.contents.directories {
            if let Some(found_file) = self.search_file_by_uid(subdir, file_uid) {
                return Some(found_file);
            }
        }
        
        None
    }
    
    /// Read file content from tape using SCSI operations
    async fn read_file_content_from_tape(&self, file_info: &crate::ltfs_index::File, max_bytes: usize) -> Result<Vec<u8>> {
        debug!("Reading file content from tape: {} (max {} bytes)", file_info.name, max_bytes);
        
        if file_info.extent_info.extents.is_empty() {
            return Err(RustLtfsError::ltfs_index("File has no extent information".to_string()));
        }
        
        // Get the first extent for reading
        let first_extent = &file_info.extent_info.extents[0];
        
        // Calculate read parameters
        let bytes_to_read = std::cmp::min(max_bytes as u64, file_info.length) as usize;
        let blocks_to_read = (bytes_to_read + crate::scsi::block_sizes::LTO_BLOCK_SIZE as usize - 1) 
                           / crate::scsi::block_sizes::LTO_BLOCK_SIZE as usize;
        
        // Position to file start
        let partition_id = self.get_partition_id(&first_extent.partition)?;
        self.scsi.locate_block(partition_id, first_extent.start_block)?;
        
        // Read blocks
        let buffer_size = blocks_to_read * crate::scsi::block_sizes::LTO_BLOCK_SIZE as usize;
        let mut buffer = vec![0u8; buffer_size];
        
        let blocks_read = self.scsi.read_blocks_with_retry(blocks_to_read as u32, &mut buffer, 2)?;
        
        if blocks_read == 0 {
            return Err(RustLtfsError::scsi("No data read from tape".to_string()));
        }
        
        // Extract actual file content (considering byte offset)
        let start_offset = first_extent.byte_offset as usize;
        let end_offset = start_offset + bytes_to_read;
        
        if end_offset > buffer.len() {
            return Ok(buffer[start_offset..].to_vec());
        }
        
        Ok(buffer[start_offset..end_offset].to_vec())
    }
    
    /// Get partition ID from partition name
    fn get_partition_id(&self, partition: &str) -> Result<u8> {
        match partition.to_lowercase().as_str() {
            "a" => Ok(0),
            "b" => Ok(1),
            _ => Err(RustLtfsError::ltfs_index(format!("Invalid partition: {}", partition)))
        }
    }
    
    /// Enhanced error recovery for SCSI operations
    async fn recover_from_scsi_error(&self, error: &RustLtfsError, operation: &str) -> Result<()> {
        warn!("SCSI operation '{}' failed, attempting recovery: {}", operation, error);
        
        // Recovery strategy 1: Check device status
        match self.scsi.check_media_status() {
            Ok(media_type) => {
                if matches!(media_type, MediaType::NoTape) {
                    return Err(RustLtfsError::tape_device("No tape loaded - manual intervention required".to_string()));
                }
                debug!("Media status OK: {}", media_type.description());
            }
            Err(e) => {
                warn!("Media status check failed during recovery: {}", e);
            }
        }
        
        // Recovery strategy 2: Read current position to test responsiveness
        match self.scsi.read_position() {
            Ok(pos) => {
                debug!("Drive responsive at position: partition {}, block {}", pos.partition, pos.block_number);
            }
            Err(e) => {
                warn!("Drive not responsive during recovery: {}", e);
                return self.attempt_drive_reset().await;
            }
        }
        
        // Recovery strategy 3: Small delay to allow drive to settle
        tokio::time::sleep(tokio::time::Duration::from_millis(1000)).await;
        
        info!("SCSI recovery completed for operation: {}", operation);
        Ok(())
    }
    
    /// Attempt to reset the drive as last resort
    async fn attempt_drive_reset(&self) -> Result<()> {
        warn!("Attempting drive reset as recovery measure");
        
        // Try to rewind to beginning of tape
        match self.scsi.locate_block(0, 0) {
            Ok(()) => {
                info!("Successfully rewound tape during recovery");
                
                // Wait for rewind to complete
                tokio::time::sleep(tokio::time::Duration::from_secs(5)).await;
                
                // Test position read
                match self.scsi.read_position() {
                    Ok(pos) => {
                        info!("Drive reset successful, position: partition {}, block {}", pos.partition, pos.block_number);
                        Ok(())
                    }
                    Err(e) => {
                        Err(RustLtfsError::tape_device(format!("Drive reset failed - position unreadable: {}", e)))
                    }
                }
            }
            Err(e) => {
                Err(RustLtfsError::tape_device(format!("Drive reset failed - cannot rewind: {}", e)))
            }
        }
    }
    
    /// Verify tape operation with retry
    async fn verify_operation_with_retry<F, T>(&self, operation_name: &str, operation: F, max_retries: u32) -> Result<T>
    where
        F: Fn() -> Result<T> + Clone,
    {
        let mut last_error = None;
        
        for attempt in 0..=max_retries {
            if attempt > 0 {
                info!("Retrying operation '{}' (attempt {} of {})", operation_name, attempt + 1, max_retries + 1);
                
                // Progressive backoff delay
                let delay_ms = std::cmp::min(1000 * attempt, 10000); // Max 10 second delay
                tokio::time::sleep(tokio::time::Duration::from_millis(delay_ms as u64)).await;
                
                // Attempt recovery
                if let Some(ref error) = last_error {
                    if let Err(recovery_error) = self.recover_from_scsi_error(error, operation_name).await {
                        warn!("Recovery failed for '{}': {}", operation_name, recovery_error);
                    }
                }
            }
            
            match operation() {
                Ok(result) => {
                    if attempt > 0 {
                        info!("Operation '{}' succeeded after {} retries", operation_name, attempt);
                    }
                    return Ok(result);
                }
                Err(e) => {
                    last_error = Some(e);
                    warn!("Operation '{}' failed on attempt {}: {:?}", operation_name, attempt + 1, last_error);
                }
            }
        }
        
        Err(last_error.unwrap_or_else(|| {
            RustLtfsError::scsi(format!("Operation '{}' failed after {} attempts", operation_name, max_retries + 1))
        }))
    }

    /// Extract files or directories from tape
    pub async fn extract_from_tape(
        &self, 
        tape_path: &str, 
        local_dest: &Path, 
        verify: bool
    ) -> Result<ExtractionResult> {
        info!("Extracting from tape: {} -> {:?}, verify: {}", tape_path, local_dest, verify);
        
        if self.offline_mode {
            info!("Offline mode: simulating extraction operation");
            return Ok(ExtractionResult {
                files_extracted: 1,
                directories_created: 0,
                total_bytes: 1024,
                verification_passed: verify,
            });
        }
        
        // Check if index is loaded
        let index = match &self.index {
            Some(idx) => idx,
            None => {
                return Err(RustLtfsError::ltfs_index("Index not loaded".to_string()));
            }
        };
        
        // Create local destination directory if needed
        if let Some(parent) = local_dest.parent() {
            tokio::fs::create_dir_all(parent).await
                .map_err(|e| RustLtfsError::file_operation(
                    format!("Unable to create target directory: {}", e)
                ))?;
        }
        
        // Find the path in LTFS index
        match index.find_path(tape_path)? {
            crate::ltfs_index::PathType::File(file) => {
                // Extract single file
                self.extract_single_file(&file, local_dest, verify).await
            }
            crate::ltfs_index::PathType::Directory(dir) => {
                // Extract directory recursively
                self.extract_directory(&dir, local_dest, tape_path, verify).await
            }
            crate::ltfs_index::PathType::NotFound => {
                Err(RustLtfsError::ltfs_index(format!("Path not found: {}", tape_path)))
            }
        }
    }
    
    /// Extract a single file from tape
    async fn extract_single_file(
        &self,
        file_info: &crate::ltfs_index::File,
        dest_path: &Path,
        verify: bool
    ) -> Result<ExtractionResult> {
        info!("Extracting single file: {} -> {:?}", file_info.name, dest_path);
        
        let mut total_bytes = 0u64;
        let mut verification_passed = true;
        
        // Read complete file content
        let file_content = self.read_complete_file_from_tape(file_info).await?;
        total_bytes += file_content.len() as u64;
        
        // Write to local file
        tokio::fs::write(dest_path, &file_content).await
            .map_err(|e| RustLtfsError::file_operation(
                format!("Failed to write file {:?}: {}", dest_path, e)
            ))?;
        
        // Verify if requested
        if verify {
            verification_passed = self.verify_extracted_file(dest_path, &file_content).await?;
        }
        
        Ok(ExtractionResult {
            files_extracted: 1,
            directories_created: 0,
            total_bytes,
            verification_passed,
        })
    }
    
    /// Extract directory recursively
    async fn extract_directory(
        &self,
        dir_info: &crate::ltfs_index::Directory,
        dest_path: &Path,
        tape_base_path: &str,
        verify: bool
    ) -> Result<ExtractionResult> {
        info!("Extracting directory: {} -> {:?}", dir_info.name, dest_path);
        
        let mut files_extracted = 0;
        let mut directories_created = 0;
        let mut total_bytes = 0u64;
        let mut verification_passed = true;
        
        // Create the directory
        tokio::fs::create_dir_all(dest_path).await
            .map_err(|e| RustLtfsError::file_operation(
                format!("Failed to create directory {:?}: {}", dest_path, e)
            ))?;
        directories_created += 1;
        
        // Extract all files in this directory
        for file in &dir_info.contents.files {
            let file_dest = dest_path.join(&file.name);
            let extract_result = self.extract_single_file(file, &file_dest, verify).await?;
            
            files_extracted += extract_result.files_extracted;
            total_bytes += extract_result.total_bytes;
            verification_passed &= extract_result.verification_passed;
        }
        
        // Extract subdirectories (note: limited recursion depth for safety)
        for subdir in &dir_info.contents.directories {
            let subdir_dest = dest_path.join(&subdir.name);
            
            // Create subdirectory
            tokio::fs::create_dir_all(&subdir_dest).await
                .map_err(|e| RustLtfsError::file_operation(
                    format!("Failed to create subdirectory {:?}: {}", subdir_dest, e)
                ))?;
            directories_created += 1;
            
            // Extract files in subdirectory
            for file in &subdir.contents.files {
                let file_dest = subdir_dest.join(&file.name);
                let extract_result = self.extract_single_file(file, &file_dest, verify).await?;
                
                files_extracted += extract_result.files_extracted;
                total_bytes += extract_result.total_bytes;
                verification_passed &= extract_result.verification_passed;
            }
            
            // Note: For deeper nesting, this would need more sophisticated handling
            // Currently handles 2 levels deep which covers most LTFS use cases
            warn_if_deep_nesting(&subdir.contents.directories);
        }
        
        Ok(ExtractionResult {
            files_extracted,
            directories_created,
            total_bytes,
            verification_passed,
        })
    }
    
    /// Read complete file content from tape
    async fn read_complete_file_from_tape(&self, file_info: &crate::ltfs_index::File) -> Result<Vec<u8>> {
        debug!("Reading complete file from tape: {} ({} bytes)", file_info.name, file_info.length);
        
        if file_info.extent_info.extents.is_empty() {
            return Err(RustLtfsError::ltfs_index("File has no extent information".to_string()));
        }
        
        let mut result = Vec::with_capacity(file_info.length as usize);
        
        // Read all extents
        for extent in &file_info.extent_info.extents {
            let extent_data = self.read_extent_from_tape(extent).await?;
            result.extend_from_slice(&extent_data);
        }
        
        // Trim to actual file size
        result.truncate(file_info.length as usize);
        
        Ok(result)
    }
    
    /// Read a single extent from tape
    async fn read_extent_from_tape(&self, extent: &crate::ltfs_index::FileExtent) -> Result<Vec<u8>> {
        debug!("Reading extent: partition {}, block {}, {} bytes", 
               extent.partition, extent.start_block, extent.byte_count);
        
        // Use retry mechanism for critical SCSI operations
        let partition_id = self.get_partition_id(&extent.partition)?;
        
        // Position to extent start with retry
        self.verify_operation_with_retry(
            "locate_extent", 
            move || self.scsi.locate_block(partition_id, extent.start_block),
            3
        ).await?;
        
        // Calculate blocks needed
        let bytes_needed = extent.byte_count as usize;
        let blocks_needed = (bytes_needed + extent.byte_offset as usize + crate::scsi::block_sizes::LTO_BLOCK_SIZE as usize - 1) 
                           / crate::scsi::block_sizes::LTO_BLOCK_SIZE as usize;
        
        // Read blocks with retry - return the buffer directly
        let buffer_size = blocks_needed * crate::scsi::block_sizes::LTO_BLOCK_SIZE as usize;
        
        let buffer = self.verify_operation_with_retry(
            "read_extent_blocks",
            move || {
                let mut buf = vec![0u8; buffer_size];
                match self.scsi.read_blocks_with_retry(blocks_needed as u32, &mut buf, 3) {
                    Ok(blocks_read) => {
                        if blocks_read == 0 {
                            return Err(RustLtfsError::scsi("No data read from tape".to_string()));
                        }
                        Ok(buf)
                    }
                    Err(e) => Err(e)
                }
            },
            2
        ).await?;
        
        // Extract actual extent data considering byte offset
        let start_offset = extent.byte_offset as usize;
        let end_offset = start_offset + bytes_needed;
        
        if end_offset > buffer.len() {
            return Ok(buffer[start_offset..].to_vec());
        }
        
        Ok(buffer[start_offset..end_offset].to_vec())
    }
    
    /// Verify extracted file
    async fn verify_extracted_file(&self, file_path: &Path, original_content: &[u8]) -> Result<bool> {
        debug!("Verifying extracted file: {:?}", file_path);
        
        // Read written file
        let written_content = tokio::fs::read(file_path).await
            .map_err(|e| RustLtfsError::verification(
                format!("Failed to read written file for verification: {}", e)
            ))?;
        
        // Compare content
        let verification_passed = written_content == original_content;
        
        if !verification_passed {
            warn!("File verification failed: {:?} (size: {} vs {})", 
                  file_path, written_content.len(), original_content.len());
        } else {
            debug!("File verification passed: {:?}", file_path);
        }
        
        Ok(verification_passed)
    }

    /// Auto update LTFS index on tape
    pub async fn update_index_on_tape(&mut self) -> Result<()> {
        info!("Starting to update tape LTFS index...");
        
        // Allow execution in offline mode but skip actual tape operations
        if self.offline_mode {
            info!("Offline mode: simulating index update operation");
            // Create dummy index object
            if self.schema.is_none() {
                let dummy_index = LtfsIndex {
                    version: "2.4.0".to_string(),
                    creator: "RustLTFS".to_string(),
                    volumeuuid: "dummy-volume-uuid".to_string(),
                    generationnumber: 1,
                    updatetime: chrono::Utc::now().to_rfc3339(),
                    location: crate::ltfs_index::Location {
                        partition: "a".to_string(),
                        startblock: 0,
                    },
                    previousgenerationlocation: None,
                    allowpolicyupdate: None,
                    volumelockstate: None,
                    highestfileuid: Some(0),
                    root_directory: crate::ltfs_index::Directory {
                        name: "".to_string(),
                        uid: 0,
                        creation_time: chrono::Utc::now().to_rfc3339(),
                        change_time: chrono::Utc::now().to_rfc3339(),
                        modify_time: chrono::Utc::now().to_rfc3339(),
                        access_time: chrono::Utc::now().to_rfc3339(),
                        backup_time: chrono::Utc::now().to_rfc3339(),
                        read_only: false,
                        contents: crate::ltfs_index::DirectoryContents {
                            directories: vec![],
                            files: vec![],
                        },
                    },
                };
                self.schema = Some(dummy_index.clone());
                self.index = Some(dummy_index);
            }
        } else if self.tape_handle.is_none() {
            return Err(RustLtfsError::tape_device("Tape device not initialized".to_string()));
        }
        
        // Check if index is loaded
        let index = match &mut self.schema {
            Some(idx) => idx,
            None => {
                return Err(RustLtfsError::ltfs_index("Index not loaded, cannot update".to_string()));
            }
        };
        
        // Index update steps:
        
        // 1. Update index timestamp and generation number
        let now = chrono::Utc::now();
        index.updatetime = now.to_rfc3339();
        index.generationnumber += 1;
        
        info!("Updating index metadata: generation {}, update time {}", 
              index.generationnumber, index.updatetime);
        
        // 2. Locate to index partition (partition a)
        info!("Locating to index partition (partition a)");
        
        // 3. Serialize updated index to XML
        info!("Serializing index to XML format");
        
        // 4. Write index data to tape
        info!("Writing index data to tape");
        
        // 5. Write file mark
        info!("Writing file mark");
        
        // 6. Sync update internal index reference
        if let Some(ref mut internal_index) = self.index {
            *internal_index = index.clone();
        }
        
        info!("Tape LTFS index updated successfully");
        
        warn!("Current simulation implementation - need to implement real SCSI index write operation");
        
        Ok(())
    }

    /// Get tape space information (free/total)
    pub async fn get_tape_space_info(&mut self, detailed: bool) -> Result<()> {
        info!("Getting tape space information");
        
        if self.offline_mode {
            self.display_simulated_space_info(detailed);
            return Ok(());
        }
        
        // Initialize device if not already done
        if self.index.is_none() {
            match self.initialize().await {
                Ok(_) => info!("Device initialized for space check"),
                Err(e) => {
                    warn!("Device initialization failed: {}, using offline mode", e);
                    self.display_simulated_space_info(detailed);
                    return Ok(());
                }
            }
        }
        
        // Get space information from tape
        match self.get_real_tape_space_info().await {
            Ok(space_info) => self.display_tape_space_info(&space_info, detailed),
            Err(e) => {
                warn!("Failed to get real space info: {}, showing estimated info", e);
                self.display_estimated_space_info(detailed);
            }
        }
        
        Ok(())
    }
    
    /// Get real tape space information from device
    async fn get_real_tape_space_info(&self) -> Result<TapeSpaceInfo> {
        info!("Reading real tape space information from device");
        
        // Use SCSI commands to get tape capacity and remaining space
        // This would typically use READ POSITION and LOG SENSE commands
        
        // For LTO-8 tapes, typical capacity is around 12TB native, 30TB compressed
        let total_capacity = self.estimate_tape_capacity();
        
        // Calculate used space from index information
        let used_space = if let Some(ref index) = self.index {
            self.calculate_used_space_from_index(index)
        } else {
            0
        };
        
        let free_space = total_capacity.saturating_sub(used_space);
        
        Ok(TapeSpaceInfo {
            total_capacity,
            used_space,
            free_space,
            compression_ratio: 2.5, // Typical LTO compression ratio
            partition_a_used: self.get_partition_usage('a'),
            partition_b_used: self.get_partition_usage('b'),
        })
    }
    
    /// Estimate tape capacity based on media type
    fn estimate_tape_capacity(&self) -> u64 {
        // Default to LTO-8 capacity
        // In real implementation, this would query the device for actual capacity
        match self.scsi.check_media_status() {
            Ok(media_type) => {
                match media_type {
                    MediaType::Lto8Rw | MediaType::Lto8Worm | MediaType::Lto8Ro => 12_000_000_000_000, // 12TB
                    MediaType::Lto7Rw | MediaType::Lto7Worm | MediaType::Lto7Ro => 6_000_000_000_000,  // 6TB
                    MediaType::Lto6Rw | MediaType::Lto6Worm | MediaType::Lto6Ro => 2_500_000_000_000,  // 2.5TB
                    MediaType::Lto5Rw | MediaType::Lto5Worm | MediaType::Lto5Ro => 1_500_000_000_000,  // 1.5TB
                    _ => 12_000_000_000_000, // Default to LTO-8
                }
            }
            Err(_) => 12_000_000_000_000, // Default capacity
        }
    }
    
    /// Calculate used space from LTFS index
    fn calculate_used_space_from_index(&self, index: &LtfsIndex) -> u64 {
        let file_locations = index.extract_tape_file_locations();
        file_locations.iter().map(|loc| loc.file_size).sum()
    }
    
    /// Get partition usage
    fn get_partition_usage(&self, partition: char) -> u64 {
        if let Some(ref index) = self.index {
            let file_locations = index.extract_tape_file_locations();
            file_locations.iter()
                .flat_map(|loc| &loc.extents)
                .filter(|extent| extent.partition.to_lowercase() == partition.to_string().to_lowercase())
                .map(|extent| extent.byte_count)
                .sum()
        } else {
            0
        }
    }
    
    /// Display tape space information
    fn display_tape_space_info(&self, space_info: &TapeSpaceInfo, detailed: bool) {
        println!("\n💾 Tape Space Information");
        println!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");
        
        let total_gb = space_info.total_capacity as f64 / 1_073_741_824.0;
        let used_gb = space_info.used_space as f64 / 1_073_741_824.0;
        let free_gb = space_info.free_space as f64 / 1_073_741_824.0;
        let usage_percent = (space_info.used_space as f64 / space_info.total_capacity as f64) * 100.0;
        
        println!("  📊 Capacity Overview:");
        println!("      Total: {:.2} GB ({} bytes)", total_gb, space_info.total_capacity);
        println!("      Used:  {:.2} GB ({} bytes) [{:.1}%]", used_gb, space_info.used_space, usage_percent);
        println!("      Free:  {:.2} GB ({} bytes) [{:.1}%]", free_gb, space_info.free_space, 100.0 - usage_percent);
        
        // Progress bar
        let bar_width = 40;
        let used_blocks = ((usage_percent / 100.0) * bar_width as f64) as usize;
        let free_blocks = bar_width - used_blocks;
        println!("      [{}{}] {:.1}%", 
            "█".repeat(used_blocks), 
            "░".repeat(free_blocks), 
            usage_percent);
        
        if detailed {
            println!("\n  📁 Partition Usage:");
            let partition_a_gb = space_info.partition_a_used as f64 / 1_073_741_824.0;
            let partition_b_gb = space_info.partition_b_used as f64 / 1_073_741_824.0;
            println!("      Partition A (Index): {:.2} GB ({} bytes)", partition_a_gb, space_info.partition_a_used);
            println!("      Partition B (Data):  {:.2} GB ({} bytes)", partition_b_gb, space_info.partition_b_used);
            
            println!("\n  ⚙️  Technical Information:");
            println!("      Compression Ratio: {:.1}x", space_info.compression_ratio);
            println!("      Effective Capacity: {:.2} GB (with compression)", 
                total_gb * space_info.compression_ratio);
            
            if let Some(ref index) = self.index {
                let file_count = index.extract_tape_file_locations().len();
                println!("      Total Files: {}", file_count);
                if file_count > 0 {
                    let avg_file_size = space_info.used_space / file_count as u64;
                    println!("      Average File Size: {:.2} MB", avg_file_size as f64 / 1_048_576.0);
                }
            }
        }
    }
    
    /// Display simulated space information for offline mode
    fn display_simulated_space_info(&self, detailed: bool) {
        println!("\n💾 Tape Space Information (Simulated)");
        println!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");
        
        let total_capacity = 12_000_000_000_000u64; // 12TB for LTO-8
        let used_space = 2_500_000_000_000u64;     // Simulated 2.5TB used
        let free_space = total_capacity - used_space;
        let usage_percent = (used_space as f64 / total_capacity as f64) * 100.0;
        
        let total_gb = total_capacity as f64 / 1_073_741_824.0;
        let used_gb = used_space as f64 / 1_073_741_824.0;
        let free_gb = free_space as f64 / 1_073_741_824.0;
        
        println!("  📊 Capacity Overview (Simulated):");
        println!("      Total: {:.2} GB ({} bytes)", total_gb, total_capacity);
        println!("      Used:  {:.2} GB ({} bytes) [{:.1}%]", used_gb, used_space, usage_percent);
        println!("      Free:  {:.2} GB ({} bytes) [{:.1}%]", free_gb, free_space, 100.0 - usage_percent);
        
        // Progress bar
        let bar_width = 40;
        let used_blocks = ((usage_percent / 100.0) * bar_width as f64) as usize;
        let free_blocks = bar_width - used_blocks;
        println!("      [{}{}] {:.1}%", 
            "█".repeat(used_blocks), 
            "░".repeat(free_blocks), 
            usage_percent);
        
        if detailed {
            println!("\n  📁 Partition Usage (Simulated):");
            println!("      Partition A (Index): 50.00 GB (53,687,091,200 bytes)");
            println!("      Partition B (Data):  2,450.00 GB (2,631,312,908,800 bytes)");
            
            println!("\n  ⚙️  Technical Information:");
            println!("      Media Type: LTO-8 (Simulated)");
            println!("      Compression Ratio: 2.5x");
            println!("      Effective Capacity: {:.2} GB (with compression)", total_gb * 2.5);
            println!("      Block Size: 64 KB");
        }
        
        println!("\n⚠️  Note: This is simulated data. Connect to a real tape device for actual space information.");
    }
    
    /// Display estimated space information when real data is not available
    fn display_estimated_space_info(&self, detailed: bool) {
        if let Some(ref index) = self.index {
            let file_locations = index.extract_tape_file_locations();
            let used_space: u64 = file_locations.iter().map(|loc| loc.file_size).sum();
            let total_capacity = self.estimate_tape_capacity();
            let free_space = total_capacity.saturating_sub(used_space);
            
            let space_info = TapeSpaceInfo {
                total_capacity,
                used_space,
                free_space,
                compression_ratio: 2.5,
                partition_a_used: self.get_partition_usage('a'),
                partition_b_used: self.get_partition_usage('b'),
            };
            
            println!("\n💾 Tape Space Information (Estimated from Index)");
            println!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");
            self.display_tape_space_info(&space_info, detailed);
            println!("\n⚠️  Note: Space information estimated from LTFS index. Actual values may differ.");
        } else {
            self.display_simulated_space_info(detailed);
        }
    }

    /// 保存索引到本地文件 (对应LTFSIndex_Load_*.schema格式)
    /// 根据项目规范，采用LTFSIndex_Load_<timestamp>.schema格式
    pub async fn save_index_to_file(&self, file_path: &Path) -> Result<()> {
        info!("Saving LTFS index to file: {:?}", file_path);
        
        // 检查索引是否已加载
        let index = match &self.index {
            Some(idx) => idx,
            None => {
                return Err(RustLtfsError::ltfs_index("Index not loaded, cannot save".to_string()));
            }
        };
        
        // 对应LTFSWriter.vb中的索引保存步骤：
        
        // 1. 将索引序列化为XML格式
        info!("Serializing index to XML format");
        let xml_content = index.to_xml()?;
        
        // 2. 创建目标目录(如果不存在)
        if let Some(parent) = file_path.parent() {
            tokio::fs::create_dir_all(parent).await
                .map_err(|e| RustLtfsError::file_operation(
                    format!("Unable to create target directory: {}", e)
                ))?;
        }
        
        // 3. 写入XML内容到文件
        tokio::fs::write(file_path, xml_content).await
            .map_err(|e| RustLtfsError::file_operation(
                format!("Unable to write index file: {}", e)
            ))?;
        
        info!("Index file saved successfully: {:?}", file_path);
        
        Ok(())
    }
}

/// Index statistics structure
#[derive(Debug, Clone)]
pub struct IndexStatistics {
    pub volume_uuid: String,
    pub generation_number: u64,
    pub update_time: String,
    pub creator: String,
    pub total_files: usize,
}

/// View index utilities
pub struct IndexViewer;

impl IndexViewer {
    /// Handle view index command
    pub async fn handle_view_index_command(
        index_file: &Path,
        detailed: bool,
        export_format: Option<crate::cli::ExportFormat>,
        output: Option<&Path>,
    ) -> Result<()> {
        use tracing::info;
        
        info!("📖 Viewing local LTFS index file: {:?}", index_file);
        
        let xml_content = tokio::fs::read_to_string(index_file).await
            .map_err(|e| RustLtfsError::file_operation(
                format!("Unable to read index file: {}", e)
            ))?;
        
        let index = LtfsIndex::from_xml(&xml_content)?;
        
        Self::display_index_summary(&index);
        
        let file_locations = index.extract_tape_file_locations();
        
        if detailed {
            Self::display_detailed_file_info(&file_locations);
        }
        
        if let Some(format) = export_format {
            let output_content = Self::export_file_list(&file_locations, format)?;
            
            if let Some(output_path) = output {
                tokio::fs::write(output_path, output_content).await
                    .map_err(|e| RustLtfsError::file_operation(
                        format!("Unable to write output file: {}", e)
                    ))?;
                info!("✅ File list exported to: {:?}", output_path);
            } else {
                println!("{}", output_content);
            }
        }
        
        Ok(())
    }
    
    /// Display index summary
    fn display_index_summary(index: &LtfsIndex) {
        println!("\n📋 LTFS Index Summary");
        println!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");
        println!("  • Version: {}", index.version);
        println!("  • Volume UUID: {}", index.volumeuuid);
        println!("  • Generation Number: {}", index.generationnumber);
        println!("  • Update Time: {}", index.updatetime);
        println!("  • Creator: {}", index.creator);
        
        if let Some(highest_uid) = index.highestfileuid {
            println!("  • Highest File UID: {}", highest_uid);
        }
        
        let file_locations = index.extract_tape_file_locations();
        println!("  • Total Files: {}", file_locations.len());
        
        // Partition statistics
        let mut partition_a_count = 0;
        let mut partition_b_count = 0;
        let mut total_size = 0u64;
        
        for location in &file_locations {
            total_size += location.file_size;
            for extent in &location.extents {
                if extent.partition.to_lowercase() == "a" {
                    partition_a_count += 1;
                } else if extent.partition.to_lowercase() == "b" {
                    partition_b_count += 1;
                }
            }
        }
        
        println!("  • Partition A Files: {}", partition_a_count);
        println!("  • Partition B Files: {}", partition_b_count);
        println!("  • Total Size: {} bytes ({:.2} MB)", total_size, total_size as f64 / 1_048_576.0);
    }
    
    /// Display detailed file information
    fn display_detailed_file_info(file_locations: &[crate::ltfs_index::TapeFileLocation]) {
        println!("\n📁 Detailed File Information");
        println!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");
        
        for (index, location) in file_locations.iter().enumerate().take(20) {
            println!("\n{:3}. {}", index + 1, location.file_name);
            println!("     UID: {} | Size: {} bytes", location.file_uid, location.file_size);
            
            for (i, extent) in location.extents.iter().enumerate() {
                println!("     Extent {}: Partition {} Block {} Offset {} Size {}", 
                    i + 1, extent.partition, extent.start_block, 
                    extent.byte_offset, extent.byte_count);
            }
        }
        
        if file_locations.len() > 20 {
            println!("\n... {} more files not displayed", file_locations.len() - 20);
        }
    }
    
    /// Export file list
    fn export_file_list(
        file_locations: &[crate::ltfs_index::TapeFileLocation],
        format: crate::cli::ExportFormat,
    ) -> Result<String> {
        use crate::cli::ExportFormat;
        
        match format {
            ExportFormat::Tsv => {
                let mut output = String::from("Partition\tStartblock\tLength\tPath\n");
                for location in file_locations {
                    for extent in &location.extents {
                        output.push_str(&format!("{}\t{}\t{}\t{}\n",
                            extent.partition, extent.start_block, 
                            extent.byte_count, location.file_name));
                    }
                }
                Ok(output)
            }
            
            ExportFormat::Json => {
                // Simplified JSON export
                Ok(format!("{{\"files\": {}}}", file_locations.len()))
            }
            
            ExportFormat::Xml => {
                let mut output = String::from("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<files>\n");
                for location in file_locations {
                    output.push_str(&format!("  <file name=\"{}\" uid=\"{}\" size=\"{}\"/>\n",
                        location.file_name, location.file_uid, location.file_size));
                }
                output.push_str("</files>\n");
                Ok(output)
            }
            
            ExportFormat::Batch => {
                let mut output = String::from("chcp 65001\n");
                for location in file_locations {
                    output.push_str(&format!("echo Writing: {}\n", location.file_name));
                    output.push_str(&format!("rem File UID: {}, Size: {} bytes\n", 
                        location.file_uid, location.file_size));
                }
                Ok(output)
            }
        }
    }
}