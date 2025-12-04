//! SCSI Device Management
//!
//! This module handles device opening, closing, and media status checking.

use crate::error::Result;
use std::ffi::CString;
use tracing::{debug, warn};

#[cfg(windows)]
use winapi::{
    shared::ntdef::HANDLE,
    um::{
        errhandlingapi::GetLastError,
        fileapi::{CreateFileA, OPEN_EXISTING},
        handleapi::{CloseHandle, INVALID_HANDLE_VALUE},
        winnt::{GENERIC_READ, GENERIC_WRITE},
    },
};

use super::ScsiInterface;
use super::constants::*;
use super::types::MediaType;

/// Device handle wrapper that ensures proper resource cleanup
pub struct DeviceHandle {
    #[cfg(windows)]
    pub(crate) handle: HANDLE,
    pub(crate) device_path: String,
}

impl ScsiInterface {
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
                // Already formatted path like \\.\\TAPE1
                device_path.to_string()
            };

            debug!("Full device path: {}", full_path);

            let path_cstring = CString::new(full_path.clone()).map_err(|e| {
                crate::error::RustLtfsError::system(format!("Device path conversion error: {}", e))
            })?;

            unsafe {
                let handle = CreateFileA(
                    path_cstring.as_ptr(),
                    GENERIC_READ | GENERIC_WRITE,
                    0, // Exclusive access (0), no sharing allowed
                    std::ptr::null_mut(),
                    OPEN_EXISTING,
                    0, // Don't use FILE_ATTRIBUTE_NORMAL, based on C code
                    std::ptr::null_mut(),
                );

                if handle == INVALID_HANDLE_VALUE {
                    let error_code = GetLastError();
                    return Err(crate::error::RustLtfsError::system(format!(
                        "Cannot open device {}: Windows error code 0x{:08X}",
                        full_path, error_code
                    )));
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
            Err(crate::error::RustLtfsError::unsupported(
                "Non-Windows platform",
            ))
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
                return Err(crate::error::RustLtfsError::scsi(
                    "read_position command failed",
                ));
            }

            // Check if sense buffer indicates no tape
            // C code: if (((senseBuffer[2] & 0x0F) == 0x02) && (senseBuffer[12] == 0x3A) && (senseBuffer[13] == 0x00))
            if (sense_buffer[2] & 0x0F) == 0x02
                && sense_buffer[12] == 0x3A
                && sense_buffer[13] == 0x00
            {
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

            let result =
                self.scsi_io_control(&cdb, Some(&mut data_buffer), SCSI_IOCTL_DATA_IN, 300, None)?;

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
            Err(crate::error::RustLtfsError::unsupported(
                "Non-Windows platform",
            ))
        }
    }
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
