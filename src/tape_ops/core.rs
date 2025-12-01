use super::partition_manager::LtfsPartitionLabel;
use super::{FileWriteEntry, LtfsAccess, WriteOptions, WriteProgress};
use crate::error::{Result, RustLtfsError};
use crate::ltfs_index::LtfsIndex;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::{Mutex, Semaphore};
use tracing::{debug, info, warn};

/// 智能速度限制器（对应LTFSCopyGUI的SpeedLimit功能增强版）
#[derive(Debug, Clone)]
pub struct SpeedLimiter {
    /// 目标速度限制 (字节/秒)
    pub target_rate_bps: u64,
    /// 最近的传输记录（用于计算实际速度）
    pub transfer_history: Vec<(Instant, u64)>, // (时间戳, 字节数)
    /// 历史记录窗口大小
    pub history_window_seconds: u64,
    /// 最后一次速度调整时间
    pub last_adjustment: Instant,
    /// 自适应模式启用状态
    pub adaptive_mode: bool,
    /// 突发传输允许的最大字节数
    pub burst_allowance: u64,
}

impl SpeedLimiter {
    /// 创建新的速度限制器
    pub fn new(target_rate_mbps: u32) -> Self {
        let target_rate_bps = (target_rate_mbps as u64) * 1024 * 1024;
        Self {
            target_rate_bps,
            transfer_history: Vec::new(),
            history_window_seconds: 10, // 10秒历史窗口
            last_adjustment: Instant::now(),
            adaptive_mode: true,
            burst_allowance: target_rate_bps / 2, // 允许50%的突发
        }
    }

    /// 记录传输数据并计算需要的延迟（对应LTFSCopyGUI的速度控制逻辑）
    pub async fn apply_rate_limit(&mut self, bytes_transferred: u64) -> Duration {
        let now = Instant::now();

        // 添加到传输历史
        self.transfer_history.push((now, bytes_transferred));

        // 清理过期的历史记录
        let cutoff_time = now - Duration::from_secs(self.history_window_seconds);
        self.transfer_history
            .retain(|(time, _)| *time >= cutoff_time);

        // 计算当前传输速度
        let total_bytes: u64 = self.transfer_history.iter().map(|(_, bytes)| *bytes).sum();
        let time_span = if let Some((earliest_time, _)) = self.transfer_history.first() {
            now.duration_since(*earliest_time)
        } else {
            Duration::from_secs(1)
        };

        let current_rate = if time_span.as_secs() > 0 {
            total_bytes / time_span.as_secs()
        } else {
            0
        };

        // 计算需要的延迟
        let delay = if current_rate > self.target_rate_bps {
            let excess_rate = current_rate - self.target_rate_bps;
            let delay_factor = excess_rate as f64 / self.target_rate_bps as f64;
            Duration::from_millis((delay_factor * 1000.0) as u64)
        } else {
            Duration::ZERO
        };

        if delay > Duration::ZERO {
            debug!(
                "Speed limiting: current {}MB/s, target {}MB/s, delaying {}ms",
                current_rate / (1024 * 1024),
                self.target_rate_bps / (1024 * 1024),
                delay.as_millis()
            );
        }

        delay
    }

    /// 获取当前实际传输速度
    pub fn get_current_rate(&self) -> u64 {
        if self.transfer_history.len() < 2 {
            return 0;
        }

        let total_bytes: u64 = self.transfer_history.iter().map(|(_, bytes)| *bytes).sum();
        let time_span = if let (Some((first_time, _)), Some((last_time, _))) =
            (self.transfer_history.first(), self.transfer_history.last())
        {
            last_time.duration_since(*first_time)
        } else {
            return 0;
        };

        if time_span.as_secs() > 0 {
            total_bytes / time_span.as_secs()
        } else {
            0
        }
    }
}

/// 性能控制状态结构体（对应LTFSCopyGUI的性能管理）
#[derive(Debug, Clone)]
pub struct PerformanceControlState {
    /// 当前传输速度 (字节/秒)
    pub current_transfer_rate: u64,
    /// 目标速度限制 (字节/秒)
    pub target_speed_limit: Option<u64>,
    /// 活跃的并发操作数
    pub active_operations: u32,
    /// 最大并发操作数（对应LTFSCopyGUI的线程池大小）
    pub max_concurrent_operations: u32,
    /// 内存使用情况 (字节)
    pub memory_usage: u64,
    /// 最大内存限制 (字节)
    pub max_memory_limit: u64,
    /// 队列中等待的操作数
    pub queued_operations: u32,
    /// 性能监控启用状态
    pub monitoring_enabled: bool,
}

impl Default for PerformanceControlState {
    fn default() -> Self {
        Self {
            current_transfer_rate: 0,
            target_speed_limit: None,
            active_operations: 0,
            max_concurrent_operations: 4, // LTFSCopyGUI默认并发数
            memory_usage: 0,
            max_memory_limit: 2 * 1024 * 1024 * 1024, // 2GB 内存限制
            queued_operations: 0,
            monitoring_enabled: true,
        }
    }
}

/// 操作类型枚举
#[derive(Debug, Clone, Copy)]
pub enum OperationType {
    Space,  // 只需要设备初始化
    Write,  // 需要设备初始化 + 索引加载
    Read,   // 需要设备初始化 + 索引加载 + 内容显示
}

