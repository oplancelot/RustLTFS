/// LTFS (Linear Tape File System) module
/// 
/// This module provides high-level LTFS operations including direct file access,
/// capacity management, and volume information handling.

pub mod direct_access;
pub mod capacity;
pub mod volume_info;
pub mod utils;
pub mod performance;

// Re-export main types and interfaces
pub use direct_access::{LtfsDirectAccess, create_ltfs_access, IndexSyncResult, SyncValidationResult};
pub use capacity::{TapeCapacity, CapacityManager};
pub use volume_info::{LtfsVolumeInfo, VolumeInfoManager};
pub use utils::{IndexLocation, find_xml_end, get_current_timestamp};
pub use performance::{
    LtfsPerformanceCache, PerformanceMonitor, BatchOperationManager, CacheKey,
    CacheConfig, BatchConfig, CacheStats, PerformanceStats,
    BatchReadRequest, BatchWriteRequest, ReadPriority, WritePriority
};