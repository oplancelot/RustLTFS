use crate::error::Result;
use crate::ltfs::LtfsDirectAccess;
use std::path::PathBuf;
use tracing::{info, debug, warn, error};
use walkdir::WalkDir;
use tokio::fs;
use indicatif::{ProgressBar, ProgressStyle};

/// 将文件或文件夹复制到磁带
pub async fn copy_to_tape(
    source: PathBuf,
    device: String,
    destination: PathBuf,
    force: bool,
    verify: bool,
    progress: bool,
) -> Result<()> {
    info!("开始复制操作: {:?} -> {}:{:?}", source, device, destination);
    
    // 检查源路径是否存在
    if !source.exists() {
        return Err(crate::error::RustLtfsError::file_operation(
            format!("源路径不存在: {:?}", source)
        ));
    }
    
    // 用户确认（除非使用 --force）
    if !force {
        if !confirm_operation(&source, &device, &destination)? {
            info!("用户取消操作");
            return Ok(());
        }
    }
    
    // 根据源路径类型执行不同操作
    if source.is_file() {
        copy_file_to_tape(source, device, destination, verify, progress).await
    } else if source.is_dir() {
        copy_directory_to_tape(source, device, destination, verify, progress).await
    } else {
        Err(crate::error::RustLtfsError::file_operation(
            "不支持的源路径类型"
        ))
    }
}

/// 从磁带读取文件到本地
pub async fn read_from_tape(
    device: String,
    source: PathBuf,
    destination: PathBuf,
    verify: bool,
) -> Result<()> {
    info!("从磁带读取: {}:{:?} -> {:?}", device, source, destination);
    
    // 创建 LTFS 直接访问实例
    let mut ltfs = LtfsDirectAccess::new(device.clone());
    ltfs.initialize()?;
    
    // 从磁带读取文件
    ltfs.read_file_direct(&source, &destination).await?;
    
    if verify {
        info!("开始验证读取数据...");
        // 验证读取的文件
        let local_data = fs::read(&destination).await?;
        
        // 创建临时文件进行二次读取验证
        let temp_dir = std::env::temp_dir();
        let temp_file = temp_dir.join(format!("rustltfs_read_verify_{}", 
            destination.file_name().unwrap().to_string_lossy()));
        
        ltfs.read_file_direct(&source, &temp_file).await?;
        let verify_data = fs::read(&temp_file).await?;
        
        if local_data == verify_data {
            info!("读取数据验证通过");
        } else {
            return Err(crate::error::RustLtfsError::verification(
                "读取数据验证失败"
            ));
        }
        
        // 清理临时文件
        let _ = fs::remove_file(temp_file).await;
    }
    
    info!("文件读取完成: {:?}", destination);
    Ok(())
}

/// 复制单个文件到磁带
async fn copy_file_to_tape(
    source: PathBuf,
    device: String,
    destination: PathBuf,
    verify: bool,
    progress: bool,
) -> Result<()> {
    debug!("复制文件: {:?} -> {}:{:?}", source, device, destination);
    
    let file_size = fs::metadata(&source).await?.len();
    info!("文件大小: {} 字节", file_size);
    
    let progress_bar = if progress {
        Some(setup_progress_bar(file_size)?)
    } else {
        None
    };
    
    // 创建 LTFS 直接访问实例
    let mut ltfs = LtfsDirectAccess::new(device.clone());
    ltfs.initialize()?;
    
    // 使用 LTFS 直接写入
    match ltfs.write_file_direct(&source, &destination).await {
        Ok(_) => {
            if let Some(pb) = progress_bar {
                pb.finish_with_message("文件写入完成");
            }
            info!("文件成功写入磁带");
        }
        Err(e) => {
            if let Some(pb) = progress_bar {
                pb.abandon_with_message("写入失败");
            }
            return Err(e);
        }
    }
    
    if verify {
        info!("开始验证写入数据...");
        verify_file_on_tape(&ltfs, &source, &destination).await?;
        info!("数据验证通过");
    }
    
    Ok(())
}