/// Tape operations - core functionality from LTFSCopyGUI
pub struct TapeOperations {
    pub(crate) device_path: String,
    pub(crate) offline_mode: bool,
    pub(crate) index: Option<LtfsIndex>,
    pub(crate) _tape_handle: Option<LtfsAccess>,
    pub(crate) _drive_handle: Option<i32>,
    pub(crate) schema: Option<LtfsIndex>,
    pub(crate) block_size: u32,
    pub(crate) _tape_drive: String,
    pub(crate) scsi: crate::scsi::ScsiInterface,
    pub(crate) partition_label: Option<LtfsPartitionLabel>, // 对应LTFSCopyGUI的plabel
    pub(crate) write_queue: Vec<FileWriteEntry>,
    pub(crate) write_progress: WriteProgress,
    pub(crate) write_options: WriteOptions,
    pub(crate) modified: bool,   // 对应LTFSCopyGUI的Modified标志
    pub(crate) stop_flag: bool,  // 对应LTFSCopyGUI的StopFlag
    pub(crate) pause_flag: bool, // 对应LTFSCopyGUI的Pause
    pub(crate) extra_partition_count: Option<u8>, // 对应LTFSCopyGUI的ExtraPartitionCount
    pub(crate) max_extra_partition_allowed: u8, // 对应LTFSCopyGUI的MaxExtraPartitionAllowed

    // === 新增性能控制组件（对应LTFSCopyGUI的性能管理） ===
    pub(crate) performance_state: PerformanceControlState,      // 性能控制状态
    pub(crate) operation_semaphore: Option<Arc<Semaphore>>,     // 并发控制信号量
    pub(crate) memory_usage_tracker: Arc<Mutex<u64>>,           // 内存使用跟踪器
}

impl TapeOperations {
    /// Create new tape operations instance with performance control
    pub fn new(device: &str, offline_mode: bool) -> Self {
        let performance_state = PerformanceControlState::default();
        let max_concurrent = performance_state.max_concurrent_operations as usize;

        Self {
            device_path: device.to_string(),
            offline_mode,
            index: None,
            tape_handle: None,
            drive_handle: None,
            schema: None,
            block_size: crate::scsi::block_sizes::LTO_BLOCK_SIZE, // Default block size (64KB)
            tape_drive: device.to_string(),
            scsi: crate::scsi::ScsiInterface::new(),
            partition_label: None, // 初始化为None，稍后读取
            write_queue: Vec::new(),
            write_progress: WriteProgress::default(),
            write_options: WriteOptions::default(),
            modified: false,
            stop_flag: false,
            pause_flag: false,
            extra_partition_count: None, // Will be detected during initialization
            max_extra_partition_allowed: 1, // LTO standard maximum

            // 性能控制组件初始化
            performance_state,
            operation_semaphore: Some(Arc::new(Semaphore::new(max_concurrent))),
            memory_usage_tracker: Arc::new(Mutex::new(0)),
            speed_limiter: None,

            // 去重管理器初始化（稍后通过configure_deduplication配置）
            dedup_manager: None,
        }
    }

    /// Set speed limit (对应LTFSCopyGUI的速度限制)
    pub fn set_speed_limit(&mut self, limit: Option<u32>) {
        if let Some(limit) = limit {
            self.speed_limiter = Some(SpeedLimiter::new(limit));
            self.performance_state.target_speed_limit = Some((limit as u64) * 1024 * 1024);
            info!(
                "Speed limit enabled: {} MB/s for device: {}",
                limit, self.device_path
            );
        } else {
            self.speed_limiter = None;
            self.performance_state.target_speed_limit = None;
            info!("Speed limit disabled for device: {}", self.device_path);
        }
    }

    /// Configure concurrency control (对应LTFSCopyGUI的并发控制)
    pub fn set_max_concurrent_operations(&mut self, max_concurrent: u32) {
        self.performance_state.max_concurrent_operations = max_concurrent;
        self.operation_semaphore = Some(Arc::new(Semaphore::new(max_concurrent as usize)));
        info!(
            "Maximum concurrent operations set to {} for device: {}",
            max_concurrent, self.device_path
        );
    }

    /// Set memory limit (对应LTFSCopyGUI的内存限制)
    pub fn set_memory_limit(&mut self, memory_limit_mb: u64) {
        self.performance_state.max_memory_limit = memory_limit_mb * 1024 * 1024;
        info!(
            "Memory limit set to {} MB for device: {}",
            memory_limit_mb, self.device_path
        );
    }

    /// Get current performance status (对应LTFSCopyGUI的性能状态查询)
    pub async fn get_performance_status(&self) -> PerformanceControlState {
        let mut state = self.performance_state.clone();

        // 更新内存使用情况
        let memory_usage = self.memory_usage_tracker.lock().await;
        state.memory_usage = *memory_usage;

        // 更新当前传输速度
        if let Some(ref speed_limiter) = self.speed_limiter {
            state.current_transfer_rate = speed_limiter.get_current_rate();
        }

        // 更新队列状态
        state.queued_operations = self.write_queue.len() as u32;

        state
    }

