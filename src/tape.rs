use crate::error::Result;
use crate::scsi::{MediaType, check_tape_media};
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

/// 磁带状态枚举 (基于新的 MediaType)
#[derive(Debug, Clone, PartialEq)]
pub enum TapeStatus {
    Ready,
    NotReady,
    WriteProtected,
    NoTape,
    Lto3Rw,
    Lto3Worm,
    Lto3Ro,
    Lto4Rw,
    Lto4Worm,
    Lto4Ro,
    Lto5Rw,
    Lto5Worm,
    Lto5Ro,
    Lto6Rw,
    Lto6Worm,
    Lto6Ro,
    Lto7Rw,
    Lto7Worm,
    Lto7Ro,
    Lto8Rw,
    Lto8Worm,
    Lto8Ro,
    LtoM8Rw,
    LtoM8Worm,
    LtoM8Ro,
    Unknown(String),
    Error(String),
}

impl From<MediaType> for TapeStatus {
    fn from(media_type: MediaType) -> Self {
        match media_type {
            MediaType::NoTape => TapeStatus::NoTape,
            MediaType::Lto3Rw => TapeStatus::Lto3Rw,
            MediaType::Lto3Worm => TapeStatus::Lto3Worm,
            MediaType::Lto3Ro => TapeStatus::Lto3Ro,
            MediaType::Lto4Rw => TapeStatus::Lto4Rw,
            MediaType::Lto4Worm => TapeStatus::Lto4Worm,
            MediaType::Lto4Ro => TapeStatus::Lto4Ro,
            MediaType::Lto5Rw => TapeStatus::Lto5Rw,
            MediaType::Lto5Worm => TapeStatus::Lto5Worm,
            MediaType::Lto5Ro => TapeStatus::Lto5Ro,
            MediaType::Lto6Rw => TapeStatus::Lto6Rw,
            MediaType::Lto6Worm => TapeStatus::Lto6Worm,
            MediaType::Lto6Ro => TapeStatus::Lto6Ro,
            MediaType::Lto7Rw => TapeStatus::Lto7Rw,
            MediaType::Lto7Worm => TapeStatus::Lto7Worm,
            MediaType::Lto7Ro => TapeStatus::Lto7Ro,
            MediaType::Lto8Rw => TapeStatus::Lto8Rw,
            MediaType::Lto8Worm => TapeStatus::Lto8Worm,
            MediaType::Lto8Ro => TapeStatus::Lto8Ro,
            MediaType::LtoM8Rw => TapeStatus::LtoM8Rw,
            MediaType::LtoM8Worm => TapeStatus::LtoM8Worm,
            MediaType::LtoM8Ro => TapeStatus::LtoM8Ro,
            MediaType::Unknown(code) => TapeStatus::Unknown(format!("0x{:04X}", code)),
        }
    }
}

/// 列出系统中可用的磁带设备
pub async fn list_devices(_detailed: bool) -> Result<()> {
    info!("开始扫描磁带设备...");
    
    #[cfg(windows)]
    {
        list_windows_tape_devices(_detailed).await
    }
    
    #[cfg(not(windows))]
    {
        error!("此工具目前仅支持 Windows 平台");
        Err(crate::error::RustLtfsError::unsupported("非 Windows 平台"))
    }
}

/// 获取指定设备的详细信息 (基于更新的 SCSI 接口)
pub async fn get_device_info(device: String) -> Result<()> {
    info!("获取设备信息: {}", device);
    
    // 直接使用便捷函数检查媒体状态
    match check_tape_media(&device) {
        Ok(media_type) => {
            println!("设备信息:");
            println!("  设备路径: {}", device);
            println!("  媒体类型: {}", media_type.description());
            
            // 显示详细的媒体信息
            match media_type {
                MediaType::NoTape => println!("  状态: 未插入磁带"),
                MediaType::Unknown(code) => println!("  状态: 未知媒体类型 (代码: 0x{:04X})", code),
                _ => {
                    println!("  状态: 磁带已装载");
                    println!("  详细信息: 支持 LTFS 直接读写");
                }
            }
            
            Ok(())
        }
        Err(e) => {
            error!("获取设备信息失败: {}", e);
            Err(e)
        }
    }
}

/// 检查设备状态 (基于更新的 TapeCheckMedia 逻辑)
pub async fn get_device_status(device: String) -> Result<()> {
    info!("检查设备状态: {}", device);
    
    match check_tape_media(&device) {
        Ok(media_type) => {
            println!("设备: {}", device);
            println!("媒体状态: {}", media_type.description());
            
            // 根据媒体类型提供详细状态信息
            match media_type {
                MediaType::NoTape => {
                    println!("建议: 请插入 LTO 磁带");
                }
                MediaType::Unknown(code) => {
                    println!("警告: 未识别的媒体类型 (代码: 0x{:04X})", code);
                    println!("建议: 确认磁带是否为 LTO3-LTO8 格式");
                }
                _ => {
                    // 判断是否为只读磁带
                    if media_type.description().contains("RO") {
                        println!("注意: 此磁带为只读模式");
                    } else if media_type.description().contains("WORM") {
                        println!("注意: 此磁带为 WORM (一次写入多次读取) 模式");
                    } else {
                        println!("状态: 磁带可读写，支持 LTFS 操作");
                    }
                }
            }
            
            Ok(())
        }
        Err(e) => {
            error!("检查设备状态失败: {}", e);
            println!("错误: {}", e);
            Err(e)
        }
    }
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
                    get_detailed_device_info(path).await.unwrap_or_else(|e| {
                        warn!("获取详细设备信息失败 {}: {}", path, e);
                        TapeDevice {
                            path: path.to_string(),
                            vendor: "未知".to_string(),
                            model: "未知".to_string(),
                            serial: "未知".to_string(),
                            status: TapeStatus::Error("无法获取信息".to_string()),
                        }
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

/// 获取详细的设备信息 (基于更新的 MediaType)
async fn get_detailed_device_info(device_path: &str) -> Result<TapeDevice> {
    debug!("获取详细设备信息: {}", device_path);
    
    // 使用便捷函数检查媒体状态
    let media_type = check_tape_media(device_path)?;
    
    Ok(TapeDevice {
        path: device_path.to_string(),
        vendor: "IBM".to_string(), // 假设是 IBM 磁带驱动器
        model: "LTO Drive".to_string(), // 通用型号
        serial: "N/A".to_string(),
        status: TapeStatus::from(media_type),
    })
}