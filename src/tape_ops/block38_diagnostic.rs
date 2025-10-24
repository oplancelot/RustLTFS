use crate::error::Result;
use crate::scsi::ScsiInterface;
use crate::tape_ops::data_partition_reader::DataPartitionIndexReader;
use tracing::{info, error};

/// P1 Block38定位问题诊断工具
/// 
/// 这个工具专门用于诊断和修复RustLTFS中P1 Block38定位问题，
/// 对比LTFSCopyGUI的行为并提供详细的诊断信息。
pub struct Block38Diagnostic {
    device_path: String,
}

impl Block38Diagnostic {
    /// 创建新的诊断工具实例
    pub fn new(device_path: String) -> Self {
        Self { device_path }
    }
    
    /// 运行完整的P1 Block38定位诊断
    pub fn run_full_diagnostic(&self) -> Result<()> {
        info!("🔍 开始P1 Block38定位问题完整诊断");
        info!("📱 目标设备: {}", self.device_path);
        
        // 1. 初始化SCSI接口
        let mut scsi = ScsiInterface::new();
        scsi.open_device(&self.device_path)?;
        
        // 2. 创建数据分区索引读取器
        let reader = DataPartitionIndexReader::new(&scsi);
        
        // 3. 运行诊断
        info!("🔧 步骤1: 运行基础诊断");
        let diagnostic_report = reader.diagnose_block38_issue()?;
        diagnostic_report.print_report();
        
        // 4. 对比LTFSCopyGUI行为
        info!("🔧 步骤2: 对比LTFSCopyGUI定位行为");
        match reader.compare_with_ltfscopygui_positioning() {
            Ok(comparison) => {
                self.print_positioning_comparison(&comparison);
            }
            Err(e) => {
                error!("❌ LTFSCopyGUI行为对比失败: {}", e);
            }
        }
        
        // 5. 尝试修复版读取
        info!("🔧 步骤3: 尝试修复版数据分区索引读取");
        match scsi.read_data_partition_index_enhanced() {
            Ok(index_data) => {
                info!("✅ 修复版读取成功: {} 字节", index_data.len());
                
                // 验证数据内容
                let xml_content = String::from_utf8_lossy(&index_data);
                if xml_content.contains("<ltfsindex") {
                    info!("✅ 索引数据验证通过: 包含有效的LTFS索引");
                } else {
                    error!("❌ 索引数据验证失败: 不包含有效的LTFS索引");
                }
            }
            Err(e) => {
                error!("❌ 修复版读取失败: {}", e);
                
                // 提供故障排除建议
                self.provide_troubleshooting_suggestions(&e);
            }
        }
        
        // 6. 诊断ReadFileMark行为对比
        info!("🔧 步骤4: 诊断ReadFileMark行为对比");
        match scsi.diagnose_read_filemark_behavior() {
            Ok(diagnostic) => {
                diagnostic.print_report();
            }
            Err(e) => {
                error!("❌ ReadFileMark行为诊断失败: {}", e);
            }
        }
        
        info!("🔍 P1 Block38定位诊断完成");
        Ok(())
    }
    
    /// 运行快速诊断（仅检查当前位置）
    pub fn run_quick_diagnostic(&self) -> Result<()> {
        info!("🔍 开始P1 Block38快速诊断");
        
        let mut scsi = ScsiInterface::new();
        scsi.open_device(&self.device_path)?;
        
        let reader = DataPartitionIndexReader::new(&scsi);
        
        // 检查当前位置
        let current_pos = scsi.read_position()?;
        info!("📍 当前磁带位置: P{} B{} FM{}", 
             current_pos.partition, current_pos.block_number, current_pos.file_number);
        
        // 验证是否在正确位置
        if reader.verify_block38_positioning()? {
            info!("✅ 快速诊断: 当前位置正确 (P1 Block38)");
        } else {
            error!("❌ 快速诊断: 当前位置不正确");
            
            // 尝试定位到正确位置
            info!("🔧 尝试定位到P1 Block38");
            scsi.locate_block(1, 38)?;
            
            if reader.verify_block38_positioning()? {
                info!("✅ 定位修复成功");
            } else {
                error!("❌ 定位修复失败");
            }
        }
        
        Ok(())
    }
    
