use crate::error::Result;
use tracing::{debug, warn};

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