    /// Apply performance controls during operation (智能性能控制应用)
    pub async fn apply_performance_controls(
        &mut self,
        bytes_processed: u64,
        memory_delta: u64,
    ) -> Result<()> {
        // 1. 内存使用控制 (使用实际内存增量)
        self.check_memory_usage(memory_delta).await?;

        // 2. 速度限制控制
        if let Some(ref mut speed_limiter) = self.speed_limiter {
            let delay = speed_limiter.apply_rate_limit(bytes_processed).await;
            if delay > Duration::ZERO {
                tokio::time::sleep(delay).await;
            }
        }

        // 3. 并发控制检查
        self.check_operation_limits().await?;

        // 4. 暂停和停止检查
        self.check_pause_and_stop().await?;

        // 性能监控记录功能已移除

        Ok(())
    }

    /// Check memory usage and apply controls
    async fn check_memory_usage(&mut self, additional_bytes: u64) -> Result<()> {
        {
            let mut memory_usage = self.memory_usage_tracker.lock().await;
            *memory_usage += additional_bytes;

            if *memory_usage > self.performance_state.max_memory_limit {
                warn!(
                    "Memory usage ({} MB) exceeds limit ({} MB), initiating memory management",
                    *memory_usage / (1024 * 1024),
                    self.performance_state.max_memory_limit / (1024 * 1024)
                );

                // 释放锁，以便调用cleanup函数
                drop(memory_usage);

                // 强制内存清理
                self.force_memory_cleanup().await?;

                // 重新获取锁并更新内存使用
                let mut memory_usage = self.memory_usage_tracker.lock().await;
                *memory_usage = self.get_actual_memory_usage().await;

                if *memory_usage > self.performance_state.max_memory_limit {
                    return Err(RustLtfsError::resource_exhausted(
                        "Memory limit exceeded and cleanup failed".to_string(),
                    ));
                }
            }
        }

        Ok(())
    }

    /// Check operation limits and manage concurrency
    async fn check_operation_limits(&mut self) -> Result<()> {
        if self.performance_state.active_operations
            >= self.performance_state.max_concurrent_operations
        {
            debug!(
                "Maximum concurrent operations ({}) reached, waiting for slot",
                self.performance_state.max_concurrent_operations
            );

            // 等待信号量可用
            if let Some(ref semaphore) = self.operation_semaphore {
                let _permit = semaphore.acquire().await.map_err(|e| {
                    RustLtfsError::resource_exhausted(format!(
                        "Semaphore acquisition failed: {}",
                        e
                    ))
                })?;
                // permit会在这个作用域结束时自动释放
            }
        }

        Ok(())
    }

    /// Check pause and stop flags
    async fn check_pause_and_stop(&self) -> Result<()> {
        if self.stop_flag {
            return Err(RustLtfsError::operation_cancelled(
                "Operation stopped by user".to_string(),
            ));
        }

        while self.pause_flag {
            debug!("Operation paused, waiting for resume");
            tokio::time::sleep(Duration::from_millis(100)).await;

            // 再次检查停止标志
            if self.stop_flag {
                return Err(RustLtfsError::operation_cancelled(
                    "Operation stopped while paused".to_string(),
                ));
            }
        }

        Ok(())
    }

    /// Force memory cleanup when approaching limits
    async fn force_memory_cleanup(&mut self) -> Result<()> {
        info!("Performing emergency memory cleanup");

        // 性能监控缓存清理功能已移除

        // 清理写入队列中不必要的数据
        if self.write_queue.len() > 100 {
            warn!(
                "Large write queue detected ({}), optimizing",
                self.write_queue.len()
            );
            // 可以实现队列优化逻辑
        }

        // 清理速度限制器历史
        if let Some(ref mut speed_limiter) = self.speed_limiter {
            speed_limiter.transfer_history.clear();
        }

        // 触发垃圾回收提示（Rust会自动管理，但可以建议）
        std::hint::black_box(vec![0u8; 1]); // 触发分配器活动

        Ok(())
    }

    /// Get actual memory usage estimation
    async fn get_actual_memory_usage(&self) -> u64 {
        let mut usage = 0u64;

        // 估算各组件内存使用
        usage += self.write_queue.len() as u64 * 1024; // 估算每个写入条目1KB
        usage += self.block_size as u64; // SCSI缓冲区

        // 性能监控内存统计功能已移除

        if let Some(ref speed_limiter) = self.speed_limiter {
            usage += speed_limiter.transfer_history.len() as u64 * 16; // 每个记录约16字节
        }

        usage
    }

