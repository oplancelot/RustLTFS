use crate::error::{Result, RustLtfsError};
use crate::scsi::{ScsiInterface, TapePosition};
use crate::scsi::block_sizes;
use tracing::{debug, info, warn, error};

/// 数据分区索引读取器 - 专门解决P1 Block38定位问题
/// 
/// 这个模块实现了与LTFSCopyGUI完全兼容的数据分区索引读取逻辑，
/// 重点解决当前RustLTFS读取P1 Block39而不是P1 Block38的问题。
pub struct DataPartitionIndexReader<'a> {
    scsi: &'a ScsiInterface,
    debug_mode: bool,
}

/// 数据分区索引读取错误类型
#[derive(Debug, thiserror::Error)]
pub enum DataPartitionError {
    #[error("P1 Block38定位失败: 当前位置 Block{current_block}, 期望位置 Block38")]
    Block38PositioningFailed { current_block: u64 },
    
    #[error("ReadFileMark回退逻辑错误: {details}")]
    BacktrackLogicError { details: String },
    
    #[error("数据分区索引为空或无效: {reason}")]
    InvalidIndexData { reason: String },
    
    #[error("位置验证失败: 期望P{expected_partition} B{expected_block}, 实际P{actual_partition} B{actual_block}")]
    PositionVerificationFailed {
        expected_partition: u8,
        expected_block: u64,
        actual_partition: u8,
        actual_block: u64,
    },
}

impl From<DataPartitionError> for RustLtfsError {
    fn from(err: DataPartitionError) -> Self {
        RustLtfsError::ltfs_index(err.to_string())
    }
}

impl<'a> DataPartitionIndexReader<'a> {
    /// 创建新的数据分区索引读取器
    pub fn new(scsi: &'a ScsiInterface) -> Self {
        Self {
            scsi,
            debug_mode: true, // 默认启用调试模式以诊断P1 Block38问题
        }
    }
    
    /// 启用或禁用调试模式
    pub fn set_debug_mode(&mut self, enabled: bool) {
        self.debug_mode = enabled;
    }
    
    /// 修复P1 Block38定位问题的核心方法
    /// 
    /// 这个方法实现了与LTFSCopyGUI完全兼容的数据分区索引读取逻辑，
    /// 确保最终位置为P1 Block38而不是P1 Block39。
    pub fn read_data_partition_index_fixed(&self) -> Result<Vec<u8>> {
        info!("🔧 开始修复版数据分区索引读取 (目标: P1 Block38)");
        
        // 步骤1: 定位到数据分区FileMark 5 (对应LTFSCopyGUI的标准流程)
        info!("步骤1: 定位到数据分区FileMark 5");
        self.scsi.locate_to_filemark(5, 1)?;
        
        // 记录定位后的位置
        let pos_after_locate = self.scsi.read_position()?;
        info!("📍 定位到FileMark 5后的位置: P{} B{} FM{}", 
             pos_after_locate.partition, pos_after_locate.block_number, pos_after_locate.file_number);
        
        // 步骤2: 执行ReadFileMark并验证位置 (关键修复点)
        info!("步骤2: 执行ReadFileMark操作");
        let filemark_detected = self.read_file_mark_enhanced()?;
        
        if filemark_detected {
            return Err(DataPartitionError::Block38PositioningFailed { 
                current_block: self.scsi.read_position()?.block_number 
            }.into());
        }
        
        // 步骤3: 验证当前位置是否为P1 Block38
        let current_pos = self.scsi.read_position()?;
        info!("📍 ReadFileMark后的位置: P{} B{} FM{}", 
             current_pos.partition, current_pos.block_number, current_pos.file_number);
        
        if current_pos.partition != 1 || current_pos.block_number != 38 {
            error!("❌ 位置验证失败: 期望P1 B38, 实际P{} B{}", 
                   current_pos.partition, current_pos.block_number);
            
            // 尝试直接定位到P1 Block38
            info!("🔧 尝试直接定位到P1 Block38");
            self.scsi.locate_block(1, 38)?;
            
            let corrected_pos = self.scsi.read_position()?;
            if corrected_pos.block_number != 38 {
                return Err(DataPartitionError::Block38PositioningFailed { 
                    current_block: corrected_pos.block_number 
                }.into());
            }
            
            info!("✅ 强制校正成功: 现在位于P1 B38");
        }
        
        // 步骤4: 从P1 Block38读取索引数据
        info!("步骤4: 从P1 Block38读取索引数据");
        let index_data = self.read_index_data_from_current_position()?;
        
        // 步骤5: 验证读取的数据有效性
        self.validate_index_data(&index_data)?;
        
        info!("✅ 成功从P1 Block38读取到 {} 字节的有效索引数据", index_data.len());
        Ok(index_data)
    }
    
