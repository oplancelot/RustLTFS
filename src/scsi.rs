use crate::error::{Result, RustLtfsError};
use std::ffi::CString;

use tracing::{debug, warn, info};

#[cfg(windows)]
use winapi::{
    shared::{
        minwindef::{DWORD, ULONG, USHORT, UCHAR},
        ntdef::{HANDLE, PVOID},
    },
    um::{
        fileapi::{CreateFileA, OPEN_EXISTING},
        handleapi::{CloseHandle, INVALID_HANDLE_VALUE},
        ioapiset::DeviceIoControl,
        winnt::{GENERIC_READ, GENERIC_WRITE, FILE_SHARE_DELETE, FILE_SHARE_READ, FILE_SHARE_WRITE},
        errhandlingapi::GetLastError,
    },
};

// Define IOCTL_SCSI_PASS_THROUGH_DIRECT constant
#[cfg(windows)]
const IOCTL_SCSI_PASS_THROUGH_DIRECT: u32 = 0x0004D014;

// Type aliases for non-Windows platforms
#[cfg(not(windows))]
type UCHAR = u8;
#[cfg(not(windows))]
type USHORT = u16;
#[cfg(not(windows))]
type ULONG = u32;
#[cfg(not(windows))]
type DWORD = u32;
#[cfg(not(windows))]
type PVOID = *mut std::ffi::c_void;
#[cfg(not(windows))]
type HANDLE = *mut std::ffi::c_void;

// SCSI constant definitions (based on C code)
const SENSE_INFO_LEN: usize = 64;
const TC_MP_PC_CURRENT: u8 = 0x00;
const TC_MP_MEDIUM_CONFIGURATION: u8 = 0x1D;

// SCSI operation code constants
const SCSIOP_READ_POSITION: u8 = 0x34;
const SCSIOP_MODE_SENSE10: u8 = 0x5A;
const SCSIOP_INQUIRY: u8 = 0x12;

// SCSI data direction
const SCSI_IOCTL_DATA_IN: u8 = 1;
const SCSI_IOCTL_DATA_OUT: u8 = 0;
const SCSI_IOCTL_DATA_UNSPECIFIED: u8 = 2;

/// SCSI Pass Through Direct structure (corresponds to SCSI_PASS_THROUGH_DIRECT in C code)
#[repr(C)]
#[derive(Debug)]
struct ScsiPassThroughDirect {
    length: USHORT,
    scsi_status: UCHAR,
    path_id: UCHAR,
    target_id: UCHAR,
    lun: UCHAR,
    cdb_length: UCHAR,
    sense_info_length: UCHAR,
    data_in: UCHAR,
    data_transfer_length: ULONG,
    timeout_value: ULONG,
    data_buffer: PVOID,
    sense_info_offset: ULONG,
    cdb: [UCHAR; 16],
}

/// Media type enumeration, based on media type detection in C code
#[derive(Debug, Clone, PartialEq)]
pub enum MediaType {
    NoTape,
    Lto3Rw,      // 0x0044
    Lto3Worm,    // 0x0144  
    Lto3Ro,      // 0x0244
    Lto4Rw,      // 0x0046
    Lto4Worm,    // 0x0146
    Lto4Ro,      // 0x0246
    Lto5Rw,      // 0x0058
    Lto5Worm,    // 0x0158
    Lto5Ro,      // 0x0258
    Lto6Rw,      // 0x005A
    Lto6Worm,    // 0x015A
    Lto6Ro,      // 0x025A
    Lto7Rw,      // 0x005C
    Lto7Worm,    // 0x015C
    Lto7Ro,      // 0x025C
    Lto8Rw,      // 0x005E
    Lto8Worm,    // 0x015E
    Lto8Ro,      // 0x025E
    Lto9Rw,      // 0x0060
    Lto9Worm,    // 0x0160
    Lto9Ro,      // 0x0260
    LtoM8Rw,     // 0x005D
    LtoM8Worm,   // 0x015D
    LtoM8Ro,     // 0x025D
    Unknown(u16),
}

impl MediaType {
    /// Convert from media type code to media type
    fn from_media_type_code(code: u16) -> Self {
        match code {
            0x0044 => MediaType::Lto3Rw,
            0x0144 => MediaType::Lto3Worm,
            0x0244 => MediaType::Lto3Ro,
            0x0046 => MediaType::Lto4Rw,
            0x0146 => MediaType::Lto4Worm,
            0x0246 => MediaType::Lto4Ro,
            0x0058 => MediaType::Lto5Rw,
            0x0158 => MediaType::Lto5Worm,
            0x0258 => MediaType::Lto5Ro,
            0x005A => MediaType::Lto6Rw,
            0x015A => MediaType::Lto6Worm,
            0x025A => MediaType::Lto6Ro,
            0x005C => MediaType::Lto7Rw,
            0x015C => MediaType::Lto7Worm,
            0x025C => MediaType::Lto7Ro,
            0x005E => MediaType::Lto8Rw,
            0x015E => MediaType::Lto8Worm,
            0x025E => MediaType::Lto8Ro,
            0x0060 => MediaType::Lto9Rw,
            0x0160 => MediaType::Lto9Worm,
            0x0260 => MediaType::Lto9Ro,
            0x005D => MediaType::LtoM8Rw,
            0x015D => MediaType::LtoM8Worm,
            0x025D => MediaType::LtoM8Ro,
            _ => MediaType::Unknown(code),
        }
    }
    
    /// Convert to description string
    pub fn description(&self) -> &'static str {
        match self {
            MediaType::NoTape => "No tape loaded",
            MediaType::Lto3Rw => "LTO3 RW",
            MediaType::Lto3Worm => "LTO3 WORM",
            MediaType::Lto3Ro => "LTO3 RO",
            MediaType::Lto4Rw => "LTO4 RW",
            MediaType::Lto4Worm => "LTO4 WORM",
            MediaType::Lto4Ro => "LTO4 RO",
            MediaType::Lto5Rw => "LTO5 RW",
            MediaType::Lto5Worm => "LTO5 WORM",
            MediaType::Lto5Ro => "LTO5 RO",
            MediaType::Lto6Rw => "LTO6 RW",
            MediaType::Lto6Worm => "LTO6 WORM",
            MediaType::Lto6Ro => "LTO6 RO",
            MediaType::Lto7Rw => "LTO7 RW",
            MediaType::Lto7Worm => "LTO7 WORM",
            MediaType::Lto7Ro => "LTO7 RO",
            MediaType::Lto8Rw => "LTO8 RW",
            MediaType::Lto8Worm => "LTO8 WORM",
            MediaType::Lto8Ro => "LTO8 RO",
            MediaType::Lto9Rw => "LTO9 RW",
            MediaType::Lto9Worm => "LTO9 WORM",
            MediaType::Lto9Ro => "LTO9 RO",
            MediaType::LtoM8Rw => "LTOM8 RW",
            MediaType::LtoM8Worm => "LTOM8 WORM",
            MediaType::LtoM8Ro => "LTOM8 RO",
            MediaType::Unknown(_) => "Unknown media type",
        }
    }
}

/// SCSI operation structure that encapsulates low-level SCSI commands
pub struct ScsiInterface {
    device_handle: Option<DeviceHandle>,
    drive_type: DriveType,
    allow_partition: bool,
}

/// Device handle wrapper that ensures proper resource cleanup
struct DeviceHandle {
    #[cfg(windows)]
    handle: HANDLE,
    device_path: String,
}

/// Tape drive information structure, corresponds to TAPE_DRIVE in C code
#[derive(Debug, Clone)]
pub struct TapeDriveInfo {
    pub vendor_id: String,
    pub product_id: String,
    pub serial_number: String,
    pub device_index: u32,
    pub device_path: String,
}

impl ScsiInterface {
    /// Create new SCSI interface instance
    pub fn new() -> Self {
        Self {
            device_handle: None,
            drive_type: DriveType::Standard,
            allow_partition: true,
        }
    }
    
    /// Create new SCSI interface with specific drive type
    pub fn with_drive_type(drive_type: DriveType) -> Self {
        Self {
            device_handle: None,
            drive_type,
            allow_partition: true,
        }
    }
    
    /// Open tape device (based on CreateFile call in C code)
    pub fn open_device(&mut self, device_path: &str) -> Result<()> {
        debug!("Opening tape device: {}", device_path);
        
        #[cfg(windows)]
        {
            // Build complete device path, similar to "\\\\.\\TAPE0" format in C code
            let full_path = if device_path.starts_with(r"\\.\") {
                device_path.to_string()
            } else if device_path.starts_with("TAPE") {
                format!(r"\\.\{}", device_path)
            } else {
                // Already formatted path like \\.\TAPE1
                device_path.to_string()
            };
            
            debug!("Full device path: {}", full_path);
            
            let path_cstring = CString::new(full_path.clone())
                .map_err(|e| crate::error::RustLtfsError::system(format!("Device path conversion error: {}", e)))?;
            
            unsafe {
                let handle = CreateFileA(
                    path_cstring.as_ptr(),
                    GENERIC_READ | GENERIC_WRITE,
                    FILE_SHARE_DELETE | FILE_SHARE_READ | FILE_SHARE_WRITE, // Based on C code
                    std::ptr::null_mut(),
                    OPEN_EXISTING,
                    0, // Don't use FILE_ATTRIBUTE_NORMAL, based on C code
                    std::ptr::null_mut(),
                );
                
                if handle == INVALID_HANDLE_VALUE {
                    let error_code = GetLastError();
                    return Err(crate::error::RustLtfsError::system(
                        format!("Cannot open device {}: Windows error code 0x{:08X}", full_path, error_code)
                    ));
                }
                
                self.device_handle = Some(DeviceHandle {
                    handle,
                    device_path: full_path,
                });
                
                debug!("Device opened successfully: {}", device_path);
                Ok(())
            }
        }
        
        #[cfg(not(windows))]
        {
            Err(crate::error::RustLtfsError::unsupported("Non-Windows platform"))
        }
    }
    
