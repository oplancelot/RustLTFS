use crate::error::Result;
use tracing::{debug, warn};
use std::ffi::CString;

#[cfg(windows)]
use winapi::{
    shared::{
        minwindef::{DWORD, ULONG, USHORT, UCHAR},
        ntdef::{HANDLE, PVOID},
    },
    um::{
        fileapi::{CreateFileA, OPEN_EXISTING},
        handleapi::{CloseHandle, INVALID_HANDLE_VALUE},
        ioapiset::DeviceIoControl,
        winnt::{GENERIC_READ, GENERIC_WRITE, FILE_SHARE_DELETE, FILE_SHARE_READ, FILE_SHARE_WRITE},
        errhandlingapi::GetLastError,
    },
};

// 定义 IOCTL_SCSI_PASS_THROUGH_DIRECT 常量
#[cfg(windows)]
const IOCTL_SCSI_PASS_THROUGH_DIRECT: u32 = 0x0004D014;

// 非 Windows 平台的类型别名
#[cfg(not(windows))]
type UCHAR = u8;
#[cfg(not(windows))]
type USHORT = u16;
#[cfg(not(windows))]
type ULONG = u32;
#[cfg(not(windows))]
type DWORD = u32;
#[cfg(not(windows))]
type PVOID = *mut std::ffi::c_void;
#[cfg(not(windows))]
type HANDLE = *mut std::ffi::c_void;

// SCSI 常量定义 (基于 C 代码)
const SENSE_INFO_LEN: usize = 64;
const TC_MP_PC_CURRENT: u8 = 0x00;
const TC_MP_MEDIUM_CONFIGURATION: u8 = 0x1D;

// SCSI 操作码常量
const SCSIOP_READ_POSITION: u8 = 0x34;
const SCSIOP_MODE_SENSE10: u8 = 0x5A;
const SCSIOP_INQUIRY: u8 = 0x12;

// SCSI 数据方向
const SCSI_IOCTL_DATA_IN: u8 = 1;
const SCSI_IOCTL_DATA_OUT: u8 = 0;
const SCSI_IOCTL_DATA_UNSPECIFIED: u8 = 2;

/// SCSI Pass Through Direct 结构 (对应 C 代码中的 SCSI_PASS_THROUGH_DIRECT)
#[repr(C)]
#[derive(Debug)]
struct ScsiPassThroughDirect {
    length: USHORT,
    scsi_status: UCHAR,
    path_id: UCHAR,
    target_id: UCHAR,
    lun: UCHAR,
    cdb_length: UCHAR,
    sense_info_length: UCHAR,
    data_in: UCHAR,
    data_transfer_length: ULONG,
    timeout_value: ULONG,
    data_buffer: PVOID,
    sense_info_offset: ULONG,
    cdb: [UCHAR; 16],
}

/// 媒体类型枚举，基于 C 代码中的媒体类型检测
#[derive(Debug, Clone, PartialEq)]
pub enum MediaType {
    NoTape,
    Lto3Rw,      // 0x0044
    Lto3Worm,    // 0x0144  
    Lto3Ro,      // 0x0244
    Lto4Rw,      // 0x0046
    Lto4Worm,    // 0x0146
    Lto4Ro,      // 0x0246
    Lto5Rw,      // 0x0058
    Lto5Worm,    // 0x0158
    Lto5Ro,      // 0x0258
    Lto6Rw,      // 0x005A
    Lto6Worm,    // 0x015A
    Lto6Ro,      // 0x025A
    Lto7Rw,      // 0x005C
    Lto7Worm,    // 0x015C
    Lto7Ro,      // 0x025C
    Lto8Rw,      // 0x005E
    Lto8Worm,    // 0x015E
    Lto8Ro,      // 0x025E
    LtoM8Rw,     // 0x005D
    LtoM8Worm,   // 0x015D
    LtoM8Ro,     // 0x025D
    Unknown(u16),
}

