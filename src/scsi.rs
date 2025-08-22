use crate::error::Result;
use tracing::{debug, warn};
use std::ffi::CString;

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
        }
    }
    
    /// Open tape device (based on CreateFile call in C code)
    pub fn open_device(&mut self, device_path: &str) -> Result<()> {
        debug!("Opening tape device: {}", device_path);
        
        #[cfg(windows)]
        {
            // Build complete device path, similar to "\\\\.\\TAPE0" format in C code
            let full_path = if device_path.starts_with("\\\\\\\\\\.\\\\") {
                device_path.to_string()
            } else {
                format!("\\\\\\\\\\.\\\\{}", device_path)
            };
            
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
            cdb[2] |= (TC_MP_PC_CURRENT << 6); // PC field
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
                media_type |= ((data_buffer[3] as u16 & 0x80) << 2);
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
    
    /// Read tape blocks (based on LTFSCopyGUI implementation)
    pub fn read_blocks(&self, block_count: u32, buffer: &mut [u8]) -> Result<u32> {
        debug!("Reading {} blocks from tape", block_count);
        
        if buffer.len() < (block_count * block_sizes::LTO_BLOCK_SIZE) as usize {
            return Err(crate::error::RustLtfsError::scsi(
                "Buffer too small for requested block count"
            ));
        }
        
        #[cfg(windows)]
        {
            let mut cdb = [0u8; 6];
            cdb[0] = scsi_commands::READ_6;
            
            // Fixed block mode (MSB=1), transfer length in blocks
            cdb[1] = 0x01; // Fixed block mode
            cdb[2] = ((block_count >> 16) & 0xFF) as u8;
            cdb[3] = ((block_count >> 8) & 0xFF) as u8;
            cdb[4] = (block_count & 0xFF) as u8;
            // cdb[5] is control byte, leave as 0
            
            let data_length = (block_count * block_sizes::LTO_BLOCK_SIZE) as usize;
            let result = self.scsi_io_control(
                &cdb,
                Some(&mut buffer[..data_length]),
                SCSI_IOCTL_DATA_IN,
                300, // 5 minute timeout for read operations
                None,
            )?;
            
            if result {
                debug!("Successfully read {} blocks", block_count);
                Ok(block_count)
            } else {
                Err(crate::error::RustLtfsError::scsi("Block read operation failed"))
            }
        }
        
        #[cfg(not(windows))]
        {
            let _ = (block_count, buffer);
            Err(crate::error::RustLtfsError::unsupported("Non-Windows platform"))
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