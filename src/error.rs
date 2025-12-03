use thiserror::Error;

pub type Result<T> = std::result::Result<T, RustLtfsError>;

#[derive(Error, Debug)]
pub enum RustLtfsError {
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
    
    #[error("SCSI operation error: {0}")]
    Scsi(String),
    
    #[error("Tape device error: {0}")]
    TapeDevice(String),
    
    #[error("File operation error: {0}")]
    FileOperation(String),
    


    


    


    
    #[error("System error: {0}")]
    System(String),
    


    


    
    #[error("Parse error: {0}")]
    Parse(String),
    


    
    #[error("LTFS index error: {0}")]
    LtfsIndex(String),
    
    #[error("Parameter validation error: {0}")]
    ParameterValidation(String),
    


    


    
    #[error("Generic error: {0}")]
    Generic(#[from] anyhow::Error),
}

impl RustLtfsError {
    pub fn scsi<T: Into<String>>(msg: T) -> Self {
        Self::Scsi(msg.into())
    }
    
    pub fn tape_device<T: Into<String>>(msg: T) -> Self {
        Self::TapeDevice(msg.into())
    }
    
    pub fn file_operation<T: Into<String>>(msg: T) -> Self {
        Self::FileOperation(msg.into())
    }
    

    

    

    
    pub fn system<T: Into<String>>(msg: T) -> Self {
        Self::System(msg.into())
    }
    

    

    
    pub fn parse<T: Into<String>>(msg: T) -> Self {
        Self::Parse(msg.into())
    }
    

    
    pub fn ltfs_index<T: Into<String>>(msg: T) -> Self {
        Self::LtfsIndex(msg.into())
    }
    
    pub fn parameter_validation<T: Into<String>>(msg: T) -> Self {
        Self::ParameterValidation(msg.into())
    }
    

    

}