    /// 获取详细的性能报告（对应LTFSCopyGUI的性能报告功能）
    pub async fn get_performance_report(&self) -> String {
        let mut report = String::new();

        report.push_str("=== RustLTFS Performance Report ===\n");
        report.push_str(&format!("Device: {}\n", self.device_path));
        report.push_str(&format!("Block size: {} bytes\n", self.block_size));

        // 性能控制状态
        let perf_state = self.get_performance_status().await;
        report.push_str("\n--- Performance Control Status ---\n");
        report.push_str(&format!(
            "Current transfer rate: {:.2} MB/s\n",
            perf_state.current_transfer_rate as f64 / (1024.0 * 1024.0)
        ));

        if let Some(target_speed) = perf_state.target_speed_limit {
            report.push_str(&format!(
                "Target speed limit: {:.2} MB/s\n",
                target_speed as f64 / (1024.0 * 1024.0)
            ));
        } else {
            report.push_str("Speed limit: Disabled\n");
        }

        report.push_str(&format!(
            "Active operations: {} / {}\n",
            perf_state.active_operations, perf_state.max_concurrent_operations
        ));
        report.push_str(&format!(
            "Memory usage: {:.2} MB / {:.2} MB\n",
            perf_state.memory_usage as f64 / (1024.0 * 1024.0),
            perf_state.max_memory_limit as f64 / (1024.0 * 1024.0)
        ));
        report.push_str(&format!(
            "Queued operations: {}\n",
            perf_state.queued_operations
        ));

        // 性能监控统计功能已移除
        report.push_str("\n--- Performance Monitoring ---\n");
        report.push_str("Monitoring: Disabled (功能已移除)\n");

        // 速度限制器详细信息
        if let Some(ref speed_limiter) = self.speed_limiter {
            report.push_str("\n--- Speed Limiter ---\n");
            report.push_str(&format!(
                "Target rate: {:.2} MB/s\n",
                speed_limiter.target_rate_bps as f64 / (1024.0 * 1024.0)
            ));
            report.push_str(&format!(
                "History window: {}s\n",
                speed_limiter.history_window_seconds
            ));
            report.push_str(&format!(
                "Transfer history entries: {}\n",
                speed_limiter.transfer_history.len()
            ));
            report.push_str(&format!("Adaptive mode: {}\n", speed_limiter.adaptive_mode));
            report.push_str(&format!(
                "Burst allowance: {:.2} MB\n",
                speed_limiter.burst_allowance as f64 / (1024.0 * 1024.0)
            ));
        } else {
            report.push_str("\n--- Speed Limiter ---\n");
            report.push_str("Speed limiting: Disabled\n");
        }

        // 写入进度
        report.push_str("\n--- Write Progress ---\n");
        report.push_str(&format!(
            "Total files processed: {}\n",
            self.write_progress.total_files_processed
        ));
        report.push_str(&format!(
            "Current files processed: {}\n",
            self.write_progress.current_files_processed
        ));
        report.push_str(&format!(
            "Total bytes processed: {:.2} MB\n",
            self.write_progress.total_bytes_processed as f64 / (1024.0 * 1024.0)
        ));
        report.push_str(&format!(
            "Files in queue: {}\n",
            self.write_progress.files_in_queue
        ));
        report.push_str(&format!(
            "Files written: {}\n",
            self.write_progress.files_written
        ));
        report.push_str(&format!(
            "Bytes written: {:.2} MB\n",
            self.write_progress.bytes_written as f64 / (1024.0 * 1024.0)
        ));
        report.push_str(&format!(
            "Current file: {}\n",
            self.write_progress.current_file
        ));

        if !self.write_progress.errors.is_empty() {
            report.push_str(&format!("Errors: {}\n", self.write_progress.errors.len()));
        }

        // 控制状态
        report.push_str("\n--- Control Status ---\n");
        report.push_str(&format!("Stop flag: {}\n", self.stop_flag));
        report.push_str(&format!("Pause flag: {}\n", self.pause_flag));
        report.push_str(&format!("Modified: {}\n", self.modified));

        report.push_str("\n=== End Performance Report ===\n");
        report
    }

    /// 重置性能统计（对应LTFSCopyGUI的统计重置功能）
    pub fn reset_performance_stats(&mut self) {
        // 性能监控缓存清理功能已移除

        if let Some(ref mut speed_limiter) = self.speed_limiter {
            speed_limiter.transfer_history.clear();
        }

        self.performance_state.current_transfer_rate = 0;
        self.performance_state.active_operations = 0;
        self.performance_state.queued_operations = 0;

        info!(
            "Performance statistics reset for device: {}",
            self.device_path
        );
    }

    /// 启用自适应性能调优（对应LTFSCopyGUI的自动性能优化）
    pub fn enable_adaptive_performance(&mut self) {
        // 启用自适应速度控制
        if let Some(ref mut speed_limiter) = self.speed_limiter {
            speed_limiter.adaptive_mode = true;
        }

        // 根据设备类型优化并发设置
        let optimal_concurrent = match self.device_path.as_str() {
            path if path.contains("tape") || path.contains("st") => 2, // 磁带设备通常较低并发
            _ => 4,                                                    // 默认并发数
        };

        self.set_max_concurrent_operations(optimal_concurrent);

        // 性能监控功能已移除，保留自适应并发控制

        info!(
            "Adaptive performance optimization enabled for device: {}",
            self.device_path
        );
    }

    /// Set write options
    pub fn set_write_options(&mut self, options: WriteOptions) {
        self.block_size = options.block_size;
        self.write_options = options;
    }



    /// Save deduplication database (对应LTFSCopyGUI的数据库保存)
    pub fn save_deduplication_database(&mut self) -> Result<()> {
        if let Some(ref mut manager) = self.dedup_manager {
            manager.save_database()?;
            info!("去重数据库已保存");
        }
        Ok(())
    }

