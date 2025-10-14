/// Performance optimization module for LTFS operations
/// 
/// This module provides caching, batching, and performance enhancement
/// features for LTFS tape operations.

use std::collections::{HashMap, VecDeque};
use std::time::{Duration, Instant};
use tracing::{debug, info, warn};

/// Cache configuration for LTFS operations
#[derive(Debug, Clone)]
pub struct CacheConfig {
    /// Maximum cache size in bytes
    pub max_cache_size: usize,
    /// Cache entry TTL in seconds
    pub ttl_seconds: u64,
    /// Maximum number of cached index entries
    pub max_index_entries: usize,
    /// Enable block-level caching
    pub enable_block_cache: bool,
    /// Block cache size in blocks
    pub block_cache_size: usize,
}

impl Default for CacheConfig {
    fn default() -> Self {
        Self {
            max_cache_size: 256 * 1024 * 1024, // 256MB default cache
            ttl_seconds: 300, // 5 minutes TTL
            max_index_entries: 1000,
            enable_block_cache: true,
            block_cache_size: 1024, // 1024 blocks (64MB with 64KB blocks)
        }
    }
}

/// Cached data entry with timestamp
#[derive(Debug, Clone)]
struct CacheEntry {
    data: Vec<u8>,
    timestamp: Instant,
    access_count: u64,
}

impl CacheEntry {
    fn new(data: Vec<u8>) -> Self {
        Self {
            data,
            timestamp: Instant::now(),
            access_count: 0,
        }
    }

    fn is_expired(&self, ttl: Duration) -> bool {
        self.timestamp.elapsed() > ttl
    }

    fn access(&mut self) -> &Vec<u8> {
        self.access_count += 1;
        &self.data
    }
}

/// Cache key for identifying cached data
#[derive(Debug, Clone, Hash, PartialEq, Eq)]
pub enum CacheKey {
    /// Block data cache key (partition, start_block, block_count)
    BlockData(u8, u64, u32),
    /// Index cache key (generation_number)
    Index(u64),
    /// File extent cache key (file_uid, extent_index)
    FileExtent(u64, usize),
    /// Directory listing cache key (path)
    DirectoryListing(String),
}

/// Performance cache for LTFS operations
pub struct LtfsPerformanceCache {
    cache: HashMap<CacheKey, CacheEntry>,
    config: CacheConfig,
    current_size: usize,
    hit_count: u64,
    miss_count: u64,
}

impl LtfsPerformanceCache {
    /// Create new performance cache with default configuration
    pub fn new() -> Self {
        Self::with_config(CacheConfig::default())
    }

    /// Create new performance cache with custom configuration
    pub fn with_config(config: CacheConfig) -> Self {
        Self {
            cache: HashMap::new(),
            config,
            current_size: 0,
            hit_count: 0,
            miss_count: 0,
        }
    }

    /// Get data from cache
    pub fn get(&mut self, key: &CacheKey) -> Option<Vec<u8>> {
        // Clean expired entries periodically
        if self.cache.len() % 100 == 0 {
            self.cleanup_expired();
        }

        if let Some(entry) = self.cache.get_mut(key) {
            if entry.is_expired(Duration::from_secs(self.config.ttl_seconds)) {
                self.cache.remove(key);
                self.miss_count += 1;
                None
            } else {
                self.hit_count += 1;
                Some(entry.access().clone())
            }
        } else {
            self.miss_count += 1;
            None
        }
    }

    /// Put data into cache
    pub fn put(&mut self, key: CacheKey, data: Vec<u8>) {
        let data_size = data.len();
        
        // Check if we need to make space
        while self.current_size + data_size > self.config.max_cache_size {
            if !self.evict_lru() {
                // Can't evict any more, cache is full
                warn!("Cache is full, cannot store new entry");
                return;
            }
        }

        // Remove existing entry if it exists
        if let Some(old_entry) = self.cache.remove(&key) {
            self.current_size -= old_entry.data.len();
        }

        // Add new entry
        let entry = CacheEntry::new(data);
        self.current_size += data_size;
        self.cache.insert(key, entry);

        debug!("Cache entry added, current size: {} bytes", self.current_size);
    }

