use crate::error::{Result, RustLtfsError};
use tracing::{debug, info, error};

/// 索引数据验证器 - 专门验证从P1 Block38读取的索引数据
/// 
/// 这个模块实现了严格的索引数据验证机制，确保拒绝全零数据，
/// 验证XML格式完整性，并检查LTFS索引的有效性。
pub struct IndexValidator {
    strict_mode: bool,
    debug_mode: bool,
}

/// 索引验证错误类型
#[derive(Debug, thiserror::Error)]
pub enum IndexValidationError {
    #[error("索引数据为空")]
    EmptyData,
    
    #[error("索引数据全为零 (读取了 {size} 字节的零数据)")]
    AllZeroData { size: usize },
    
    #[error("XML格式无效: {reason}")]
    InvalidXmlFormat { reason: String },
    
    #[error("LTFS索引标识缺失: {details}")]
    MissingLtfsIdentifier { details: String },
    
    #[error("LTFS索引结构不完整: {missing_elements}")]
    IncompleteStructure { missing_elements: String },
    
    #[error("索引内容验证失败: {reason}")]
    ContentValidationFailed { reason: String },
    
    #[error("索引版本不支持: {version}")]
    UnsupportedVersion { version: String },
}

impl From<IndexValidationError> for RustLtfsError {
    fn from(err: IndexValidationError) -> Self {
        RustLtfsError::ltfs_index(err.to_string())
    }
}

/// 索引验证结果
#[derive(Debug, Clone)]
pub struct ValidationResult {
    pub is_valid: bool,
    pub data_size: usize,
    pub has_ltfs_identifier: bool,
    pub xml_well_formed: bool,
    pub ltfs_version: Option<String>,
    pub volume_uuid: Option<String>,
    pub generation_number: Option<u64>,
    pub file_count_estimate: Option<usize>,
    pub warnings: Vec<String>,
    pub errors: Vec<String>,
}

impl IndexValidator {
    /// 创建新的索引验证器
    pub fn new() -> Self {
        Self {
            strict_mode: true,  // 默认启用严格模式
            debug_mode: false,
        }
    }
    
    /// 创建宽松模式的验证器（用于兼容性测试）
    pub fn new_lenient() -> Self {
        Self {
            strict_mode: false,
            debug_mode: false,
        }
    }
    
    /// 启用或禁用严格模式
    pub fn set_strict_mode(&mut self, enabled: bool) {
        self.strict_mode = enabled;
    }
    
    /// 启用或禁用调试模式
    pub fn set_debug_mode(&mut self, enabled: bool) {
        self.debug_mode = enabled;
    }
    
    /// 验证索引数据的完整性和有效性
    pub fn validate_index_data(&self, data: &[u8]) -> Result<ValidationResult> {
        info!("🔍 开始验证索引数据 (大小: {} 字节, 严格模式: {})", 
             data.len(), self.strict_mode);
        
        let mut result = ValidationResult {
            is_valid: false,
            data_size: data.len(),
            has_ltfs_identifier: false,
            xml_well_formed: false,
            ltfs_version: None,
            volume_uuid: None,
            generation_number: None,
            file_count_estimate: None,
            warnings: Vec::new(),
            errors: Vec::new(),
        };
        
        // 步骤1: 基本数据检查
        self.validate_basic_data(data, &mut result)?;
        
        // 步骤2: XML格式验证
        let xml_content = String::from_utf8_lossy(data);
        self.validate_xml_format(&xml_content, &mut result)?;
        
        // 步骤3: LTFS标识验证
        self.validate_ltfs_identifier(&xml_content, &mut result)?;
        
        // 步骤4: LTFS结构验证
        self.validate_ltfs_structure(&xml_content, &mut result)?;
        
        // 步骤5: 内容完整性验证
        self.validate_content_integrity(&xml_content, &mut result)?;
        
        // 步骤6: 最终验证结果
        result.is_valid = result.errors.is_empty() && result.has_ltfs_identifier && result.xml_well_formed;
        
        if result.is_valid {
            info!("✅ 索引数据验证通过: {} 字节的有效LTFS索引", result.data_size);
        } else {
            error!("❌ 索引数据验证失败: {} 个错误", result.errors.len());
        }
        
        if self.debug_mode {
            self.print_validation_details(&result);
        }
        
        Ok(result)
    }
    