    /// 测试ReadFileMark回退逻辑
    pub fn test_read_filemark_backtrack(&self) -> Result<()> {
        info!("🔧 测试ReadFileMark回退逻辑");
        
        let mut scsi = ScsiInterface::new();
        scsi.open_device(&self.device_path)?;
        
        // 记录初始位置
        let initial_pos = scsi.read_position()?;
        info!("📍 初始位置: P{} B{} FM{}", 
             initial_pos.partition, initial_pos.block_number, initial_pos.file_number);
        
        // 定位到FileMark 5
        info!("🔧 定位到FileMark 5");
        scsi.locate_to_filemark(5, 1)?;
        
        let pos_after_fm5 = scsi.read_position()?;
        info!("📍 FileMark 5后位置: P{} B{} FM{}", 
             pos_after_fm5.partition, pos_after_fm5.block_number, pos_after_fm5.file_number);
        
        // 执行原版ReadFileMark
        info!("🔧 执行原版ReadFileMark");
        let fm_detected_orig = scsi.read_file_mark()?;
        
        let pos_after_readfm_orig = scsi.read_position()?;
        info!("📍 原版ReadFileMark后位置: P{} B{} FM{} (FileMark检测: {})", 
             pos_after_readfm_orig.partition, pos_after_readfm_orig.block_number, 
             pos_after_readfm_orig.file_number, fm_detected_orig);
        
        // 重新定位到FileMark 5测试修复版
        info!("🔧 重新定位到FileMark 5测试修复版");
        scsi.locate_to_filemark(5, 1)?;
        
        // 执行修复版ReadFileMark
        info!("🔧 执行修复版ReadFileMark");
        let fm_detected_fixed = scsi.read_file_mark_fixed()?;
        
        let pos_after_readfm_fixed = scsi.read_position()?;
        info!("📍 修复版ReadFileMark后位置: P{} B{} FM{} (FileMark检测: {})", 
             pos_after_readfm_fixed.partition, pos_after_readfm_fixed.block_number, 
             pos_after_readfm_fixed.file_number, fm_detected_fixed);
        
        // 分析对比结果
        println!("\n=== ReadFileMark回退测试对比结果 ===");
        println!("原版ReadFileMark:");
        if pos_after_readfm_orig.partition == 1 && pos_after_readfm_orig.block_number == 38 {
            println!("  ✅ 成功到达P1 Block38");
        } else if pos_after_readfm_orig.block_number == 39 {
            println!("  ❌ 停留在Block39，回退逻辑有问题");
        } else {
            println!("  ❌ 意外位置Block{}", pos_after_readfm_orig.block_number);
        }
        
        println!("修复版ReadFileMark:");
        if pos_after_readfm_fixed.partition == 1 && pos_after_readfm_fixed.block_number == 38 {
            println!("  ✅ 成功到达P1 Block38");
        } else if pos_after_readfm_fixed.block_number == 39 {
            println!("  ❌ 仍停留在Block39，修复未成功");
        } else {
            println!("  ❌ 意外位置Block{}", pos_after_readfm_fixed.block_number);
        }
        
        if pos_after_readfm_orig.block_number != pos_after_readfm_fixed.block_number {
            if pos_after_readfm_fixed.block_number == 38 && pos_after_readfm_orig.block_number == 39 {
                println!("🎉 修复版成功解决了P1 Block38定位问题！");
            } else {
                println!("⚠️ 修复版和原版结果不同，需要进一步分析");
            }
        } else {
            if pos_after_readfm_fixed.block_number == 38 {
                println!("ℹ️ 原版和修复版都正确到达P1 Block38");
            } else {
                println!("❌ 原版和修复版都未能正确到达P1 Block38");
            }
        }
        println!("=====================================\n");
        
        Ok(())
    }
    
    /// 打印位置对比结果
    fn print_positioning_comparison(&self, comparison: &crate::tape_ops::data_partition_reader::PositionComparison) {
        println!("\n=== LTFSCopyGUI定位行为对比 ===");
        println!("初始位置: P{} B{} FM{}", 
                comparison.initial_position.partition, 
                comparison.initial_position.block_number, 
                comparison.initial_position.file_number);
        println!("FileMark5后: P{} B{} FM{}", 
                comparison.after_filemark5.partition, 
                comparison.after_filemark5.block_number, 
                comparison.after_filemark5.file_number);
        println!("ReadFileMark后: P{} B{} FM{}", 
                comparison.after_read_filemark.partition, 
                comparison.after_read_filemark.block_number, 
                comparison.after_read_filemark.file_number);
        println!("FileMark检测: {}", comparison.filemark_detected);
        println!("期望最终位置: Block{}", comparison.expected_final_block);
        println!("实际最终位置: Block{}", comparison.actual_final_block);
        println!("定位正确性: {}", if comparison.positioning_correct { "✅ 正确" } else { "❌ 错误" });
        println!("==============================\n");
    }
    
    /// 提供故障排除建议
    fn provide_troubleshooting_suggestions(&self, error: &crate::error::RustLtfsError) {
        println!("\n=== 故障排除建议 ===");
        
        let error_str = error.to_string();
        
        if error_str.contains("Block38PositioningFailed") {
            println!("问题: P1 Block38定位失败");
            println!("可能原因:");
            println!("  1. ReadFileMark回退逻辑计算错误");
            println!("  2. locate_block方法实现有问题");
            println!("  3. 磁带位置读取不准确");
            println!("建议解决方案:");
            println!("  1. 检查ReadFileMark中的回退计算 (current_pos.block_number - 1)");
            println!("  2. 验证locate_block方法是否正确执行SCSI LOCATE命令");
            println!("  3. 添加更多位置验证日志");
        } else if error_str.contains("BacktrackLogicError") {
            println!("问题: ReadFileMark回退逻辑错误");
            println!("建议解决方案:");
            println!("  1. 对比LTFSCopyGUI的ReadFileMark实现");
            println!("  2. 检查AllowPartition模式的处理");
            println!("  3. 验证Space6命令的参数");
        } else if error_str.contains("InvalidIndexData") {
            println!("问题: 索引数据无效");
            println!("建议解决方案:");
            println!("  1. 检查是否读取到全零数据");
            println!("  2. 验证XML格式完整性");
            println!("  3. 确认磁带包含有效的LTFS索引");
        }
        
        println!("==================\n");
    }
}

/// 便捷函数：运行P1 Block38诊断
pub fn diagnose_block38_issue(device_path: &str) -> Result<()> {
    let diagnostic = Block38Diagnostic::new(device_path.to_string());
    diagnostic.run_full_diagnostic()
}

/// 便捷函数：快速检查P1 Block38位置
pub fn quick_check_block38(device_path: &str) -> Result<()> {
    let diagnostic = Block38Diagnostic::new(device_path.to_string());
    diagnostic.run_quick_diagnostic()
}

/// 便捷函数：测试ReadFileMark回退
pub fn test_readfilemark_backtrack(device_path: &str) -> Result<()> {
    let diagnostic = Block38Diagnostic::new(device_path.to_string());
    diagnostic.test_read_filemark_backtrack()
}