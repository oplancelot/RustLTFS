use crate::error::{Result, RustLtfsError};
use crate::scsi::ScsiInterface;
use std::sync::Arc;
use tracing::{debug, info, warn};

/// 磁带容量信息结构（对应LTFSCopyGUI的RefreshCapacity返回值）
#[derive(Debug, Clone)]
pub struct TapeCapacityInfo {
    /// 分区0剩余容量 (KB)
    pub p0_remaining: u64,
    /// 分区0最大容量 (KB)  
    pub p0_maximum: u64,
    /// 分区1剩余容量 (KB) - 仅多分区磁带
    pub p1_remaining: u64,
    /// 分区1最大容量 (KB) - 仅多分区磁带
    pub p1_maximum: u64,
    /// 媒体描述字符串
    pub media_description: String,
    /// 错误率日志值
    pub error_rate_log_value: f64,
    /// 容量损失 (bytes) - 可选
    pub capacity_loss: Option<u64>,
    /// 是否为WORM磁带
    pub is_worm: bool,
    /// 是否为只读磁带
    pub is_write_protected: bool,
    /// 磁带代数信息
    pub generation_info: String,
}

/// 磁带错误率信息
#[derive(Debug, Clone)]
pub struct ErrorRateInfo {
    pub channel_error_rates: Vec<f64>,
    pub overall_error_rate: f64,
    pub c1_errors: Vec<u32>,
    pub no_ccps: Vec<u32>,
}

/// 分区容量页面数据解析器（对应LTFSCopyGUI的TapeCapacityLogPage）
pub struct CapacityPageParser {
    page_data: Vec<u8>,
}

impl CapacityPageParser {
    pub fn new(page_data: Vec<u8>) -> Self {
        Self { page_data }
    }

    /// 获取指定分区的剩余容量（精确对应LTFSCopyGUI的TryGetPage功能）
    pub fn get_remaining_capacity(&self, partition_id: u8) -> Result<u64> {
        // 根据LTFSCopyGUI实现：
        // - Parameter Code 1: P0剩余容量  
        // - Parameter Code 2: P1剩余容量
        let param_code = partition_id + 1;
        self.extract_capacity_value(param_code)
    }

    /// 获取指定分区的最大容量（精确对应LTFSCopyGUI的TryGetPage功能）
    pub fn get_maximum_capacity(&self, partition_id: u8) -> Result<u64> {
        // 根据LTFSCopyGUI实现：
        // - Parameter Code 3: P0最大容量
        // - Parameter Code 4: P1最大容量
        let param_code = partition_id + 3;
        self.extract_capacity_value(param_code)
    }

