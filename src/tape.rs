use crate::error::Result;
use crate::scsi::{ScsiInterface, TapeDriveInfo, InquiryData, bytes_to_string};
use tracing::{info, debug, error, warn};

/// 磁带设备信息结构
#[derive(Debug, Clone)]
pub struct TapeDevice {
    pub path: String,
    pub vendor: String,
    pub model: String,
    pub serial: String,
    pub status: TapeStatus,
}

/// 磁带状态枚举
#[derive(Debug, Clone, PartialEq)]
pub enum TapeStatus {
    Ready,
    NotReady,
    WriteProtected,
    NoTape,
    Error(String),
}

/// 列出系统中可用的磁带设备
pub async fn list_devices(detailed: bool) -> Result<()> {
    info!("开始扫描磁带设备...");
    
    #[cfg(windows)]
    {
        list_windows_tape_devices(detailed).await
    }
    
    #[cfg(not(windows))]
    {
        error!("此工具目前仅支持 Windows 平台");
        Err(crate::error::RustLtfsError::unsupported("非 Windows 平台"))
    }
}

/// 获取指定设备的详细信息
pub async fn get_device_info(device: String) -> Result<()> {
    info!("获取设备信息: {}", device);
    
    let mut scsi = ScsiInterface::new();
    scsi.open_device(&device)?;
    
    match scsi.inquiry() {
        Ok(inquiry_data) => {
            let vendor = bytes_to_string(&inquiry_data.vendor_id);
            let product = bytes_to_string(&inquiry_data.product_id);
            let revision = bytes_to_string(&inquiry_data.product_revision);
            
            println!("设备信息:");
            println!("  厂商: {}", vendor);
            println!("  型号: {}", product);
            println!("  版本: {}", revision);
            println!("  设备类型: 0x{:02X}", inquiry_data.device_type);
            
            Ok(())
        }
        Err(e) => {
            error!("获取设备信息失败: {}", e);
            Err(e)
        }
    }
}

/// 检查设备状态
pub async fn get_device_status(device: String) -> Result<()> {
    info!("检查设备状态: {}", device);
    
    let mut scsi = ScsiInterface::new();
    scsi.open_device(&device)?;
    
    let status = scsi.check_media_status()?;
    
    match status {
        crate::scsi::MediaStatus::Ready => println!("设备状态: 就绪"),
        crate::scsi::MediaStatus::NotReady => println!("设备状态: 未就绪"),
        crate::scsi::MediaStatus::WriteProtected => println!("设备状态: 写保护"),
        crate::scsi::MediaStatus::NoMedia => println!("设备状态: 无媒体"),
        crate::scsi::MediaStatus::Error(msg) => println!("设备状态: 错误 - {}", msg),
    }
    
    Ok(())
}

#[cfg(windows)]
async fn list_windows_tape_devices(detailed: bool) -> Result<()> {
    use winapi::um::fileapi::{CreateFileA, OPEN_EXISTING};
    use winapi::um::winnt::{FILE_ATTRIBUTE_NORMAL, GENERIC_READ, GENERIC_WRITE};
    use std::ffi::CString;
    
    debug!("扫描 Windows 磁带设备");
    
    // 检查常见的磁带设备路径
    let tape_paths = vec![
        r"\\.\TAPE0",
        r"\\.\TAPE1", 
        r"\\.\TAPE2",
        r"\\.\TAPE3",
        r"\\.\TAPE4",
        r"\\.\TAPE5",
    ];
    
    let mut found_devices = Vec::new();
    
    for path in tape_paths {
        debug!("检查设备路径: {}", path);
        
        let path_cstring = CString::new(path).map_err(|e| {
            crate::error::RustLtfsError::system(format!("路径转换错误: {}", e))
        })?;
        
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
            
            if handle != winapi::um::handleapi::INVALID_HANDLE_VALUE {
                winapi::um::handleapi::CloseHandle(handle);
                
                let device_info = if detailed {
                    get_detailed_device_info(path).await.unwrap_or_else(|_| TapeDevice {
                        path: path.to_string(),
                        vendor: "未知".to_string(),
                        model: "未知".to_string(),
                        serial: "未知".to_string(),
                        status: TapeStatus::Ready,
                    })
                } else {
                    TapeDevice {
                        path: path.to_string(),
                        vendor: "未知".to_string(),
                        model: "未知".to_string(),
                        serial: "未知".to_string(),
                        status: TapeStatus::Ready,
                    }
                };
                
                found_devices.push(device_info);
            }
        }
    }
    
    if found_devices.is_empty() {
        println!("未找到可用的磁带设备");
        println!("请确保:");
        println!("1. 磁带驱动器已正确连接");
        println!("2. 驱动程序已安装");
        println!("3. 以管理员权限运行此工具");
    } else {
        println!("找到 {} 个磁带设备:", found_devices.len());
        
        for device in &found_devices {
            println!("  设备: {}", device.path);
            if detailed {
                println!("    厂商: {}", device.vendor);
                println!("    型号: {}", device.model);
                println!("    序列号: {}", device.serial);
                println!("    状态: {:?}", device.status);
            }
        }
        
        info!("找到 {} 个磁带设备", found_devices.len());
    }
    
    Ok(())
}

/// 获取详细的设备信息
async fn get_detailed_device_info(device_path: &str) -> Result<TapeDevice> {
    let mut scsi = ScsiInterface::new();
    scsi.open_device(device_path)?;
    
    let inquiry_data = scsi.inquiry()?;
    let status = scsi.check_media_status()?;
    
    let vendor = bytes_to_string(&inquiry_data.vendor_id);
    let model = bytes_to_string(&inquiry_data.product_id);
    let revision = bytes_to_string(&inquiry_data.product_revision);
    
    Ok(TapeDevice {
        path: device_path.to_string(),
        vendor,
        model,
        serial: revision, // 使用 revision 作为临时序列号
        status: match status {
            crate::scsi::MediaStatus::Ready => TapeStatus::Ready,
            crate::scsi::MediaStatus::NotReady => TapeStatus::NotReady,
            crate::scsi::MediaStatus::WriteProtected => TapeStatus::WriteProtected,
            crate::scsi::MediaStatus::NoMedia => TapeStatus::NoTape,
            crate::scsi::MediaStatus::Error(msg) => TapeStatus::Error(msg),
        },
    })
}