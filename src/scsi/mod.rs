use crate::error::Result;

use tracing::{debug, info, warn};

#[cfg(windows)]
use winapi::{
    shared::{
        minwindef::{DWORD, UCHAR, ULONG, USHORT},
        ntdef::{HANDLE, PVOID},
    },
    um::{
        errhandlingapi::GetLastError,
        ioapiset::DeviceIoControl,
    },
};

pub mod constants;
pub mod types;
pub mod ffi;
mod sense;
mod device;
mod commands;

pub use constants::*;
pub use types::{DriveType, MediaType, TapePosition, SpaceType, LocateDestType};
pub use ffi::*;

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


    /// Send SCSI command with simplified interface (for compatibility with tape_ops.rs)




    /// INQUIRY command to get device information (based on LTFSCopyGUI implementation)


    /// READ BLOCK LIMITS command (based on LTFSCopyGUI implementation)
    /// Returns (max_block_length, min_block_length)



    /// READ EOW POSITION command (based on LTFSCopyGUI implementation)
    /// LTFSCopyGUI: {&HA3, &H1F, &H45, 2, 0, 0, 0, 0, len >> 8, len And &HFF, 0, 0}



    /// Read tape blocks (enhanced implementation for large file support)
    pub fn read_blocks(&self, block_count: u32, buffer: &mut [u8]) -> Result<u32> {
        debug!(
            "read_blocks called: requesting {} blocks, buffer size: {} bytes",
            block_count,
            buffer.len()
        );


        // 对应LTFSCopyGUI的自适应缓冲区逻辑，不预先检查缓冲区大小
        // 让SCSI驱动返回实际读取的字节数或错误信息

        // For large block counts, break into smaller chunks to avoid SCSI timeout
        const MAX_BLOCKS_PER_READ: u32 = 256; // 16MB chunks (256 * 64KB)

        if block_count <= MAX_BLOCKS_PER_READ {
            // Direct read for smaller requests
            debug!("Using direct read for {} blocks", block_count);
            self.read_blocks_direct(block_count, buffer)
        } else {
            // Chunked read for larger requests
            debug!("Using chunked read for {} blocks", block_count);
            self.read_blocks_chunked(block_count, buffer)
        }
    }

    /// Direct block read implementation (private)
    fn read_blocks_direct(&self, block_count: u32, buffer: &mut [u8]) -> Result<u32> {
        debug!("Direct reading {} blocks", block_count);

        #[cfg(windows)]
        {
            // Use READ(6) command for tape devices (sequential access)
            // READ(10) LBA addressing is inappropriate for tape devices
            let mut cdb = [0u8; 6];
            cdb[0] = scsi_commands::READ_6;

            // LTFSCopyGUI compatibility: Use variable length mode, no SILI flag
            // Matches LTFSCopyGUI: cdbData:={8, 0, ...} - second byte is 0
            cdb[1] = 0x00; // No flags set - variable length mode like LTFSCopyGUI

            // Transfer Length - 精确对应LTFSCopyGUI: BlockSizeLimit >> 16 And &HFF (字节数而非块数)
            // Critical fix: LTFSCopyGUI使用字节数，而不是块数
            let byte_count = std::cmp::min(
                buffer.len(),
                (block_count * block_sizes::LTO_BLOCK_SIZE) as usize,
            ) as u32;
            cdb[2] = ((byte_count >> 16) & 0xFF) as u8;
            cdb[3] = ((byte_count >> 8) & 0xFF) as u8;
            cdb[4] = (byte_count & 0xFF) as u8;
            cdb[5] = 0x00; // Control byte

            debug!("READ(6) CDB: [{:02X}, {:02X}, {:02X}, {:02X}, {:02X}, {:02X}] - requesting {} bytes",
                   cdb[0], cdb[1], cdb[2], cdb[3], cdb[4], cdb[5], byte_count);

            // 使用实际要传输的字节数作为缓冲区大小
            let actual_buffer_size = byte_count as usize;

            // Adjust timeout based on data size
            let timeout = std::cmp::max(300u32, ((actual_buffer_size / (64 * 1024)) * 60) as u32);
            debug!(
                "Using timeout: {} seconds for {} bytes",
                timeout, actual_buffer_size
            );

            // 创建sense数据缓冲区用于分析
            let mut sense_buffer = [0u8; SENSE_INFO_LEN];

            let result = self.scsi_io_control(
                &cdb,
                Some(&mut buffer[..actual_buffer_size]),
                SCSI_IOCTL_DATA_IN,
                timeout,
                Some(&mut sense_buffer),
            )?;

            if result {
                debug!(
                    "Successfully read {} bytes directly (requested {} blocks)",
                    actual_buffer_size, block_count
                );
                Ok(block_count)
            } else {
                // 即使失败也分析sense数据确定实际传输的数据量
                debug!("READ(6) returned error, analyzing sense data for file mark detection");

                // 分析sense数据确定实际传输的数据量和是否遇到文件标记
                let (actual_blocks_read, is_file_mark) =
                    self.analyze_read_sense_data(&sense_buffer, byte_count)?;

                if is_file_mark {
                    info!(
                        "✅ File mark detected via sense data - read {} blocks before mark",
                        actual_blocks_read
                    );
                    Ok(actual_blocks_read)
                } else {
                    warn!(
                        "❌ READ(6) command failed with sense: {}",
                        self.parse_sense_data(&sense_buffer)
                    );
                    Err(crate::error::RustLtfsError::scsi(format!(
                        "Direct block read operation failed: {}",
                        self.parse_sense_data(&sense_buffer)
                    )))
                }
            }
        }

        #[cfg(not(windows))]
        {
            let _ = (block_count, buffer);
            Err(crate::error::RustLtfsError::unsupported(
                "Non-Windows platform",
            ))
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

            debug!(
                "Reading chunk: {} blocks (offset: {} bytes)",
                current_chunk, offset
            );

            // Read current chunk
            let chunk_buffer = &mut buffer
                [offset..(offset + (current_chunk * block_sizes::LTO_BLOCK_SIZE) as usize)];

            match self.read_blocks_direct(current_chunk, chunk_buffer) {
                Ok(read_count) => {
                    if read_count != current_chunk {
                        warn!(
                            "Partial chunk read: expected {}, got {}",
                            current_chunk, read_count
                        );
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

        info!(
            "Chunked read completed: {} of {} blocks",
            total_read, block_count
        );
        Ok(total_read)
    }

    /// Read blocks with retry mechanism for improved reliability


    /// Attempt to recover tape position after read error



    /// Write tape blocks (based on LTFSCopyGUI implementation)
    pub fn write_blocks(&self, block_count: u32, buffer: &[u8]) -> Result<u32> {
        debug!("Writing {} blocks to tape", block_count);

        // LTFSCopyGUI compatibility: write actual buffer length, not block_count * LTO_BLOCK_SIZE
        // This allows writing 524288-byte blocks (LTFSCopyGUI's plabel.blocksize) instead of 65536

        #[cfg(windows)]
        {
            let mut cdb = [0u8; 6];
            cdb[0] = scsi_commands::WRITE_6;

            // LTFSCopyGUI compatibility: Use variable length mode
            // Matches LTFSCopyGUI: cdbData = {&HA, 0, ...} - second byte is 0
            cdb[1] = 0x00; // Variable length mode like LTFSCopyGUI
                           // Transfer Length - LTFSCopyGUI compatibility: use actual buffer length
                           // LTFSCopyGUI: TapeUtils.Write(handle, Data, BytesReaded) writes BytesReaded bytes
            let byte_count = buffer.len() as u32;
            cdb[2] = ((byte_count >> 16) & 0xFF) as u8;
            cdb[3] = ((byte_count >> 8) & 0xFF) as u8;
            cdb[4] = (byte_count & 0xFF) as u8;
            // cdb[5] is control byte, leave as 0

            let data_length = buffer.len();
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
                Err(crate::error::RustLtfsError::scsi(
                    "Block write operation failed",
                ))
            }
        }

        #[cfg(not(windows))]
        {
            let _ = (block_count, buffer);
            Err(crate::error::RustLtfsError::unsupported(
                "Non-Windows platform",
            ))
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

    /// Read MAM (Medium Auxiliary Memory) attributes for capacity information


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

    /// ReadToFileMark - 读取数据直到遇到FileMark (完全对应LTFSCopyGUI的ReadToFileMark实现)
    /// 这个方法精确复制LTFSCopyGUI TapeUtils.ReadToFileMark的FileMark检测逻辑
    pub fn read_to_file_mark(&self, block_size_limit: u32) -> Result<Vec<u8>> {
        debug!(
            "🔧 ReadToFileMark: Starting with block_size_limit={} (LTFSCopyGUI compatible)",
            block_size_limit
        );

        #[cfg(windows)]
        {
            let mut buffer = Vec::new();
            let actual_block_limit = std::cmp::min(block_size_limit, block_sizes::LTO_BLOCK_SIZE);

            debug!("📊 Using actual block limit: {} bytes", actual_block_limit);

            loop {
                let mut sense_buffer = [0u8; SENSE_INFO_LEN];
                let mut read_buffer = vec![0u8; actual_block_limit as usize];

                // 使用READ(6)命令读取一个块
                let mut cdb = [0u8; 6];
                cdb[0] = scsi_commands::READ_6;
                cdb[1] = 0x00; // Variable length mode like LTFSCopyGUI

                let byte_count = actual_block_limit;
                cdb[2] = ((byte_count >> 16) & 0xFF) as u8;
                cdb[3] = ((byte_count >> 8) & 0xFF) as u8;
                cdb[4] = (byte_count & 0xFF) as u8;
                cdb[5] = 0x00;

                let result = self.scsi_io_control(
                    &cdb,
                    Some(&mut read_buffer),
                    SCSI_IOCTL_DATA_IN,
                    300,
                    Some(&mut sense_buffer),
                )?;

                // 🎯 精确复制LTFSCopyGUI的FileMark检测逻辑和DiffBytes计算
                // LTFSCopyGUI: Dim Add_Key As UInt16 = CInt(sense(12)) << 8 Or sense(13)
                let add_key = ((sense_buffer[12] as u16) << 8) | (sense_buffer[13] as u16);

                // 🔧 关键修复：实现LTFSCopyGUI的DiffBytes计算逻辑 (Line 638-641)
                // For i As Integer = 3 To 6: DiffBytes <<= 8: DiffBytes = DiffBytes Or sense(i)
                let mut diff_bytes: i32 = 0;
                for i in 3..=6 {
                    diff_bytes <<= 8;
                    diff_bytes |= sense_buffer[i] as i32;
                }

                debug!("🔍 Sense analysis: result={}, Add_Key=0x{:04X} (ASC=0x{:02X}, ASCQ=0x{:02X}), DiffBytes={}",
                      result, add_key, sense_buffer[12], sense_buffer[13], diff_bytes);
                debug!(
                    "🔍 Detailed sense analysis: result={}, DiffBytes={}, BlockSizeLimit={}",
                    result, diff_bytes, block_size_limit
                );

                if result {
                    // 读取成��，检查是否需要自动回退 (LTFSCopyGUI Line 644-648)
                    let block_size_limit_i32 = block_size_limit as i32;
                    // 🔧 关键修复：使用LTFSCopyGUI的GlobalBlockLimit值 (1048576)
                    let global_block_limit = 1048576i32; // LTFSCopyGUI默认值

                    debug!("🔍 Auto-backtrack condition check: DiffBytes={}, DiffBytes<0={}, BlockSize={}, (BlockSize-DiffBytes)={}, GlobalLimit={}, Condition={}",
                         diff_bytes, diff_bytes < 0, block_size_limit_i32,
                         block_size_limit_i32 - diff_bytes, global_block_limit,
                         diff_bytes < 0 && (block_size_limit_i32 - diff_bytes) < global_block_limit);

                    if diff_bytes < 0 && (block_size_limit_i32 - diff_bytes) < global_block_limit {
                        info!(
                            "🔄 LTFSCopyGUI auto-backtrack triggered: DiffBytes={}, condition met",
                            diff_bytes
                        );

                        // Additional diagnostic logging to aid debugging when auto-backtrack triggers
                        // Dump sense buffer hex, a preview of the read buffer, and write a temporary reread dump
                        {
                            // Sense hex
                            let sense_hex = sense_buffer
                                .iter()
                                .map(|b| format!("{:02X}", b))
                                .collect::<Vec<_>>()
                                .join(" ");
                            debug!("🔍 Diagnostic - sense buffer HEX: {}", sense_hex);

                            // Preview of the read_buffer (first and last up to 64 bytes)
                            let preview_len = std::cmp::min(64, read_buffer.len());
                            if preview_len > 0 {
                                debug!(
                                    "🔍 Diagnostic - read_buffer preview (first {} bytes): {:02X?}",
                                    preview_len,
                                    &read_buffer[..preview_len]
                                );
                            } else {
                                debug!("🔍 Diagnostic - read_buffer is empty for this iteration");
                            }
                        }

                        // 🎯 关键修复：实现LTFSCopyGUI的自动回退逻辑（同时增强诊断信息）
                        if let Ok(current_pos) = self.read_position() {
                            if current_pos.block_number > 0 {
                                info!(
                                    "🔧 Auto-backtrack: moving from P{} B{} to P{} B{}",
                                    current_pos.partition,
                                    current_pos.block_number,
                                    current_pos.partition,
                                    current_pos.block_number - 1
                                );

                                // 回退到前一个Block (use LOCATE(16) to match LTFSCopyGUI behavior)
                                self.locate(
                                    current_pos.block_number - 1,
                                    current_pos.partition,
                                    LocateDestType::Block,
                                )?;

                                // 再次记录回退后位置做对比诊断
                                if let Ok(pos_after_locate) = self.read_position() {
                                    info!(
                                        "🔍 Diagnostic - position after locate: P{} B{} FM{}",
                                        pos_after_locate.partition,
                                        pos_after_locate.block_number,
                                        pos_after_locate.file_number
                                    );
                                } else {
                                    warn!("🔍 Diagnostic - failed to read position after locate");
                                }

                                // 🔄 重新读取 (使用调整后的block size)
                                let adjusted_block_size =
                                    std::cmp::max(0, block_size_limit_i32 - diff_bytes) as u32;
                                let adjusted_limit =
                                    std::cmp::min(adjusted_block_size, actual_block_limit);

                                info!(
                                    "🔧 Re-reading with adjusted block size: {} bytes (was {})",
                                    adjusted_limit, actual_block_limit
                                );

                                let mut adjusted_buffer = vec![0u8; adjusted_limit as usize];
                                let reread_result = self.scsi_io_control(
                                    &cdb,
                                    Some(&mut adjusted_buffer),
                                    SCSI_IOCTL_DATA_IN,
                                    300,
                                    Some(&mut sense_buffer),
                                )?;

                                // Log sense buffer after reread (hex)
                                let sense_hex_after = sense_buffer
                                    .iter()
                                    .map(|b| format!("{:02X}", b))
                                    .collect::<Vec<_>>()
                                    .join(" ");
                                debug!(
                                    "🔍 Diagnostic - sense buffer after reread HEX: {}",
                                    sense_hex_after
                                );

                                if reread_result && !adjusted_buffer.is_empty() {
                                    // Write a short preview of reread buffer to logs for debugging
                                    let preview_len = std::cmp::min(128, adjusted_buffer.len());
                                    debug!(
                                            "🔍 Diagnostic - reread buffer preview (first {} bytes): {:02X?}",
                                            preview_len,
                                            &adjusted_buffer[..preview_len]
                                        );

                                    // Try to persist the reread buffer to a temp file for offline analysis
                                    // Only write dumps in debug builds to avoid polluting production temp dirs
                                    #[cfg(debug_assertions)]
                                    {
                                        // Use a simple timestamp-based name
                                        let dump_filename = std::format!(
                                            "reread_dump_{}.bin",
                                            std::time::SystemTime::now()
                                                .duration_since(std::time::UNIX_EPOCH)
                                                .map(|d| d.as_micros())
                                                .unwrap_or(0)
                                        );
                                        let dump_path = std::env::temp_dir().join(dump_filename);
                                        if let Err(e) = std::fs::write(&dump_path, &adjusted_buffer)
                                        {
                                            warn!(
                                                "🔍 Diagnostic - failed to write reread dump: {}",
                                                e
                                            );
                                        } else {
                                            info!(
                                                "🔍 Diagnostic - reread dump written to: {:?}",
                                                dump_path
                                            );
                                        }
                                    }
                                    #[cfg(not(debug_assertions))]
                                    {
                                        debug!(
                                            "🔍 Diagnostic - reread dump skipped (release build)"
                                        );
                                    }

                                    // Replace the previously-read block data with the adjusted reread result
                                    // (match LTFSCopyGUI recursive ReadBlock semantics instead of appending)
                                    read_buffer = adjusted_buffer;
                                    info!(
                                        "✅ Auto-backtrack successful: {} bytes read (replaced previous block) from P{} B{}",
                                        read_buffer.len(),
                                        current_pos.partition,
                                        current_pos.block_number - 1
                                    );
                                } else {
                                    warn!("⚠️ Auto-backtrack reread returned no data or failed");
                                }

                                // 重新计算add_key用于FileMark检测
                                let reread_add_key =
                                    ((sense_buffer[12] as u16) << 8) | (sense_buffer[13] as u16);
                                debug!("🔍 Re-read Add_Key: 0x{:04X}", reread_add_key);

                                // 🎯 使用重新读取后的add_key进行FileMark检测
                                if reread_add_key >= 1 && reread_add_key != 4 {
                                    info!("FileMark detected after auto-backtrack: Add_Key=0x{:04X}", reread_add_key);
                                    break;
                                }
                                continue;
                            } else {
                                debug!("⚠️ Cannot backtrack: already at block 0");
                            }
                        }
                    }

                    // 正常情况：将数据添加到缓冲区（使用当前的 `read_buffer`，它可能已经被 auto-backtrack 的重新读取替换）
                    // 这里故意使用 `read_buffer` 变量以保证如果 auto-backtrack 已经将其替换为调整后的数据，
                    // 我们将追加的是替换后的数据（匹配 LTFSCopyGUI 的行为语义：使用重读结果）。
                    if !read_buffer.is_empty() {
                        // Append the current read_buffer (which may have been replaced by adjusted reread result)
                        buffer.extend_from_slice(&read_buffer);
                        debug!(
                            "📝 Added {} bytes to buffer, total: {} bytes",
                            read_buffer.len(),
                            buffer.len()
                        );
                    }
                }

                // 🎯 关键的FileMark检测规则 (精确对应LTFSCopyGUI)
                // LTFSCopyGUI: If (Add_Key >= 1 And Add_Key <> 4) Then Exit While
                if add_key >= 1 && add_key != 4 {
                    debug!("🎯 FileMark detected: Add_Key=0x{:04X} matches criteria (>=1 and !=4)", add_key);
                    break;
                }

                // 如果没有检测到FileMark且没有读取到数据，可能到达了EOD
                if !result && read_buffer.is_empty() {
                    debug!("📄 No more data available, stopping read");
                    break;
                }
            }

            debug!(
                "✅ ReadToFileMark completed: {} total bytes read using LTFSCopyGUI method",
                buffer.len()
            );
            Ok(buffer)
        }

        #[cfg(not(windows))]
        {
            let _ = block_size_limit;
            Err(crate::error::RustLtfsError::unsupported(
                "Non-Windows platform",
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

    /// Enhanced locate_block method that uses the comprehensive locate function


    /// Set drive type for optimization


    /// Set partition support flag


    /// Get current drive type


    /// MODE SENSE command to read partition page 0x11 (对应LTFSCopyGUI的ModeSense实现)
    /// 这个方法复制LTFSCopyGUI的精确实现：TapeUtils.ModeSense(handle, &H11)
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





}


/// Implement Drop trait to ensure SCSI interface is properly cleaned up
impl Drop for ScsiInterface {
    fn drop(&mut self) {
        debug!("SCSI interface cleanup completed");
    }
}