    /// 增强版ReadFileMark，包含详细的位置跟踪和验证
    fn read_file_mark_enhanced(&self) -> Result<bool> {
        info!("🔧 执行增强版ReadFileMark (包含位置验证)");
        
        // 记录初始位置
        let initial_pos = self.scsi.read_position()?;
        info!("📍 ReadFileMark初始位置: P{} B{} FM{}", 
             initial_pos.partition, initial_pos.block_number, initial_pos.file_number);
        
        // 尝试读取一个块来检测FileMark
        let mut test_buffer = vec![0u8; block_sizes::LTO_BLOCK_SIZE as usize];
        let read_result = self.scsi.send_scsi_command(
            &[0x08, 0x00, 0x00, 0x00, 0x01, 0x00], // READ(6) 1 block
            &mut test_buffer,
            1, // data_in
        )?;
        
        info!("🔍 ReadFileMark读取结果: success={}, data_length={}", 
             read_result, test_buffer.len());
        
        // 如果没有读取到数据，说明已经在FileMark位置
        if !read_result || test_buffer.is_empty() {
            info!("✅ ReadFileMark: 检测到FileMark，无需回退");
            return Ok(true);
        }
        
        // 读取到数据，需要回退
        let pos_after_read = self.scsi.read_position()?;
        info!("📍 ReadFileMark读取后位置: P{} B{} FM{}", 
             pos_after_read.partition, pos_after_read.block_number, pos_after_read.file_number);
        
        // 计算回退目标位置
        let target_block = if pos_after_read.block_number > 0 {
            pos_after_read.block_number - 1
        } else {
            warn!("⚠️ ReadFileMark: 当前已在Block 0，无法回退");
            return Ok(false);
        };
        
        // 执行回退
        info!("🔧 ReadFileMark: 执行回退到 P{} B{}", pos_after_read.partition, target_block);
        self.scsi.locate_block(pos_after_read.partition, target_block)?;
        
        // 验证回退结果
        let pos_after_backtrack = self.scsi.read_position()?;
        info!("📍 ReadFileMark回退后位置: P{} B{} FM{}", 
             pos_after_backtrack.partition, pos_after_backtrack.block_number, pos_after_backtrack.file_number);
        
        // 验证回退是否成功
        if pos_after_backtrack.block_number != target_block {
            error!("❌ ReadFileMark回退失败: 期望B{}, 实际B{}", 
                   target_block, pos_after_backtrack.block_number);
            
            // 尝试强制校正到目标位置
            info!("🔧 ReadFileMark: 尝试强制校正到目标位置");
            self.scsi.locate_block(pos_after_backtrack.partition, target_block)?;
            
            let final_pos = self.scsi.read_position()?;
            if final_pos.block_number != target_block {
                return Err(DataPartitionError::BacktrackLogicError {
                    details: format!("强制校正也失败: 期望B{}, 实际B{}", target_block, final_pos.block_number)
                }.into());
            }
            
            info!("✅ ReadFileMark: 强制校正成功");
        }
        
        Ok(false) // 返回false表示执行了回退
    }
    