    /// 从页面数据中提取容量值（精确对应LTFSCopyGUI的CapacityLogPage.TryGetPage）
    fn extract_capacity_value(&self, param_code: u8) -> Result<u64> {
        debug!("Searching for parameter code {} in capacity log page", param_code);
        
        if self.page_data.len() < 4 {
            warn!("Capacity log page too short: {} bytes", self.page_data.len());
            return Ok(0);
        }

        // SCSI log page format:
        // Bytes 0-1: Page code (0x31) + flags  
        // Bytes 2-3: Page length
        let page_length = u16::from_be_bytes([self.page_data[2], self.page_data[3]]) as usize;
        debug!("Capacity log page length: {} bytes, actual data length: {} bytes", page_length, self.page_data.len());

        // Parameter entries start at offset 4
        let mut offset = 4;
        
        while offset + 4 <= self.page_data.len() && offset < 4 + page_length {
            if offset + 3 >= self.page_data.len() {
                warn!("Parameter header extends beyond page boundary at offset {}", offset);
                break;
            }

            // Parameter entry format:
            // Bytes 0-1: Parameter code (big-endian)
            // Byte 2: Control flags
            // Byte 3: Parameter length
            let current_param_code = u16::from_be_bytes([
                self.page_data[offset], 
                self.page_data[offset + 1]
            ]);
            let param_length = self.page_data[offset + 3] as usize;

            debug!("Found parameter code: {}, length: {}, offset: {}", current_param_code, param_length, offset);

            if current_param_code == param_code as u16 {
                debug!("Found target parameter code {}", param_code);
                
                // 容量数据从parameter header之后开始（offset + 4）
                let data_start = offset + 4;
                
                // 检查数据是否超出页面边界
                if data_start + param_length > self.page_data.len() {
                    warn!("Parameter {} data extends beyond page boundary: start={}, length={}, page_size={}", 
                          param_code, data_start, param_length, self.page_data.len());
                    
                    // 如果数据被截断，尝试使用可用的数据
                    let available_bytes = self.page_data.len().saturating_sub(data_start);
                    if available_bytes >= 4 {
                        warn!("Using truncated 4-byte capacity value for parameter {}", param_code);
                        let capacity = u32::from_be_bytes([
                            self.page_data[data_start],
                            self.page_data[data_start + 1], 
                            self.page_data[data_start + 2],
                            self.page_data[data_start + 3],
                        ]) as u64;
                        debug!("Extracted truncated capacity value for param code {} (4 bytes): {} KB", param_code, capacity);
                        return Ok(capacity);
                    } else {
                        warn!("Insufficient data for parameter {}: only {} bytes available", param_code, available_bytes);
                        return Ok(0);
                    }
                }
                
                let capacity = if param_length >= 8 {
                    // 8字节容量值（标准SCSI格式）
                    u64::from_be_bytes([
                        self.page_data[data_start],
                        self.page_data[data_start + 1], 
                        self.page_data[data_start + 2],
                        self.page_data[data_start + 3],
                        self.page_data[data_start + 4],
                        self.page_data[data_start + 5],
                        self.page_data[data_start + 6],
                        self.page_data[data_start + 7],
                    ])
                } else if param_length >= 4 {
                    // 4字节容量值（实际遇到的格式）
                    u32::from_be_bytes([
                        self.page_data[data_start],
                        self.page_data[data_start + 1], 
                        self.page_data[data_start + 2],
                        self.page_data[data_start + 3],
                    ]) as u64
                } else {
                    warn!("Parameter data too short: {} bytes, need at least 4", param_length);
                    return Ok(0);
                };
                
                debug!("Extracted capacity value for param code {} ({} bytes): {} KB", param_code, param_length, capacity);
                return Ok(capacity);
            }

            // 移动到下一个parameter entry，但要检查边界
            let next_offset = offset + 4 + param_length;
            if next_offset > self.page_data.len() {
                debug!("Next parameter offset {} exceeds page boundary, stopping search", next_offset);
                break;
            }
            offset = next_offset;
        }

        debug!("Parameter code {} not found in capacity log page", param_code);
        Ok(0)
    }
}

/// 卷统计页面数据解析器（对应LTFSCopyGUI的VolumeStatisticsLogPage）
pub struct VolumeStatsPageParser {
    page_data: Vec<u8>,
}

impl VolumeStatsPageParser {
    pub fn new(page_data: Vec<u8>) -> Self {
        Self { page_data }
    }

    /// 获取磁带代数信息（页面0x45）
    pub fn get_generation_info(&self) -> Result<Option<String>> {
        if let Some(gen_data) = self.get_page_data(0x45)? {
            if let Ok(gen_str) = String::from_utf8(gen_data) {
                let cleaned = gen_str.trim_end_matches('\0').trim();
                if !cleaned.is_empty() {
                    return Ok(Some(cleaned.to_string()));
                }
            }
        }
        Ok(None)
    }

    /// 获取WORM状态（页面0x81）
    pub fn get_worm_status(&self) -> Result<bool> {
        if let Some(worm_data) = self.get_page_data(0x81)? {
            if !worm_data.is_empty() {
                return Ok(worm_data[worm_data.len() - 1] != 0);
            }
        }
        Ok(false)
    }

    /// 获取写保护状态（页面0x80）
    pub fn get_write_protection_status(&self) -> Result<bool> {
        if let Some(wp_data) = self.get_page_data(0x80)? {
            if !wp_data.is_empty() {
                return Ok(wp_data[wp_data.len() - 1] != 0);
            }
        }
        Ok(false)
    }

    /// 获取指定页面的数据
    fn get_page_data(&self, page_id: u8) -> Result<Option<Vec<u8>>> {
        let mut offset = 4; // 跳过页面头部
        
        while offset + 4 < self.page_data.len() {
            let current_page = self.page_data[offset + 1];
            let param_length = u16::from_be_bytes([
                self.page_data[offset + 2], 
                self.page_data[offset + 3]
            ]);

            if current_page == page_id {
                // 找到目标页面
                if offset + 4 + param_length as usize <= self.page_data.len() {
                    let data = self.page_data[offset + 4..offset + 4 + param_length as usize].to_vec();
                    return Ok(Some(data));
                }
                break;
            }

            // 移动到下一个页面
            offset += 4 + param_length as usize;
        }

        Ok(None)
    }
}