    /// Send SCSI command general interface (based on ScsiIoControl in C code)
    fn scsi_io_control(
        &self,
        cdb: &[u8],
        mut data_buffer: Option<&mut [u8]>,
        data_in: u8,
        timeout: u32,
        sense_buffer: Option<&mut [u8; SENSE_INFO_LEN]>,
    ) -> Result<bool> {
        #[cfg(windows)]
        {
            if let Some(ref device) = self.device_handle {
                let buffer_length = data_buffer.as_ref().map_or(0, |buf| buf.len()) as ULONG;
                let data_ptr = data_buffer
                    .as_mut()
                    .map_or(std::ptr::null_mut(), |buf| buf.as_mut_ptr() as PVOID);
                
                // Create SCSI Pass Through Direct buffer
                let mut scsi_buffer = vec![0u8; std::mem::size_of::<ScsiPassThroughDirect>() + SENSE_INFO_LEN];
                
                unsafe {
                    let scsi_direct = scsi_buffer.as_mut_ptr() as *mut ScsiPassThroughDirect;
                    std::ptr::write_bytes(scsi_direct, 0, 1);
                    
                    (*scsi_direct).length = std::mem::size_of::<ScsiPassThroughDirect>() as USHORT;
                    (*scsi_direct).cdb_length = cdb.len() as UCHAR;
                    (*scsi_direct).data_buffer = data_ptr;
                    (*scsi_direct).sense_info_length = SENSE_INFO_LEN as UCHAR;
                    (*scsi_direct).sense_info_offset = std::mem::size_of::<ScsiPassThroughDirect>() as ULONG;
                    (*scsi_direct).data_transfer_length = buffer_length;
                    (*scsi_direct).timeout_value = timeout;
                    (*scsi_direct).data_in = data_in;
                    
                    // Copy CDB
                    std::ptr::copy_nonoverlapping(cdb.as_ptr(), (*scsi_direct).cdb.as_mut_ptr(), cdb.len());
                    
                    let mut bytes_returned: DWORD = 0;
                    let result = DeviceIoControl(
                        device.handle,
                        IOCTL_SCSI_PASS_THROUGH_DIRECT,
                        scsi_buffer.as_mut_ptr() as PVOID,
                        scsi_buffer.len() as DWORD,
                        scsi_buffer.as_mut_ptr() as PVOID,
                        scsi_buffer.len() as DWORD,
                        &mut bytes_returned,
                        std::ptr::null_mut(),
                    ) != 0;
                    
                    // Copy sense buffer if provided
                    if let Some(sense_buf) = sense_buffer {
                        std::ptr::copy_nonoverlapping(
                            scsi_buffer.as_ptr().add(std::mem::size_of::<ScsiPassThroughDirect>()),
                            sense_buf.as_mut_ptr(),
                            SENSE_INFO_LEN,
                        );
                    }
                    
                    if !result {
                        let error_code = GetLastError();
                        debug!("SCSI command failed: Windows error code 0x{:08X}", error_code);
                        return Ok(false);
                    }
                    
                    Ok(true)
                }
            } else {
                Err(crate::error::RustLtfsError::scsi("Device not opened"))
            }
        }
        
        #[cfg(not(windows))]
        {
            // Use parameters on non-Windows platforms to avoid warnings
            let _ = (cdb, data_buffer, data_in, timeout, sense_buffer);
            Err(crate::error::RustLtfsError::unsupported("Non-Windows platform"))
        }
    }
    
    /// Check tape media status (based on TapeCheckMedia function in C code)
    pub fn check_media_status(&self) -> Result<MediaType> {
        debug!("Checking tape media status");
        
        #[cfg(windows)]
        {
            // Step 1: Use READ POSITION to check if tape is present
            // "There doesn't appear to be a direct way to tell if there's anything in the drive,
            // so instead we just try and read the position which won't fuck up a mounted LTFS volume."
            let mut cdb = [0u8; 10];
            let mut data_buffer = [0u8; 64];
            let mut sense_buffer = [0u8; SENSE_INFO_LEN];
            
            // Set read POSITION CDB
            cdb[0] = SCSIOP_READ_POSITION; // Operation Code
            cdb[1] = 0x03; // Reserved1，based on C code
            
            let result = self.scsi_io_control(
                &cdb,
                Some(&mut data_buffer),
                SCSI_IOCTL_DATA_IN,
                300, // 300 second timeout, based on C code
                Some(&mut sense_buffer),
            )?;
            
            if !result {
                return Err(crate::error::RustLtfsError::scsi("read_position command failed"));
            }
            
            // Check if sense buffer indicates no tape
            // C code: if (((senseBuffer[2] & 0x0F) == 0x02) && (senseBuffer[12] == 0x3A) && (senseBuffer[13] == 0x00))
            if (sense_buffer[2] & 0x0F) == 0x02 && sense_buffer[12] == 0x3A && sense_buffer[13] == 0x00 {
                debug!("No tape detected");
                return Ok(MediaType::NoTape);
            }
            
            // Step 2: Use MODE SENSE 10 to get media type
            // "This will only tell us the *last* tape that was in the drive, which is why we have to do the above check first"
            cdb.fill(0);
            data_buffer.fill(0);
            
            cdb[0] = SCSIOP_MODE_SENSE10; // Operation Code
            cdb[2] = TC_MP_MEDIUM_CONFIGURATION; // Page Code
            cdb[2] |= TC_MP_PC_CURRENT << 6; // PC field
            cdb[7] = (data_buffer.len() >> 8) as u8; // Allocation Length MSB
            cdb[8] = (data_buffer.len() & 0xFF) as u8; // Allocation Length LSB
            
            let result = self.scsi_io_control(
                &cdb,
                Some(&mut data_buffer),
                SCSI_IOCTL_DATA_IN,
                300,
                None,
            )?;
            
            if !result {
                warn!("MODE_SENSE10 command failed, but tape may exist");
                return Ok(MediaType::Unknown(0));
            }
            
            // Parse media type, based on C code logic
            let mut media_type = data_buffer[8] as u16 + ((data_buffer[18] as u16 & 0x01) << 8);
            
            // Check if it's not WORM type, based on C code comments
            if (media_type & 0x100) == 0 {
                media_type |= (data_buffer[3] as u16 & 0x80) << 2;
            }
            
            debug!("Detected media type code: 0x{:04X}", media_type);
            
            Ok(MediaType::from_media_type_code(media_type))
        }
        
        #[cfg(not(windows))]
        {
            Err(crate::error::RustLtfsError::unsupported("Non-Windows platform"))
        }
    }
    
    /// Tape loading (based on TapeLoad function in C code)
    pub fn load_tape(&self) -> Result<bool> {
        debug!("Loading tape");
        
        #[cfg(windows)]
        {
            let mut cdb = [0u8; 6];
            cdb[0] = 0x1B; // SCSIOP_LOAD_UNLOAD，based on C code
            cdb[4] = 1; // Start = 1，based on C code
            
            let result = self.scsi_io_control(
                &cdb,
                None,
                SCSI_IOCTL_DATA_UNSPECIFIED,
                300,
                None,
            )?;
            
            Ok(result)
        }
        
        #[cfg(not(windows))]
        {
            Err(crate::error::RustLtfsError::unsupported("Non-Windows platform"))
        }
    }
    
    /// Send SCSI command with simplified interface (for compatibility with tape_ops.rs)
    pub fn send_scsi_command(&self, cdb: &[u8], data_buffer: &mut [u8], data_direction: u8) -> Result<bool> {
        let data_in = match data_direction {
            0 => SCSI_IOCTL_DATA_OUT,
            1 => SCSI_IOCTL_DATA_IN,
            _ => SCSI_IOCTL_DATA_UNSPECIFIED,
        };

        self.scsi_io_control(cdb, Some(data_buffer), data_in, 300, None)
    }

    /// Test Unit Ready command - check if device is ready
    pub fn test_unit_ready(&self) -> Result<Vec<u8>> {
        debug!("Executing Test Unit Ready command");
        
        #[cfg(windows)]
        {
            let mut cdb = [0u8; 6];
            cdb[0] = scsi_commands::TEST_UNIT_READY;
            // Other bytes remain 0 for standard Test Unit Ready
            
            let mut sense_buffer = [0u8; SENSE_INFO_LEN];
            
            let result = self.scsi_io_control(
                &cdb,
                None,
                SCSI_IOCTL_DATA_UNSPECIFIED,
                30, // 30 second timeout for Test Unit Ready
                Some(&mut sense_buffer),
            )?;
            
            if result {
                debug!("Test Unit Ready completed successfully");
                Ok(sense_buffer.to_vec())
            } else {
                debug!("Test Unit Ready failed, returning sense data");
                Ok(sense_buffer.to_vec())
            }
        }
        
        #[cfg(not(windows))]
        {
            Err(crate::error::RustLtfsError::unsupported("Non-Windows platform"))
        }
    }