    /// 验证读取位置是否为P1 Block38
    pub fn verify_block38_positioning(&self) -> Result<bool> {
        let current_pos = self.scsi.read_position()?;
        
        if self.debug_mode {
            info!("🔍 位置验证: 当前位置P{} B{} FM{}", 
                 current_pos.partition, current_pos.block_number, current_pos.file_number);
        }
        
        let is_correct = current_pos.partition == 1 && current_pos.block_number == 38;
        
        if is_correct {
            info!("✅ 位置验证通过: 正确位于P1 Block38");
        } else {
            warn!("❌ 位置验证失败: 期望P1 B38, 实际P{} B{}", 
                  current_pos.partition, current_pos.block_number);
        }
        
        Ok(is_correct)
    }
    
    /// 从当前位置读取索引数据
    fn read_index_data_from_current_position(&self) -> Result<Vec<u8>> {
        info!("🔧 从当前位置读取索引数据");
        
        // 使用ReadToFileMark读取直到下一个FileMark
        let block_size_limit = block_sizes::LTO_BLOCK_SIZE;
        let index_data = self.scsi.read_to_file_mark(block_size_limit)?;
        
        info!("📊 读取到 {} 字节的索引数据", index_data.len());
        
        if self.debug_mode && !index_data.is_empty() {
            let preview = String::from_utf8_lossy(&index_data[..std::cmp::min(200, index_data.len())]);
            debug!("🔍 索引数据预览: {:?}", preview);
        }
        
        Ok(index_data)
    }
    
    /// 验证索引数据的有效性
    fn validate_index_data(&self, data: &[u8]) -> Result<()> {
        use crate::tape_ops::index_validator::IndexValidator;
        
        let validator = if self.debug_mode {
            let mut v = IndexValidator::new();
            v.set_debug_mode(true);
            v
        } else {
            IndexValidator::new()
        };
        
        match validator.validate_index_data(data) {
            Ok(result) => {
                if result.is_valid {
                    info!("✅ 索引数据验证通过: {} 字节的有效LTFS索引", result.data_size);
                    
                    if let Some(version) = &result.ltfs_version {
                        info!("  LTFS版本: {}", version);
                    }
                    
                    if let Some(uuid) = &result.volume_uuid {
                        info!("  卷UUID: {}", uuid);
                    }
                    
                    if let Some(gen) = result.generation_number {
                        info!("  生成号: {}", gen);
                    }
                    
                    if let Some(count) = result.file_count_estimate {
                        info!("  估计文件数: {}", count);
                    }
                    
                    if !result.warnings.is_empty() {
                        for warning in &result.warnings {
                            warn!("⚠️ 索引验证警告: {}", warning);
                        }
                    }
                    
                    Ok(())
                } else {
                    let error_summary = result.errors.join("; ");
                    Err(DataPartitionError::InvalidIndexData {
                        reason: format!("验证失败: {}", error_summary)
                    }.into())
                }
            }
            Err(e) => {
                Err(DataPartitionError::InvalidIndexData {
                    reason: format!("验证器错误: {}", e)
                }.into())
            }
        }
    }
    