/// 容量管理器 - 专门处理磁带容量管理的结构体
pub struct CapacityManager {
    scsi: Arc<ScsiInterface>,
    offline_mode: bool,
    /// 上次C1错误计数（用于错误率计算）
    last_c1_errors: Vec<u32>,
    /// 上次CCP计数（用于错误率计算）
    last_no_ccps: Vec<u32>,
    /// 错误率历史记录
    error_rate_history: Vec<f64>,
}

impl CapacityManager {
    /// 创建新的容量管理器实例
    pub fn new(scsi: Arc<ScsiInterface>, offline_mode: bool) -> Self {
        Self {
            scsi,
            offline_mode,
            last_c1_errors: vec![0; 8], // 支持最多8个通道
            last_no_ccps: vec![0; 8],
            error_rate_history: Vec::new(),
        }
    }

    /// 完整的容量刷新功能（精确对应LTFSCopyGUI RefreshCapacity）
    pub async fn refresh_capacity(&mut self, extra_partition_count: u8) -> Result<TapeCapacityInfo> {
        info!("Starting RefreshCapacity (LTFSCopyGUI compatible)");

        if self.offline_mode {
            info!("Offline mode: returning simulated capacity data");
            return Ok(self.get_simulated_capacity_info());
        }

        let mut capacity_info = TapeCapacityInfo {
            p0_remaining: 0,
            p0_maximum: 0,
            p1_remaining: 0,
            p1_maximum: 0,
            media_description: "Unknown".to_string(),
            error_rate_log_value: 0.0,
            capacity_loss: None,
            is_worm: false,
            is_write_protected: false,
            generation_info: "".to_string(),
        };

        // 步骤1: 读取磁带容量日志页面（0x31）
        info!("Reading tape capacity log page (0x31)");
        let capacity_log_data = match self.scsi.log_sense(0x31, 1) {
            Ok(data) => data,
            Err(e) => {
                warn!("Failed to read capacity log page: {}", e);
                return Ok(capacity_info);
            }
        };

        // 步骤2: 读取卷统计日志页面（0x17）
        info!("Reading volume statistics log page (0x17)");
        let volume_stats_data = match self.scsi.log_sense(0x17, 1) {
            Ok(data) => data,
            Err(e) => {
                warn!("Failed to read volume statistics page: {}", e);
                Vec::new()
            }
        };

        // 步骤3: 解析容量信息
        let capacity_parser = CapacityPageParser::new(capacity_log_data);
        
        capacity_info.p0_remaining = capacity_parser.get_remaining_capacity(0)?;
        capacity_info.p0_maximum = capacity_parser.get_maximum_capacity(0)?;
        
        if extra_partition_count > 0 {
            capacity_info.p1_remaining = capacity_parser.get_remaining_capacity(1)?;
            capacity_info.p1_maximum = capacity_parser.get_maximum_capacity(1)?;
        }

        // 步骤4: 解析卷统计信息
        if !volume_stats_data.is_empty() {
            let volume_parser = VolumeStatsPageParser::new(volume_stats_data);
            
            if let Ok(Some(gen_info)) = volume_parser.get_generation_info() {
                capacity_info.generation_info = self.parse_generation_string(&gen_info);
            }
            
            capacity_info.is_worm = volume_parser.get_worm_status()?;
            capacity_info.is_write_protected = volume_parser.get_write_protection_status()?;
        }

        // 步骤5: 读取错误率信息
        capacity_info.error_rate_log_value = self.read_error_rate_info().await?;

        // 步骤6: 构建媒体描述
        capacity_info.media_description = self.build_media_description(
            &capacity_info.generation_info,
            capacity_info.is_worm,
            capacity_info.is_write_protected,
        );

        // 步骤7: 尝试读取MAM属性作为后备（如果日志页面没有提供足够信息）
        if capacity_info.p0_maximum == 0 {
            capacity_info.p0_maximum = self.read_mam_capacity(0, extra_partition_count)?;
        }
        
        if extra_partition_count > 0 && capacity_info.p1_maximum == 0 {
            capacity_info.p1_maximum = self.read_mam_capacity(1, extra_partition_count)?;
        }

        info!("RefreshCapacity completed: P0={}/{} KB, P1={}/{} KB, Media={}", 
              capacity_info.p0_remaining, capacity_info.p0_maximum,
              capacity_info.p1_remaining, capacity_info.p1_maximum,
              capacity_info.media_description);

        Ok(capacity_info)
    }

