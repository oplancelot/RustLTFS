use crate::error::{Result, RustLtfsError};
use crate::scsi::{MamAttributeFormat, MediaType};
use super::partition_manager::LtfsPartitionLabel;
use tracing::{debug, info, warn};
use uuid::Uuid;
use chrono;

/// MKLTFS参数结构 (对应LTFSCopyGUI的MKLTFS_Param类)
#[derive(Debug, Clone)]
pub struct MkltfsParams {
    /// 条形码（最多20个ASCII字符）
    pub barcode: String,
    /// 卷标签名称
    pub volume_label: String,
    /// 额外分区数量（0或1，默认为1）
    pub extra_partition_count: u8,
    /// 块大小（512到2097152字节，默认524288）
    pub block_length: u32,
    /// 立即模式（是否异步执行）
    pub immediate_mode: bool,
    /// 磁带容量（0-65535，默认65535表示最大容量）
    pub capacity: u16,
    /// P0分区大小（GB，默认1）
    pub p0_size: u16,
    /// P1分区大小（GB，默认65535表示剩余空间）
    pub p1_size: u16,
    /// 加密密钥（可选）
    pub encryption_key: Option<Vec<u8>>,
}

impl Default for MkltfsParams {
    fn default() -> Self {
        Self {
            barcode: String::new(),
            volume_label: String::new(),
            extra_partition_count: 1,
            block_length: 524288, // 512KB默认块大小
            immediate_mode: true,
            capacity: 0xFFFF, // 65535，表示使用最大容量
            p0_size: 1,       // 1GB索引分区
            p1_size: 0xFFFF,  // 65535，表示剩余空间给数据分区
            encryption_key: None,
        }
    }
}

impl MkltfsParams {
    /// 创建新的MKLTFS参数实例
    pub fn new(max_extra_partitions: u8) -> Self {
        let mut params = Self::default();
        params.extra_partition_count =
            std::cmp::min(params.extra_partition_count, max_extra_partitions);
        params
    }

    /// 设置条形码（自动过滤非ASCII字符并限制长度）
    pub fn set_barcode(&mut self, barcode: &str) -> &mut Self {
        let filtered: String = barcode
            .chars()
            .filter(|c| c.is_ascii() && (*c as u8) <= 127)
            .take(20)
            .collect();
        self.barcode = filtered;
        self
    }

    /// 设置P0分区大小，自动调整P1大小
    pub fn set_p0_size(&mut self, size: u16) -> &mut Self {
        self.p0_size = size;
        if size < 0xFFFF {
            self.p1_size = 0xFFFF; // 如果P0不是最大值，P1设为剩余空间
        } else {
            self.p1_size = 1; // 如果P0是最大值，P1设为1GB
        }
        self
    }

    /// 设置P1分区大小，自动调整P0大小
    pub fn set_p1_size(&mut self, size: u16) -> &mut Self {
        self.p1_size = size;
        if size < 0xFFFF {
            self.p0_size = 0xFFFF; // 如果P1不是最大值，P0设为剩余空间
        } else {
            self.p0_size = 1; // 如果P1是最大值，P0设为1GB
        }
        self
    }

    /// 验证参数有效性
    pub fn validate(&self) -> Result<()> {
        // 验证块大小
        if self.block_length < 512 || self.block_length > 2_097_152 {
            return Err(RustLtfsError::parameter_validation(format!(
                "Block length must be between 512 and 2097152, got {}",
                self.block_length
            )));
        }

        // 验证额外分区数量
        if self.extra_partition_count > 1 {
            return Err(RustLtfsError::parameter_validation(format!(
                "Extra partition count must be 0 or 1, got {}",
                self.extra_partition_count
            )));
        }

        // 验证P0Size和P1Size规则：至多一个为65535
        if self.p0_size == 0xFFFF && self.p1_size == 0xFFFF {
            return Err(RustLtfsError::parameter_validation(
                "P0Size and P1Size cannot both be 65535 (maximum value)".to_string(),
            ));
        }

        // 验证条形码长度
        if self.barcode.len() > 20 {
            return Err(RustLtfsError::parameter_validation(format!(
                "Barcode length must not exceed 20 characters, got {}",
                self.barcode.len()
            )));
        }

        Ok(())
    }
}