    /// Evict least recently used entry
    fn evict_lru(&mut self) -> bool {
        if self.cache.is_empty() {
            return false;
        }

        // Find the entry with the oldest timestamp and lowest access count
        let mut oldest_key = None;
        let mut oldest_time = Instant::now();
        let mut lowest_access = u64::MAX;

        for (key, entry) in &self.cache {
            if entry.timestamp < oldest_time || 
               (entry.timestamp == oldest_time && entry.access_count < lowest_access) {
                oldest_time = entry.timestamp;
                lowest_access = entry.access_count;
                oldest_key = Some(key.clone());
            }
        }

        if let Some(key) = oldest_key {
            if let Some(entry) = self.cache.remove(&key) {
                self.current_size -= entry.data.len();
                debug!("Evicted cache entry, freed {} bytes", entry.data.len());
                return true;
            }
        }

        false
    }

    /// Clean up expired entries
    fn cleanup_expired(&mut self) {
        let ttl = Duration::from_secs(self.config.ttl_seconds);
        let mut to_remove = Vec::new();

        for (key, entry) in &self.cache {
            if entry.is_expired(ttl) {
                to_remove.push(key.clone());
            }
        }

        for key in to_remove {
            if let Some(entry) = self.cache.remove(&key) {
                self.current_size -= entry.data.len();
            }
        }
    }

    /// Get cache statistics
    pub fn get_stats(&self) -> CacheStats {
        let total_requests = self.hit_count + self.miss_count;
        let hit_rate = if total_requests > 0 {
            (self.hit_count as f64 / total_requests as f64) * 100.0
        } else {
            0.0
        };

        CacheStats {
            entries: self.cache.len(),
            total_size: self.current_size,
            max_size: self.config.max_cache_size,
            hit_count: self.hit_count,
            miss_count: self.miss_count,
            hit_rate,
        }
    }

    /// Clear all cached data
    pub fn clear(&mut self) {
        self.cache.clear();
        self.current_size = 0;
        self.hit_count = 0;
        self.miss_count = 0;
        info!("Cache cleared");
    }
}

/// Cache statistics
#[derive(Debug)]
pub struct CacheStats {
    pub entries: usize,
    pub total_size: usize,
    pub max_size: usize,
    pub hit_count: u64,
    pub miss_count: u64,
    pub hit_rate: f64,
}

/// Batch operation manager for optimizing multiple LTFS operations
pub struct BatchOperationManager {
    /// Pending read operations
    read_queue: VecDeque<BatchReadRequest>,
    /// Pending write operations  
    write_queue: VecDeque<BatchWriteRequest>,
    /// Batch configuration
    config: BatchConfig,
}

/// Configuration for batch operations
#[derive(Debug, Clone)]
pub struct BatchConfig {
    /// Maximum batch size (number of operations)
    pub max_batch_size: usize,
    /// Maximum wait time before processing batch
    pub max_wait_duration: Duration,
    /// Minimum batch size to trigger processing
    pub min_batch_size: usize,
    /// Enable read-ahead optimization
    pub enable_read_ahead: bool,
    /// Read-ahead window size in blocks
    pub read_ahead_blocks: u32,
}

impl Default for BatchConfig {
    fn default() -> Self {
        Self {
            max_batch_size: 100,
            max_wait_duration: Duration::from_millis(500),
            min_batch_size: 5,
            enable_read_ahead: true,
            read_ahead_blocks: 32, // 2MB read-ahead with 64KB blocks
        }
    }
}

/// Batch read request
#[derive(Debug)]
pub struct BatchReadRequest {
    pub file_uid: u64,
    pub offset: u64,
    pub length: u64,
    pub priority: ReadPriority,
    pub timestamp: Instant,
}

/// Batch write request
#[derive(Debug)]
pub struct BatchWriteRequest {
    pub data: Vec<u8>,
    pub target_path: String,
    pub priority: WritePriority,
    pub timestamp: Instant,
}

/// Read operation priority
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum ReadPriority {
    Low = 0,
    Normal = 1,
    High = 2,
    Critical = 3,
}

/// Write operation priority  
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum WritePriority {
    Low = 0,
    Normal = 1,
    High = 2,
    Critical = 3,
}

impl BatchOperationManager {
    /// Create new batch operation manager
    pub fn new() -> Self {
        Self::with_config(BatchConfig::default())
    }

    /// Create with custom configuration
    pub fn with_config(config: BatchConfig) -> Self {
        Self {
            read_queue: VecDeque::new(),
            write_queue: VecDeque::new(),
            config,
        }
    }

