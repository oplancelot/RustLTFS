use crate::scsi::MediaType;
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