/// MKLTFS进度回调类型
pub type MkltfsProgressCallback = std::sync::Arc<dyn Fn(&str) + Send + Sync>;
pub type MkltfsFinishCallback = std::sync::Arc<dyn Fn(&str) + Send + Sync>;
pub type MkltfsErrorCallback = std::sync::Arc<dyn Fn(&str) + Send + Sync>;

/// TapeOperations的格式化操作实现
impl super::TapeOperations {
    /// 执行MKLTFS磁带格式化 (对应LTFSCopyGUI的mkltfs方法)
    pub async fn mkltfs(
        &mut self,
        params: MkltfsParams,
        progress_callback: Option<MkltfsProgressCallback>,
        finish_callback: Option<MkltfsFinishCallback>,
        error_callback: Option<MkltfsErrorCallback>,
    ) -> Result<bool> {
        info!("Starting MKLTFS tape formatting process");
        info!(
            "Parameters: barcode={}, volume_label={}, partition_count={}, P0={}GB, P1={}GB",
            params.barcode,
            params.volume_label,
            params.extra_partition_count,
            params.p0_size,
            params.p1_size
        );

        // 验证参数
        params.validate()?;

        if self.offline_mode {
            warn!("Offline mode: cannot execute actual MKLTFS operations");
            return Ok(false);
        }

        let progress = move |msg: &str| {
            info!("{}", msg);
            if let Some(ref callback) = progress_callback {
                callback(msg);
            }
        };

        let on_error_for_sequence = {
            let error_callback = error_callback.clone();
            move |msg: &str| {
                warn!("MKLTFS error: {}", msg);
                if let Some(ref callback) = error_callback {
                    callback(msg);
                }
            }
        };

        let on_finish = move |msg: &str| {
            info!("MKLTFS completed: {}", msg);
            if let Some(ref callback) = finish_callback {
                callback(msg);
            }
        };

        // 开始格式化过程
        match self
            .execute_mkltfs_sequence(&params, Box::new(progress), Box::new(on_error_for_sequence))
            .await
        {
            Ok(()) => {
                on_finish("MKLTFS tape formatting completed");
                Ok(true)
            }
            Err(e) => {
                let msg = format!("MKLTFS failed: {}", e);
                warn!("MKLTFS error: {}", &msg);
                if let Some(callback) = error_callback {
                    callback(&msg);
                }
                Err(e)
            }
        }
    }