    /// 读取错误率信息（对应LTFSCopyGUI ReadChanLRInfo）
    pub async fn read_error_rate_info(&mut self) -> Result<f64> {
        info!("Reading channel error rate information (ReadChanLRInfo)");

        // 读取WERL（Write Error Log）页面数据
        // 这是LTFSCopyGUI中用于计算错误率的特殊SCSI命令
        let werl_header = self.read_werl_header()?;
        if werl_header.len() != 4 {
            warn!("Invalid WERL page header, skipping error rate check");
            return Ok(0.0);
        }

        let page_length = ((werl_header[2] as u16) << 8) | (werl_header[3] as u16);
        if page_length == 0 {
            warn!("WERL page is empty, skipping error rate check");
            return Ok(0.0);
        }

        let full_page = self.read_werl_page(page_length + 4)?;
        if full_page.len() < 4 {
            return Ok(0.0);
        }

        // 解析WERL数据并计算错误率
        let error_rate = self.parse_werl_data(&full_page[4..])?;
        
        info!("Error rate calculation completed: {}", error_rate);
        Ok(error_rate)
    }

    /// 读取WERL页面头部
    fn read_werl_header(&self) -> Result<Vec<u8>> {
        // 构建SCSI CDB以读取WERL页面头部
        let cdb = [0x1C, 0x01, 0x88, 0x00, 0x04, 0x00]; // 对应LTFSCopyGUI的CDB
        let mut buffer = vec![0u8; 4];
        
        match self.scsi.send_scsi_command(&cdb, &mut buffer, 1) {
            Ok(true) => Ok(buffer),
            Ok(false) => Err(RustLtfsError::scsi("WERL header read failed")),
            Err(e) => Err(e),
        }
    }

    /// 读取完整WERL页面
    fn read_werl_page(&self, page_length: u16) -> Result<Vec<u8>> {
        let cdb = [
            0x1C, 0x01, 0x88, 
            ((page_length >> 8) & 0xFF) as u8, 
            (page_length & 0xFF) as u8, 
            0x00
        ];
        let mut buffer = vec![0u8; page_length as usize];
        
        match self.scsi.send_scsi_command(&cdb, &mut buffer, 1) {
            Ok(true) => Ok(buffer),
            Ok(false) => Err(RustLtfsError::scsi("WERL page read failed")),
            Err(e) => Err(e),
        }
    }

    /// 解析WERL数据并计算错误率
    fn parse_werl_data(&mut self, werl_data: &[u8]) -> Result<f64> {
        // 将二进制数据转换为ASCII字符串（对应LTFSCopyGUI逻辑）
        let werl_text = String::from_utf8_lossy(werl_data);
        let data_entries: Vec<&str> = werl_text
            .split(|c| c == '\r' || c == '\n' || c == '\t')
            .filter(|s| !s.is_empty())
            .collect();

        let mut all_results = Vec::new();
        let mut result = f64::NEG_INFINITY;

        // 按5个条目为一组处理（对应LTFSCopyGUI的步长5循环）
        for (ch_idx, chunk) in data_entries.chunks(5).enumerate() {
            if chunk.len() < 5 {
                break;
            }

            let channel = ch_idx;
            let c1_err = u32::from_str_radix(chunk[0], 16)
                .unwrap_or(0);
            let no_ccps = u32::from_str_radix(chunk[4], 16)
                .unwrap_or(0);

            debug!("Channel {}: C1Err={}, NoCCPs={}", channel, c1_err, no_ccps);

            // 计算错误率（对应LTFSCopyGUI的数学公式）
            if channel < self.last_no_ccps.len() && 
               no_ccps > self.last_no_ccps[channel] {
                
                let delta_ccps = no_ccps - self.last_no_ccps[channel];
                let delta_c1 = c1_err.saturating_sub(self.last_c1_errors[channel]);
                
                if delta_ccps > 0 {
                    // 精确对应LTFSCopyGUI的错误率计算公式：
                    // Math.Log10((C1err - LastC1Err(chan)) / (NoCCPs - LastNoCCPs(chan)) / 2 / 1920)
                    let error_rate_log_value = ((delta_c1 as f64) / (delta_ccps as f64) / 2.0 / 1920.0).log10();
                    
                    all_results.push(error_rate_log_value);
                    
                    if error_rate_log_value < 0.0 {
                        result = result.max(error_rate_log_value);
                    }

                    debug!("Channel {} error rate log value: {}", channel, error_rate_log_value);
                }
            }

            // 更新历史记录
            if channel < self.last_c1_errors.len() {
                self.last_c1_errors[channel] = c1_err;
                self.last_no_ccps[channel] = no_ccps;
            }
        }

        // 应用LTFSCopyGUI的阈值逻辑
        if result < -10.0 {
            result = 0.0;
        }
        
        if result < 0.0 {
            self.error_rate_history.push(result);
        }

        Ok(result)
    }