    /// Parse sense data for Test Unit Ready (similar to LTFSCopyGUI's ParseSenseData)
    pub fn parse_sense_data(&self, sense_data: &[u8]) -> String {
        if sense_data.len() < 3 {
            return "Invalid sense data (too short)".to_string();
        }
        
        let sense_key = sense_data[2] & 0x0F;
        let asc = if sense_data.len() > 12 { sense_data[12] } else { 0 };
        let ascq = if sense_data.len() > 13 { sense_data[13] } else { 0 };
        
        debug!("Sense data - Key: 0x{:02X}, ASC: 0x{:02X}, ASCQ: 0x{:02X}", sense_key, asc, ascq);
        
        match (sense_key, asc, ascq) {
            (0x00, _, _) => "Device ready".to_string(),
            (0x02, 0x3A, 0x00) => "No tape loaded".to_string(),
            (0x02, 0x04, 0x00) => "Drive not ready".to_string(),
            (0x02, 0x3B, 0x0D) => "Medium not present".to_string(),
            (0x04, 0x00, 0x00) => "Drive not ready - becoming ready".to_string(),
            (0x06, 0x28, 0x00) => "Unit attention - not ready to ready transition".to_string(),
            _ => format!("Device not ready - Sense Key: 0x{:02X}, ASC/ASCQ: 0x{:02X}/0x{:02X}", 
                        sense_key, asc, ascq)
        }
    }

    /// Tape ejection (based on TapeEject function logic in C code)
    pub fn eject_tape(&self) -> Result<bool> {
        debug!("Ejecting tape");
        
        #[cfg(windows)]
        {
            if let Some(ref device) = self.device_handle {
                unsafe {
                    let mut bytes_returned: DWORD = 0;
                    
                    // 1. Lock volume
                    let lock_result = DeviceIoControl(
                        device.handle,
                        0x00090018, // FSCTL_LOCK_VOLUME
                        std::ptr::null_mut(),
                        0,
                        std::ptr::null_mut(),
                        0,
                        &mut bytes_returned,
                        std::ptr::null_mut(),
                    ) != 0;
                    
                    if !lock_result {
                        warn!("lock failed");
                        return Ok(false);
                    }
                    
                    // 2. Dismount volume
                    let dismount_result = DeviceIoControl(
                        device.handle,
                        0x00090020, // FSCTL_DISMOUNT_VOLUME
                        std::ptr::null_mut(),
                        0,
                        std::ptr::null_mut(),
                        0,
                        &mut bytes_returned,
                        std::ptr::null_mut(),
                    ) != 0;
                    
                    if !dismount_result {
                        warn!("Dismount volume failed");
                        return Ok(false);
                    }
                    
                    // 3. Eject media
                    let eject_result = DeviceIoControl(
                        device.handle,
                        0x002D4808, // IOCTL_DISK_EJECT_MEDIA
                        std::ptr::null_mut(),
                        0,
                        std::ptr::null_mut(),
                        0,
                        &mut bytes_returned,
                        std::ptr::null_mut(),
                    ) != 0;
                    
                    Ok(eject_result)
                }
            } else {
                Err(crate::error::RustLtfsError::scsi("Device not opened"))
            }
        }
        
        #[cfg(not(windows))]
        {
            Err(crate::error::RustLtfsError::unsupported("Non-Windows platform"))
        }
    }
    
    /// Read tape blocks (enhanced implementation for large file support)
    pub fn read_blocks(&self, block_count: u32, buffer: &mut [u8]) -> Result<u32> {
        debug!("Reading {} blocks from tape", block_count);
        
        // 移除硬编码的LTO_BLOCK_SIZE检查，改为动态缓冲区处理
        // 对应LTFSCopyGUI的自适应缓冲区逻辑，不预先检查缓冲区大小
        // 让SCSI驱动返回实际读取的字节数或错误信息
        
        // For large block counts, break into smaller chunks to avoid SCSI timeout
        const MAX_BLOCKS_PER_READ: u32 = 256; // 16MB chunks (256 * 64KB)
        
        if block_count <= MAX_BLOCKS_PER_READ {
            // Direct read for smaller requests
            self.read_blocks_direct(block_count, buffer)
        } else {
            // Chunked read for larger requests
            self.read_blocks_chunked(block_count, buffer)
        }
    }
    
    /// Direct block read implementation (private)
    fn read_blocks_direct(&self, block_count: u32, buffer: &mut [u8]) -> Result<u32> {
        debug!("Direct reading {} blocks", block_count);
        
        #[cfg(windows)]
        {
            // Use READ(10) command for better parameter range support
            let mut cdb = [0u8; 10];
            cdb[0] = scsi_commands::READ_10;
            
            // Fixed block mode - use DPO=0, FUA=0, RelAddr=0
            cdb[1] = 0x00;
            
            // Logical Block Address (LBA) - set to 0 for sequential access
            cdb[2] = 0x00;
            cdb[3] = 0x00;
            cdb[4] = 0x00;
            cdb[5] = 0x00;
            
            // Reserved
            cdb[6] = 0x00;
            
            // Transfer Length (in blocks)
            cdb[7] = ((block_count >> 8) & 0xFF) as u8;
            cdb[8] = (block_count & 0xFF) as u8;
            
            // Control
            cdb[9] = 0x00;
            
            // 使用实际提供的缓冲区大小，而不是假定的块大小
            // 对应LTFSCopyGUI的动态缓冲区处理逻辑
            let data_length = buffer.len();
            
            // Adjust timeout based on buffer size
            let timeout = std::cmp::max(300, (data_length / (64 * 1024)) * 60); // Min 5min, scale with size
            
            let result = self.scsi_io_control(
                &cdb,
                Some(&mut buffer[..data_length]),
                SCSI_IOCTL_DATA_IN,
                timeout,
                None,
            )?;
            
            if result {
                debug!("Successfully read {} blocks directly", block_count);
                Ok(block_count)
            } else {
                Err(crate::error::RustLtfsError::scsi("Direct block read operation failed"))
            }
        }
        
        #[cfg(not(windows))]
        {
            let _ = (block_count, buffer);
            Err(crate::error::RustLtfsError::unsupported("Non-Windows platform"))
        }
    }
    
    /// Chunked block read for large files (private)
    fn read_blocks_chunked(&self, block_count: u32, buffer: &mut [u8]) -> Result<u32> {
        debug!("Chunked reading {} blocks", block_count);
        
        const CHUNK_SIZE: u32 = 128; // 8MB chunks for better performance
        let mut total_read = 0u32;
        let mut remaining = block_count;
        
        while remaining > 0 {
            let current_chunk = std::cmp::min(remaining, CHUNK_SIZE);
            let offset = (total_read * block_sizes::LTO_BLOCK_SIZE) as usize;
            
            debug!("Reading chunk: {} blocks (offset: {} bytes)", current_chunk, offset);
            
            // Read current chunk
            let chunk_buffer = &mut buffer[offset..(offset + (current_chunk * block_sizes::LTO_BLOCK_SIZE) as usize)];
            
            match self.read_blocks_direct(current_chunk, chunk_buffer) {
                Ok(read_count) => {
                    if read_count != current_chunk {
                        warn!("Partial chunk read: expected {}, got {}", current_chunk, read_count);
                        total_read += read_count;
                        break; // Stop on partial read
                    }
                    total_read += read_count;
                    remaining -= read_count;
                    
                    // Small delay between chunks to prevent overloading the drive
                    std::thread::sleep(std::time::Duration::from_millis(10));
                }
                Err(e) => {
                    if total_read > 0 {
                        warn!("Chunked read failed after {} blocks: {}", total_read, e);
                        break; // Return partial success
                    } else {
                        return Err(e); // Return error if no data read
                    }
                }
            }
        }
        
        info!("Chunked read completed: {} of {} blocks", total_read, block_count);
        Ok(total_read)
    }
    
    /// Read blocks with retry mechanism for improved reliability
    pub fn read_blocks_with_retry(&self, block_count: u32, buffer: &mut [u8], max_retries: u32) -> Result<u32> {
        debug!("Reading {} blocks with retry (max {} retries)", block_count, max_retries);
        
        let mut last_error = None;
        
        for retry in 0..=max_retries {
            if retry > 0 {
                warn!("Retrying block read, attempt {} of {}", retry + 1, max_retries + 1);
                
                // Progressive backoff delay
                let delay_ms = std::cmp::min(1000 * retry, 5000); // Max 5 second delay
                std::thread::sleep(std::time::Duration::from_millis(delay_ms as u64));
                
                // Try to recover tape position on retry
                if let Err(e) = self.recover_tape_position() {
                    debug!("Position recovery failed: {}", e);
                }
            }
            
            match self.read_blocks(block_count, buffer) {
                Ok(result) => {
                    if retry > 0 {
                        info!("Block read succeeded on retry {}", retry);
                    }
                    return Ok(result);
                }
                Err(e) => {
                    last_error = Some(e);
                    debug!("Block read attempt {} failed: {:?}", retry + 1, last_error);
                }
            }
        }
        
        Err(last_error.unwrap_or_else(|| {
            crate::error::RustLtfsError::scsi("All retry attempts failed")
        }))
    }
    