    /// 执行MKLTFS命令序列 (对应LTFSCopyGUI的mkltfs内部实现)
    async fn execute_mkltfs_sequence(
        &mut self,
        params: &MkltfsParams,
        progress: Box<dyn Fn(&str)>,
        on_error: Box<dyn Fn(&str)>,
    ) -> Result<()> {
        // Step 1: Load tape
        progress("Loading tape...");
        if !self.scsi.load_tape()? {
            on_error("Tape loading failed");
            return Err(RustLtfsError::scsi("Failed to load tape".to_string()));
        }
        progress("Tape loaded successfully");

        // Step 2: MODE SENSE - Check partition capabilities
        progress("Checking partition support capabilities...");
        let mode_data = self.scsi.mode_sense_partition_info()?;
        let max_extra_partitions = if mode_data.len() >= 3 {
            mode_data[2]
        } else {
            1
        };
        let extra_partition_count =
            std::cmp::min(max_extra_partitions, params.extra_partition_count);
        progress(&format!(
            "Supported extra partitions: {}",
            extra_partition_count
        ));

        // Step 3: Set capacity
        progress("Setting tape capacity...");
        self.scsi.set_capacity(params.capacity)?;
        progress("Capacity setting completed");

        // Step 4: Initialize tape
        progress("Initializing tape...");

        // Check if LTO9+ tape should skip format
        let should_skip_format = self.should_skip_format_for_lto9_plus().await;
        if should_skip_format {
            progress("Detected LTO9 or higher version tape, skipping initialization step");
        } else {
            self.scsi.format_tape(false)?; // Non-immediate mode for full formatting
            progress("Tape initialization completed");
        }

        // Step 5: Partition configuration (if needed)
        if extra_partition_count > 0 {
            progress("Configuring partition mode...");
            self.scsi.mode_select_partition(
                max_extra_partitions,
                extra_partition_count,
                &mode_data,
                params.p0_size,
                params.p1_size,
            )?;
            progress("Partition mode configuration completed");

            progress("Creating partitions...");
            let partition_type = self.get_partition_type_for_drive();
            self.scsi.partition_tape(partition_type)?;
            progress("Partition creation completed");
        }

        // Step 6: Set MAM attributes
        self.set_ltfs_mam_attributes(params, &progress).await?;

        // Step 7: Set barcode
        if !params.barcode.is_empty() {
            progress(&format!("Setting barcode: {}", params.barcode));
            self.scsi.set_barcode(&params.barcode)?;
            progress("Barcode setting completed");
        }

        // Step 8: Write LTFS volume label
        self.write_ltfs_volume_label(params, extra_partition_count, &progress)
            .await?;

        Ok(())
    }

    /// 检查是否应该跳过LTO9+磁带的格式化
    async fn should_skip_format_for_lto9_plus(&self) -> bool {
        // 简化实现：根据媒体类型判断
        // 实际LTFSCopyGUI会解析CM数据来判断
        match self.scsi.check_media_status() {
            Ok(media_type) => {
                matches!(
                    media_type,
                    MediaType::Lto9Rw | MediaType::Lto9Worm | MediaType::Lto9Ro
                )
            }
            Err(_) => false,
        }
    }

    /// 获取驱动器的分区类型
    fn get_partition_type_for_drive(&self) -> u8 {
        // 根据驱动器类型返回分区类型
        // T10K使用类型2，其他使用类型1
        // 这里简化处理，实际应该根据驱动器类型判断
        1 // 标准分区类型
    }

    /// 设置LTFS相关的MAM属性 (对应LTFSCopyGUI的MAM属性设置)
    async fn set_ltfs_mam_attributes(
        &self,
        params: &MkltfsParams,
        progress: &Box<dyn Fn(&str)>,
    ) -> Result<()> {
        // MAM attribute 0x800: Vendor = "OPEN"
        progress("Setting MAM attribute: Vendor information");
        let vendor_data = "OPEN".to_string().into_bytes();
        let mut padded_vendor = vec![b' '; 8];
        let copy_len = std::cmp::min(vendor_data.len(), 8);
        padded_vendor[..copy_len].copy_from_slice(&vendor_data[..copy_len]);
        self.scsi
            .set_mam_attribute(0x800, &padded_vendor, MamAttributeFormat::Text)?;

        // MAM attribute 0x801: Application name = "RustLTFS"
        progress("Setting MAM attribute: Application name");
        let app_name = "RustLTFS".to_string().into_bytes();
        let mut padded_app_name = vec![b' '; 32];
        let copy_len = std::cmp::min(app_name.len(), 32);
        padded_app_name[..copy_len].copy_from_slice(&app_name[..copy_len]);
        self.scsi
            .set_mam_attribute(0x801, &padded_app_name, MamAttributeFormat::Text)?;

        // MAM attribute 0x802: Application version
        progress("Setting MAM attribute: Application version");
        let version = env!("CARGO_PKG_VERSION").to_string().into_bytes();
        let mut padded_version = vec![b' '; 8];
        let copy_len = std::cmp::min(version.len(), 8);
        padded_version[..copy_len].copy_from_slice(&version[..copy_len]);
        self.scsi
            .set_mam_attribute(0x802, &padded_version, MamAttributeFormat::Text)?;

        // MAM attribute 0x803: Text label (empty)
        progress("Setting MAM attribute: Text label");
        let text_label = vec![b' '; 160];
        self.scsi
            .set_mam_attribute(0x803, &text_label, MamAttributeFormat::Text)?;

        // MAM attribute 0x805: Localization identifier = 0
        progress("Setting MAM attribute: Localization identifier");
        let localization_id = vec![0u8];
        self.scsi
            .set_mam_attribute(0x805, &localization_id, MamAttributeFormat::Binary)?;

        // MAM attribute 0x80B: LTFS format version
        progress("Setting MAM attribute: LTFS format version");
        let ltfs_version = if params.extra_partition_count == 0 {
            "2.4.1" // Single partition
        } else {
            "2.4.0" // Multi-partition
        };
        let version_data = ltfs_version.to_string().into_bytes();
        let mut padded_ltfs_version = vec![b' '; 16];
        let copy_len = std::cmp::min(version_data.len(), 16);
        padded_ltfs_version[..copy_len].copy_from_slice(&version_data[..copy_len]);
        self.scsi
            .set_mam_attribute(0x80B, &padded_ltfs_version, MamAttributeFormat::Text)?;

        progress("All MAM attributes set successfully");
        Ok(())
    }