    /// 验证基本数据完整性
    fn validate_basic_data(&self, data: &[u8], result: &mut ValidationResult) -> Result<()> {
        debug!("🔍 验证基本数据完整性");
        
        // 检查数据是否为空
        if data.is_empty() {
            result.errors.push("索引数据为空".to_string());
            if self.strict_mode {
                return Err(IndexValidationError::EmptyData.into());
            }
        }
        
        // 检查是否为全零数据
        if data.iter().all(|&b| b == 0) {
            let error_msg = format!("索引数据全为零 ({} 字节)", data.len());
            result.errors.push(error_msg.clone());
            if self.strict_mode {
                return Err(IndexValidationError::AllZeroData { size: data.len() }.into());
            }
        }
        
        // 检查数据大小合理性
        if data.len() < 100 {
            result.warnings.push("索引数据过小，可能不完整".to_string());
        } else if data.len() > 100_000_000 {  // 100MB
            result.warnings.push("索引数据过大，可能包含非索引内容".to_string());
        }
        
        // 检查是否包含可打印字符
        let printable_count = data.iter().filter(|&&b| b >= 32 && b <= 126).count();
        let printable_ratio = printable_count as f64 / data.len() as f64;
        
        if printable_ratio < 0.5 {
            result.warnings.push(format!("可打印字符比例较低 ({:.1}%)，可能不是文本数据", printable_ratio * 100.0));
        }
        
        debug!("✅ 基本数据检查完成: {} 字节, {:.1}% 可打印字符", data.len(), printable_ratio * 100.0);
        Ok(())
    }
    
    /// 验证XML格式
    fn validate_xml_format(&self, xml_content: &str, result: &mut ValidationResult) -> Result<()> {
        debug!("🔍 验证XML格式");
        
        // 检查XML声明
        if !xml_content.trim_start().starts_with("<?xml") {
            result.warnings.push("缺少XML声明".to_string());
        }
        
        // 检查基本XML结构
        let open_tags = xml_content.matches('<').count();
        let close_tags = xml_content.matches('>').count();
        
        if open_tags != close_tags {
            result.errors.push(format!("XML标签不匹配: {} 个开标签, {} 个闭标签", open_tags, close_tags));
            if self.strict_mode {
                return Err(IndexValidationError::InvalidXmlFormat {
                    reason: "标签不匹配".to_string()
                }.into());
            }
        }
        
        // 检查是否包含控制字符
        if xml_content.chars().any(|c| c.is_control() && c != '\n' && c != '\r' && c != '\t') {
            result.warnings.push("XML包含控制字符".to_string());
        }
        
        // 尝试基本的XML解析验证
        match self.basic_xml_parse_check(xml_content) {
            Ok(_) => {
                result.xml_well_formed = true;
                debug!("✅ XML格式验证通过");
            }
            Err(e) => {
                result.errors.push(format!("XML解析失败: {}", e));
                if self.strict_mode {
                    return Err(IndexValidationError::InvalidXmlFormat {
                        reason: e.to_string()
                    }.into());
                }
            }
        }
        
        Ok(())
    }
    
    /// 验证LTFS标识
    fn validate_ltfs_identifier(&self, xml_content: &str, result: &mut ValidationResult) -> Result<()> {
        debug!("🔍 验证LTFS标识");
        
        // 检查LTFS索引根元素
        if xml_content.contains("<ltfsindex") {
            result.has_ltfs_identifier = true;
            debug!("✅ 发现LTFS索引根元素");
        } else {
            result.errors.push("缺少LTFS索引根元素 <ltfsindex>".to_string());
            if self.strict_mode {
                return Err(IndexValidationError::MissingLtfsIdentifier {
                    details: "未找到 <ltfsindex> 元素".to_string()
                }.into());
            }
        }
        
        // 检查LTFS索引结束标签
        if !xml_content.contains("</ltfsindex>") {
            result.errors.push("缺少LTFS索引结束标签 </ltfsindex>".to_string());
            if self.strict_mode {
                return Err(IndexValidationError::MissingLtfsIdentifier {
                    details: "未找到 </ltfsindex> 结束标签".to_string()
                }.into());
            }
        }
        
        // 提取LTFS版本
        if let Some(version) = self.extract_ltfs_version(xml_content) {
            result.ltfs_version = Some(version.clone());
            debug!("✅ LTFS版本: {}", version);
            
            // 检查版本兼容性
            if !version.starts_with("2.") {
                result.warnings.push(format!("LTFS版本 {} 可能不完全兼容", version));
            }
        } else {
            result.warnings.push("无法提取LTFS版本信息".to_string());
        }
        
        Ok(())
    }
    