impl MediaType {
    /// 从媒体类型代码转换为媒体类型
    fn from_media_type_code(code: u16) -> Self {
        match code {
            0x0044 => MediaType::Lto3Rw,
            0x0144 => MediaType::Lto3Worm,
            0x0244 => MediaType::Lto3Ro,
            0x0046 => MediaType::Lto4Rw,
            0x0146 => MediaType::Lto4Worm,
            0x0246 => MediaType::Lto4Ro,
            0x0058 => MediaType::Lto5Rw,
            0x0158 => MediaType::Lto5Worm,
            0x0258 => MediaType::Lto5Ro,
            0x005A => MediaType::Lto6Rw,
            0x015A => MediaType::Lto6Worm,
            0x025A => MediaType::Lto6Ro,
            0x005C => MediaType::Lto7Rw,
            0x015C => MediaType::Lto7Worm,
            0x025C => MediaType::Lto7Ro,
            0x005E => MediaType::Lto8Rw,
            0x015E => MediaType::Lto8Worm,
            0x025E => MediaType::Lto8Ro,
            0x005D => MediaType::LtoM8Rw,
            0x015D => MediaType::LtoM8Worm,
            0x025D => MediaType::LtoM8Ro,
            _ => MediaType::Unknown(code),
        }
    }
    
    /// 转换为描述字符串
    pub fn description(&self) -> &'static str {
        match self {
            MediaType::NoTape => "No tape loaded",
            MediaType::Lto3Rw => "LTO3 RW",
            MediaType::Lto3Worm => "LTO3 WORM",
            MediaType::Lto3Ro => "LTO3 RO",
            MediaType::Lto4Rw => "LTO4 RW",
            MediaType::Lto4Worm => "LTO4 WORM",
            MediaType::Lto4Ro => "LTO4 RO",
            MediaType::Lto5Rw => "LTO5 RW",
            MediaType::Lto5Worm => "LTO5 WORM",
            MediaType::Lto5Ro => "LTO5 RO",
            MediaType::Lto6Rw => "LTO6 RW",
            MediaType::Lto6Worm => "LTO6 WORM",
            MediaType::Lto6Ro => "LTO6 RO",
            MediaType::Lto7Rw => "LTO7 RW",
            MediaType::Lto7Worm => "LTO7 WORM",
            MediaType::Lto7Ro => "LTO7 RO",
            MediaType::Lto8Rw => "LTO8 RW",
            MediaType::Lto8Worm => "LTO8 WORM",
            MediaType::Lto8Ro => "LTO8 RO",
            MediaType::LtoM8Rw => "LTOM8 RW",
            MediaType::LtoM8Worm => "LTOM8 WORM",
            MediaType::LtoM8Ro => "LTOM8 RO",
            MediaType::Unknown(_) => "Unknown media type",
        }
    }
}

/// SCSI 操作结构体，封装低级 SCSI 命令
pub struct ScsiInterface {
    device_handle: Option<DeviceHandle>,
}

/// 设备句柄包装器，确保资源正确释放
struct DeviceHandle {
    #[cfg(windows)]
    handle: HANDLE,
    device_path: String,
}

/// 磁带驱动器信息结构，对应 C 代码中的 TAPE_DRIVE
#[derive(Debug, Clone)]
pub struct TapeDriveInfo {
    pub vendor_id: String,
    pub product_id: String,
    pub serial_number: String,
    pub device_index: u32,
    pub device_path: String,
}

impl ScsiInterface {
    /// 创建新的 SCSI 接口实例
    pub fn new() -> Self {
        Self {
            device_handle: None,
        }
    }
    