    /// Get current write progress
    pub fn get_write_progress(&self) -> &WriteProgress {
        &self.write_progress
    }

    /// Stop write operations
    pub fn stop_write(&mut self) {
        self.stop_flag = true;
        info!("Write operations stopped by user request");
    }

    /// Pause/resume write operations
    pub fn set_pause(&mut self, pause: bool) {
        self.pause_flag = pause;
        if pause {
            info!("Write operations paused");
        } else {
            info!("Write operations resumed");
        }
    }

    /// 初始化分区检测 (精确对应LTFSCopyGUI的初始化逻辑)
    /// 检测ExtraPartitionCount并设置分区策略 - 修复版本：直接使用已打开的SCSI设备
    pub async fn initialize_partition_detection(&mut self) -> Result<()> {
        debug!(
            "Initializing partition detection (LTFSCopyGUI compatible) - using opened SCSI device"
        );

        if self.offline_mode {
            info!("Offline mode: skipping partition detection");
            self.extra_partition_count = Some(1); // Assume dual-partition in offline mode
            return Ok(());
        }

        // 直接使用已打开的self.scsi进行MODE SENSE检测 (对应LTFSCopyGUI的MODE SENSE检测)
        debug!("🔧 Using opened SCSI device for MODE SENSE (fixing device handle inconsistency)");

        match self.scsi.mode_sense_partition_info() {
            Ok(mode_data) => {
                // 精确匹配LTFSCopyGUI逻辑: If PModeData.Length >= 4 Then ExtraPartitionCount = PModeData(3)
                if mode_data.len() >= 4 {
                    let detected_count = mode_data[3];
                    debug!(
                        "✅ ExtraPartitionCount detected from MODE SENSE: {}",
                        detected_count
                    );

                    // 应用LTFSCopyGUI的验证逻辑: Math.Min(1, value)
                    let validated_count = std::cmp::min(1, detected_count);
                    let final_count =
                        std::cmp::min(validated_count, self.max_extra_partition_allowed);

                    if final_count != detected_count {
                        debug!(
                            "ExtraPartitionCount limited from {} to {} (Math.Min validation)",
                            detected_count, final_count
                        );
                    }

                    self.extra_partition_count = Some(final_count);
                    debug!(
                        "✅ ExtraPartitionCount initialized: {} (detected: {}, validated: {})",
                        final_count, detected_count, final_count
                    );

                    // 设置modified状态 (对应LTFSCopyGUI的Modified = ExtraPartitionCount > 0)
                    self.modified = final_count > 0;
                } else {
                    warn!(
                        "MODE SENSE data too short ({} bytes), defaulting to single partition",
                        mode_data.len()
                    );
                    self.extra_partition_count = Some(0);
                    self.modified = false;
                }
            }
            Err(e) => {
                warn!(
                    "MODE SENSE 0x11 failed: {}, defaulting to single partition",
                    e
                );
                self.extra_partition_count = Some(0);
                self.modified = false;
            }
        }

        Ok(())
    }

    /// 获取当前ExtraPartitionCount
    pub fn get_extra_partition_count(&self) -> u8 {
        self.extra_partition_count.unwrap_or(0)
    }

    /// 获取目标分区号 (正确的LTFS分区映射逻辑)
    /// 修复关键Bug：之前的Math.Min逻辑导致数据写入错误分区
    pub fn get_target_partition(&self, logical_partition: u8) -> u8 {
        let extra_partition_count = self.get_extra_partition_count();

        debug!(
            "Computing target partition: logical={}, ExtraPartitionCount={}",
            logical_partition, extra_partition_count
        );

        match extra_partition_count {
            0 => {
                // 单分区磁带：所有数据和索引都在分区0
                debug!("Single-partition tape: all data goes to partition 0");
                0
            }
            1 => {
                // 双分区磁带：分区0=索引分区，分区1=数据分区
                match logical_partition {
                    0 => {
                        debug!(
                            "Dual-partition tape: index data goes to partition 0 (index partition)"
                        );
                        0 // 索引分区
                    }
                    1 => {
                        debug!(
                            "Dual-partition tape: file data goes to partition 1 (data partition)"
                        );
                        1 // 数据分区
                    }
                    _ => {
                        warn!(
                            "Unexpected logical partition {}, defaulting to data partition",
                            logical_partition
                        );
                        1
                    }
                }
            }
            _ => {
                warn!(
                    "Unexpected ExtraPartitionCount {}, using dual-partition logic",
                    extra_partition_count
                );
                if logical_partition == 0 {
                    0
                } else {
                    1
                }
            }
        }
    }

    /// 创建分区管理器 (已废弃：使用直接SCSI方法替代，仅保留以防向后兼容)
    #[deprecated(note = "Use direct SCSI methods instead to avoid device handle inconsistency")]
    #[allow(dead_code)]
    pub fn create_partition_manager(&self) -> super::partition_manager::PartitionManager {
        super::partition_manager::PartitionManager::new(
            std::sync::Arc::new(crate::scsi::ScsiInterface::new()),
            self.offline_mode,
        )
    }

