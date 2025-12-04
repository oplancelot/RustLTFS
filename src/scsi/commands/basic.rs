//! SCSI Basic Commands
//!
//! This module contains basic SCSI commands like TEST UNIT READY and LOG SENSE.

use crate::error::Result;
use tracing::debug;

use super::super::{ScsiInterface, constants::*};

impl ScsiInterface {
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
            Err(crate::error::RustLtfsError::unsupported(
                "Non-Windows platform",
            ))
        }
    }

    /// LOG SENSE command (based on LTFSCopyGUI implementation)
    /// LTFSCopyGUI: {&H4D, 0, PageControl << 6 Or PageCode, 0, 0, 0, 0, (PageLen + 4) >> 8 And &HFF, (PageLen + 4) And &HFF, 0}
    pub fn log_sense(&self, page_code: u8, page_control: u8) -> Result<Vec<u8>> {
        debug!(
            "Executing LOG SENSE command: page_code=0x{:02X}, page_control=0x{:02X}",
            page_code, page_control
        );

        #[cfg(windows)]
        {
            // Step 1: Get header to determine page length
            let mut header_cdb = [0u8; 10];
            let mut header_buffer = [0u8; 4];

            header_cdb[0] = scsi_commands::LOG_SENSE;
            header_cdb[1] = 0x00;
            header_cdb[2] = (page_control << 6) | page_code;
            header_cdb[3] = 0x00;
            header_cdb[4] = 0x00;
            header_cdb[5] = 0x00;
            header_cdb[6] = 0x00;
            header_cdb[7] = 0x00;
            header_cdb[8] = 4; // Allocation length for header
            header_cdb[9] = 0x00;

            let result = self.scsi_io_control(
                &header_cdb,
                Some(&mut header_buffer),
                SCSI_IOCTL_DATA_IN,
                30,
                None,
            )?;

            if !result || header_buffer.len() < 4 {
                return Ok(vec![0, 0, 0, 0]);
            }

            // Parse page length from header
            let page_len = ((header_buffer[2] as u16) << 8) | (header_buffer[3] as u16);
            let total_len = page_len + 4;

            // Step 2: Read full page data
            let mut full_cdb = [0u8; 10];
            let mut full_buffer = vec![0u8; total_len as usize];

            full_cdb[0] = scsi_commands::LOG_SENSE;
            full_cdb[1] = 0x00;
            full_cdb[2] = (page_control << 6) | page_code;
            full_cdb[3] = 0x00;
            full_cdb[4] = 0x00;
            full_cdb[5] = 0x00;
            full_cdb[6] = 0x00;
            full_cdb[7] = (total_len >> 8) as u8;
            full_cdb[8] = (total_len & 0xFF) as u8;
            full_cdb[9] = 0x00;

            let full_result = self.scsi_io_control(
                &full_cdb,
                Some(&mut full_buffer),
                SCSI_IOCTL_DATA_IN,
                30,
                None,
            )?;

            if full_result {
                debug!(
                    "LOG SENSE completed successfully, {} bytes returned",
                    full_buffer.len()
                );
                Ok(full_buffer)
            } else {
                Err(crate::error::RustLtfsError::scsi(
                    "LOG SENSE command failed",
                ))
            }
        }

        #[cfg(not(windows))]
        {
            let _ = (page_code, page_control);
            Err(crate::error::RustLtfsError::unsupported(
                "Non-Windows platform",
            ))
        }
    }
}