    /// 打开磁带设备 (基于 C 代码中的 CreateFile 调用)
    pub fn open_device(&mut self, device_path: &str) -> Result<()> {
        debug!("打开磁带设备: {}", device_path);
        
        #[cfg(windows)]
        {
            // 构建完整的设备路径，类似 C 代码中的 "\\\\.\\TAPE0" 格式
            let full_path = if device_path.starts_with("\\\\\\\\\\.\\\\") {
                device_path.to_string()
            } else {
                format!("\\\\\\\\\\.\\\\{}", device_path)
            };
            
            let path_cstring = CString::new(full_path.clone())
                .map_err(|e| crate::error::RustLtfsError::system(format!("设备路径转换错误: {}", e)))?;
            
            unsafe {
                let handle = CreateFileA(
                    path_cstring.as_ptr(),
                    GENERIC_READ | GENERIC_WRITE,
                    FILE_SHARE_DELETE | FILE_SHARE_READ | FILE_SHARE_WRITE, // 基于 C 代码
                    std::ptr::null_mut(),
                    OPEN_EXISTING,
                    0, // 不使用 FILE_ATTRIBUTE_NORMAL，基于 C 代码
                    std::ptr::null_mut(),
                );
                
                if handle == INVALID_HANDLE_VALUE {
                    let error_code = GetLastError();
                    return Err(crate::error::RustLtfsError::system(
                        format!("无法打开设备 {}: Windows 错误码 0x{:08X}", full_path, error_code)
                    ));
                }
                
                self.device_handle = Some(DeviceHandle {
                    handle,
                    device_path: full_path,
                });
                
                debug!("设备打开成功: {}", device_path);
                Ok(())
            }
        }
        
        #[cfg(not(windows))]
        {
            Err(crate::error::RustLtfsError::unsupported("非 Windows 平台"))
        }
    }
    
    /// 发送 SCSI 命令通用接口 (基于 C 代码中的 ScsiIoControl)
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
                
                // 创建 SCSI Pass Through Direct 缓冲区
                let mut scsi_buffer = vec![0u8; std::mem::size_of::<ScsiPassThroughDirect>() + SENSE_INFO_LEN];
                
