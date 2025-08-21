use thiserror::Error;

pub type Result<T> = std::result::Result<T, RustLtfsError>;

#[derive(Error, Debug)]
pub enum RustLtfsError {
    #[error("IO 错误: {0}")]
    Io(#[from] std::io::Error),
    
    #[error("SCSI 操作错误: {0}")]
    Scsi(String),
    
    #[error("磁带设备错误: {0}")]
    TapeDevice(String),
    
    #[error("文件操作错误: {0}")]
    FileOperation(String),
    
    #[error("配置错误: {0}")]
    Config(String),
    
    #[error("验证错误: {0}")]
    Verification(String),
    
    #[error("不支持的操作: {0}")]
    UnsupportedOperation(String),
    
    #[error("系统错误: {0}")]
    System(String),
    
    #[error("权限错误: {0}")]
    Permission(String),
    
    #[error("网络或连接错误: {0}")]
    Connection(String),
    
    #[error("解析错误: {0}")]
    Parse(String),
    
    #[error("通用错误: {0}")]
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
    
    pub fn config<T: Into<String>>(msg: T) -> Self {
        Self::Config(msg.into())
    }
    
    pub fn verification<T: Into<String>>(msg: T) -> Self {
        Self::Verification(msg.into())
    }
    
    pub fn unsupported<T: Into<String>>(msg: T) -> Self {
        Self::UnsupportedOperation(msg.into())
    }
    
    pub fn system<T: Into<String>>(msg: T) -> Self {
        Self::System(msg.into())
    }
    
    pub fn permission<T: Into<String>>(msg: T) -> Self {
        Self::Permission(msg.into())
    }
    
    pub fn connection<T: Into<String>>(msg: T) -> Self {
        Self::Connection(msg.into())
    }
    
    pub fn parse<T: Into<String>>(msg: T) -> Self {
        Self::Parse(msg.into())
    }
}