    /// Wait for device ready using TestUnitReady retry logic (对应LTFSCopyGUI的TestUnitReady重试逻辑)
    pub async fn wait_for_device_ready(&self) -> Result<()> {
        debug!("Starting TestUnitReady retry logic");

        let max_retries = 5; // 对应LTFSCopyGUI的5次重试
        let retry_delay_ms = 200; // 对应LTFSCopyGUI的200ms延迟

        for retry_count in (1..=max_retries).rev() {
            debug!(
                "TestUnitReady attempt {} (remaining: {})",
                max_retries - retry_count + 1,
                retry_count
            );

            // 执行SCSI Test Unit Ready命令
            match self.scsi.test_unit_ready() {
                Ok(sense_data) => {
                    if sense_data.is_empty() {
                        // 无sense数据表示设备就绪
                        debug!("✅ Device is ready (TestUnitReady successful, no sense data)");
                        return Ok(());
                    } else {
                        // 有sense数据，需要分析
                        let sense_info = self.scsi.parse_sense_data(&sense_data);
                        debug!("TestUnitReady returned sense data: {}", sense_info);

                        // 检查是否为"设备准备就绪"的状态
                        if sense_info.contains("No additional sense information") ||
                           sense_info.contains("ready") ||  // 改为小写匹配
                           sense_info.contains("Ready") ||
                           sense_info.contains("Good") ||
                           sense_info == "Device ready"
                        {
                            // 精确匹配SCSI返回的"Device ready"
                            debug!(
                                "✅ Device is ready (TestUnitReady with ready sense: {})",
                                sense_info
                            );
                            return Ok(());
                        }

                        // 检查是否为可重试的错误
                        if sense_info.contains("Not ready")
                            || sense_info.contains("Unit attention")
                            || sense_info.contains("Medium may have changed")
                        {
                            if retry_count > 1 {
                                debug!("⏳ Device not ready ({}), retrying in {}ms (attempts remaining: {})",
                                     sense_info, retry_delay_ms, retry_count - 1);
                                tokio::time::sleep(tokio::time::Duration::from_millis(
                                    retry_delay_ms,
                                ))
                                .await;
                                continue;
                            } else {
                                warn!(
                                    "❌ Device not ready after {} attempts: {}",
                                    max_retries, sense_info
                                );
                                return Err(RustLtfsError::scsi(format!(
                                    "Device not ready after {} retries: {}",
                                    max_retries, sense_info
                                )));
                            }
                        } else {
                            // 非可重试错误，立即返回
                            return Err(RustLtfsError::scsi(format!(
                                "TestUnitReady failed: {}",
                                sense_info
                            )));
                        }
                    }
                }
                Err(e) => {
                    if retry_count > 1 {
                        warn!("🔄 TestUnitReady SCSI command failed: {}, retrying in {}ms (attempts remaining: {})",
                             e, retry_delay_ms, retry_count - 1);
                        tokio::time::sleep(tokio::time::Duration::from_millis(retry_delay_ms))
                            .await;
                        continue;
                    } else {
                        return Err(RustLtfsError::scsi(format!(
                            "TestUnitReady failed after {} retries: {}",
                            max_retries, e
                        )));
                    }
                }
            }
        }

        Ok(())
    }

    /// Initialize tape operations
    pub async fn initialize(&mut self, operation_type: Option<OperationType>) -> Result<()> {
        let op_type = operation_type.unwrap_or(OperationType::Write); // 默认为写入模式

        if self.offline_mode {
            if matches!(op_type, OperationType::Read | OperationType::Write) {
                info!("Offline mode, skipping device initialization");
            }
            return Ok(());
        }

        // 设备初始化（所有操作都需要）
        self.scsi.open_device(&self.device_path)?;
        self.wait_for_device_ready().await?;

        match self.scsi.check_media_status()? {
            crate::scsi::MediaType::NoTape => {
                return Err(RustLtfsError::tape_device("No tape loaded".to_string()));
            }
            crate::scsi::MediaType::Unknown(_) => {
                // Continue with unknown media
            }
            _ => {
                // Media detected, continue
            }
        }

        self.initialize_partition_detection().await?;

        match op_type {
            OperationType::Space => {
                debug!("Device initialization completed");
                return Ok(());
            }
            OperationType::Write => {
                debug!("Device initialization completed");
                
                // 尝试加载现有的LTFS索引
                match self.read_index_from_tape().await {
                    Ok(()) => {
                        let file_count = self
                            .index
                            .as_ref()
                            .map(|idx| idx.root_directory.contents.files.len())
                            .unwrap_or(0);
                        debug!("Index loaded successfully ({} files)", file_count);
                    }
                    Err(_) => {
                        info!("Will create new index");
                    }
                }
            }
            OperationType::Read => {
                debug!("Device initialization completed");
                
                // 读取操作必须成功加载索引
                match self.read_index_from_tape().await {
                    Ok(()) => {
                        debug!("Index loaded successfully");
                        
                        // 显示索引内容概览
                        if let Some(stats) = self.get_index_statistics() {
                            info!("Tape contents: {} files, {} directories", stats.total_files, stats.total_directories);
                        }
                    }
                    Err(e) => {
                        return Err(RustLtfsError::ltfs_index(format!("Index reading failed: {}", e)));
                    }
                }
            }
        }

        self.partition_label = Some(LtfsPartitionLabel::default());
        Ok(())
    }