    /// Add read request to batch queue
    pub fn queue_read(&mut self, request: BatchReadRequest) {
        self.read_queue.push_back(request);
        
        // Sort by priority and optimize for sequential access
        self.optimize_read_queue();
        
        debug!("Read request queued, total pending: {}", self.read_queue.len());
    }

    /// Add write request to batch queue
    pub fn queue_write(&mut self, request: BatchWriteRequest) {
        self.write_queue.push_back(request);
        
        // Sort by priority
        self.optimize_write_queue();
        
        debug!("Write request queued, total pending: {}", self.write_queue.len());
    }

    /// Check if batch should be processed
    pub fn should_process_batch(&self) -> bool {
        let read_ready = self.read_queue.len() >= self.config.min_batch_size;
        let write_ready = self.write_queue.len() >= self.config.min_batch_size;
        
        let read_timeout = self.read_queue.front()
            .map(|req| req.timestamp.elapsed() > self.config.max_wait_duration)
            .unwrap_or(false);
            
        let write_timeout = self.write_queue.front()
            .map(|req| req.timestamp.elapsed() > self.config.max_wait_duration)
            .unwrap_or(false);

        read_ready || write_ready || read_timeout || write_timeout
    }

    /// Get next batch of read operations
    pub fn get_read_batch(&mut self) -> Vec<BatchReadRequest> {
        let batch_size = std::cmp::min(self.config.max_batch_size, self.read_queue.len());
        
        let mut batch = Vec::with_capacity(batch_size);
        for _ in 0..batch_size {
            if let Some(request) = self.read_queue.pop_front() {
                batch.push(request);
            }
        }
        
        info!("Processing read batch of {} operations", batch.len());
        batch
    }

    /// Get next batch of write operations
    pub fn get_write_batch(&mut self) -> Vec<BatchWriteRequest> {
        let batch_size = std::cmp::min(self.config.max_batch_size, self.write_queue.len());
        
        let mut batch = Vec::with_capacity(batch_size);
        for _ in 0..batch_size {
            if let Some(request) = self.write_queue.pop_front() {
                batch.push(request);
            }
        }
        
        info!("Processing write batch of {} operations", batch.len());
        batch
    }

    /// Optimize read queue for sequential access patterns
    fn optimize_read_queue(&mut self) {
        // Sort by priority first, then by file position for sequential access
        let mut queue_vec: Vec<_> = self.read_queue.drain(..).collect();
        
        queue_vec.sort_by(|a, b| {
            // First sort by priority (higher priority first)
            match b.priority.cmp(&a.priority) {
                std::cmp::Ordering::Equal => {
                    // Then by file UID and offset for sequential access
                    match a.file_uid.cmp(&b.file_uid) {
                        std::cmp::Ordering::Equal => a.offset.cmp(&b.offset),
                        other => other,
                    }
                }
                other => other,
            }
        });
        
        self.read_queue.extend(queue_vec);
    }

    /// Optimize write queue by priority
    fn optimize_write_queue(&mut self) {
        let mut queue_vec: Vec<_> = self.write_queue.drain(..).collect();
        
        // Sort by priority (higher priority first)
        queue_vec.sort_by(|a, b| b.priority.cmp(&a.priority));
        
        self.write_queue.extend(queue_vec);
    }

    /// Get queue statistics
    pub fn get_queue_stats(&self) -> QueueStats {
        QueueStats {
            pending_reads: self.read_queue.len(),
            pending_writes: self.write_queue.len(),
            oldest_read_wait: self.read_queue.front()
                .map(|req| req.timestamp.elapsed())
                .unwrap_or(Duration::ZERO),
            oldest_write_wait: self.write_queue.front()
                .map(|req| req.timestamp.elapsed())
                .unwrap_or(Duration::ZERO),
        }
    }
}

/// Queue statistics
#[derive(Debug)]
pub struct QueueStats {
    pub pending_reads: usize,
    pub pending_writes: usize,
    pub oldest_read_wait: Duration,
    pub oldest_write_wait: Duration,
}

/// Performance monitoring and optimization manager
pub struct PerformanceMonitor {
    cache: LtfsPerformanceCache,
    batch_manager: BatchOperationManager,
    start_time: Instant,
    operation_count: u64,
    total_bytes_processed: u64,
}

