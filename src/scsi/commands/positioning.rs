//! SCSI Positioning Commands
//!
//! This module contains commands for positioning the tape, including LOCATE, SPACE, and READ POSITION.

use crate::error::Result;
use tracing::{debug, info, warn};

use super::super::{ScsiInterface, constants::*, types::{SpaceType, LocateDestType, TapePosition, DriveType}};
use super::super::constants::block_sizes; // Explicitly import block_sizes

impl ScsiInterface {
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
                cdb[2] = partition; // Partition number goes in byte 2, not byte 3!
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
                debug!(
                    "Successfully positioned to partition {} block {}",
                    partition, block_number
                );
                Ok(())
            } else {
                Err(crate::error::RustLtfsError::scsi("Locate operation failed"))
            }
        }

        #[cfg(not(windows))]
        {
            let _ = (partition, block_number);
            Err(crate::error::RustLtfsError::unsupported(
                "Non-Windows platform",
            ))
        }
    }

    /// Space operation (move by specified count of objects) - LTFSCopyGUI compatible
    pub fn space(&self, space_type: SpaceType, count: i32) -> Result<()> {
        debug!(
            "Space operation (LTFSCopyGUI compatible): type={:?}, count={}",
            space_type, count
        );

        #[cfg(windows)]
        {
            let mut cdb = [0u8; 6];
            cdb[0] = scsi_commands::SPACE; // 0x11
            cdb[1] = space_type as u8;

            // Handle EndOfData specially - should use count=1 according to SCSI standards
            let actual_count = match space_type {
                SpaceType::EndOfData => {
                    debug!("EndOfData operation: using standard count=1 for SCSI compliance");
                    1 // SCSI standard requires count=1 for EndOfData positioning
                }
                _ => count,
            };

            // LTFSCopyGUI方式：直接将count作为有符号整数处理
            // LTFSCopyGUI: {&H11, Code, Count >> 16 And &HFF, Count >> 8 And &HFF, Count And &HFF, 0}
            cdb[2] = ((actual_count >> 16) & 0xFF) as u8;
            cdb[3] = ((actual_count >> 8) & 0xFF) as u8;
            cdb[4] = (actual_count & 0xFF) as u8;
            cdb[5] = 0x00;

            debug!("SPACE command: {:02X?}", &cdb[..]);

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
                Err(crate::error::RustLtfsError::scsi(
                    "Space operation failed".to_string(),
                ))
            }
        }

        #[cfg(not(windows))]
        {
            let _ = (space_type, count);
            Err(crate::error::RustLtfsError::unsupported(
                "Non-Windows platform".to_string(),
            ))
        }
    }

    /// Space6 - SPACE(6) 命令实现 (对应LTFSCopyGUI的Space6)
    /// 用于在磁带上进行相对定位操作
    pub fn space6(&self, count: i32, code: u8) -> Result<u16> {
        debug!("🔧 Space6: count={}, code={}", count, code);

        #[cfg(windows)]
        {
            let mut cdb = [0u8; 6];
            cdb[0] = scsi_commands::SPACE; // 0x11
            cdb[1] = code; // LocateDestType: 0=Block, 1=FileMark, 2=SequentialFileMark

            // Count是24位有符号数
            if count < 0 {
                // 对于负数，使用24位二进制补码表示
                let abs_count = (-count) as u32;
                let complement = (!abs_count + 1) & 0xFFFFFF; // 24位二进制补码
                cdb[2] = ((complement >> 16) & 0xFF) as u8;
                cdb[3] = ((complement >> 8) & 0xFF) as u8;
                cdb[4] = (complement & 0xFF) as u8;
            } else {
                cdb[2] = ((count >> 16) & 0xFF) as u8;
                cdb[3] = ((count >> 8) & 0xFF) as u8;
                cdb[4] = (count & 0xFF) as u8;
            }

            let mut sense_buffer = [0u8; SENSE_INFO_LEN];
            let result = self.scsi_io_control(
                &cdb,
                None,
                SCSI_IOCTL_DATA_UNSPECIFIED,
                600, // 10分钟超时
                Some(&mut sense_buffer),
            )?;

            if result {
                // 返回Add_Code (sense[12] << 8 | sense[13])
                let add_code = ((sense_buffer[12] as u16) << 8) | (sense_buffer[13] as u16);
                debug!("✅ Space6 completed with Add_Code: 0x{:04X}", add_code);
                Ok(add_code)
            } else {
                Err(crate::error::RustLtfsError::scsi("Space6 command failed"))
            }
        }

        #[cfg(not(windows))]
        {
            let _ = (count, code);
            Err(crate::error::RustLtfsError::unsupported(
                "Non-Windows platform",
            ))
        }
    }

    /// ReadFileMark - 跳过当前FileMark标记 (完全对应LTFSCopyGUI的ReadFileMark实现)
    /// 这个方法精确复制LTFSCopyGUI TapeUtils.ReadFileMark的行为
    pub fn read_file_mark(&self) -> Result<bool> {
        debug!("🔧 ReadFileMark: Starting FileMark detection");

        #[cfg(windows)]
        {
            // 🎯 精确复制LTFSCopyGUI ReadFileMark逻辑 (Line 785-792)
            // 1. 总是尝试读取一个块 (对应 ReadBlock)
            let mut sense_buffer = [0u8; SENSE_INFO_LEN];
            let mut test_buffer = vec![0u8; block_sizes::LTO_BLOCK_SIZE as usize];

            let result = self.scsi_io_control(
                &[scsi_commands::READ_6, 0x00, 0x00, 0x00, 0x01, 0x00], // READ(6) 1 block
                Some(&mut test_buffer),
                SCSI_IOCTL_DATA_IN,
                30,
                Some(&mut sense_buffer),
            )?;

            debug!(
                "🔍 ReadFileMark: Read result={}, data_length={}",
                result,
                test_buffer.len()
            );

            // 2. 检查是否读取到数据 (对应 If data.Length = 0 Then Return True)
            if !result || test_buffer.is_empty() {
                debug!("✅ ReadFileMark: No data read, already positioned at FileMark");
                return Ok(true);
            }

            // 3. 读取到数据，说明不在FileMark位置 - 使用LTFSCopyGUI回退策略
            debug!("🔄 ReadFileMark: Data read, not at FileMark - executing backtrack strategy");

            // 获取当前位置
            let current_pos = self.read_position()?;
            debug!(
                "📍 ReadFileMark current position: P{} B{} FM{}",
                current_pos.partition, current_pos.block_number, current_pos.file_number
            );

            // 🎯 关键：根据AllowPartition状态选择回退策略 (对应LTFSCopyGUI Line 788-792)
            if self.allow_partition {
                // AllowPartition=true: 使用Locate命令回退
                // 🔧 修复：使用comprehensive locate()方法（LOCATE(16)）而不是locate_block()（LOCATE(10)）
                debug!(
                    "🔧 ReadFileMark: Using AllowPartition mode - Locate backtrack to Block {}",
                    current_pos.block_number.saturating_sub(1)
                );
                if current_pos.block_number > 0 {
                    // 使用self.locate()代替locate_block()，它会正确使用LOCATE(16)命令和CP标志
                    self.locate(
                        current_pos.block_number - 1,
                        current_pos.partition,
                        LocateDestType::Block,
                    )?;
                }
            } else {
                // AllowPartition=false: 使用Space6命令回退 (Space6(handle, -1, Block))
                info!("ReadFileMark: Using non-AllowPartition mode - Space6 backtrack");
                self.space6(-1, 0)?; // Count=-1, Code=0 (Block)
            }

            // 验证回退后的位置
            let new_pos = self.read_position()?;
            debug!(
                "✅ ReadFileMark: Backtrack completed - now at P{} B{} FM{}",
                new_pos.partition, new_pos.block_number, new_pos.file_number
            );

            Ok(false) // 返回false表示执行了回退
        }

        #[cfg(not(windows))]
        {
            Err(crate::error::RustLtfsError::unsupported(
                "Non-Windows platform",
            ))
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

            let result =
                self.scsi_io_control(&cdb, None, SCSI_IOCTL_DATA_UNSPECIFIED, 300, None)?;

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
            Err(crate::error::RustLtfsError::unsupported(
                "Non-Windows platform",
            ))
        }
    }

    /// Read tape position information (LTFSCopyGUI compatible implementation)
    pub fn read_position(&self) -> Result<TapePosition> {
        debug!("Reading tape position");

        #[cfg(windows)]
        {
            let mut cdb = [0u8; 10];
            let mut data_buffer = [0u8; 32];

            // 🔧 修复：LTFSCopyGUI在AllowPartition=true时使用Service Action 6
            // AllowPartition模式: {&H34, 6, 0, 0, 0, 0, 0, 0, 0, 0}
            // DisablePartition模式: {&H34, 0, 0, 0, 0, 0, 0, 0, 0, 0}
            // 对于多分区支持，我们使用AllowPartition模式
            cdb[0] = scsi_commands::READ_POSITION; // 0x34
            cdb[1] = 0x06; // Service Action = 6 (LTFSCopyGUI AllowPartition模式)
            cdb[2] = 0x00;
            cdb[3] = 0x00;
            cdb[4] = 0x00;
            cdb[5] = 0x00;
            cdb[6] = 0x00;
            cdb[7] = 0x00;
            cdb[8] = 0x00;
            cdb[9] = 0x00;

            debug!(
                "🔧 Sending READ POSITION command (LTFSCopyGUI AllowPartition mode): {:02X?}",
                &cdb[..]
            );

            let result =
                self.scsi_io_control(&cdb, Some(&mut data_buffer), SCSI_IOCTL_DATA_IN, 300, None)?;

            if result {
                debug!(
                    "🔧 READ POSITION raw data (Service Action 6): {:02X?}",
                    &data_buffer[..]
                );

                // 🔍 详细分析SCSI返回数据的每个字节段 (对应LTFSCopyGUI TapeUtils.vb)
                debug!("🔍 Raw data analysis:");
                debug!("  Flags (byte 0): 0x{:02X}", data_buffer[0]);
                debug!(
                    "  Bytes 1-3: {:02X} {:02X} {:02X}",
                    data_buffer[1], data_buffer[2], data_buffer[3]
                );
                debug!(
                    "  Partition (bytes 4-7): {:02X} {:02X} {:02X} {:02X}",
                    data_buffer[4], data_buffer[5], data_buffer[6], data_buffer[7]
                );
                debug!(
                    "  Block (bytes 8-15): {:02X} {:02X} {:02X} {:02X} {:02X} {:02X} {:02X} {:02X}",
                    data_buffer[8],
                    data_buffer[9],
                    data_buffer[10],
                    data_buffer[11],
                    data_buffer[12],
                    data_buffer[13],
                    data_buffer[14],
                    data_buffer[15]
                );
                debug!("  File/FileMark (bytes 16-23): {:02X} {:02X} {:02X} {:02X} {:02X} {:02X} {:02X} {:02X}",
                       data_buffer[16], data_buffer[17], data_buffer[18], data_buffer[19],
                       data_buffer[20], data_buffer[21], data_buffer[22], data_buffer[23]);
                debug!(
                    "  Set (bytes 24-31): {:02X} {:02X} {:02X} {:02X} {:02X} {:02X} {:02X} {:02X}",
                    data_buffer[24],
                    data_buffer[25],
                    data_buffer[26],
                    data_buffer[27],
                    data_buffer[28],
                    data_buffer[29],
                    data_buffer[30],
                    data_buffer[31]
                );

                // 按照LTFSCopyGUI的解析方式（TapeUtils.vb第1858-1870行）
                // AllowPartition = true时的数据结构：
                let flags = data_buffer[0];

                // 🔧 修复分区号解析：LTFSCopyGUI使用4字节循环 (bytes 4-7)
                // For i As Integer = 0 To 3: result.PartitionNumber = result.PartitionNumber Or param(4 + i)
                let mut partition_number = 0u32;
                for i in 0..4 {
                    partition_number <<= 8;
                    partition_number |= data_buffer[4 + i] as u32;
                }
                let partition = partition_number as u8; // 转换为u8以保持兼容性

                // Block number: 8字节，从第8字节开始
                let mut block_number = 0u64;
                for i in 0..8 {
                    block_number <<= 8;
                    block_number |= data_buffer[8 + i] as u64;
                }

                // File number (FileMark): 8字节，从第16字节开始
                let mut file_number = 0u64;
                for i in 0..8 {
                    file_number <<= 8;
                    file_number |= data_buffer[16 + i] as u64;
                }

                // Set number: 8字节，从第24字节开始
                let mut set_number = 0u64;
                for i in 0..8 {
                    set_number <<= 8;
                    set_number |= data_buffer[24 + i] as u64;
                }

                let position = TapePosition {
                    partition,
                    block_number,
                    file_number,
                    set_number,
                    end_of_data: (flags & 0x04) != 0, // EOD flag
                    beginning_of_partition: (flags & 0x08) != 0, // BOP flag
                };

                // 🔍 显示解析后的值与LTFSCopyGUI格式对比
                debug!("🔍 Parsed values:");
                debug!(
                    "  - Flags: 0x{:02X} (BOP={}, EOD={})",
                    flags, position.beginning_of_partition, position.end_of_data
                );
                debug!(
                    "  - Partition: {} (from 4-byte value: 0x{:08X})",
                    partition, partition_number
                );
                debug!("  - Block Number: {}", block_number);
                debug!("  - File Number (FileMark): {}", file_number);
                debug!("  - Set Number: {}", set_number);

                debug!(
                    "LTFSCopyGUI compatible position: partition={}, block={}, file={}, set={}",
                    position.partition,
                    position.block_number,
                    position.file_number,
                    position.set_number
                );

                Ok(position)
            } else {
                Err(crate::error::RustLtfsError::scsi(
                    "Read position failed".to_string(),
                ))
            }
        }

        #[cfg(not(windows))]
        {
            Err(crate::error::RustLtfsError::unsupported(
                "Non-Windows platform".to_string(),
            ))
        }
    }

    /// Comprehensive locate method (based on LTFSCopyGUI TapeUtils.Locate)
    /// Supports block, file mark, and EOD positioning with drive-specific optimizations
    pub fn locate(
        &self,
        block_address: u64,
        partition: u8,
        dest_type: LocateDestType,
    ) -> Result<u16> {
        debug!(
            "Locating to partition {} {} {:?} {}",
            partition,
            match dest_type {
                LocateDestType::Block => "block",

                LocateDestType::EOD => "EOD",
            },
            dest_type,
            block_address
        );

        #[cfg(windows)]
        {
            let mut sense_buffer = [0u8; SENSE_INFO_LEN];

            // Execute locate command based on drive type
            // Execute locate command based on drive type
            match self.drive_type {
                DriveType::Standard => {
                    self.locate_standard(block_address, partition, dest_type, &mut sense_buffer)
                }
            }
        }

        #[cfg(not(windows))]
        {
            let _ = (block_address, partition, dest_type);
            Err(crate::error::RustLtfsError::unsupported(
                "Non-Windows platform",
            ))
        }
    }

    /// Standard/modern drive locate implementation
    #[cfg(windows)]
    fn locate_standard(
        &self,
        block_address: u64,
        partition: u8,
        dest_type: LocateDestType,
        sense_buffer: &mut [u8; SENSE_INFO_LEN],
    ) -> Result<u16> {
        // 🎯 关键修复：FileMark定位必须使用LTFSCopyGUI逻辑 (Line 972-974)
        // ElseIf DestType = LocateDestType.FileMark Then
        //     Locate(handle, 0, 0)
        //     Space6(handle:=handle, Count:=BlockAddress, Code:=LocateDestType.FileMark)
        match dest_type {

            _ => {
                // 对于Block和EOD，使用标准的LOCATE(16)命令
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
        }
    }

    /// Execute locate command and handle errors (based on LTFSCopyGUI error handling)
    #[cfg(windows)]
    fn execute_locate_command(
        &self,
        cdb: &[u8],
        sense_buffer: &mut [u8; SENSE_INFO_LEN],
    ) -> Result<u16> {
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
            warn!(
                "Locate command returned error: ASC/ASCQ = 0x{:04X}",
                asc_ascq
            );

            // Retry with different strategy if first attempt failed
            self.retry_locate_on_error(cdb, sense_buffer, asc_ascq)
        } else {
            debug!("Locate command completed successfully");
            Ok(0)
        }
    }

    /// Retry locate operation on error (based on LTFSCopyGUI retry logic)
    #[cfg(windows)]
    fn retry_locate_on_error(
        &self,
        original_cdb: &[u8],
        sense_buffer: &mut [u8; SENSE_INFO_LEN],
        error_code: u16,
    ) -> Result<u16> {
        debug!(
            "Attempting locate retry for error code: 0x{:04X}",
            error_code
        );

        // Parse original command to determine retry strategy
        let original_command = original_cdb[0];

        match original_command {
            0x92 => {
                // LOCATE(16) failed, try LOCATE(10)
                if original_cdb.len() >= 12 {
                    let block_address = ((original_cdb[8] as u64) << 24)
                        | ((original_cdb[9] as u64) << 16)
                        | ((original_cdb[10] as u64) << 8)
                        | (original_cdb[11] as u64);

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
                        let retry_asc_ascq =
                            ((sense_buffer[12] as u16) << 8) | (sense_buffer[13] as u16);
                        debug!("Retry result: ASC/ASCQ = 0x{:04X}", retry_asc_ascq);
                        Ok(retry_asc_ascq)
                    } else {
                        Err(crate::error::RustLtfsError::scsi(
                            "Locate retry also failed",
                        ))
                    }
                } else {
                    Err(crate::error::RustLtfsError::scsi("Invalid CDB for retry"))
                }
            }
            _ => {
                // For other commands, return the original error
                Err(crate::error::RustLtfsError::scsi(format!(
                    "Locate operation failed with ASC/ASCQ: 0x{:04X}",
                    error_code
                )))
            }
        }
    }

    /// Convenience method: locate to file mark
    pub fn locate_to_filemark(&self, filemark_number: u64, partition: u8) -> Result<()> {
        // 🎯 关键修复：避免无限递归，直接使用LTFSCopyGUI逻辑
        // 对应: Locate(handle, 0, 0) + Space6(handle, Count, FileMark)
        debug!(
            "🔧 locate_to_filemark: FileMark {} in partition {} using LTFSCopyGUI method",
            filemark_number, partition
        );

        // Step 1: 先定位到指定分区的开头
        self.locate(0, partition, LocateDestType::Block)?;

        // Step 2: 然后用Space命令移动到FileMark
        self.space(SpaceType::FileMarks, filemark_number as i32)?;

        Ok(())
    }

    /// Convenience method: locate to end of data
    pub fn locate_to_eod(&self, partition: u8) -> Result<()> {
        self.locate(0, partition, LocateDestType::EOD)?;
        Ok(())
    }
}
