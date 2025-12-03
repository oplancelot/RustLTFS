

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



/// Space types for SPACE command
#[derive(Debug, Clone, Copy)]
pub enum SpaceType {

    FileMarks = 1,

    EndOfData = 3,
}

/// Locate destination types (corresponding to LTFSCopyGUI LocateDestType)
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum LocateDestType {
    /// Locate to specific block number
    Block = 0,
    /// Locate to file mark

    /// Locate to end of data
    EOD = 3,
}

/// Drive type enumeration for specific driver optimizations
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum DriveType {
    /// Standard/Generic drive
    Standard,


}