    /// Attempt to recover tape position after read error
    fn recover_tape_position(&self) -> Result<()> {
        debug!("Attempting tape position recovery");
        
        // Try to read current position
        match self.read_position() {
            Ok(pos) => {
                debug!("Current position: partition {}, block {}", pos.partition, pos.block_number);
                
                // If we can read position, try a small test read
                let mut test_buffer = vec![0u8; block_sizes::LTO_BLOCK_SIZE as usize];
                match self.read_blocks_direct(1, &mut test_buffer) {
                    Ok(_) => {
                        debug!("Position recovery successful - test read OK");
                        Ok(())
                    }
                    Err(e) => {
                        debug!("Position recovery failed - test read failed: {}", e);
                        Err(e)
                    }
                }
            }
            Err(e) => {
                debug!("Position recovery failed - cannot read position: {}", e);
                Err(e)
            }
        }
    }
    
    /// Write tape blocks (based on LTFSCopyGUI implementation)  
    pub fn write_blocks(&self, block_count: u32, buffer: &[u8]) -> Result<u32> {
        debug!("Writing {} blocks to tape", block_count);
        
        if buffer.len() < (block_count * block_sizes::LTO_BLOCK_SIZE) as usize {
            return Err(crate::error::RustLtfsError::scsi(
                "Buffer too small for requested block count"
            ));
        }
        
        #[cfg(windows)]
        {
            let mut cdb = [0u8; 6];
            cdb[0] = scsi_commands::WRITE_6;
            
            // Fixed block mode (MSB=1), transfer length in blocks
            cdb[1] = 0x01; // Fixed block mode
            cdb[2] = ((block_count >> 16) & 0xFF) as u8;
            cdb[3] = ((block_count >> 8) & 0xFF) as u8;
            cdb[4] = (block_count & 0xFF) as u8;
            // cdb[5] is control byte, leave as 0
            
            let data_length = (block_count * block_sizes::LTO_BLOCK_SIZE) as usize;
            let result = self.scsi_io_control(
                &cdb,
                Some(&mut buffer[..data_length].to_vec().as_mut_slice()),
                SCSI_IOCTL_DATA_OUT,
                600, // 10 minute timeout for write operations
                None,
            )?;
            
            if result {
                debug!("Successfully wrote {} blocks", block_count);
                Ok(block_count)
            } else {
                Err(crate::error::RustLtfsError::scsi("Block write operation failed"))
            }
        }
        
        #[cfg(not(windows))]
        {
            let _ = (block_count, buffer);
            Err(crate::error::RustLtfsError::unsupported("Non-Windows platform"))
        }
    }
    
    /// Position tape to specific block (based on SCSI LOCATE command)
    pub fn locate_block(&self, partition: u8, block_number: u64) -> Result<()> {
        debug!("Locating to partition {} block {}", partition, block_number);
        
        #[cfg(windows)]
        {
            let mut cdb = [0u8; 10];
            cdb[0] = scsi_commands::LOCATE;
            
            // BT=1 (Block address type), CP=0 (Change partition if different)
            cdb[1] = 0x02; // Block address type
            if partition != 0 {
                cdb[1] |= 0x01; // Change partition flag
                cdb[3] = partition;
            }
            
            // Block address (64-bit, but only use lower 32 bits for now)
            cdb[4] = ((block_number >> 24) & 0xFF) as u8;
            cdb[5] = ((block_number >> 16) & 0xFF) as u8;
            cdb[6] = ((block_number >> 8) & 0xFF) as u8;
            cdb[7] = (block_number & 0xFF) as u8;
            
            let result = self.scsi_io_control(
                &cdb,
                None,
                SCSI_IOCTL_DATA_UNSPECIFIED,
                600, // 10 minute timeout for positioning
                None,
            )?;
            
            if result {
                debug!("Successfully positioned to partition {} block {}", partition, block_number);
                Ok(())
            } else {
                Err(crate::error::RustLtfsError::scsi("Locate operation failed"))
            }
        }
        
        #[cfg(not(windows))]
        {
            let _ = (partition, block_number);
            Err(crate::error::RustLtfsError::unsupported("Non-Windows platform"))
        }
    }
    
    /// Space operation (move by specified count of objects)
    pub fn space(&self, space_type: SpaceType, count: i32) -> Result<()> {
        debug!("Space operation: type={:?}, count={}", space_type, count);
        
        #[cfg(windows)]
        {
            let mut cdb = [0u8; 6];
            cdb[0] = scsi_commands::SPACE;
            cdb[1] = space_type as u8;
            
            // Handle negative counts (reverse direction)
            let abs_count = count.abs() as u32;
            if count < 0 {
                // Two's complement for negative values
                let neg_count = (!abs_count + 1) & 0xFFFFFF; // 24-bit field
                cdb[2] = ((neg_count >> 16) & 0xFF) as u8;
                cdb[3] = ((neg_count >> 8) & 0xFF) as u8;
                cdb[4] = (neg_count & 0xFF) as u8;
            } else {
                cdb[2] = ((abs_count >> 16) & 0xFF) as u8;
                cdb[3] = ((abs_count >> 8) & 0xFF) as u8;
                cdb[4] = (abs_count & 0xFF) as u8;
            }
            
            let result = self.scsi_io_control(
                &cdb,
                None,
                SCSI_IOCTL_DATA_UNSPECIFIED,
                600, // 10 minute timeout for space operations
                None,
            )?;
            
            if result {
                debug!("Space operation completed successfully");
                Ok(())
            } else {
                Err(crate::error::RustLtfsError::scsi("Space operation failed"))
            }
        }
        
        #[cfg(not(windows))]
        {
            let _ = (space_type, count);
            Err(crate::error::RustLtfsError::unsupported("Non-Windows platform"))
        }
    }
    
    /// Read tape position information
    pub fn read_position(&self) -> Result<TapePosition> {
        debug!("Reading tape position");
        
        #[cfg(windows)]
        {
            let mut cdb = [0u8; 10];
            let mut data_buffer = [0u8; 32];
            
            cdb[0] = scsi_commands::READ_POSITION;
            cdb[1] = 0x00; // Short form
            
            let result = self.scsi_io_control(
                &cdb,
                Some(&mut data_buffer),
                SCSI_IOCTL_DATA_IN,
                300,
                None,
            )?;
            
            if result {
                // Parse position data according to SCSI standards
                let flags = data_buffer[0];
                let partition = data_buffer[1];
                
                // Block number (32-bit in short form)
                let block_number = ((data_buffer[4] as u64) << 24) |
                                 ((data_buffer[5] as u64) << 16) |
                                 ((data_buffer[6] as u64) << 8) |
                                 (data_buffer[7] as u64);
                
                // File number (32-bit)
                let file_number = ((data_buffer[8] as u64) << 24) |
                                ((data_buffer[9] as u64) << 16) |
                                ((data_buffer[10] as u64) << 8) |
                                (data_buffer[11] as u64);
                
                let position = TapePosition {
                    partition,
                    block_number,
                    file_number,
                    set_number: 0, // Not available in short form
                    end_of_data: (flags & 0x04) != 0,
                    beginning_of_partition: (flags & 0x08) != 0,
                };
                
                debug!("Current position: partition={}, block={}, file={}", 
                    position.partition, position.block_number, position.file_number);
                
                Ok(position)
            } else {
                Err(crate::error::RustLtfsError::scsi("Read position failed"))
            }
        }
        
        #[cfg(not(windows))]
        {
            Err(crate::error::RustLtfsError::unsupported("Non-Windows platform"))
        }
    }
    
    /// Read MAM (Medium Auxiliary Memory) attributes for capacity information
    pub fn read_mam_attribute(&self, attribute_id: u16) -> Result<MamAttribute> {
        debug!("Reading MAM attribute ID: 0x{:04X}", attribute_id);
        
        #[cfg(windows)]
        {
            let mut cdb = [0u8; 16];
            let mut data_buffer = [0u8; 512]; // Sufficient for most MAM attributes
            
            cdb[0] = scsi_commands::READ_ATTRIBUTE;
            cdb[1] = 0x00; // Service action = 0 (VALUES)
            cdb[7] = 0x01; // Restrict to single attribute
            
            // Attribute ID
            cdb[8] = ((attribute_id >> 8) & 0xFF) as u8;
            cdb[9] = (attribute_id & 0xFF) as u8;
            
            // Allocation length
            let alloc_length = data_buffer.len() as u32;
            cdb[10] = ((alloc_length >> 24) & 0xFF) as u8;
            cdb[11] = ((alloc_length >> 16) & 0xFF) as u8;
            cdb[12] = ((alloc_length >> 8) & 0xFF) as u8;
            cdb[13] = (alloc_length & 0xFF) as u8;
            
            let result = self.scsi_io_control(
                &cdb,
                Some(&mut data_buffer),
                SCSI_IOCTL_DATA_IN,
                300,
                None,
            )?;
            
            if result {
                // Parse MAM attribute header
                if data_buffer.len() < 4 {
                    return Err(crate::error::RustLtfsError::scsi("Invalid MAM response"));
                }
                
                // Skip header, find attribute
                let data_length = ((data_buffer[0] as u32) << 24) |
                                ((data_buffer[1] as u32) << 16) |
                                ((data_buffer[2] as u32) << 8) |
                                (data_buffer[3] as u32);
                
                if data_length < 5 {
                    return Err(crate::error::RustLtfsError::scsi("No MAM attributes found"));
                }
                
                // Parse first attribute (simplified - assumes single attribute response)
                let attr_id = ((data_buffer[4] as u16) << 8) | (data_buffer[5] as u16);
                let attr_format = data_buffer[6];
                let attr_length = ((data_buffer[7] as u16) << 8) | (data_buffer[8] as u16);
                
                if attr_id != attribute_id {
                    return Err(crate::error::RustLtfsError::scsi("Unexpected attribute ID in response"));
                }
                
                let mut attr_data = Vec::new();
                if attr_length > 0 && data_buffer.len() >= (9 + attr_length as usize) {
                    attr_data.extend_from_slice(&data_buffer[9..9 + attr_length as usize]);
                }
                
                let attribute = MamAttribute {
                    attribute_id: attr_id,
                    attribute_format: attr_format,
                    length: attr_length,
                    data: attr_data,
                };
                
                debug!("Read MAM attribute: ID=0x{:04X}, format={}, length={}", 
                    attribute.attribute_id, attribute.attribute_format, attribute.length);
                
                Ok(attribute)
            } else {
                Err(crate::error::RustLtfsError::scsi("Read MAM attribute failed"))
            }
        }
        
        #[cfg(not(windows))]
        {
            let _ = attribute_id;
            Err(crate::error::RustLtfsError::unsupported("Non-Windows platform"))
        }
    }
    