    /// 解析磁带代数字符串
    fn parse_generation_string(&self, gen_str: &str) -> String {
        if gen_str.is_empty() {
            return "Unknown".to_string();
        }

        // 提取代数信息（对应LTFSCopyGUI的解析逻辑）
        if let Some(last_char) = gen_str.chars().last() {
            if last_char.is_ascii_digit() {
                let gen_num: u8 = last_char.to_digit(10).unwrap_or(0) as u8;
                
                if gen_str.to_uppercase().contains("T10K") {
                    return format!("T{}", gen_num);
                } else {
                    return format!("L{}", gen_num);
                }
            }
        }

        gen_str.to_string()
    }

    /// 构建媒体描述字符串
    fn build_media_description(&self, generation: &str, is_worm: bool, is_wp: bool) -> String {
        let mut description = generation.to_string();
        
        if is_worm {
            description.push_str(" WORM");
        }
        
        if is_wp {
            description.push_str(" RO");
        } else {
            description.push_str(" RW");
        }
        
        description
    }

    /// 通过MAM属性读取容量信息（后备方法）
    fn read_mam_capacity(&self, partition: u8, _extra_partition_count: u8) -> Result<u64> {
        // MAM属性ID：
        // - 0x0000: P0剩余容量
        // - 0x0001: P0最大容量  
        // - 0x0100: P1剩余容量（如果存在）
        // - 0x0101: P1最大容量（如果存在）
        
        let attribute_id = if partition == 0 {
            0x0001 // P0最大容量
        } else {
            0x0101 // P1最大容量
        };

        match self.scsi.read_mam_attribute(attribute_id) {
            Ok(mam_attr) => {
                if mam_attr.data.len() >= 8 {
                    let capacity = u64::from_be_bytes([
                        mam_attr.data[0], mam_attr.data[1], mam_attr.data[2], mam_attr.data[3],
                        mam_attr.data[4], mam_attr.data[5], mam_attr.data[6], mam_attr.data[7],
                    ]);
                    debug!("MAM capacity for partition {}: {} KB", partition, capacity);
                    Ok(capacity)
                } else {
                    Ok(0)
                }
            }
            Err(e) => {
                debug!("Failed to read MAM attribute 0x{:04X}: {}", attribute_id, e);
                Ok(0)
            }
        }
    }

    /// 获取模拟容量信息（离线模式）
    fn get_simulated_capacity_info(&self) -> TapeCapacityInfo {
        TapeCapacityInfo {
            p0_remaining: 100 * 1024 * 1024, // 100 GB
            p0_maximum: 100 * 1024 * 1024,   // 100 GB
            p1_remaining: 5400 * 1024 * 1024, // 5.4 TB
            p1_maximum: 5400 * 1024 * 1024,   // 5.4 TB
            media_description: "L7 RW (Simulated)".to_string(),
            error_rate_log_value: 0.0,
            capacity_loss: None,
            is_worm: false,
            is_write_protected: false,
            generation_info: "L7".to_string(),
        }
    }

    /// 获取当前错误率历史
    pub fn get_error_rate_history(&self) -> &[f64] {
        &self.error_rate_history
    }

    /// 清理错误率历史
    pub fn clear_error_rate_history(&mut self) {
        self.error_rate_history.clear();
        self.last_c1_errors.fill(0);
        self.last_no_ccps.fill(0);
    }
}