    /// 写入LTFS卷标签 (对应LTFSCopyGUI的卷标签写入)
    async fn write_ltfs_volume_label(
        &mut self,
        params: &MkltfsParams,
        extra_partition_count: u8,
        progress: &Box<dyn Fn(&str)>,
    ) -> Result<()> {
        progress("Writing LTFS volume label");

        // Position to tape beginning
        self.scsi.locate_block(0, 0)?;

        // Create LTFS volume label content
        let volume_label = self.create_ltfs_volume_label(params, extra_partition_count)?;

        // Write volume label (simplified implementation, should write in LTFS format)
        let blocks_needed = (volume_label.len() + self.block_size as usize - 1) / self.block_size as usize;
        let mut buffer = vec![0u8; blocks_needed * self.block_size as usize];
        buffer[..volume_label.len()].copy_from_slice(&volume_label);

        self.scsi.write_blocks(blocks_needed as u32, &buffer)?;

        progress("LTFS volume label write completed");
        Ok(())
    }

    /// 创建LTFS卷标签内容
    fn create_ltfs_volume_label(
        &self,
        params: &MkltfsParams,
        _extra_partition_count: u8,
    ) -> Result<Vec<u8>> {
        // 创建基本的LTFS VOL1标签结构
        let mut vol1_label = vec![0u8; 80];

        // VOL1标签格式
        vol1_label[0..4].copy_from_slice(b"VOL1");

        // 卷序列号（位置4-9）
        let volume_id = if params.volume_label.is_empty() {
            format!("{:06}", chrono::Utc::now().timestamp() % 1000000)
        } else {
            params.volume_label.clone()
        };
        let volume_id_bytes = volume_id.as_bytes();
        let copy_len = std::cmp::min(volume_id_bytes.len(), 6);
        vol1_label[4..4 + copy_len].copy_from_slice(&volume_id_bytes[..copy_len]);

        // LTFS标识符（位置24-27）
        vol1_label[24..28].copy_from_slice(b"LTFS");

        // 其他标准字段可以根据需要填充

        Ok(vol1_label)
    }

