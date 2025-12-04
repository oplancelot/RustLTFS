//! SCSI Core Implementation
//!
//! This module contains the ScsiInterface struct and core IO control logic.

use crate::error::Result;
use tracing::{debug, warn};

#[cfg(windows)]
use winapi::{
    shared::{
        minwindef::{DWORD, UCHAR, ULONG, USHORT},
        ntdef::{PVOID},
    },
    um::{
        errhandlingapi::GetLastError,
        ioapiset::DeviceIoControl,
    },
};

use super::constants::*;
use super::{DriveType, ScsiPassThroughDirect};
use super::device::DeviceHandle;

/// SCSI operation structure that encapsulates low-level SCSI commands
pub struct ScsiInterface {
    pub(crate) device_handle: Option<DeviceHandle>,
    pub(crate) drive_type: DriveType,
    pub(crate) allow_partition: bool,
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

    /// Send SCSI command general interface (based on ScsiIoControl in C code)
    pub fn scsi_io_control(
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
                let mut scsi_buffer =
                    vec![0u8; std::mem::size_of::<ScsiPassThroughDirect>() + SENSE_INFO_LEN];

                unsafe {
                    let scsi_direct = scsi_buffer.as_mut_ptr() as *mut ScsiPassThroughDirect;
                    std::ptr::write_bytes(scsi_direct, 0, 1);

                    (*scsi_direct).length = std::mem::size_of::<ScsiPassThroughDirect>() as USHORT;
                    (*scsi_direct).cdb_length = cdb.len() as UCHAR;
                    (*scsi_direct).data_buffer = data_ptr;
                    (*scsi_direct).sense_info_length = SENSE_INFO_LEN as UCHAR;
                    (*scsi_direct).sense_info_offset =
                        std::mem::size_of::<ScsiPassThroughDirect>() as ULONG;
                    (*scsi_direct).data_transfer_length = buffer_length;
                    (*scsi_direct).timeout_value = timeout;
                    (*scsi_direct).data_in = data_in;

                    // Copy CDB
                    std::ptr::copy_nonoverlapping(
                        cdb.as_ptr(),
                        (*scsi_direct).cdb.as_mut_ptr(),
                        cdb.len(),
                    );

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
                            scsi_buffer
                                .as_ptr()
                                .add(std::mem::size_of::<ScsiPassThroughDirect>()),
                            sense_buf.as_mut_ptr(),
                            SENSE_INFO_LEN,
                        );
                    }

                    if !result {
                        let error_code = GetLastError();
                        warn!(
                            "SCSI command failed: Windows error code 0x{:08X}, CDB: {:?}",
                            error_code, cdb
                        );
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
            Err(crate::error::RustLtfsError::unsupported(
                "Non-Windows platform",
            ))
        }
    }
}

/// Implement Drop trait to ensure SCSI interface is properly cleaned up
impl Drop for ScsiInterface {
    fn drop(&mut self) {
        debug!("SCSI interface cleanup completed");
    }
}