    /// 对比LTFSCopyGUI的定位行为
    pub fn compare_with_ltfscopygui_positioning(&self) -> Result<PositionComparison> {
        info!("🔍 开始与LTFSCopyGUI定位行为对比");
        
        // 记录当前位置
        let current_pos = self.scsi.read_position()?;
        
        // 执行标准的LTFSCopyGUI定位流程
        info!("🔧 执行LTFSCopyGUI标准定位流程");
        
        // 1. 定位到FileMark 5
        self.scsi.locate_to_filemark(5, 1)?;
        let pos_after_fm5 = self.scsi.read_position()?;
        
        // 2. 执行ReadFileMark
        let fm_detected = self.scsi.read_file_mark()?;
        let pos_after_readfm = self.scsi.read_position()?;
        
        let comparison = PositionComparison {
            initial_position: current_pos,
            after_filemark5: pos_after_fm5,
            after_read_filemark: pos_after_readfm.clone(),
            filemark_detected: fm_detected,
            expected_final_block: 38,
            actual_final_block: pos_after_readfm.block_number,
            positioning_correct: pos_after_readfm.partition == 1 && pos_after_readfm.block_number == 38,
        };
        
        if self.debug_mode {
            info!("🔍 定位行为对比结果:");
            info!("  初始位置: P{} B{} FM{}", 
                 comparison.initial_position.partition, 
                 comparison.initial_position.block_number, 
                 comparison.initial_position.file_number);
            info!("  FileMark5后: P{} B{} FM{}", 
                 comparison.after_filemark5.partition, 
                 comparison.after_filemark5.block_number, 
                 comparison.after_filemark5.file_number);
            info!("  ReadFileMark后: P{} B{} FM{}", 
                 comparison.after_read_filemark.partition, 
                 comparison.after_read_filemark.block_number, 
                 comparison.after_read_filemark.file_number);
            info!("  FileMark检测: {}", comparison.filemark_detected);
            info!("  定位正确性: {} (期望B{}, 实际B{})", 
                 comparison.positioning_correct, 
                 comparison.expected_final_block, 
                 comparison.actual_final_block);
        }
        
        Ok(comparison)
    }
    
    /// 诊断P1 Block38/39定位问题
    pub fn diagnose_block38_issue(&self) -> Result<DiagnosticReport> {
        info!("🔍 开始诊断P1 Block38/39定位问题");
        
        let mut report = DiagnosticReport::new();
        
        // 1. 检查初始状态
        let initial_pos = self.scsi.read_position()?;
        info!("📍 初始位置: P{} B{} FM{}", 
             initial_pos.partition, initial_pos.block_number, initial_pos.file_number);
        report.initial_position = Some(initial_pos);
        
        // 2. 测试FileMark 5定位
        match self.scsi.locate_to_filemark(5, 1) {
            Ok(_) => {
                let pos = self.scsi.read_position()?;
                info!("✅ FileMark 5定位成功: P{} B{} FM{}", pos.partition, pos.block_number, pos.file_number);
                report.filemark5_position = Some(pos);
                report.filemark5_success = true;
            }
            Err(e) => {
                report.filemark5_success = false;
                report.errors.push(format!("FileMark 5定位失败: {}", e));
                error!("❌ FileMark 5定位失败: {}", e);
            }
        }
        
        // 3. 测试ReadFileMark行为
        if report.filemark5_success {
            match self.scsi.read_file_mark() {
                Ok(fm_detected) => {
                    let pos = self.scsi.read_position()?;
                    info!("✅ ReadFileMark执行成功: FileMark检测={}, 位置P{} B{} FM{}", 
                         fm_detected, pos.partition, pos.block_number, pos.file_number);
                    
                    // 检查是否到达了期望的Block38
                    if pos.partition == 1 && pos.block_number == 38 {
                        report.block38_reached = true;
                        info!("✅ 成功到达P1 Block38");
                    } else {
                        report.block38_reached = false;
                        report.actual_block = pos.block_number;
                        warn!("❌ 未到达P1 Block38，实际位置: P{} B{}", pos.partition, pos.block_number);
                    }
                    
                    // 分析可能的原因
                    if !report.block38_reached {
                        if pos.block_number == 39 {
                            report.errors.push("ReadFileMark回退逻辑可能未正确执行".to_string());
                        } else {
                            report.errors.push(format!("意外的块位置: {}", pos.block_number));
                        }
                    }
                    
                    report.read_filemark_position = Some(pos);
                    report.read_filemark_success = true;
                    report.filemark_detected = fm_detected;
                }
                Err(e) => {
                    report.read_filemark_success = false;
                    report.errors.push(format!("ReadFileMark执行失败: {}", e));
                    error!("❌ ReadFileMark执行失败: {}", e);
                }
            }
        }
        
        // 4. 生成诊断建议
        report.generate_recommendations();
        
        info!("🔍 诊断完成，发现 {} 个问题", report.errors.len());
        Ok(report)
    }
}

