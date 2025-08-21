mod cli;
mod error;
mod logger;
mod scsi;
mod tape;
mod ltfs;
mod file_ops;

#[cfg(test)]
mod tests;

use crate::cli::{Cli, Commands};
use crate::error::Result;
use tracing::{info, error};

#[tokio::main]
async fn main() -> Result<()> {
    let args = Cli::parse_args();
    
    // 初始化日志系统
    logger::init(args.verbose)?;
    
    info!("RustLTFS CLI 启动");
    
    match run(args).await {
        Ok(_) => {
            info!("操作成功完成");
            Ok(())
        }
        Err(e) => {
            error!("操作失败: {}", e);
            std::process::exit(1);
        }
    }
}

async fn run(args: Cli) -> Result<()> {
    match args.command {
        Commands::Copy { 
            source, 
            device, 
            destination, 
            force, 
            verify, 
            progress 
        } => {
            info!("开始复制操作: {:?} -> {}:{:?}", source, device, destination);
            file_ops::copy_to_tape(source, device, destination, force, verify, progress).await
        }
        
        Commands::List { detailed } => {
            info!("列出磁带设备");
            tape::list_devices(detailed).await
        }
        
        Commands::Info { device } => {
            info!("获取设备信息: {}", device);
            tape::get_device_info(device).await
        }
        
        Commands::Read { 
            device, 
            source, 
            destination, 
            verify 
        } => {
            info!("从磁带读取: {}:{:?} -> {:?}", device, source, destination);
            file_ops::read_from_tape(device, source, destination, verify).await
        }
        
        Commands::Status { device } => {
            info!("检查设备状态: {}", device);
            tape::get_device_status(device).await
        }
    }
}