    /// 验证LTFS结构
    fn validate_ltfs_structure(&self, xml_content: &str, result: &mut ValidationResult) -> Result<()> {
        debug!("🔍 验证LTFS结构");
        
        let mut missing_elements = Vec::new();
        
        // 检查必需的LTFS元素
        let required_elements = [
            ("volumeuuid", "卷UUID"),
            ("generationnumber", "生成号"),
            ("updatetime", "更新时间"),
            ("directory", "根目录"),
        ];
        
        for (element, description) in &required_elements {
            if !xml_content.contains(&format!("<{}>", element)) && !xml_content.contains(&format!("<{} ", element)) {
                missing_elements.push(description.to_string());
            }
        }
        
        if !missing_elements.is_empty() {
            let missing_str = missing_elements.join(", ");
            result.errors.push(format!("缺少必需元素: {}", missing_str));
            if self.strict_mode {
                return Err(IndexValidationError::IncompleteStructure {
                    missing_elements: missing_str
                }.into());
            }
        }
        
        // 提取关键信息
        result.volume_uuid = self.extract_volume_uuid(xml_content);
        result.generation_number = self.extract_generation_number(xml_content);
        result.file_count_estimate = self.estimate_file_count(xml_content);
        
        if let Some(uuid) = &result.volume_uuid {
            debug!("✅ 卷UUID: {}", uuid);
        }
        
        if let Some(gen) = result.generation_number {
            debug!("✅ 生成号: {}", gen);
        }
        
        if let Some(count) = result.file_count_estimate {
            debug!("✅ 估计文件数: {}", count);
        }
        
        Ok(())
    }
    
    /// 验证内容完整性
    fn validate_content_integrity(&self, xml_content: &str, result: &mut ValidationResult) -> Result<()> {
        debug!("🔍 验证内容完整性");
        
        // 检查XML是否被截断
        if !xml_content.trim_end().ends_with("</ltfsindex>") {
            result.errors.push("LTFS索引可能被截断".to_string());
            if self.strict_mode {
                return Err(IndexValidationError::ContentValidationFailed {
                    reason: "索引内容不完整".to_string()
                }.into());
            }
        }
        
        // 检查是否包含文件或目录信息
        let has_files = xml_content.contains("<file>");
        let has_directories = xml_content.contains("<directory>");
        
        if !has_files && !has_directories {
            result.warnings.push("索引中未发现文件或目录信息".to_string());
        } else {
            if has_files {
                debug!("✅ 索引包含文件信息");
            }
            if has_directories {
                debug!("✅ 索引包含目录信息");
            }
        }
        
        // 检查字符编码
        if xml_content.contains('\0') {
            result.warnings.push("索引包含空字符，可能存在编码问题".to_string());
        }
        
        // 检查内容长度合理性
        if xml_content.len() < 500 {
            result.warnings.push("索引内容过短，可能不完整".to_string());
        }
        
        debug!("✅ 内容完整性检查完成");
        Ok(())
    }
    
    /// 基本XML解析检查
    fn basic_xml_parse_check(&self, xml_content: &str) -> Result<()> {
        // 简单的XML平衡检查
        let mut stack = Vec::new();
        let mut in_tag = false;
        let mut tag_name = String::new();
        let mut is_closing = false;
        
        for ch in xml_content.chars() {
            match ch {
                '<' => {
                    in_tag = true;
                    tag_name.clear();
                    is_closing = false;
                }
                '>' => {
                    if in_tag && !tag_name.is_empty() {
                        if tag_name.starts_with('?') || tag_name.starts_with('!') {
                            // XML声明或注释，忽略
                        } else if tag_name.ends_with('/') {
                            // 自闭合标签，忽略
                        } else if is_closing {
                            // 闭合标签
                            if let Some(last_tag) = stack.pop() {
                                if last_tag != tag_name {
                                    return Err(RustLtfsError::parse(format!(
                                        "XML标签不匹配: 期望 {}, 实际 {}", last_tag, tag_name
                                    )));
                                }
                            } else {
                                return Err(RustLtfsError::parse(format!(
                                    "多余的闭合标签: {}", tag_name
                                )));
                            }
                        } else {
                            // 开放标签
                            stack.push(tag_name.clone());
                        }
                    }
                    in_tag = false;
                }
                '/' if in_tag && tag_name.is_empty() => {
                    is_closing = true;
                }
                _ if in_tag => {
                    if ch.is_alphanumeric() || ch == '_' || ch == '-' || ch == ':' {
                        tag_name.push(ch);
                    }
                }
                _ => {}
            }
        }
        
        if !stack.is_empty() {
            return Err(RustLtfsError::parse(format!(
                "未闭合的XML标签: {:?}", stack
            )));
        }
        
        Ok(())
    }
    
