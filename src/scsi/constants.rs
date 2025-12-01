#![allow(dead_code)]

// SCSI constant definitions (based on C code)
pub const SENSE_INFO_LEN: usize = 64;
pub const TC_MP_PC_CURRENT: u8 = 0x00;
pub const TC_MP_MEDIUM_CONFIGURATION: u8 = 0x1D;

// SCSI operation code constants
pub const SCSIOP_READ_POSITION: u8 = 0x34;
pub const SCSIOP_MODE_SENSE10: u8 = 0x5A;
pub const SCSIOP_INQUIRY: u8 = 0x12;

// SCSI data direction
pub const SCSI_IOCTL_DATA_IN: u8 = 1;
pub const SCSI_IOCTL_DATA_OUT: u8 = 0;
pub const SCSI_IOCTL_DATA_UNSPECIFIED: u8 = 2;

// Define IOCTL_SCSI_PASS_THROUGH_DIRECT constant
#[cfg(windows)]
pub const IOCTL_SCSI_PASS_THROUGH_DIRECT: u32 = 0x0004D014;

// SCSI Commands Module
pub mod scsi_commands {
    pub const TEST_UNIT_READY: u8 = 0x00;
    pub const REWIND: u8 = 0x01;
    pub const REQUEST_SENSE: u8 = 0x03;
    pub const FORMAT_UNIT: u8 = 0x04;
    pub const READ_6: u8 = 0x08;
    pub const WRITE_6: u8 = 0x0A;
    pub const WRITE_FILEMARKS_6: u8 = 0x10;
    pub const SPACE: u8 = 0x11;
    pub const INQUIRY: u8 = 0x12;
    pub const MODE_SELECT_6: u8 = 0x15;
    pub const RESERVE_6: u8 = 0x16;
    pub const RELEASE_6: u8 = 0x17;
    pub const ERASE: u8 = 0x19;
    pub const MODE_SENSE_6: u8 = 0x1A;
    pub const LOAD_UNLOAD: u8 = 0x1B;
    pub const LOCATE: u8 = 0x2B;
    pub const READ_POSITION: u8 = 0x34;
    pub const READ_BUFFER: u8 = 0x3C;
    pub const WRITE_BUFFER: u8 = 0x3B;
    pub const READ_BLOCK_LIMITS: u8 = 0x05;
    pub const LOG_SENSE: u8 = 0x4D;
    pub const READ_ATTRIBUTE: u8 = 0x8C;
    pub const WRITE_ATTRIBUTE: u8 = 0x8D;
}

pub mod block_sizes {
    pub const LTO_BLOCK_SIZE: u32 = 65536; // 64KB standard LTO block size
    pub const LTO_BLOCK_SIZE_512K: u32 = 524288; // 512KB LTFSCopyGUI BlockSizeLimit (&H80000)
    pub const MIN_BLOCK_SIZE: u32 = 512;
    pub const MAX_BLOCK_SIZE: u32 = 1048576; // 1MB maximum
}
