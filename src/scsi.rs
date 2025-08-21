use crate::error::Result;
use tracing::{debug, error, warn};
use std::ffi::CString;
use std::mem;

#[cfg(windows)]
use winapi::{
    shared::{
        minwindef::{BOOL, DWORD, ULONG, USHORT, UCHAR},
        ntdef::{HANDLE, PVOID},
    },
    um::{
        fileapi::{CreateFileA, OPEN_EXISTING},
        handleapi::{CloseHandle, INVALID_HANDLE_VALUE},
        ioapiset::DeviceIoControl,
        winnt::{FILE_ATTRIBUTE_NORMAL, GENERIC_READ, GENERIC_WRITE},
        errhandlingapi::GetLastError,
    },
};

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

/// SCSI 命令数据块 (CDB) 结构
#[repr(C)]
#[derive(Debug, Clone)]
pub struct ScsiCdb {
    pub operation_code: u8,
    pub misc_cdb_flags: u8,
    pub logical_block_address: u32,
    pub transfer_length: u16,
    pub control: u8,
    pub reserved: [u8; 3],
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

/// SCSI 查询数据结构
#[repr(C)]
pub struct InquiryData {
    pub device_type: u8,
    pub device_type_modifier: u8,
    pub versions: u8,
    pub response_data_format: u8,
    pub additional_length: u8,
    pub reserved1: [u8; 2],
    pub misc_flags: u8,
    pub vendor_id: [u8; 8],
    pub product_id: [u8; 16],
    pub product_revision: [u8; 4],
}

impl ScsiInterface {
    /// 创建新的 SCSI 接口实例
    pub fn new() -> Self {
        Self {
            device_handle: None,
        }
    }
    
    /// 打开磁带设备
    pub fn open_device(&mut self, device_path: &str) -> Result<()> {
        debug!("打开磁带设备: {}", device_path);
        
        #[cfg(windows)]
        {
            let path_cstring = CString::new(device_path)
                .map_err(|e| crate::error::RustLtfsError::system(format!("设备路径转换错误: {}", e)))?;
            
            unsafe {
                let handle = CreateFileA(
                    path_cstring.as_ptr(),
                    GENERIC_READ | GENERIC_WRITE,
                    0,
                    std::ptr::null_mut(),
                    OPEN_EXISTING,
                    FILE_ATTRIBUTE_NORMAL,
                    std::ptr::null_mut(),
                );
                
                if handle == INVALID_HANDLE_VALUE {
                    let error_code = GetLastError();
                    return Err(crate::error::RustLtfsError::system(
                        format!("无法打开设备 {}: Windows 错误码 0x{:08X}", device_path, error_code)
                    ));
                }
                
                self.device_handle = Some(DeviceHandle {
                    handle,
                    device_path: device_path.to_string(),
                });
                
                debug!("设备打开成功: {}", device_path);
            }
        }
        
        #[cfg(not(windows))]
        {
            return Err(crate::error::RustLtfsError::unsupported("非 Windows 平台"));
        }
        
        Ok(())
    }
    
    /// 发送 SCSI 查询命令
    pub fn inquiry(&self) -> Result<InquiryData> {
        debug!("发送 SCSI INQUIRY 命令");
        
        #[cfg(windows)]
        {
            if let Some(ref device) = self.device_handle {
                unsafe {
                    let mut inquiry_data = InquiryData {
                        device_type: 0,
                        device_type_modifier: 0,
                        versions: 0,
                        response_data_format: 0,
                        additional_length: 0,
                        reserved1: [0; 2],
                        misc_flags: 0,
                        vendor_id: [0; 8],
                        product_id: [0; 16],
                        product_revision: [0; 4],
                    };
                    
                    // TODO: 实现具体的 SCSI INQUIRY 命令发送逻辑
                    // 这里需要使用 DeviceIoControl 和 SCSI_PASS_THROUGH 结构
                    
                    warn!("SCSI INQUIRY 命令实现待完成");
                    
                    Ok(inquiry_data)
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
    
    /// 检查磁带媒体状态
    pub fn check_media_status(&self) -> Result<MediaStatus> {
        debug!("检查媒体状态");
        
        // TODO: 实现媒体状态检查
        warn!("媒体状态检查功能待实现");
        
        Ok(MediaStatus::Ready)
    }
    
    /// 发送通用 SCSI 命令
    pub fn send_scsi_command(&self, cdb: &ScsiCdb, data_buffer: Option<&mut [u8]>) -> Result<()> {
        debug!("发送 SCSI 命令: 操作码 0x{:02X}", cdb.operation_code);
        
        #[cfg(windows)]
        {
            if let Some(ref device) = self.device_handle {
                // TODO: 实现 SCSI 命令发送逻辑
                // 需要使用 SCSI_PASS_THROUGH_DIRECT 结构
                warn!("SCSI 命令发送功能待实现");
                Ok(())
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

/// 媒体状态枚举
#[derive(Debug, Clone, PartialEq)]
pub enum MediaStatus {
    Ready,
    NotReady,
    WriteProtected,
    NoMedia,
    Error(String),
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

/// SCSI 命令常量定义
pub mod scsi_commands {
    pub const TEST_UNIT_READY: u8 = 0x00;
    pub const INQUIRY: u8 = 0x12;
    pub const READ_6: u8 = 0x08;
    pub const WRITE_6: u8 = 0x0A;
    pub const READ_10: u8 = 0x28;
    pub const WRITE_10: u8 = 0x2A;
    pub const MODE_SENSE_6: u8 = 0x1A;
    pub const MODE_SELECT_6: u8 = 0x15;
    pub const LOAD_UNLOAD: u8 = 0x1B;
    pub const REWIND: u8 = 0x01;
    pub const READ_POSITION: u8 = 0x34;
    pub const SPACE: u8 = 0x11;
}

/// 辅助函数：将字节数组转换为安全的字符串
pub fn bytes_to_string(bytes: &[u8]) -> String {
    let end = bytes.iter().position(|&b| b == 0).unwrap_or(bytes.len());
    String::from_utf8_lossy(&bytes[..end]).trim().to_string()
}