    /// 格式化磁带为LTFS格式 (简化版本)
    pub async fn format_tape(&mut self, force: bool) -> Result<()> {
        info!("Starting tape format operation (force: {})", force);

        if self.offline_mode {
            info!("Offline mode: simulating tape format");
            return Ok(());
        }

        // 检查磁带状态
        let media_status = self.scsi.check_media_status()?;
        match media_status {
            MediaType::NoTape => {
                return Err(RustLtfsError::tape_device("No tape loaded".to_string()));
            }
            _ => {
                info!("Media detected: {}", media_status.description());
            }
        }

        // 如果不是强制格式化，检查现有LTFS格式
        if !force {
            match self.detect_ltfs_format_status().await {
                Ok(status) => {
                    if status.is_ltfs_formatted() {
                        return Err(RustLtfsError::parameter_validation(
                            "Tape already contains LTFS format. Use --force to overwrite".to_string(),
                        ));
                    }
                }
                Err(_) => {
                    // 检测失败，继续格式化
                    debug!("Format detection failed, proceeding with format");
                }
            }
        }

        // 使用默认参数执行MKLTFS
        let mut params = MkltfsParams::default();
        params.volume_label = format!("RUSTLTFS_{}", chrono::Utc::now().format("%Y%m%d"));

        self.mkltfs(params, None, None, None).await?;

        info!("Tape format operation completed successfully");
        Ok(())
    }

    /// 验证LTFS格式完整性
    pub async fn verify_format(&mut self) -> Result<bool> {
        info!("Verifying LTFS format integrity");

        if self.offline_mode {
            info!("Offline mode: returning dummy verification result");
            return Ok(true);
        }

        // 检查LTFS格式状态
        let format_status = self.detect_ltfs_format_status().await?;

        match format_status {
            LtfsFormatStatus::Formatted => {
                info!("✅ LTFS format verification passed");
                
                // 尝试读取索引验证完整性
                match self.read_index_from_tape().await {
                    Ok(_) => {
                        info!("✅ LTFS index verification passed");
                        Ok(true)
                    }
                    Err(e) => {
                        warn!("⚠️ LTFS index verification failed: {}", e);
                        Ok(false)
                    }
                }
            }
            _ => {
                warn!("❌ LTFS format verification failed: {}", format_status.description());
                Ok(false)
            }
        }
    }

    /// 创建LTFS分区
    pub async fn create_partitions(&mut self, p0_size_gb: u16, p1_size_gb: u16) -> Result<()> {
        info!("Creating LTFS partitions: P0={}GB, P1={}GB", p0_size_gb, p1_size_gb);

        if self.offline_mode {
            info!("Offline mode: simulating partition creation");
            return Ok(());
        }

        // 检查分区参数
        if p0_size_gb == 0xFFFF && p1_size_gb == 0xFFFF {
            return Err(RustLtfsError::parameter_validation(
                "Both partitions cannot be set to maximum size".to_string(),
            ));
        }

        // 检查磁带驱动器的分区支持
        let mode_data = self.scsi.mode_sense_partition_info()?;
        let max_extra_partitions = if mode_data.len() >= 3 {
            mode_data[2]
        } else {
            0
        };

        if max_extra_partitions == 0 {
            return Err(RustLtfsError::unsupported(
                "Drive does not support partitioning".to_string(),
            ));
        }

        // 配置分区模式
        self.scsi.mode_select_partition(
            max_extra_partitions,
            1, // 创建一个额外分区
            &mode_data,
            p0_size_gb,
            p1_size_gb,
        )?;

        // 执行分区创建
        let partition_type = self.get_partition_type_for_drive();
        self.scsi.partition_tape(partition_type)?;

        info!("Partition creation completed successfully");
        Ok(())
    }

    /// 设置分区标签
    pub async fn setup_partition_labels(&mut self, label: LtfsPartitionLabel) -> Result<()> {
        info!("Setting up partition labels");

        if self.offline_mode {
            info!("Offline mode: simulating partition label setup");
            self.partition_label = Some(label);
            return Ok(());
        }

        // 定位到分区开始位置
        self.scsi.locate_block(0, 0)?;

        // 写入分区标签信息
        // 这里应该实现完整的LTFS分区标签格式
        self.partition_label = Some(label.clone());

        // 更新块大小设置
        self.block_size = label.blocksize;

        info!("Partition labels setup completed");
        Ok(())
    }