    /// 提取LTFS版本
    fn extract_ltfs_version(&self, xml_content: &str) -> Option<String> {
        // 查找 version="..." 属性
        if let Some(start) = xml_content.find("version=\"") {
            let start = start + 9; // "version=\"".len()
            if let Some(end) = xml_content[start..].find('"') {
                return Some(xml_content[start..start + end].to_string());
            }
        }
        None
    }
    
    /// 提取卷UUID
    fn extract_volume_uuid(&self, xml_content: &str) -> Option<String> {
        if let Some(start) = xml_content.find("<volumeuuid>") {
            let start = start + 12; // "<volumeuuid>".len()
            if let Some(end) = xml_content[start..].find("</volumeuuid>") {
                return Some(xml_content[start..start + end].to_string());
            }
        }
        None
    }
    
    /// 提取生成号
    fn extract_generation_number(&self, xml_content: &str) -> Option<u64> {
        if let Some(start) = xml_content.find("<generationnumber>") {
            let start = start + 18; // "<generationnumber>".len()
            if let Some(end) = xml_content[start..].find("</generationnumber>") {
                let gen_str = &xml_content[start..start + end];
                return gen_str.parse().ok();
            }
        }
        None
    }
    
    /// 估计文件数量
    fn estimate_file_count(&self, xml_content: &str) -> Option<usize> {
        let file_count = xml_content.matches("<file>").count();
        if file_count > 0 {
            Some(file_count)
        } else {
            None
        }
    }
    
    /// 打印验证详情
    fn print_validation_details(&self, result: &ValidationResult) {
        println!("\n=== 索引数据验证详情 ===");
        println!("数据大小: {} 字节", result.data_size);
        println!("LTFS标识: {}", if result.has_ltfs_identifier { "✅ 存在" } else { "❌ 缺失" });
        println!("XML格式: {}", if result.xml_well_formed { "✅ 有效" } else { "❌ 无效" });
        
        if let Some(version) = &result.ltfs_version {
            println!("LTFS版本: {}", version);
        }
        
        if let Some(uuid) = &result.volume_uuid {
            println!("卷UUID: {}", uuid);
        }
        
        if let Some(gen) = result.generation_number {
            println!("生成号: {}", gen);
        }
        
        if let Some(count) = result.file_count_estimate {
            println!("估计文件数: {}", count);
        }
        
        if !result.warnings.is_empty() {
            println!("\n警告:");
            for warning in &result.warnings {
                println!("  ⚠️ {}", warning);
            }
        }
        
        if !result.errors.is_empty() {
            println!("\n错误:");
            for error in &result.errors {
                println!("  ❌ {}", error);
            }
        }
        
        println!("验证结果: {}", if result.is_valid { "✅ 通过" } else { "❌ 失败" });
        println!("========================\n");
    }
}

impl Default for IndexValidator {
    fn default() -> Self {
        Self::new()
    }
}

/// 便捷函数：验证索引数据
pub fn validate_index_data(data: &[u8]) -> Result<ValidationResult> {
    let validator = IndexValidator::new();
    validator.validate_index_data(data)
}

/// 便捷函数：验证索引数据（宽松模式）
pub fn validate_index_data_lenient(data: &[u8]) -> Result<ValidationResult> {
    let validator = IndexValidator::new_lenient();
    validator.validate_index_data(data)
}

/// 便捷函数：快速检查索引数据是否有效
pub fn is_valid_index_data(data: &[u8]) -> bool {
    match validate_index_data(data) {
        Ok(result) => result.is_valid,
        Err(_) => false,
    }
}