    /// Write file mark (end of file marker)
    pub fn write_filemarks(&self, count: u32) -> Result<()> {
        debug!("Writing {} filemarks", count);
        
        #[cfg(windows)]
        {
            let mut cdb = [0u8; 6];
            cdb[0] = 0x10; // WRITE_FILEMARKS
            cdb[1] = 0x01; // Immediate bit set for better performance
            
            // Transfer length (number of filemarks)
            cdb[2] = ((count >> 16) & 0xFF) as u8;
            cdb[3] = ((count >> 8) & 0xFF) as u8;
            cdb[4] = (count & 0xFF) as u8;
            
            let result = self.scsi_io_control(
                &cdb,
                None,
                SCSI_IOCTL_DATA_UNSPECIFIED,
                300,
                None,
            )?;
            
            if result {
                debug!("Successfully wrote {} filemarks", count);
                Ok(())
            } else {
                Err(crate::error::RustLtfsError::scsi("Write filemarks failed"))
            }
        }
        
        #[cfg(not(windows))]
        {
            let _ = count;
            Err(crate::error::RustLtfsError::unsupported("Non-Windows platform"))
        }
    }
    
    /// Comprehensive locate method (based on LTFSCopyGUI TapeUtils.Locate)
    /// Supports block, file mark, and EOD positioning with drive-specific optimizations
    pub fn locate(&self, block_address: u64, partition: u8, dest_type: LocateDestType) -> Result<u16> {
        debug!("Locating to partition {} {} {:?} {}", 
               partition, 
               match dest_type {
                   LocateDestType::Block => "block",
                   LocateDestType::FileMark => "filemark",
                   LocateDestType::EOD => "EOD",
               },
               dest_type,
               block_address);
        
        #[cfg(windows)]
        {
            let mut sense_buffer = [0u8; SENSE_INFO_LEN];
            
            // Execute locate command based on drive type
            match self.drive_type {
                DriveType::M2488 => {
                    // M2488 specific implementation (placeholder)
                    warn!("M2488 drive type not fully implemented");
                    self.locate_standard(block_address, partition, dest_type, &mut sense_buffer)
                }
                DriveType::SLR3 => {
                    self.locate_slr3(block_address, partition, dest_type, &mut sense_buffer)
                }
                DriveType::SLR1 => {
                    self.locate_slr1(block_address, partition, dest_type, &mut sense_buffer)
                }
                DriveType::Standard => {
                    self.locate_standard(block_address, partition, dest_type, &mut sense_buffer)
                }
            }
        }
        
        #[cfg(not(windows))]
        {
            let _ = (block_address, partition, dest_type);
            Err(crate::error::RustLtfsError::unsupported("Non-Windows platform"))
        }
    }
    
    /// Standard/modern drive locate implementation
    #[cfg(windows)]
    fn locate_standard(&self, block_address: u64, partition: u8, dest_type: LocateDestType, sense_buffer: &mut [u8; SENSE_INFO_LEN]) -> Result<u16> {
        if self.allow_partition || dest_type != LocateDestType::Block {
            // Use LOCATE(16) command for modern drives with partition support
            let mut cp = 0u8;
            if let Ok(current_pos) = self.read_position() {
                if current_pos.partition != partition {
                    cp = 1; // Change partition flag
                }
            }
            
            let mut cdb = [0u8; 16];
            cdb[0] = 0x92; // LOCATE(16)
            cdb[1] = (dest_type as u8) << 3 | (cp << 1);
            cdb[2] = 0;
            cdb[3] = partition;
            
            // 64-bit block address
            cdb[4] = ((block_address >> 56) & 0xFF) as u8;
            cdb[5] = ((block_address >> 48) & 0xFF) as u8;
            cdb[6] = ((block_address >> 40) & 0xFF) as u8;
            cdb[7] = ((block_address >> 32) & 0xFF) as u8;
            cdb[8] = ((block_address >> 24) & 0xFF) as u8;
            cdb[9] = ((block_address >> 16) & 0xFF) as u8;
            cdb[10] = ((block_address >> 8) & 0xFF) as u8;
            cdb[11] = (block_address & 0xFF) as u8;
            
            self.execute_locate_command(&cdb, sense_buffer)
        } else {
            // Use LOCATE(10) for simple block positioning
            let mut cdb = [0u8; 10];
            cdb[0] = 0x2B; // LOCATE(10)
            cdb[1] = 0;
            cdb[2] = 0;
            cdb[3] = ((block_address >> 24) & 0xFF) as u8;
            cdb[4] = ((block_address >> 16) & 0xFF) as u8;
            cdb[5] = ((block_address >> 8) & 0xFF) as u8;
            cdb[6] = (block_address & 0xFF) as u8;
            cdb[7] = 0;
            cdb[8] = 0;
            cdb[9] = 0;
            
            self.execute_locate_command(&cdb, sense_buffer)
        }
    }
    
    /// SLR3 drive specific locate implementation
    #[cfg(windows)]
    fn locate_slr3(&self, block_address: u64, _partition: u8, dest_type: LocateDestType, sense_buffer: &mut [u8; SENSE_INFO_LEN]) -> Result<u16> {
        match dest_type {
            LocateDestType::Block => {
                let mut cdb = [0u8; 10];
                cdb[0] = 0x2B; // LOCATE(10)
                cdb[1] = 4; // SLR3 specific flags
                cdb[2] = 0;
                cdb[3] = ((block_address >> 24) & 0xFF) as u8;
                cdb[4] = ((block_address >> 16) & 0xFF) as u8;
                cdb[5] = ((block_address >> 8) & 0xFF) as u8;
                cdb[6] = (block_address & 0xFF) as u8;
                cdb[7] = 0;
                cdb[8] = 0;
                cdb[9] = 0;
                
                self.execute_locate_command(&cdb, sense_buffer)
            }
            LocateDestType::FileMark => {
                // First locate to beginning, then space to filemark
                self.locate(0, 0, LocateDestType::Block)?;
                self.space(SpaceType::FileMarks, block_address as i32)?;
                Ok(0)
            }
            LocateDestType::EOD => {
                if let Ok(pos) = self.read_position() {
                    if !pos.end_of_data {
                        let mut cdb = [0u8; 6];
                        cdb[0] = 0x11; // SPACE
                        cdb[1] = 3; // EOD
                        cdb[2] = 0;
                        cdb[3] = 0;
                        cdb[4] = 0;
                        cdb[5] = 0;
                        
                        self.execute_locate_command(&cdb, sense_buffer)
                    } else {
                        Ok(0)
                    }
                } else {
                    Err(crate::error::RustLtfsError::scsi("Cannot read position for EOD locate"))
                }
            }
        }
    }
    
    /// SLR1 drive specific locate implementation
    #[cfg(windows)]
    fn locate_slr1(&self, block_address: u64, _partition: u8, dest_type: LocateDestType, sense_buffer: &mut [u8; SENSE_INFO_LEN]) -> Result<u16> {
        match dest_type {
            LocateDestType::Block => {
                let mut cdb = [0u8; 6];
                cdb[0] = 0x0C; // SLR1 specific locate command
                cdb[1] = 0;
                cdb[2] = ((block_address >> 16) & 0x0F) as u8; // Only 20-bit address
                cdb[3] = ((block_address >> 8) & 0xFF) as u8;
                cdb[4] = (block_address & 0xFF) as u8;
                cdb[5] = 0;
                
                self.execute_locate_command(&cdb, sense_buffer)
            }
            LocateDestType::FileMark => {
                // First locate to beginning, then space to filemark
                self.locate(0, 0, LocateDestType::Block)?;
                self.space(SpaceType::FileMarks, block_address as i32)?;
                Ok(0)
            }
            LocateDestType::EOD => {
                if let Ok(pos) = self.read_position() {
                    if !pos.end_of_data {
                        let mut cdb = [0u8; 6];
                        cdb[0] = 0x11; // SPACE
                        cdb[1] = 3; // EOD
                        cdb[2] = 0;
                        cdb[3] = 0;
                        cdb[4] = 0;
                        cdb[5] = 0;
                        
                        self.execute_locate_command(&cdb, sense_buffer)
                    } else {
                        Ok(0)
                    }
                } else {
                    Err(crate::error::RustLtfsError::scsi("Cannot read position for EOD locate"))
                }
            }
        }
    }
    