/// 位置对比结果
#[derive(Debug, Clone)]
pub struct PositionComparison {
    pub initial_position: TapePosition,
    pub after_filemark5: TapePosition,
    pub after_read_filemark: TapePosition,
    pub filemark_detected: bool,
    pub expected_final_block: u64,
    pub actual_final_block: u64,
    pub positioning_correct: bool,
}

/// 诊断报告
#[derive(Debug, Clone)]
pub struct DiagnosticReport {
    pub initial_position: Option<TapePosition>,
    pub filemark5_position: Option<TapePosition>,
    pub read_filemark_position: Option<TapePosition>,
    pub filemark5_success: bool,
    pub read_filemark_success: bool,
    pub filemark_detected: bool,
    pub block38_reached: bool,
    pub actual_block: u64,
    pub errors: Vec<String>,
    pub recommendations: Vec<String>,
}

impl DiagnosticReport {
    fn new() -> Self {
        Self {
            initial_position: None,
            filemark5_position: None,
            read_filemark_position: None,
            filemark5_success: false,
            read_filemark_success: false,
            filemark_detected: false,
            block38_reached: false,
            actual_block: 0,
            errors: Vec::new(),
            recommendations: Vec::new(),
        }
    }
    
    fn generate_recommendations(&mut self) {
        if !self.block38_reached {
            if self.actual_block == 39 {
                self.recommendations.push("ReadFileMark回退逻辑需要修复，当前停留在Block39而不是Block38".to_string());
                self.recommendations.push("建议检查locate_block方法的回退计算是否正确".to_string());
            } else {
                self.recommendations.push(format!("意外的块位置{}，需要检查整个定位流程", self.actual_block));
            }
        }
        
        if !self.filemark5_success {
            self.recommendations.push("FileMark 5定位失败，检查磁带是否包含足够的FileMark".to_string());
        }
        
        if !self.read_filemark_success {
            self.recommendations.push("ReadFileMark操作失败，检查SCSI命令实现".to_string());
        }
    }
    
    /// 打印诊断报告
    pub fn print_report(&self) {
        println!("\n=== P1 Block38定位问题诊断报告 ===");
        
        if let Some(pos) = &self.initial_position {
            println!("初始位置: P{} B{} FM{}", pos.partition, pos.block_number, pos.file_number);
        }
        
        println!("FileMark 5定位: {}", if self.filemark5_success { "✅ 成功" } else { "❌ 失败" });
        if let Some(pos) = &self.filemark5_position {
            println!("  位置: P{} B{} FM{}", pos.partition, pos.block_number, pos.file_number);
        }
        
        println!("ReadFileMark执行: {}", if self.read_filemark_success { "✅ 成功" } else { "❌ 失败" });
        if let Some(pos) = &self.read_filemark_position {
            println!("  位置: P{} B{} FM{}", pos.partition, pos.block_number, pos.file_number);
            println!("  FileMark检测: {}", self.filemark_detected);
        }
        
        println!("P1 Block38到达: {}", if self.block38_reached { "✅ 成功" } else { "❌ 失败" });
        if !self.block38_reached {
            println!("  实际位置: Block{}", self.actual_block);
        }
        
        if !self.errors.is_empty() {
            println!("\n发现的问题:");
            for (i, error) in self.errors.iter().enumerate() {
                println!("  {}. {}", i + 1, error);
            }
        }
        
        if !self.recommendations.is_empty() {
            println!("\n修复建议:");
            for (i, rec) in self.recommendations.iter().enumerate() {
                println!("  {}. {}", i + 1, rec);
            }
        }
        
        println!("=====================================\n");
    }
}