    /// 保存索引到文件
    pub async fn save_index_to_file(&self, file_path: &std::path::Path) -> Result<()> {
        debug!("Saving LTFS index to file: {:?}", file_path);

        if let Some(ref index) = self.index {
            let xml_content = index.to_xml()?;
            std::fs::write(file_path, xml_content)?;
            debug!("Index saved successfully to {:?}", file_path);
            Ok(())
        } else {
            Err(RustLtfsError::ltfs_index(
                "No index loaded to save".to_string(),
            ))
        }
    }

    /// 获取索引统计信息
    pub fn get_index_statistics(&self) -> Option<IndexStatistics> {
        if let Some(ref index) = self.index {
            let mut stats = IndexStatistics::default();
            stats.total_files = count_files_in_directory(&index.root_directory);
            stats.total_directories = count_directories_in_directory(&index.root_directory);
            stats.total_size = calculate_total_size(&index.root_directory);
            stats.volume_uuid = index.volumeuuid.clone();
            stats.generation_number = index.generationnumber;
            stats.update_time = index.updatetime.clone();
            Some(stats)
        } else {
            None
        }
    }

    /// 打印目录树
    pub fn print_directory_tree(&self) {
        if let Some(ref index) = self.index {
            println!("LTFS Directory Tree:");
            print_directory_recursive(&index.root_directory, 0);
        } else {
            println!("No index loaded");
        }
    }

    /// 列出指定目录的内容
    pub fn list_directory_contents(&self, path: &str) -> Result<()> {
        if let Some(ref index) = self.index {
            if path.is_empty() || path == "/" {
                // 列出根目录
                self.print_directory_contents(&index.root_directory, 0);
            } else {
                // 查找指定目录
                let target_dir = self.find_directory_by_path(&index.root_directory, path);
                match target_dir {
                    Some(dir) => {
                        println!("📁 Contents of: {}", path);
                        self.print_directory_contents(dir, 0);
                    }
                    None => {
                        println!("❌ Directory not found: {}", path);
                        return Err(RustLtfsError::ltfs_index(format!("Directory not found: {}", path)));
                    }
                }
            }
        } else {
            return Err(RustLtfsError::ltfs_index("No index loaded".to_string()));
        }
        Ok(())
    }

    /// 打印目录内容（不递归）
    fn print_directory_contents(&self, dir: &crate::ltfs_index::Directory, depth: usize) {
        let indent = "  ".repeat(depth);
        
        // 打印文件
        for file in &dir.contents.files {
            println!("{}📄 {} ({} bytes)", indent, file.name, file.length);
        }
        
        // 打印子目录
        for subdir in &dir.contents.directories {
            println!("{}📁 {}/", indent, subdir.name);
        }
    }