    /// Execute locate command and handle errors (based on LTFSCopyGUI error handling)
    #[cfg(windows)]
    fn execute_locate_command(&self, cdb: &[u8], sense_buffer: &mut [u8; SENSE_INFO_LEN]) -> Result<u16> {
        let result = self.scsi_io_control(
            cdb,
            None,
            SCSI_IOCTL_DATA_UNSPECIFIED,
            600, // 10 minute timeout for positioning
            Some(sense_buffer),
        )?;
        
        if !result {
            return Err(crate::error::RustLtfsError::scsi("Locate command failed"));
        }
        
        // Parse sense data for additional status code (ASC/ASCQ)
        let asc_ascq = ((sense_buffer[12] as u16) << 8) | (sense_buffer[13] as u16);
        
        if asc_ascq != 0 && (sense_buffer[2] & 0x0F) != 8 {
            // Error occurred, attempt recovery based on LTFSCopyGUI logic
            warn!("Locate command returned error: ASC/ASCQ = 0x{:04X}", asc_ascq);
            
            // Retry with different strategy if first attempt failed
            self.retry_locate_on_error(cdb, sense_buffer, asc_ascq)
        } else {
            debug!("Locate command completed successfully");
            Ok(0)
        }
    }
    
    /// Retry locate operation on error (based on LTFSCopyGUI retry logic)
    #[cfg(windows)]
    fn retry_locate_on_error(&self, original_cdb: &[u8], sense_buffer: &mut [u8; SENSE_INFO_LEN], error_code: u16) -> Result<u16> {
        debug!("Attempting locate retry for error code: 0x{:04X}", error_code);
        
        // Parse original command to determine retry strategy
        let original_command = original_cdb[0];
        
        match original_command {
            0x92 => { // LOCATE(16) failed, try LOCATE(10)
                if original_cdb.len() >= 12 {
                    let block_address = ((original_cdb[8] as u64) << 24) |
                                       ((original_cdb[9] as u64) << 16) |
                                       ((original_cdb[10] as u64) << 8) |
                                       (original_cdb[11] as u64);
                    
                    let mut retry_cdb = [0u8; 10];
                    retry_cdb[0] = 0x2B; // LOCATE(10)
                    retry_cdb[1] = (original_cdb[1] & 0x07) << 3; // Preserve destination type
                    retry_cdb[2] = 0;
                    retry_cdb[3] = ((block_address >> 24) & 0xFF) as u8;
                    retry_cdb[4] = ((block_address >> 16) & 0xFF) as u8;
                    retry_cdb[5] = ((block_address >> 8) & 0xFF) as u8;
                    retry_cdb[6] = (block_address & 0xFF) as u8;
                    retry_cdb[7] = 0;
                    retry_cdb[8] = 0;
                    retry_cdb[9] = 0;
                    
                    debug!("Retrying with LOCATE(10) command");
                    
                    let result = self.scsi_io_control(
                        &retry_cdb,
                        None,
                        SCSI_IOCTL_DATA_UNSPECIFIED,
                        600,
                        Some(sense_buffer),
                    )?;
                    
                    if result {
                        let retry_asc_ascq = ((sense_buffer[12] as u16) << 8) | (sense_buffer[13] as u16);
                        debug!("Retry result: ASC/ASCQ = 0x{:04X}", retry_asc_ascq);
                        Ok(retry_asc_ascq)
                    } else {
                        Err(crate::error::RustLtfsError::scsi("Locate retry also failed"))
                    }
                } else {
                    Err(crate::error::RustLtfsError::scsi("Invalid CDB for retry"))
                }
            }
            _ => {
                // For other commands, return the original error
                Err(crate::error::RustLtfsError::scsi(
                    format!("Locate operation failed with ASC/ASCQ: 0x{:04X}", error_code)
                ))
            }
        }
    }
    
    /// Convenience method: locate to file mark
    pub fn locate_to_filemark(&self, filemark_number: u64, partition: u8) -> Result<()> {
        self.locate(filemark_number, partition, LocateDestType::FileMark)?;
        Ok(())
    }
    
    /// Convenience method: locate to end of data  
    pub fn locate_to_eod(&self, partition: u8) -> Result<()> {
        self.locate(0, partition, LocateDestType::EOD)?;
        Ok(())
    }
    
    /// Enhanced locate_block method that uses the comprehensive locate function
    pub fn locate_block_enhanced(&self, partition: u8, block_number: u64) -> Result<()> {
        let error_code = self.locate(block_number, partition, LocateDestType::Block)?;
        if error_code == 0 {
            debug!("Successfully positioned to partition {} block {}", partition, block_number);
            Ok(())
        } else {
            Err(crate::error::RustLtfsError::scsi(
                format!("Locate operation completed with warning: 0x{:04X}", error_code)
            ))
        }
    }
    
    /// Set drive type for optimization
    pub fn set_drive_type(&mut self, drive_type: DriveType) {
        self.drive_type = drive_type;
        debug!("Drive type set to: {:?}", drive_type);
    }
    
    /// Set partition support flag
    pub fn set_allow_partition(&mut self, allow: bool) {
        self.allow_partition = allow;
        debug!("Partition support set to: {}", allow);
    }
    
    /// Get current drive type
    pub fn get_drive_type(&self) -> DriveType {
        self.drive_type
    }

    /// MODE SENSE command to read partition page 0x11 (对应LTFSCopyGUI的ModeSense实现)
    /// 这个方法复制LTFSCopyGUI的精确实现：TapeUtils.ModeSense(handle, &H11)
    pub fn mode_sense_partition_page_0x11(&self) -> Result<Vec<u8>> {
        debug!("Executing MODE SENSE page 0x11 for partition detection (LTFSCopyGUI compatible)");
        
        #[cfg(windows)]
        {
            // 第一步：获取页面头信息（对应LTFSCopyGUI的Header读取）
            let mut header_cdb = [0u8; 6];
            header_cdb[0] = 0x1A; // MODE SENSE 6命令
            header_cdb[1] = 0x00; // Reserved
            header_cdb[2] = 0x11; // Page 0x11 (分区模式页)
            header_cdb[3] = 0x00; // Reserved  
            header_cdb[4] = 4;    // Allocation Length = 4 bytes
            header_cdb[5] = 0x00; // Control
            
            let mut header_buffer = vec![0u8; 4];
            let mut sense_buffer = [0u8; SENSE_INFO_LEN];
            
            let result = self.scsi_io_control(
                &header_cdb,
                Some(&mut header_buffer),
                SCSI_IOCTL_DATA_IN,
                30, // 30 second timeout
                Some(&mut sense_buffer),
            )?;
            
            if !result {
                let sense_info = self.parse_sense_data(&sense_buffer);
                return Err(crate::error::RustLtfsError::scsi(format!("MODE SENSE header failed: {}", sense_info)));
            }
            
            if header_buffer.len() == 0 {
                return Ok(vec![0, 0, 0, 0]);
            }
            
            let page_len = header_buffer[0] as usize;
            if page_len == 0 {
                return Ok(vec![0, 0, 0, 0]);
            }
            
            let descriptor_len = header_buffer[3] as usize;
            
            // 第二步：读取完整页面数据
            let mut full_cdb = [0u8; 6];
            full_cdb[0] = 0x1A; // MODE SENSE 6命令
            full_cdb[1] = 0x00; // Reserved
            full_cdb[2] = 0x11; // Page 0x11
            full_cdb[3] = 0x00; // Reserved
            full_cdb[4] = (page_len + 1) as u8; // Allocation Length
            full_cdb[5] = 0x00; // Control
            
            let mut full_buffer = vec![0u8; page_len + 1];
            let mut full_sense_buffer = [0u8; SENSE_INFO_LEN];
            
            let full_result = self.scsi_io_control(
                &full_cdb,
                Some(&mut full_buffer),
                SCSI_IOCTL_DATA_IN,
                30, // 30 second timeout
                Some(&mut full_sense_buffer),
            )?;
            
            if full_result {
                // 跳过header和descriptor，返回页面数据（对应LTFSCopyGUI的SkipHeader逻辑）
                let skip_bytes = 4 + descriptor_len;
                if full_buffer.len() > skip_bytes {
                    let page_data = full_buffer[skip_bytes..].to_vec();
                    debug!("MODE SENSE page 0x11 successful, returned {} bytes (after skipping {} header bytes)", 
                          page_data.len(), skip_bytes);
                    Ok(page_data)
                } else {
                    debug!("MODE SENSE page 0x11 data too short after header skip");
                    Ok(full_buffer)
                }
            } else {
                let sense_info = self.parse_sense_data(&full_sense_buffer);
                Err(crate::error::RustLtfsError::scsi(format!("MODE SENSE page 0x11 failed: {}", sense_info)))
            }
        }
        
        #[cfg(not(windows))]
        {
            Err(crate::error::RustLtfsError::unsupported("Non-Windows platform"))
        }
    }

    /// MODE SENSE command to read partition table (对应LTFSCopyGUI的分区检测)
    pub fn mode_sense_partition_info(&self) -> Result<Vec<u8>> {
        debug!("Executing MODE SENSE command for partition information");
        
        #[cfg(windows)]
        {
            let mut cdb = [0u8; 10];
            cdb[0] = SCSIOP_MODE_SENSE10;
            cdb[1] = 0x00; // Reserved
            cdb[2] = TC_MP_MEDIUM_CONFIGURATION | TC_MP_PC_CURRENT; // Page Code + PC
            cdb[3] = 0x00; // Subpage Code
            cdb[7] = 0x01; // Allocation Length (high byte)
            cdb[8] = 0x00; // Allocation Length (low byte) - 256 bytes
            
            let mut data_buffer = vec![0u8; 256];
            let mut sense_buffer = [0u8; SENSE_INFO_LEN];
            
            let result = self.scsi_io_control(
                &cdb,
                Some(&mut data_buffer),
                SCSI_IOCTL_DATA_IN,
                30, // 30 second timeout
                Some(&mut sense_buffer),
            )?;
            
            if result {
                debug!("MODE SENSE completed successfully, {} bytes returned", data_buffer.len());
                Ok(data_buffer)
            } else {
                let sense_info = self.parse_sense_data(&sense_buffer);
                Err(crate::error::RustLtfsError::scsi(format!("MODE SENSE failed: {}", sense_info)))
            }
        }
        
        #[cfg(not(windows))]
        {
            Err(crate::error::RustLtfsError::unsupported("Non-Windows platform"))
        }
    }

