//! SCSI Sense Data Parsing
//!
//! This module handles parsing and interpretation of SCSI sense data.

use crate::error::Result;
use tracing::{debug, info};

use super::constants::block_sizes;
use super::ScsiInterface;

impl ScsiInterface {
    /// Parse sense data for Test Unit Ready (similar to LTFSCopyGUI's ParseSenseData)
    pub fn parse_sense_data(&self, sense_data: &[u8]) -> String {
        if sense_data.len() < 3 {
            return "Invalid sense data (too short)".to_string();
        }

        let sense_key = sense_data[2] & 0x0F;
        let asc = if sense_data.len() > 12 {
            sense_data[12]
        } else {
            0
        };
        let ascq = if sense_data.len() > 13 {
            sense_data[13]
        } else {
            0
        };

        debug!(
            "Sense data - Key: 0x{:02X}, ASC: 0x{:02X}, ASCQ: 0x{:02X}",
            sense_key, asc, ascq
        );

        match (sense_key, asc, ascq) {
            (0x00, _, _) => "Device ready".to_string(),
            (0x02, 0x3A, 0x00) => "No tape loaded".to_string(),
            (0x02, 0x04, 0x00) => "Drive not ready".to_string(),
            (0x02, 0x3B, 0x0D) => "Medium not present".to_string(),
            (0x04, 0x00, 0x00) => "Drive not ready - becoming ready".to_string(),
            (0x06, 0x28, 0x00) => "Unit attention - not ready to ready transition".to_string(),
            _ => format!(
                "Device not ready - Sense Key: 0x{:02X}, ASC/ASCQ: 0x{:02X}/0x{:02X}",
                sense_key, asc, ascq
            ),
        }
    }

    /// 分析READ命令的sense数据 (对应LTFSCopyGUI的ReadBlock中的sense数据分析)
    /// 返回 (实际读取的块数, 是否遇到文件标记)
    pub(super) fn analyze_read_sense_data(
        &self,
        sense_data: &[u8],
        requested_bytes: u32,
    ) -> Result<(u32, bool)> {
        if sense_data.len() < 18 {
            return Ok((0, false));
        }

        // 分析sense key和additional sense code (对应VB.NET的Add_Key检测)
        let sense_key = sense_data[2] & 0x0F;
        let asc = sense_data[12]; // Additional Sense Code
        let ascq = sense_data[13]; // Additional Sense Code Qualifier

        info!(
            "🔍 Sense analysis: SenseKey=0x{:02X}, ASC=0x{:02X}, ASCQ=0x{:02X}",
            sense_key, asc, ascq
        );

        // 从sense数据的字节3-6提取DiffBytes (对应VB.NET代码的逻辑)
        // 修复符号位扩展：直接构造 32 位有符号整数，并进行符号位扩展
        // VB.NET 中 DiffBytes 是 Integer (32位有符号)，会自动处理符号位扩展
        let diff_bytes = if sense_data.len() >= 7 {
            // 构造 32 位大端序有符号整数
            let mut bytes = [0u8; 4];
            bytes[0] = sense_data[3];
            bytes[1] = sense_data[4];
            bytes[2] = sense_data[5];
            bytes[3] = sense_data[6];
            i32::from_be_bytes(bytes)
        } else {
            0
        };

        info!(
            "🔍 DiffBytes from sense data: {} (requested {} bytes)",
            diff_bytes, requested_bytes
        );

        // 计算实际读取的数据长度 (对应VB.NET的DataLen计算)
        // DataLen = Math.Min(BlockSizeLimit, BlockSizeLimit - DiffBytes)
        let actual_bytes_read = if diff_bytes < 0 {
            // 如果DiffBytes为负数，说明读取了更多数据
            std::cmp::min(requested_bytes as i32, requested_bytes as i32 - diff_bytes) as u32
        } else {
            // 如果DiffBytes为正数，说明读取了更少数据
            std::cmp::max(0, requested_bytes as i32 - diff_bytes) as u32
        };

        // 转换为块数
        let actual_blocks_read = if actual_bytes_read > 0 {
            (actual_bytes_read / block_sizes::LTO_BLOCK_SIZE)
                + if (actual_bytes_read % block_sizes::LTO_BLOCK_SIZE) > 0 {
                    1
                } else {
                    0
                }
        } else {
            0
        };

        info!(
            "🔍 Calculated: {} bytes read = {} blocks",
            actual_bytes_read, actual_blocks_read
        );

        // 检测文件标记 (对应VB.NET的Add_Key >= 1 And Add_Key <> 4逻辑)
        // VB.NET: Add_Key = (sense(12) << 8) Or sense(13)
        let add_key = ((asc as u16) << 8) | (ascq as u16);
        let is_file_mark = add_key >= 1 && add_key != 4;

        if is_file_mark {
            info!(
                "🎯 File mark detected: Add_Key=0x{:04X} (ASC:0x{:02X}, ASCQ:0x{:02X})",
                add_key, asc, ascq
            );
        } else {
            info!("Normal data read: Add_Key=0x{:04X}", add_key);
        }

        // 特殊情况：如果sense key表示文件标记或EOD
        let is_filemark_or_eod = sense_key == 0x00 || // No Sense (可能遇到文件标记)
                                sense_key == 0x01 || // Recovered Error
                                (sense_key == 0x03 && asc == 0x00 && ascq == 0x01); // Filemark detected

        let final_is_file_mark = is_file_mark || is_filemark_or_eod;

        if final_is_file_mark {
            info!(
                "✅ Final determination: FILE MARK detected - {} blocks read before mark",
                actual_blocks_read
            );
        }

        Ok((actual_blocks_read, final_is_file_mark))
    }
}
