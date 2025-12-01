
#[cfg(windows)]
use winapi::{
    shared::{
        minwindef::{UCHAR, ULONG, USHORT},
        ntdef::{PVOID},
    },
};

// Type aliases for non-Windows platforms
#[cfg(not(windows))]
pub type UCHAR = u8;
#[cfg(not(windows))]
pub type USHORT = u16;
#[cfg(not(windows))]
pub type ULONG = u32;
#[cfg(not(windows))]
pub type DWORD = u32;
#[cfg(not(windows))]
pub type PVOID = *mut std::ffi::c_void;
#[cfg(not(windows))]
pub type HANDLE = *mut std::ffi::c_void;

/// SCSI Pass Through Direct structure (corresponds to SCSI_PASS_THROUGH_DIRECT in C code)
#[repr(C)]
#[derive(Debug)]
pub struct ScsiPassThroughDirect {
    pub length: USHORT,
    pub scsi_status: UCHAR,
    pub path_id: UCHAR,
    pub target_id: UCHAR,
    pub lun: UCHAR,
    pub cdb_length: UCHAR,
    pub sense_info_length: UCHAR,
    pub data_in: UCHAR,
    pub data_transfer_length: ULONG,
    pub timeout_value: ULONG,
    pub data_buffer: PVOID,
    pub sense_info_offset: ULONG,
    pub cdb: [UCHAR; 16],
}
