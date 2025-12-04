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


    /// Read MAM (Medium Auxiliary Memory) attributes for capacity information

    /// Read MAM (Medium Auxiliary Memory) attributes for capacity information

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


    /// Enhanced locate_block method that uses the comprehensive locate function

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