    /// READ POSITION command to get raw position data
    pub fn read_position_raw(&self) -> Result<Vec<u8>> {
        debug!("Executing READ POSITION command");
        
        #[cfg(windows)]
        {
            let mut cdb = [0u8; 10];
            cdb[0] = SCSIOP_READ_POSITION;
            cdb[1] = 0x06; // Service Action: Long form
            cdb[8] = 0x20; // Allocation Length: 32 bytes
            
            let mut data_buffer = vec![0u8; 32];
            let mut sense_buffer = [0u8; SENSE_INFO_LEN];
            
            let result = self.scsi_io_control(
                &cdb,
                Some(&mut data_buffer),
                SCSI_IOCTL_DATA_IN,
                30, // 30 second timeout
                Some(&mut sense_buffer),
            )?;
            
            if result {
                debug!("READ POSITION completed successfully");
                Ok(data_buffer)
            } else {
                let sense_info = self.parse_sense_data(&sense_buffer);
                Err(crate::error::RustLtfsError::scsi(format!("READ POSITION failed: {}", sense_info)))
            }
        }
        
        #[cfg(not(windows))]
        {
            Err(crate::error::RustLtfsError::unsupported("Non-Windows platform"))
        }
    }

    /// 解析MODE SENSE返回的分区信息
    pub fn parse_partition_info(&self, mode_sense_data: &[u8]) -> Result<(u64, u64)> {
        if mode_sense_data.len() < 8 {
            return Err(crate::error::RustLtfsError::scsi("MODE SENSE data too short".to_string()));
        }
        
        // 解析MODE SENSE返回的数据结构
        // 这需要根据SCSI标准和LTO设备规范来解析
        
        // Mode Parameter Header (8 bytes)
        let mode_data_length = u16::from_be_bytes([mode_sense_data[0], mode_sense_data[1]]);
        debug!("Mode data length: {}", mode_data_length);
        
        if mode_data_length < 8 || mode_sense_data.len() < (mode_data_length as usize + 2) {
            return Err(crate::error::RustLtfsError::scsi("Invalid MODE SENSE response".to_string()));
        }
        
        // 查找Medium Configuration Mode Page (0x1D)
        let mut offset = 8; // Skip mode parameter header
        while offset < mode_sense_data.len() - 1 {
            let page_code = mode_sense_data[offset] & 0x3F;
            let page_length = mode_sense_data[offset + 1] as usize;
            
            if page_code == TC_MP_MEDIUM_CONFIGURATION {
                debug!("Found Medium Configuration Mode Page at offset {}", offset);
                
                if offset + page_length + 2 <= mode_sense_data.len() {
                    return self.parse_medium_configuration_page(&mode_sense_data[offset..offset + page_length + 2]);
                } else {
                    return Err(crate::error::RustLtfsError::scsi("Medium Configuration Page truncated".to_string()));
                }
            }
            
            offset += page_length + 2;
        }
        
        Err(crate::error::RustLtfsError::scsi("Medium Configuration Mode Page not found".to_string()))
    }
    
    /// 解析Medium Configuration Mode Page获取分区大小
    fn parse_medium_configuration_page(&self, page_data: &[u8]) -> Result<(u64, u64)> {
        if page_data.len() < 16 {
            return Err(crate::error::RustLtfsError::scsi("Medium Configuration Page too short".to_string()));
        }
        
        // Medium Configuration Page格式 (根据SCSI标准)
        // Byte 2-3: Active Partition
        // Byte 4: Medium Format Recognition
        // Byte 8-15: Partition Size (Partition 0)
        // Byte 16-23: Partition Size (Partition 1)
        
        let active_partition = u16::from_be_bytes([page_data[2], page_data[3]]);
        debug!("Active partition: {}", active_partition);
        
        if page_data.len() >= 24 {
            // 读取分区0大小 (8字节，大端序)
            let partition_0_size = u64::from_be_bytes([
                page_data[8], page_data[9], page_data[10], page_data[11],
                page_data[12], page_data[13], page_data[14], page_data[15]
            ]);
            
            // 读取分区1大小 (8字节，大端序)
            let partition_1_size = u64::from_be_bytes([
                page_data[16], page_data[17], page_data[18], page_data[19],
                page_data[20], page_data[21], page_data[22], page_data[23]
            ]);
            
            info!("Parsed partition sizes: p0={}MB, p1={}MB", 
                 partition_0_size / 1_048_576, partition_1_size / 1_048_576);
            
            Ok((partition_0_size, partition_1_size))
        } else {
            // 如果数据不够，返回估算值
            debug!("Insufficient data for partition sizes, using estimation");
            Err(crate::error::RustLtfsError::scsi("Insufficient data for partition size parsing".to_string()))
        }
    }
}

/// Convenience function: Directly check media status of specified device
pub fn check_tape_media(tape_drive: &str) -> Result<MediaType> {
    let mut scsi = ScsiInterface::new();
    scsi.open_device(tape_drive)?;
    scsi.check_media_status()
}

/// Convenience function: Directly load tape of specified device
pub fn load_tape(tape_drive: &str) -> Result<bool> {
    let mut scsi = ScsiInterface::new();
    scsi.open_device(tape_drive)?;
    scsi.load_tape()
}

/// Convenience function: Directly eject tape of specified device  
pub fn eject_tape(tape_drive: &str) -> Result<bool> {
    let mut scsi = ScsiInterface::new();
    scsi.open_device(tape_drive)?;
    scsi.eject_tape()
}

/// Convenience function: Locate to specific block (corresponding to LTFSCopyGUI overloads)
pub fn locate_block(tape_drive: &str, block_address: u64, partition: u8) -> Result<u16> {
    let mut scsi = ScsiInterface::new();
    scsi.open_device(tape_drive)?;
    scsi.locate(block_address, partition, LocateDestType::Block)
}

/// Convenience function: Locate with destination type
pub fn locate_with_type(tape_drive: &str, block_address: u64, partition: u8, dest_type: LocateDestType) -> Result<u16> {
    let mut scsi = ScsiInterface::new();
    scsi.open_device(tape_drive)?;
    scsi.locate(block_address, partition, dest_type)
}

/// Convenience function: Locate with drive type optimization
pub fn locate_with_drive_type(tape_drive: &str, block_address: u64, partition: u8, dest_type: LocateDestType, drive_type: DriveType) -> Result<u16> {
    let mut scsi = ScsiInterface::with_drive_type(drive_type);
    scsi.open_device(tape_drive)?;
    scsi.locate(block_address, partition, dest_type)
}

/// Convenience function: Locate to file mark
pub fn locate_to_filemark(tape_drive: &str, filemark_number: u64, partition: u8) -> Result<u16> {
    let mut scsi = ScsiInterface::new();
    scsi.open_device(tape_drive)?;
    scsi.locate(filemark_number, partition, LocateDestType::FileMark)
}

/// Convenience function: Locate to end of data
pub fn locate_to_eod(tape_drive: &str, partition: u8) -> Result<u16> {
    let mut scsi = ScsiInterface::new();
    scsi.open_device(tape_drive)?;
    scsi.locate(0, partition, LocateDestType::EOD)
}

/// Implement Drop trait to ensure device handle is properly closed
impl Drop for DeviceHandle {
    fn drop(&mut self) {
        #[cfg(windows)]
        unsafe {
            if self.handle != INVALID_HANDLE_VALUE {
                CloseHandle(self.handle);
                debug!("Device handle closed: {}", self.device_path);
            }
        }
    }
}

/// Implement Drop trait to ensure SCSI interface is properly cleaned up
impl Drop for ScsiInterface {
    fn drop(&mut self) {
        debug!("SCSI interface cleanup completed");
    }
}

/// SCSI command constant definitions (based on C code)
pub mod scsi_commands {
    pub const TEST_UNIT_READY: u8 = 0x00;
    pub const INQUIRY: u8 = 0x12;
    pub const READ_6: u8 = 0x08;
    pub const WRITE_6: u8 = 0x0A;
    pub const READ_10: u8 = 0x28;
    pub const WRITE_10: u8 = 0x2A;
    pub const MODE_SENSE_6: u8 = 0x1A;
    pub const MODE_SENSE_10: u8 = 0x5A;
    pub const MODE_SELECT_6: u8 = 0x15;
    pub const LOAD_UNLOAD: u8 = 0x1B;
    pub const REWIND: u8 = 0x01;
    pub const READ_POSITION: u8 = 0x34;
    pub const SPACE: u8 = 0x11;
    pub const START_STOP_UNIT: u8 = 0x1B;
    pub const READ_ATTRIBUTE: u8 = 0x8C;
    pub const WRITE_ATTRIBUTE: u8 = 0x8D;
    pub const LOCATE: u8 = 0x2B;
    pub const SEEK: u8 = 0x2B;
}