                unsafe {
                    let scsi_direct = scsi_buffer.as_mut_ptr() as *mut ScsiPassThroughDirect;
                    std::ptr::write_bytes(scsi_direct, 0, 1);
                    
                    (*scsi_direct).length = std::mem::size_of::<ScsiPassThroughDirect>() as USHORT;
                    (*scsi_direct).cdb_length = cdb.len() as UCHAR;
                    (*scsi_direct).data_buffer = data_ptr;
                    (*scsi_direct).sense_info_length = SENSE_INFO_LEN as UCHAR;
                    (*scsi_direct).sense_info_offset = std::mem::size_of::<ScsiPassThroughDirect>() as ULONG;
                    (*scsi_direct).data_transfer_length = buffer_length;
                    (*scsi_direct).timeout_value = timeout;
                    (*scsi_direct).data_in = data_in;
                    
                    // 复制 CDB
                    std::ptr::copy_nonoverlapping(cdb.as_ptr(), (*scsi_direct).cdb.as_mut_ptr(), cdb.len());
                    
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
                    
                    // 复制 sense buffer 如果提供
                    if let Some(sense_buf) = sense_buffer {
                        std::ptr::copy_nonoverlapping(
                            scsi_buffer.as_ptr().add(std::mem::size_of::<ScsiPassThroughDirect>()),
                            sense_buf.as_mut_ptr(),
                            SENSE_INFO_LEN,
                        );
                    }
                    
                    if !result {
                        let error_code = GetLastError();
                        debug!("SCSI 命令失败: Windows 错误码 0x{:08X}", error_code);
                        return Ok(false);
                    }
                    
                    Ok(true)
                }
            } else {
                Err(crate::error::RustLtfsError::scsi("设备未打开"))
            }
        }
        
        #[cfg(not(windows))]
        {
            // 非 Windows 平台使用参数以避免警告
            let _ = (cdb, data_buffer, data_in, timeout, sense_buffer);
            Err(crate::error::RustLtfsError::unsupported("非 Windows 平台"))
        }
    }
    
    /// 检查磁带媒体状态 (基于 C 代码中的 TapeCheckMedia 函数)
    pub fn check_media_status(&self) -> Result<MediaType> {
        debug!("检查磁带媒体状态");
        
        #[cfg(windows)]
        {
            // 第一步：使用 READ POSITION 检查是否有磁带
            // "There doesn't appear to be a direct way to tell if there's anything in the drive,
            // so instead we just try and read the position which won't fuck up a mounted LTFS volume."
            let mut cdb = [0u8; 10];
            let mut data_buffer = [0u8; 64];
            let mut sense_buffer = [0u8; SENSE_INFO_LEN];
            
            // 设置 read POSITION CDB
            cdb[0] = SCSIOP_READ_POSITION; // Operation Code
            cdb[1] = 0x03; // Reserved1，基于 C 代码
            
            let result = self.scsi_io_control(
                &cdb,
                Some(&mut data_buffer),
                SCSI_IOCTL_DATA_IN,
                300, // 300 秒超时，基于 C 代码
                Some(&mut sense_buffer),
            )?;
            
            if !result {
                return Err(crate::error::RustLtfsError::scsi("read_position 命令失败"));
            }
            
            // 检查 sense buffer 是否表示没有磁带
            // C 代码: if (((senseBuffer[2] & 0x0F) == 0x02) && (senseBuffer[12] == 0x3A) && (senseBuffer[13] == 0x00))
            if (sense_buffer[2] & 0x0F) == 0x02 && sense_buffer[12] == 0x3A && sense_buffer[13] == 0x00 {
                debug!("检测到没有磁带");
                return Ok(MediaType::NoTape);
            }
            
            // 第二步：使用 MODE SENSE 10 获取媒体类型
            // "This will only tell us the *last* tape that was in the drive, which is why we have to do the above check first"
            cdb.fill(0);
            data_buffer.fill(0);
            
            cdb[0] = SCSIOP_MODE_SENSE10; // Operation Code
            cdb[2] = TC_MP_MEDIUM_CONFIGURATION; // Page Code
            cdb[2] |= (TC_MP_PC_CURRENT << 6); // PC field
            cdb[7] = (data_buffer.len() >> 8) as u8; // Allocation Length MSB
            cdb[8] = (data_buffer.len() & 0xFF) as u8; // Allocation Length LSB
            
            let result = self.scsi_io_control(
                &cdb,
                Some(&mut data_buffer),
                SCSI_IOCTL_DATA_IN,
                300,
                None,
            )?;
            
            if !result {
                warn!("MODE_SENSE10 命令失败，但磁带可能存在");
                return Ok(MediaType::Unknown(0));
            }
            
            // 解析媒体类型，基于 C 代码逻辑
            let mut media_type = data_buffer[8] as u16 + ((data_buffer[18] as u16 & 0x01) << 8);
            
            // 检查是否不是 WORM 类型，基于 C 代码注释
            if (media_type & 0x100) == 0 {
                media_type |= ((data_buffer[3] as u16 & 0x80) << 2);
            }
            
            debug!("检测到媒体类型代码: 0x{:04X}", media_type);
            
            Ok(MediaType::from_media_type_code(media_type))
        }
        
        #[cfg(not(windows))]
        {
            Err(crate::error::RustLtfsError::unsupported("非 Windows 平台"))
        }
    }
    
    /// 磁带装载 (基于 C 代码中的 TapeLoad 函数)
    pub fn load_tape(&self) -> Result<bool> {
        debug!("装载磁带");
        
        #[cfg(windows)]
        {
            let mut cdb = [0u8; 6];
            cdb[0] = 0x1B; // SCSIOP_LOAD_UNLOAD，基于 C 代码
            cdb[4] = 1; // Start = 1，基于 C 代码
            
            let result = self.scsi_io_control(
                &cdb,
                None,
                SCSI_IOCTL_DATA_UNSPECIFIED,
                300,
                None,
            )?;
            
            Ok(result)
        }
        
        #[cfg(not(windows))]
        {
            Err(crate::error::RustLtfsError::unsupported("非 Windows 平台"))
        }
    }
    
    /// 磁带弹出 (基于 C 代码中的 TapeEject 函数逻辑)
    pub fn eject_tape(&self) -> Result<bool> {
        debug!("弹出磁带");
        
        #[cfg(windows)]
        {
            if let Some(ref device) = self.device_handle {
                unsafe {
                    let mut bytes_returned: DWORD = 0;
                    
                    // 1. 锁定卷
                    let lock_result = DeviceIoControl(
                        device.handle,
                        0x00090018, // FSCTL_LOCK_VOLUME
                        std::ptr::null_mut(),
                        0,
                        std::ptr::null_mut(),
                        0,
                        &mut bytes_returned,
                        std::ptr::null_mut(),
                    ) != 0;
                    
                    if !lock_result {
                        warn!("锁定卷失败");
                        return Ok(false);
                    }
                    
                    // 2. 卸载卷
                    let dismount_result = DeviceIoControl(
                        device.handle,
                        0x00090020, // FSCTL_DISMOUNT_VOLUME
                        std::ptr::null_mut(),
                        0,
                        std::ptr::null_mut(),
                        0,
                        &mut bytes_returned,
                        std::ptr::null_mut(),
                    ) != 0;
                    
                    if !dismount_result {
                        warn!("卸载卷失败");
                        return Ok(false);
                    }
                    
                    // 3. 弹出媒体
                    let eject_result = DeviceIoControl(
                        device.handle,
                        0x002D4808, // IOCTL_DISK_EJECT_MEDIA
                        std::ptr::null_mut(),
                        0,
                        std::ptr::null_mut(),
                        0,
                        &mut bytes_returned,
                        std::ptr::null_mut(),
                    ) != 0;
                    
                    Ok(eject_result)
                }
            } else {
                Err(crate::error::RustLtfsError::scsi("设备未打开"))
            }
        }
        
        #[cfg(not(windows))]
        {
            Err(crate::error::RustLtfsError::unsupported("非 Windows 平台"))
        }
    }
}