    /// 写入卷标签
    pub async fn write_volume_labels(&mut self, volume_uuid: Option<String>) -> Result<()> {
        info!("Writing volume labels");

        if self.offline_mode {
            info!("Offline mode: simulating volume label write");
            return Ok(());
        }

        let uuid = volume_uuid.unwrap_or_else(|| Uuid::new_v4().to_string());

        // 创建VOL1标签
        let mut vol1_label = vec![0u8; 80];
        vol1_label[0..4].copy_from_slice(b"VOL1");
        
        // 设置卷序列号
        let volume_serial = format!("{:06}", chrono::Utc::now().timestamp() % 1000000);
        let serial_bytes = volume_serial.as_bytes();
        let copy_len = std::cmp::min(serial_bytes.len(), 6);
        vol1_label[4..4 + copy_len].copy_from_slice(&serial_bytes[..copy_len]);

        // 设置LTFS标识符
        vol1_label[24..28].copy_from_slice(b"LTFS");

        // 定位并写入VOL1标签
        self.scsi.locate_block(0, 0)?;
        let buffer_size = self.block_size as usize;
        let mut buffer = vec![0u8; buffer_size];
        buffer[..vol1_label.len()].copy_from_slice(&vol1_label);

        self.scsi.write_blocks(1, &buffer)?;

        // 更新分区标签中的UUID
        if let Some(ref mut label) = self.partition_label {
            label.volume_uuid = uuid;
            label.format_time = chrono::Utc::now().to_rfc3339();
        }

        info!("Volume labels written successfully");
        Ok(())
    }

    /// 配置LTFS参数
    pub async fn configure_ltfs_parameters(&mut self, params: MkltfsParams) -> Result<()> {
        info!("Configuring LTFS parameters");

        // 验证参数
        params.validate()?;

        if self.offline_mode {
            info!("Offline mode: parameters configured for simulation");
            return Ok(());
        }

        // 设置块大小
        self.block_size = params.block_length;

        // 配置分区标签
        let mut label = LtfsPartitionLabel::default();
        label.blocksize = params.block_length;
        label.volume_uuid = Uuid::new_v4().to_string();
        label.format_time = chrono::Utc::now().to_rfc3339();

        self.partition_label = Some(label);

        info!("LTFS parameters configured successfully");
        Ok(())
    }

    /// 初始化磁带格式
    pub async fn initialize_tape_format(&mut self, params: MkltfsParams) -> Result<()> {
        info!("Initializing tape format with LTFS");

        // 验证参数
        params.validate()?;

        if self.offline_mode {
            info!("Offline mode: simulating tape format initialization");
            return Ok(());
        }

        // 执行完整的LTFS格式化流程
        self.mkltfs(params, None, None, None).await?;

        info!("Tape format initialization completed");
        Ok(())
    }

    /// 检查格式完整性
    pub async fn check_format_integrity(&mut self) -> Result<bool> {
        info!("Checking LTFS format integrity");

        if self.offline_mode {
            info!("Offline mode: returning dummy integrity check result");
            return Ok(true);
        }

        // 执行格式验证
        self.verify_format().await
    }

    /// 验证LTFS结构
    pub async fn validate_ltfs_structure(&mut self) -> Result<bool> {
        info!("Validating LTFS structure");

        if self.offline_mode {
            info!("Offline mode: returning dummy structure validation result");
            return Ok(true);
        }

        // 检查格式状态
        let format_status = self.detect_ltfs_format_status().await?;
        
        if !format_status.is_ltfs_formatted() {
            warn!("Tape is not LTFS formatted");
            return Ok(false);
        }

        // 尝试读取和解析索引
        match self.read_index_from_tape().await {
            Ok(_) => {
                // 检查分区标签
                match self.read_partition_label().await {
                    Ok(_) => {
                        info!("✅ LTFS structure validation passed");
                        Ok(true)
                    }
                    Err(e) => {
                        warn!("⚠️ Partition label validation failed: {}", e);
                        Ok(false)
                    }
                }
            }
            Err(e) => {
                warn!("⚠️ Index validation failed: {}", e);
                Ok(false)
            }
        }
    }