/// 验证磁带上的文件
async fn verify_file_on_tape(
    ltfs: &LtfsDirectAccess, 
    original_path: &PathBuf, 
    tape_path: &PathBuf
) -> Result<()> {
    debug!("验证文件: {:?} <-> {:?}", original_path, tape_path);
    
    // 读取原始文件
    let original_data = fs::read(original_path).await?;
    
    // 创建临时目录用于验证
    let temp_dir = std::env::temp_dir();
    let temp_file = temp_dir.join(format!("rustltfs_verify_{}", 
        original_path.file_name().unwrap().to_string_lossy()));
    
    // 从磁带读取文件
    ltfs.read_file_direct(tape_path, &temp_file).await?;
    
    // 读取验证文件
    let tape_data = fs::read(&temp_file).await?;
    
    // 比较文件内容
    if original_data == tape_data {
        debug!("文件验证成功");
    } else {
        return Err(crate::error::RustLtfsError::verification(
            "文件内容不匹配"
        ));
    }
    
    // 清理临时文件
    let _ = fs::remove_file(temp_file).await;
    
    Ok(())
}

/// 复制目录到磁带
async fn copy_directory_to_tape(
    source: PathBuf,
    device: String,
    destination: PathBuf,
    verify: bool,
    progress: bool,
) -> Result<()> {
    debug!("复制目录: {:?} -> {}:{:?}", source, device, destination);
    
    // 遍历目录获取所有文件
    let mut files = Vec::new();
    let mut total_size = 0u64;
    
    for entry in WalkDir::new(&source) {
        let entry = entry.map_err(|e| crate::error::RustLtfsError::file_operation(
            format!("目录遍历错误: {}", e)
        ))?;
        
        if entry.file_type().is_file() {
            let metadata = fs::metadata(entry.path()).await?;
            total_size += metadata.len();
            
            // 计算相对路径用于磁带目标路径
            let relative_path = entry.path().strip_prefix(&source)
                .map_err(|e| crate::error::RustLtfsError::file_operation(
                    format!("路径计算错误: {}", e)
                ))?;
            let tape_file_path = destination.join(relative_path);
            
            files.push((entry.path().to_path_buf(), tape_file_path, metadata.len()));
        }
    }
    
    info!("找到 {} 个文件，总大小: {} 字节", files.len(), total_size);
    
    let progress_bar = if progress {
        Some(setup_progress_bar(total_size)?)
    } else {
        None
    };
    
    // 创建 LTFS 直接访问实例
    let mut ltfs = LtfsDirectAccess::new(device.clone());
    ltfs.initialize()?;
    
    // 批量写入文件到磁带
    let mut processed_size = 0u64;
    let files_count = files.len();
    for (file_path, tape_path, file_size) in files {
        debug!("处理文件: {:?} -> {:?} ({} 字节)", file_path, tape_path, file_size);
        
        match ltfs.write_file_direct(&file_path, &tape_path).await {
            Ok(_) => {
                processed_size += file_size;
                if let Some(pb) = &progress_bar {
                    pb.set_position(processed_size);
                }
                info!("文件写入成功: {:?}", file_path);
            }
            Err(e) => {
                error!("文件写入失败: {:?} - {}", file_path, e);
                if let Some(pb) = progress_bar {
                    pb.abandon_with_message("批量写入失败");
                }
                return Err(e);
            }
        }
        
        if verify {
            debug!("验证文件: {:?}", file_path);
            verify_file_on_tape(&ltfs, &file_path, &tape_path).await?;
        }
    }
    
    if let Some(pb) = progress_bar {
        pb.finish_with_message("目录写入完成");
    }
    
    info!("目录成功写入磁带，共处理 {} 个文件", files_count);
    Ok(())
}

/// 用户确认操作
fn confirm_operation(source: &PathBuf, device: &str, destination: &PathBuf) -> Result<bool> {
    println!("即将执行以下操作:");
    println!("  源路径: {:?}", source);
    println!("  磁带设备: {}", device);
    println!("  目标路径: {:?}", destination);
    println!();
    print!("确认继续? (y/N): ");
    
    use std::io::{self, Write};
    io::stdout().flush().map_err(|e| crate::error::RustLtfsError::Io(e))?;
    
    let mut input = String::new();
    io::stdin().read_line(&mut input).map_err(|e| crate::error::RustLtfsError::Io(e))?;
    
    let input = input.trim().to_lowercase();
    Ok(input == "y" || input == "yes")
}

/// 设置进度条
fn setup_progress_bar(total_size: u64) -> Result<ProgressBar> {
    let pb = ProgressBar::new(total_size);
    pb.set_style(
        ProgressStyle::default_bar()
            .template("{spinner:.green} [{elapsed_precise}] [{wide_bar:.cyan/blue}] {bytes}/{total_bytes} ({eta})")
            .map_err(|e| crate::error::RustLtfsError::system(format!("进度条样式错误: {}", e)))?
            .progress_chars("#>-")
    );
    Ok(pb)
}