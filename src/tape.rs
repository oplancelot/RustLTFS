#![allow(dead_code)]
use crate::error::Result;
use crate::scsi::{MediaType, check_tape_media};
use tracing::{info, debug, error, warn};
use serde::{Serialize, Deserialize};

/// Tape device information structure
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TapeDevice {
    pub path: String,
    pub vendor: String,
    pub model: String,
    pub serial: String,
    pub status: TapeStatus,
}

/// Tape status enumeration (based on new MediaType)
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
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
    Lto9Rw,
    Lto9Worm,
    Lto9Ro,
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
            MediaType::Lto9Rw => TapeStatus::Lto9Rw,
            MediaType::Lto9Worm => TapeStatus::Lto9Worm,
            MediaType::Lto9Ro => TapeStatus::Lto9Ro,
            MediaType::Unknown(code) => TapeStatus::Unknown(format!("0x{:04X}", code)),
        }
    }
}

/// List available tape devices in the system
pub async fn list_devices(_detailed: bool) -> Result<()> {
    info!("Starting to scan tape devices...");
    
    #[cfg(windows)]
    {
        list_windows_tape_devices(_detailed).await
    }
    
    #[cfg(not(windows))]
    {
        error!("This tool currently only supports Windows platform");
        Err(crate::error::RustLtfsError::unsupported("Non-Windows platform"))
    }
}

/// Get detailed information for the specified device (based on updated SCSI interface)
pub async fn get_device_info(device: String) -> Result<()> {
    info!("Getting device information: {}", device);
    
    // Use convenience function to check media status
    match check_tape_media(&device) {
        Ok(media_type) => {
            println!("Device information:");
            println!("  Device path: {}", device);
            println!("  Media type: {}", media_type.description());
            
            // Display detailed media information
            match media_type {
                MediaType::NoTape => println!("  Status: No tape inserted"),
                MediaType::Unknown(code) => println!("  Status: Unknown media type (code: 0x{:04X})", code),
                _ => {
                    println!("  Status: Tape loaded");
                    println!("  Detail: Supports LTFS direct read/write");
                }
            }
            
            Ok(())
        }
        Err(e) => {
            error!("Failed to get device information: {}", e);
            Err(e)
        }
    }
}

/// Check device status (based on updated TapeCheckMedia logic)
pub async fn get_device_status(device: String) -> Result<()> {
    info!("Checking device status: {}", device);
    
    match check_tape_media(&device) {
        Ok(media_type) => {
            println!("Device: {}", device);
            println!("Media status: {}", media_type.description());
            
            // Provide detailed status information based on media type
            match media_type {
                MediaType::NoTape => {
                    println!("Suggestion: Please insert LTO tape");
                }
                MediaType::Unknown(code) => {
                    println!("Warning: Unrecognized media type (code: 0x{:04X})", code);
                    println!("Suggestion: Confirm if tape is LTO3-LTO8 format");
                }
                _ => {
                    // Check if it's a read-only tape
                    if media_type.description().contains("RO") {
                        println!("Note: This tape is in read-only mode");
                    } else if media_type.description().contains("WORM") {
                        println!("Note: This tape is in WORM (Write Once Read Many) mode");
                    } else {
                        println!("Status: Tape is readable and writable, supports LTFS operations");
                    }
                }
            }
            
            Ok(())
        }
        Err(e) => {
            error!("Failed to check device status: {}", e);
            println!("Error: {}", e);
            Err(e)
        }
    }
}

#[cfg(windows)]
async fn list_windows_tape_devices(detailed: bool) -> Result<()> {
    use winapi::um::fileapi::{CreateFileA, OPEN_EXISTING};
    use winapi::um::winnt::{FILE_ATTRIBUTE_NORMAL, GENERIC_READ, GENERIC_WRITE};
    use std::ffi::CString;
    
    debug!("Scanning Windows tape devices");
    
    // Check common tape device paths
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
        debug!("Checking device path: {}", path);
        
        let path_cstring = CString::new(path).map_err(|e| {
            crate::error::RustLtfsError::system(format!("Path conversion error: {}", e))
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
                        warn!("Failed to get detailed device information {}: {}", path, e);
                        TapeDevice {
                            path: path.to_string(),
                            vendor: "Unknown".to_string(),
                            model: "Unknown".to_string(),
                            serial: "Unknown".to_string(),
                            status: TapeStatus::Error("Unable to get information".to_string()),
                        }
                    })
                } else {
                    TapeDevice {
                        path: path.to_string(),
                        vendor: "Unknown".to_string(),
                        model: "Unknown".to_string(),
                        serial: "Unknown".to_string(),
                        status: TapeStatus::Ready,
                    }
                };
                
                found_devices.push(device_info);
            }
        }
    }
    
    if found_devices.is_empty() {
        println!("No available tape devices found");
        println!("Please ensure:");
        println!("1. Tape drive is properly connected");
        println!("2. Drivers are installed");
        println!("3. Run this tool with administrator privileges");
    } else {
        println!("Found {} tape devices found:", found_devices.len());
        
        for device in &found_devices {
            println!("  Device: {}", device.path);
            if detailed {
                println!("    Vendor: {}", device.vendor);
                println!("    Model: {}", device.model);
                println!("    Serial number: {}", device.serial);
                println!("    Status: {:?}", device.status);
            }
        }
        
        info!("Found {} tape devices found", found_devices.len());
    }
    
    Ok(())
}

/// Get detailed device information (based on updated MediaType)
async fn get_detailed_device_info(device_path: &str) -> Result<TapeDevice> {
    debug!("Getting detailed device information: {}", device_path);
    
    // Use convenience function to check media status
    let media_type = check_tape_media(device_path)?;
    
    Ok(TapeDevice {
        path: device_path.to_string(),
        vendor: "IBM".to_string(), // Assuming IBM tape drive
        model: "LTO Drive".to_string(), // Generic model
        serial: "N/A".to_string(),
        status: TapeStatus::from(media_type),
    })
}
