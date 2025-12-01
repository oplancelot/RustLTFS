

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MediaType {
    NoTape,
    Lto3Rw,    // 0x0044
    Lto3Worm,  // 0x0144
    Lto3Ro,    // 0x0244
    Lto4Rw,    // 0x0046
    Lto4Worm,  // 0x0146
    Lto4Ro,    // 0x0246
    Lto5Rw,    // 0x0058
    Lto5Worm,  // 0x0158
    Lto5Ro,    // 0x0258
    Lto6Rw,    // 0x005A
    Lto6Worm,  // 0x015A
    Lto6Ro,    // 0x025A
    Lto7Rw,    // 0x005C
    Lto7Worm,  // 0x015C
    Lto7Ro,    // 0x025C
    Lto8Rw,    // 0x005E
    Lto8Worm,  // 0x015E
    Lto8Ro,    // 0x025E
    Lto9Rw,    // 0x0060
    Lto9Worm,  // 0x0160
    Lto9Ro,    // 0x0260
    LtoM8Rw,   // 0x005D
    LtoM8Worm, // 0x015D
    LtoM8Ro,   // 0x025D
    Unknown(u16),
}

impl MediaType {
    /// Convert from media type code to media type
    pub fn from_media_type_code(code: u16) -> Self {
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
            0x0060 => MediaType::Lto9Rw,
            0x0160 => MediaType::Lto9Worm,
            0x0260 => MediaType::Lto9Ro,
            0x005D => MediaType::LtoM8Rw,
            0x015D => MediaType::LtoM8Worm,
            0x025D => MediaType::LtoM8Ro,
            _ => MediaType::Unknown(code),
        }
    }

    /// Convert to description string
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
            MediaType::Lto9Rw => "LTO9 RW",
            MediaType::Lto9Worm => "LTO9 WORM",
            MediaType::Lto9Ro => "LTO9 RO",
            MediaType::LtoM8Rw => "LTOM8 RW",
            MediaType::LtoM8Worm => "LTOM8 WORM",
            MediaType::LtoM8Ro => "LTOM8 RO",
            MediaType::Unknown(_) => "Unknown media type",
        }
    }
}

/// Tape position information structure
#[derive(Debug, Clone)]
pub struct TapePosition {
    pub partition: u8,
    pub block_number: u64,
    pub file_number: u64,
    pub set_number: u64,
    pub end_of_data: bool,
    pub beginning_of_partition: bool,
}

/// MAM (Medium Auxiliary Memory) attribute structure
#[derive(Debug, Clone)]
pub struct MamAttribute {
    pub attribute_id: u16,
    pub attribute_format: u8,
    pub length: u16,
    pub data: Vec<u8>,
}

/// Space types for SPACE command
#[derive(Debug, Clone, Copy)]
pub enum SpaceType {
    Blocks = 0,
    FileMarks = 1,
    SequentialFileMarks = 2,
    EndOfData = 3,
}

/// Locate destination types (corresponding to LTFSCopyGUI LocateDestType)
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum LocateDestType {
    /// Locate to specific block number
    Block = 0,
    /// Locate to file mark
    FileMark = 1,
    /// Locate to end of data
    EOD = 3,
}

/// Drive type enumeration for specific driver optimizations
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum DriveType {
    /// Standard/Generic drive
    Standard,
    /// Legacy SLR3 drive type
    SLR3,
    /// Legacy SLR1 drive type
    SLR1,
    /// M2488 drive type
    M2488,
}

/// Load option enumeration for LOAD_UNLOAD command (based on LTFSCopyGUI)
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum LoadOption {
    /// Unthread the tape (0)
    Unthread = 0,
    /// Load and thread the tape (1)
    LoadThreaded = 1,
    /// Retension operation (2)
    Retension = 2,
}

/// MAM attribute format types (对应LTFSCopyGUI的AttributeFormat)
#[derive(Debug, Clone, Copy, PartialEq)]
#[repr(u8)]
pub enum MamAttributeFormat {
    Binary = 0x00,
    Ascii = 0x01,
    Text = 0x02,
}

/// Tape drive information structure, corresponds to TAPE_DRIVE in C code
#[derive(Debug, Clone)]
pub struct TapeDriveInfo {
    pub vendor_id: String,
    pub product_id: String,
    pub serial_number: String,
    pub device_index: u32,
    pub device_path: String,
}