/// 便捷函数：直接检查指定设备的媒体状态
pub fn check_tape_media(tape_drive: &str) -> Result<MediaType> {
    let mut scsi = ScsiInterface::new();
    scsi.open_device(tape_drive)?;
    scsi.check_media_status()
}

/// 便捷函数：直接装载指定设备的磁带
pub fn load_tape(tape_drive: &str) -> Result<bool> {
    let mut scsi = ScsiInterface::new();
    scsi.open_device(tape_drive)?;
    scsi.load_tape()
}

/// 便捷函数：直接弹出指定设备的磁带  
pub fn eject_tape(tape_drive: &str) -> Result<bool> {
    let mut scsi = ScsiInterface::new();
    scsi.open_device(tape_drive)?;
    scsi.eject_tape()
}

/// 实现 Drop trait 确保设备句柄正确关闭
impl Drop for DeviceHandle {
    fn drop(&mut self) {
        #[cfg(windows)]
        unsafe {
            if self.handle != INVALID_HANDLE_VALUE {
                CloseHandle(self.handle);
                debug!("设备句柄已关闭: {}", self.device_path);
            }
        }
    }
}

/// 实现 Drop trait 确保 SCSI 接口正确清理
impl Drop for ScsiInterface {
    fn drop(&mut self) {
        debug!("SCSI 接口清理完成");
    }
}

/// SCSI 命令常量定义 (基于 C 代码)
pub mod scsi_commands {
    pub const TEST_UNIT_READY: u8 = 0x00;
    pub const INQUIRY: u8 = 0x12;
    pub const read_6: u8 = 0x08;
    pub const WRITE_6: u8 = 0x0A;
    pub const read_10: u8 = 0x28;
    pub const WRITE_10: u8 = 0x2A;
    pub const MODE_SENSE_6: u8 = 0x1A;
    pub const MODE_SENSE_10: u8 = 0x5A;
    pub const MODE_SELECT_6: u8 = 0x15;
    pub const LOAD_UNLOAD: u8 = 0x1B;
    pub const REWIND: u8 = 0x01;
    pub const read_POSITION: u8 = 0x34;
    pub const SPACE: u8 = 0x11;
    pub const START_STOP_UNIT: u8 = 0x1B;
}

/// 辅助函数：将字节数组转换为安全的字符串
pub fn bytes_to_string(bytes: &[u8]) -> String {
    let end = bytes.iter().position(|&b| b == 0).unwrap_or(bytes.len());
    String::from_utf8_lossy(&bytes[..end]).trim().to_string()
}