    /// 根据路径查找目录
    fn find_directory_by_path<'a>(&self, root: &'a crate::ltfs_index::Directory, path: &str) -> Option<&'a crate::ltfs_index::Directory> {
        // 标准化路径
        let path = path.trim_start_matches('/').trim_end_matches('/');
        if path.is_empty() {
            return Some(root);
        }

        let path_parts: Vec<&str> = path.split('/').collect();
        self.find_directory_recursive(root, &path_parts, 0)
    }

    /// 递归查找目录
    fn find_directory_recursive<'a>(&self, current_dir: &'a crate::ltfs_index::Directory, path_parts: &[&str], index: usize) -> Option<&'a crate::ltfs_index::Directory> {
        if index >= path_parts.len() {
            return Some(current_dir);
        }

        let target_name = path_parts[index];
        for subdir in &current_dir.contents.directories {
            if subdir.name == target_name {
                return self.find_directory_recursive(subdir, path_parts, index + 1);
            }
        }

        None
    }

    /// 手动更新磁带索引
    pub async fn update_index_on_tape_manual_new(&mut self) -> Result<()> {
        info!("Manually updating index on tape");

        if self.index.is_none() {
            return Err(RustLtfsError::ltfs_index("No index to update".to_string()));
        }

        // 这里应该实现索引更新逻辑
        // 暂时返回成功
        warn!("Manual index update is not fully implemented yet");
        Ok(())
    }

    /// 刷新磁带容量信息（精确对应LTFSCopyGUI RefreshCapacity）
    pub async fn refresh_capacity(&mut self) -> Result<super::capacity_manager::TapeCapacityInfo> {
        info!("Refreshing tape capacity information");

        let mut capacity_info = super::capacity_manager::TapeCapacityInfo {
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

        // 直接使用self.scsi来读取容量信息
        info!("Reading tape capacity log page (0x31)");
        let capacity_log_data = match self.scsi.log_sense(0x31, 1) {
            Ok(data) => {
                info!("📊 Capacity log data length: {} bytes", data.len());
                if data.len() > 0 {
                    info!("📊 Capacity log data preview: {:02X?}", &data[..std::cmp::min(32, data.len())]);
                }
                data
            },
            Err(e) => {
                warn!("Failed to read capacity log page: {}", e);
                return Ok(capacity_info);
            }
        };

        // 解析容量信息
        let capacity_parser = super::capacity_manager::CapacityPageParser::new(capacity_log_data);
        
        capacity_info.p0_remaining = capacity_parser.get_remaining_capacity(0).unwrap_or(0);
        capacity_info.p0_maximum = capacity_parser.get_maximum_capacity(0).unwrap_or(0);

        let extra_partition_count = self.get_extra_partition_count();
        if extra_partition_count > 0 {
            capacity_info.p1_remaining = capacity_parser.get_remaining_capacity(1).unwrap_or(0);
            capacity_info.p1_maximum = capacity_parser.get_maximum_capacity(1).unwrap_or(0);
        }

        info!("Capacity refresh completed: P0({:.2}/{:.2}) GB, P1({:.2}/{:.2}) GB", 
              capacity_info.p0_remaining as f64 / 1024.0,
              capacity_info.p0_maximum as f64 / 1024.0,
              capacity_info.p1_remaining as f64 / 1024.0, 
              capacity_info.p1_maximum as f64 / 1024.0);

        Ok(capacity_info)
    }

    /// 读取错误率信息（对应LTFSCopyGUI ReadChanLRInfo）
    pub async fn read_error_rate_info(&mut self) -> Result<f64> {
        info!("Reading tape error rate information");

        // 直接使用self.scsi读取错误率信息
        match self.scsi.log_sense(0x02, 1) {
            Ok(_data) => {
                // 简单解析错误率（可以后续完善）
                Ok(0.0)
            }
            Err(e) => {
                warn!("Failed to read error rate: {}", e);
                Ok(0.0)
            }
        }
    }

    /// 获取磁带容量信息（简化版本，用于向后兼容）
    pub async fn get_tape_capacity_info(&mut self) -> Result<TapeSpaceInfo> {
        let capacity_info = self.refresh_capacity().await?;

        // 根据ExtraPartitionCount决定使用哪个分区的容量
        let (used_space, total_capacity) = if self.get_extra_partition_count() > 0 {
            // 多分区磁带：显示P0+P1的总容量（剩余容量）
            let p0_remaining_bytes = capacity_info.p0_remaining * 1024; // KB转字节
            let p1_remaining_bytes = capacity_info.p1_remaining * 1024; // KB转字节
            let total_remaining = p0_remaining_bytes + p1_remaining_bytes;
            
            // 计算已使用空间（如果有最大容量数据）
            let used_space = if capacity_info.p0_maximum > 0 && capacity_info.p1_maximum > 0 {
                let p0_used = capacity_info.p0_maximum.saturating_sub(capacity_info.p0_remaining);
                let p1_used = capacity_info.p1_maximum.saturating_sub(capacity_info.p1_remaining);
                (p0_used + p1_used) * 1024 // KB转字节
            } else {
                // 如果没有最大容量数据，假设已使用很少
                0
            };
            
            (used_space, total_remaining)
        } else {
            // 单分区磁带：使用P0容量
            let used_p0 = capacity_info
                .p0_maximum
                .saturating_sub(capacity_info.p0_remaining);
            ((used_p0 * 1024), (capacity_info.p0_maximum * 1024)) // KB转换为字节
        };

        Ok(TapeSpaceInfo {
            total_capacity,
            used_space,
            available_space: total_capacity.saturating_sub(used_space),
        })
    }
}

/// 索引统计信息
#[derive(Debug, Default)]
pub struct IndexStatistics {
    pub total_files: u64,
    pub total_directories: u64,
    pub total_size: u64,
    pub volume_uuid: String,
    pub generation_number: u64,
    pub update_time: String,
}

/// 磁带空间信息
#[derive(Debug)]
pub struct TapeSpaceInfo {
    pub total_capacity: u64,
    pub used_space: u64,
    pub available_space: u64,
}

/// 文件提取结果
#[derive(Debug)]
pub struct ExtractResult {
    pub files_extracted: u64,
    pub directories_created: u64,
    pub total_bytes: u64,
    pub verification_passed: bool,
}

// 辅助函数
fn count_files_in_directory(dir: &crate::ltfs_index::Directory) -> u64 {
    let mut count = dir.contents.files.len() as u64;
    for subdir in &dir.contents.directories {
        count += count_files_in_directory(subdir);
    }
    count
}

fn count_directories_in_directory(dir: &crate::ltfs_index::Directory) -> u64 {
    let mut count = dir.contents.directories.len() as u64;
    for subdir in &dir.contents.directories {
        count += count_directories_in_directory(subdir);
    }
    count
}

fn calculate_total_size(dir: &crate::ltfs_index::Directory) -> u64 {
    let mut size = 0;
    // 计算文件大小
    for file in &dir.contents.files {
        size += file.length;
    }
    // 递归计算子目录大小
    for subdir in &dir.contents.directories {
        size += calculate_total_size(subdir);
    }
    size
}

fn print_directory_recursive(dir: &crate::ltfs_index::Directory, depth: usize) {
    let indent = "  ".repeat(depth);
    // 打印文件
    for file in &dir.contents.files {
        println!("{}📄 {} ({} bytes)", indent, file.name, file.length);
    }
    // 打印并递归子目录
    for subdir in &dir.contents.directories {
        println!("{}📁 {}/", indent, subdir.name);
        print_directory_recursive(subdir, depth + 1);
    }
}