/// Tape position information structure
#[derive(Debug, Clone)]
pub struct TapePosition {
    pub partition: u8,
    pub block_number: u64,
    pub file_number: u64,
    pub set_number: u64,
    pub end_of_data: bool,
    pub beginning_of_partition: bool,
}

/// MAM (Medium Auxiliary Memory) attribute structure
#[derive(Debug, Clone)]
pub struct MamAttribute {
    pub attribute_id: u16,
    pub attribute_format: u8,
    pub length: u16,
    pub data: Vec<u8>,
}

/// Space types for SPACE command
#[derive(Debug, Clone, Copy)]
pub enum SpaceType {
    Blocks = 0,
    FileMarks = 1,
    SequentialFileMarks = 2,
    EndOfData = 3,
}

/// Locate destination types (corresponding to LTFSCopyGUI LocateDestType)
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum LocateDestType {
    /// Locate to specific block number
    Block = 0,
    /// Locate to file mark
    FileMark = 1,
    /// Locate to end of data
    EOD = 3,
}

/// Drive type enumeration for specific driver optimizations
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum DriveType {
    /// Standard/Generic drive
    Standard,
    /// Legacy SLR3 drive type
    SLR3,
    /// Legacy SLR1 drive type
    SLR1,
    /// M2488 drive type
    M2488,
}

/// Block size constants for LTO tapes
pub mod block_sizes {
    pub const LTO_BLOCK_SIZE: u32 = 65536; // 64KB standard LTO block size
    pub const MIN_BLOCK_SIZE: u32 = 512;
    pub const MAX_BLOCK_SIZE: u32 = 1048576; // 1MB maximum
}

/// Helper function: Convert byte array to safe string
pub fn bytes_to_string(bytes: &[u8]) -> String {
    let end = bytes.iter().position(|&b| b == 0).unwrap_or(bytes.len());
    String::from_utf8_lossy(&bytes[..end]).trim().to_string()
}

impl ScsiInterface {
    /// Set tape capacity using SCSI Space command (对应LTFSCopyGUI的Set Capacity)
    pub fn set_capacity(&self, capacity: u16) -> Result<()> {
        debug!("Setting tape capacity to: {}", capacity);
        
        // SCSI Space command for capacity: 0B 00 00 (capacity >> 8) (capacity & 0xFF) 00
        let cdb = vec![
            0x0B, 0x00, 0x00, 
            ((capacity >> 8) & 0xFF) as u8,
            (capacity & 0xFF) as u8,
            0x00
        ];
        let mut buffer = vec![];
        
        match self.send_scsi_command(&cdb, &mut buffer, 0) {
            Ok(_) => {
                info!("✅ Tape capacity set to: {}", capacity);
                Ok(())
            }
            Err(e) => {
                warn!("❌ Failed to set capacity: {}", e);
                Err(RustLtfsError::scsi(format!("Failed to set capacity: {}", e)))
            }
        }
    }

    /// Format/Initialize tape using SCSI Format command (对应LTFSCopyGUI的Initialize操作)
    pub fn format_tape(&self, immediate_mode: bool) -> Result<()> {
        debug!("Formatting/initializing tape, immediate mode: {}", immediate_mode);
        
        // SCSI Format Unit command: 04 (immediate_flag) 00 00 00 00
        let immediate_flag = if immediate_mode { 0x02 } else { 0x00 };
        let cdb = vec![0x04, immediate_flag, 0x00, 0x00, 0x00, 0x00];
        let mut buffer = vec![];
        
        match self.send_scsi_command(&cdb, &mut buffer, 0) {
            Ok(_) => {
                info!("✅ Tape format/initialization started");
                Ok(())
            }
            Err(e) => {
                warn!("❌ Tape format failed: {}", e);
                Err(RustLtfsError::scsi(format!("Failed to format tape: {}", e)))
            }
        }
    }

    /// Partition tape using FORMAT command (对应LTFSCopyGUI的分区创建)
    pub fn partition_tape(&self, partition_type: u8) -> Result<()> {
        debug!("Creating tape partitions with type: {}", partition_type);
        
        // SCSI Format Unit command for partitioning: 04 00 (partition_type) 00 00 00
        // partition_type: 1 = standard partitioning, 2 = T10K partitioning
        let cdb = vec![0x04, 0x00, partition_type, 0x00, 0x00, 0x00];
        let mut buffer = vec![];
        
        match self.send_scsi_command(&cdb, &mut buffer, 0) {
            Ok(_) => {
                info!("✅ Tape partitioning completed with type: {}", partition_type);
                Ok(())
            }
            Err(e) => {
                warn!("❌ Tape partitioning failed: {}", e);
                Err(RustLtfsError::scsi(format!("Failed to partition tape: {}", e)))
            }
        }
    }

    /// Set MAM (Media Auxiliary Memory) attribute (对应LTFSCopyGUI的SetMAMAttribute)
    pub fn set_mam_attribute(&self, attribute_id: u16, data: &[u8], format: MamAttributeFormat) -> Result<()> {
        debug!("Setting MAM attribute 0x{:04X} with {} bytes", attribute_id, data.len());
        
        let data_len = data.len();
        let total_len = data_len + 5; // 5 bytes header + data
        
        // SCSI Write Attribute command: 8C 00 00 00 00 00 00 00 (id_high) (id_low) (len_high) (len_low) (len_3) (len_4) 00 00
        let cdb = vec![
            0x8C, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
            ((attribute_id >> 8) & 0xFF) as u8,
            (attribute_id & 0xFF) as u8,
            0x00, 0x00,
            ((total_len >> 8) & 0xFF) as u8,
            (total_len & 0xFF) as u8,
            0x00, 0x00
        ];
        
        // Prepare data buffer with attribute header
        let mut write_data = Vec::with_capacity(total_len);
        write_data.extend_from_slice(&[
            ((attribute_id >> 8) & 0xFF) as u8,  // Attribute ID high
            (attribute_id & 0xFF) as u8,         // Attribute ID low
            format as u8,                        // Format
            ((data_len >> 8) & 0xFF) as u8,      // Length high
            (data_len & 0xFF) as u8,             // Length low
        ]);
        write_data.extend_from_slice(data);
        
        match self.send_scsi_command(&cdb, &mut write_data, 0) {
            Ok(_) => {
                debug!("✅ MAM attribute 0x{:04X} set successfully", attribute_id);
                Ok(())
            }
            Err(e) => {
                warn!("❌ Failed to set MAM attribute 0x{:04X}: {}", attribute_id, e);
                Err(RustLtfsError::scsi(format!("Failed to set MAM attribute 0x{:04X}: {}", attribute_id, e)))
            }
        }
    }

    /// Set barcode using MAM attribute (对应LTFSCopyGUI的SetBarcode)
    pub fn set_barcode(&self, barcode: &str) -> Result<()> {
        debug!("Setting barcode: {}", barcode);
        
        // Barcode is stored in MAM attribute 0x806, padded to 32 bytes
        let mut barcode_data = vec![0u8; 32];
        let barcode_bytes = barcode.as_bytes();
        let copy_len = std::cmp::min(barcode_bytes.len(), 32);
        barcode_data[..copy_len].copy_from_slice(&barcode_bytes[..copy_len]);
        
        self.set_mam_attribute(0x806, &barcode_data, MamAttributeFormat::Text)
    }

    /// MODE SELECT command for partition configuration (对应LTFSCopyGUI的MODE SELECT 11h)
    pub fn mode_select_partition(&self, max_extra_partitions: u8, extra_partition_count: u8, 
                                mode_data: &[u8], p0_size: u16, p1_size: u16) -> Result<()> {
        debug!("Setting partition configuration: max_extra={}, extra_count={}, p0_size={}, p1_size={}", 
               max_extra_partitions, extra_partition_count, p0_size, p1_size);
        
        // SCSI MODE SELECT command: 15 10 00 00 10 00
        let cdb = vec![0x15, 0x10, 0x00, 0x00, 0x10, 0x00];
        
        // Prepare data for MODE SELECT (16 bytes total)
        let mut select_data = vec![
            0x00, 0x00, 0x10, 0x00,  // Mode data header
            0x11, 0x0A,              // Page code 0x11, page length 0x0A
            max_extra_partitions,    // Maximum allowed extra partitions
            extra_partition_count,   // Current extra partition count
        ];
        
        // Add original mode data bytes 4-7
        if mode_data.len() >= 8 {
            select_data.extend_from_slice(&mode_data[4..8]);
        } else {
            select_data.extend_from_slice(&[0x00, 0x00, 0x00, 0x00]);
        }
        
        // Add partition sizes (big-endian)
        select_data.push(((p0_size >> 8) & 0xFF) as u8);
        select_data.push((p0_size & 0xFF) as u8);
        select_data.push(((p1_size >> 8) & 0xFF) as u8);
        select_data.push((p1_size & 0xFF) as u8);
        
        match self.send_scsi_command(&cdb, &mut select_data, 0) {
            Ok(_) => {
                info!("✅ Partition configuration set successfully");
                Ok(())
            }
            Err(e) => {
                warn!("❌ Failed to set partition configuration: {}", e);
                Err(RustLtfsError::scsi(format!("Failed to set partition configuration: {}", e)))
            }
        }
    }
}

/// MAM attribute format types (对应LTFSCopyGUI的AttributeFormat)
#[derive(Debug, Clone, Copy, PartialEq)]
#[repr(u8)]
pub enum MamAttributeFormat {
    Binary = 0x00,
    Ascii = 0x01,
    Text = 0x02,
}
