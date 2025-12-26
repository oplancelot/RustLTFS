//! SCSI Configuration Commands
//!
//! This module contains commands for reading and writing device configuration,
//! such as MODE SENSE.

use crate::error::Result;
use tracing::debug;

use super::super::{ScsiInterface, constants::*};

impl ScsiInterface {
    /// MODE SENSE command to read partition page 0x11 (对应LTFSCopyGUI的ModeSense实现)
    /// 这个方法复制LTFSCopyGUI的精确实现：TapeUtils.ModeSense(handle, &H11)
    #[allow(dead_code)]
    pub fn mode_sense_partition_page_0x11(&self) -> Result<Vec<u8>> {
        debug!("Executing MODE SENSE page 0x11 for partition detection");

        #[cfg(windows)]
        {
            // 第一步：获取页面头信息（对应LTFSCopyGUI的Header读取）
            let mut header_cdb = [0u8; 6];
            header_cdb[0] = 0x1A; // MODE SENSE 6命令
            header_cdb[1] = 0x00; // Reserved
            header_cdb[2] = 0x11; // Page 0x11 (分区模式页)
            header_cdb[3] = 0x00; // Reserved
            header_cdb[4] = 4; // Allocation Length = 4 bytes
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
                return Err(crate::error::RustLtfsError::scsi(format!(
                    "MODE SENSE header failed: {}",
                    sense_info
                )));
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
                Err(crate::error::RustLtfsError::scsi(format!(
                    "MODE SENSE page 0x11 failed: {}",
                    sense_info
                )))
            }
        }

        #[cfg(not(windows))]
        {
            Err(crate::error::RustLtfsError::unsupported(
                "Non-Windows platform",
            ))
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
                debug!(
                    "MODE SENSE completed successfully, {} bytes returned",
                    data_buffer.len()
                );
                Ok(data_buffer)
            } else {
                let sense_info = self.parse_sense_data(&sense_buffer);
                Err(crate::error::RustLtfsError::scsi(format!(
                    "MODE SENSE failed: {}",
                    sense_info
                )))
            }
        }

        #[cfg(not(windows))]
        {
            Err(crate::error::RustLtfsError::unsupported(
                "Non-Windows platform",
            ))
        }
    }
    /// Sets the block size for the tape drive using MODE SELECT (6).
    /// If block_size > 0, the drive is set to Fixed Block Mode with the specified size.
    /// If block_size == 0, the drive is set to Variable Block Mode.
    /// This corresponds to LTFSCopyGUI's TapeUtils.SetBlockSize
    pub fn set_block_size(&self, block_size: u32) -> Result<()> {
        debug!("Executing MODE SELECT to set block size to {}", block_size);

        #[cfg(windows)]
        {
            // CDB for MODE SELECT (6)
            let mut cdb = [0u8; 6];
            cdb[0] = 0x15; // MODE SELECT (6)
            cdb[1] = 0x10; // PF=1 (Page Format, standard SCSI-2)
            cdb[2] = 0x00; // Reserved
            cdb[3] = 0x00; // Reserved
            cdb[4] = 12;   // Parameter List Length (4 header + 8 block descriptor)
            cdb[5] = 0x00; // Control

            // Parameter List
            // Header (4 bytes) + Block Descriptor (8 bytes)
            let mut param_list = vec![0u8; 12];
            
            // Header: 
            // Byte 0: Mode Data Length (Reserved)
            // Byte 1: Medium Type
            // Byte 2: Device-Specific Parameter
            // Byte 3: Block Descriptor Length
            
            param_list[3] = 0x08; // Block Descriptor Length = 8 bytes

            // Block Descriptor (8 bytes)
            // Bytes 0: Density Code (00)
            // Bytes 1-3: Number of Blocks (00 00 00 = all remaining)
            // Byte 4: Reserved
            // Bytes 5-7: Block Length
            
            if block_size > 0 {
                param_list[9] = ((block_size >> 16) & 0xFF) as u8;
                param_list[10] = ((block_size >> 8) & 0xFF) as u8;
                param_list[11] = (block_size & 0xFF) as u8;
            }

            let mut sense_buffer = [0u8; SENSE_INFO_LEN];

            let result = self.scsi_io_control(
                &cdb,
                Some(&mut param_list),
                SCSI_IOCTL_DATA_OUT,
                30,
                Some(&mut sense_buffer),
            )?;

            if result {
                debug!("MODE SELECT (Set Block Size) successful");
                Ok(())
            } else {
                let sense_info = self.parse_sense_data(&sense_buffer);
                Err(crate::error::RustLtfsError::scsi(format!(
                    "MODE SELECT failed to set block size: {}",
                    sense_info
                )))
            }
        }

        #[cfg(not(windows))]
        {
            Err(crate::error::RustLtfsError::unsupported("Non-Windows platform"))
        }
    }
}