    /// 创建LTFS格式 (高级接口)
    pub async fn create_ltfs_format(
        &mut self,
        volume_label: Option<String>,
        barcode: Option<String>,
        p0_size_gb: Option<u16>,
        p1_size_gb: Option<u16>,
        block_size: Option<u32>,
    ) -> Result<()> {
        info!("Creating LTFS format with custom parameters");

        // 构建参数
        let mut params = MkltfsParams::default();
        
        if let Some(label) = volume_label {
            params.volume_label = label;
        }
        
        if let Some(barcode) = barcode {
            params.set_barcode(&barcode);
        }
        
        if let Some(p0) = p0_size_gb {
            params.set_p0_size(p0);
        }
        
        if let Some(p1) = p1_size_gb {
            params.set_p1_size(p1);
        }
        
        if let Some(block_size) = block_size {
            params.block_length = block_size;
        }

        // 执行格式化
        self.initialize_tape_format(params).await
    }
    
    /// 检测LTFS格式状态
    pub async fn detect_ltfs_format_status(&mut self) -> Result<LtfsFormatStatus> {
        info!("Detecting LTFS format status on tape");
        
        // 尝试读取分区标签来检测格式
        match self.read_partition_label().await {
            Ok(_) => Ok(LtfsFormatStatus::Formatted),
            Err(_) => {
                // 检查是否有数据但不是LTFS格式
                match self.scsi.test_unit_ready() {
                    Ok(_) => Ok(LtfsFormatStatus::UnknownFormat),
                    Err(_) => Ok(LtfsFormatStatus::Empty),
                }
            }
        }
    }
    
    /// 读取分区标签
    pub async fn read_partition_label(&mut self) -> Result<LtfsPartitionLabel> {
        info!("Reading LTFS partition label");
        
        // 定位到partition A, block 0
        self.scsi.locate_block(0, 0)?;
        
        let mut buffer = vec![0u8; crate::scsi::block_sizes::LTO_BLOCK_SIZE as usize];
        self.scsi.read_blocks(1, &mut buffer)?;
        
        // 解析LTFS分区标签
        self.parse_partition_label(&buffer)
    }
    
    /// 解析分区标签
    fn parse_partition_label(&self, buffer: &[u8]) -> Result<LtfsPartitionLabel> {
        // 简化的分区标签解析
        // 在真实实现中需要按照LTFS规范解析
        let mut label = LtfsPartitionLabel::default();
        
        // 查找LTFS标识
        if buffer.windows(4).any(|w| w == b"LTFS") {
            label.volume_uuid = "detected-ltfs-volume".to_string();
            label.format_time = chrono::Utc::now().to_rfc3339();
            Ok(label)
        } else {
            Err(RustLtfsError::ltfs_index("No LTFS partition label found".to_string()))
        }
    }
}

/// LTFS格式状态
#[derive(Debug, Clone, PartialEq)]
pub enum LtfsFormatStatus {
    /// 已格式化为LTFS
    Formatted,
    /// 有数据但格式未知
    UnknownFormat,
    /// 空磁带
    Empty,
}

impl LtfsFormatStatus {
    pub fn is_ltfs_formatted(&self) -> bool {
        matches!(self, LtfsFormatStatus::Formatted)
    }
    
    pub fn description(&self) -> &'static str {
        match self {
            LtfsFormatStatus::Formatted => "LTFS formatted",
            LtfsFormatStatus::UnknownFormat => "Unknown format",
            LtfsFormatStatus::Empty => "Empty tape",
        }
    }
}