impl PerformanceMonitor {
    /// Create new performance monitor
    pub fn new() -> Self {
        Self {
            cache: LtfsPerformanceCache::new(),
            batch_manager: BatchOperationManager::new(),
            start_time: Instant::now(),
            operation_count: 0,
            total_bytes_processed: 0,
        }
    }

    /// Create with custom configurations
    pub fn with_configs(cache_config: CacheConfig, batch_config: BatchConfig) -> Self {
        Self {
            cache: LtfsPerformanceCache::with_config(cache_config),
            batch_manager: BatchOperationManager::with_config(batch_config),
            start_time: Instant::now(),
            operation_count: 0,
            total_bytes_processed: 0,
        }
    }

    /// Record operation completion
    pub fn record_operation(&mut self, bytes_processed: u64) {
        self.operation_count += 1;
        self.total_bytes_processed += bytes_processed;
    }

    /// Get performance statistics
    pub fn get_performance_stats(&self) -> PerformanceStats {
        let elapsed = self.start_time.elapsed();
        let throughput = if elapsed.as_secs() > 0 {
            self.total_bytes_processed as f64 / elapsed.as_secs_f64()
        } else {
            0.0
        };
        
        let operations_per_second = if elapsed.as_secs() > 0 {
            self.operation_count as f64 / elapsed.as_secs_f64()
        } else {
            0.0
        };

        PerformanceStats {
            elapsed_time: elapsed,
            total_operations: self.operation_count,
            total_bytes: self.total_bytes_processed,
            throughput_bytes_per_sec: throughput,
            operations_per_second,
            cache_stats: self.cache.get_stats(),
            queue_stats: self.batch_manager.get_queue_stats(),
        }
    }

    /// Get cache reference for external access
    pub fn cache(&mut self) -> &mut LtfsPerformanceCache {
        &mut self.cache
    }

    /// Get batch manager reference for external access
    pub fn batch_manager(&mut self) -> &mut BatchOperationManager {
        &mut self.batch_manager
    }
}

/// Combined performance statistics
#[derive(Debug)]
pub struct PerformanceStats {
    pub elapsed_time: Duration,
    pub total_operations: u64,
    pub total_bytes: u64,
    pub throughput_bytes_per_sec: f64,
    pub operations_per_second: f64,
    pub cache_stats: CacheStats,
    pub queue_stats: QueueStats,
}

impl PerformanceStats {
    /// Format performance statistics for display
    pub fn format_summary(&self) -> String {
        format!(
            "Performance Summary:\n\
             - Elapsed: {:.2}s\n\
             - Operations: {} ({:.1}/s)\n\
             - Data: {} ({:.1} MB/s)\n\
             - Cache: {:.1}% hit rate ({} entries)\n\
             - Queue: {} reads, {} writes pending",
            self.elapsed_time.as_secs_f64(),
            self.total_operations,
            self.operations_per_second,
            format_bytes(self.total_bytes),
            self.throughput_bytes_per_sec / 1_000_000.0,
            self.cache_stats.hit_rate,
            self.cache_stats.entries,
            self.queue_stats.pending_reads,
            self.queue_stats.pending_writes
        )
    }
}

/// Helper function to format byte counts
fn format_bytes(bytes: u64) -> String {
    const UNITS: &[&str] = &["B", "KB", "MB", "GB", "TB"];
    const THRESHOLD: f64 = 1024.0;

    if bytes == 0 {
        return "0 B".to_string();
    }

    let mut size = bytes as f64;
    let mut unit_index = 0;

    while size >= THRESHOLD && unit_index < UNITS.len() - 1 {
        size /= THRESHOLD;
        unit_index += 1;
    }

    if unit_index == 0 {
        format!("{} {}", bytes, UNITS[unit_index])
    } else {
        format!("{:.2} {}", size, UNITS[unit_index])
    }
}

/// Export cache key constructors for external use
impl CacheKey {
    pub fn block_data(partition: u8, start_block: u64, block_count: u32) -> Self {
        Self::BlockData(partition, start_block, block_count)
    }

    pub fn index(generation: u64) -> Self {
        Self::Index(generation)
    }

    pub fn file_extent(file_uid: u64, extent_index: usize) -> Self {
        Self::FileExtent(file_uid, extent_index)
    }

    pub fn directory_listing(path: String) -> Self {
        Self::DirectoryListing(path)
    }
}