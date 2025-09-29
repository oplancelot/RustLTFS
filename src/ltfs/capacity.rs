/// Tape capacity management for LTFS operations

use crate::error::{Result, RustLtfsError};
use crate::scsi::ScsiInterface;
use tracing::{debug, info, warn};

/// Tape capacity information
#[derive(Debug, Clone)]
pub struct TapeCapacity {
    pub total_capacity: u64,
    pub used_capacity: u64,
    pub available_capacity: u64,
}

impl TapeCapacity {
    /// Create new tape capacity information
    pub fn new(total_capacity: u64, used_capacity: u64) -> Self {
        let available_capacity = total_capacity.saturating_sub(used_capacity);
        Self {
            total_capacity,
            used_capacity,
            available_capacity,
        }
    }

    /// Get capacity utilization as percentage
    pub fn utilization_percentage(&self) -> f64 {
        if self.total_capacity == 0 {
            0.0
        } else {
            (self.used_capacity as f64 / self.total_capacity as f64) * 100.0
        }
    }

    /// Check if tape has enough space for the given size
    pub fn has_space_for(&self, required_size: u64) -> bool {
        self.available_capacity >= required_size
    }

    /// Format capacity as human-readable string
    pub fn format_capacity(&self) -> String {
        format!(
            "Total: {}, Used: {}, Available: {} ({:.1}% used)",
            format_bytes(self.total_capacity),
            format_bytes(self.used_capacity),
            format_bytes(self.available_capacity),
            self.utilization_percentage()
        )
    }
}

/// Capacity management implementation
pub struct CapacityManager {
    scsi: ScsiInterface,
}

impl CapacityManager {
    /// Create new capacity manager
    pub fn new(scsi: ScsiInterface) -> Self {
        Self { scsi }
    }

    /// Get tape capacity information
    pub fn get_capacity_info(&self) -> Result<TapeCapacity> {
        debug!("Getting tape capacity information");

        // Try to get capacity from MAM attributes first
        match self.get_remaining_capacity_from_mam() {
            Ok(remaining) => {
                // Estimate total capacity based on media type
                let total = self.estimate_total_capacity()?;
                let used = total.saturating_sub(remaining);
                
                info!("Capacity from MAM: Total={}, Used={}, Available={}", 
                      format_bytes(total), format_bytes(used), format_bytes(remaining));
                
                Ok(TapeCapacity::new(total, used))
            }
            Err(_) => {
                warn!("Could not read capacity from MAM, using estimates");
                self.estimate_capacity()
            }
        }
    }

    /// Check available space for write operations
    pub fn check_available_space(&self, required_size: u64) -> Result<bool> {
        let capacity = self.get_capacity_info()?;
        
        if capacity.has_space_for(required_size) {
            info!("Sufficient space available: {} required, {} available", 
                  format_bytes(required_size), format_bytes(capacity.available_capacity));
            Ok(true)
        } else {
            warn!("Insufficient space: {} required, {} available", 
                  format_bytes(required_size), format_bytes(capacity.available_capacity));
            Ok(false)
        }
    }

    /// Get remaining capacity from MAM attributes (enhanced implementation)
    fn get_remaining_capacity_from_mam(&self) -> Result<u64> {
        debug!("Reading remaining capacity from MAM");

        // Enhanced MAM attribute IDs based on LTFSCopyGUI implementation
        const REMAINING_CAPACITY_ATTR_ID: u16 = 0x0220;     // Remaining capacity in partition
        const MAXIMUM_CAPACITY_ATTR_ID: u16 = 0x0221;       // Maximum capacity of partition
        const TOTAL_MB_WRITTEN_ATTR_ID: u16 = 0x0204;       // Total MB written
        const TOTAL_LOADS_ATTR_ID: u16 = 0x0206;            // Total loads count
        const THREAD_COUNT_ATTR_ID: u16 = 0x0202;           // Thread count (for wear estimation)
        
        // Strategy 1: Try to read remaining capacity directly (most accurate)
        if let Ok(remaining_capacity) = self.read_mam_capacity_attribute(REMAINING_CAPACITY_ATTR_ID) {
            info!("Got remaining capacity from MAM: {} bytes", remaining_capacity);
            return Ok(remaining_capacity);
        }
        
        // Strategy 2: Calculate from maximum capacity and used space
        if let Ok(max_capacity) = self.read_mam_capacity_attribute(MAXIMUM_CAPACITY_ATTR_ID) {
            if let Ok(used_capacity) = self.calculate_used_capacity_from_mam() {
                let remaining = max_capacity.saturating_sub(used_capacity);
                info!("Calculated remaining capacity: {} bytes (max: {}, used: {})", 
                      remaining, max_capacity, used_capacity);
                return Ok(remaining);
            }
        }
        
        // Strategy 3: Estimate based on wear indicators (LTFSCopyGUI approach)
        if let Ok(estimated_capacity) = self.estimate_capacity_from_wear_indicators() {
            return Ok(estimated_capacity);
        }
        
        // Strategy 4: Estimate based on tape position and media type
        if let Ok(estimated_capacity) = self.estimate_capacity_from_position() {
            return Ok(estimated_capacity);
        }
        
        // Fallback: Conservative estimate
        let fallback_capacity = 1_000_000_000_000u64; // 1TB fallback
        warn!("All MAM capacity detection failed, using conservative fallback: {} bytes", fallback_capacity);
        Ok(fallback_capacity)
    }
    
    /// Estimate capacity from wear indicators (new method based on LTFSCopyGUI)
    fn estimate_capacity_from_wear_indicators(&self) -> Result<u64> {
        debug!("Estimating capacity from wear indicators");
        
        const THREAD_COUNT_ATTR_ID: u16 = 0x0202;
        const TOTAL_LOADS_ATTR_ID: u16 = 0x0206;
        
        // Read wear indicators
        let thread_count = self.read_mam_capacity_attribute(THREAD_COUNT_ATTR_ID).unwrap_or(0);
        let total_loads = self.read_mam_capacity_attribute(TOTAL_LOADS_ATTR_ID).unwrap_or(0);
        
        if thread_count > 0 || total_loads > 0 {
            // Get theoretical capacity
            let theoretical_capacity = self.estimate_total_capacity()?;
            
            // Apply wear-based reduction (following LTFSCopyGUI logic)
            let wear_factor = if total_loads > 1000 {
                0.85 // High usage tapes may have reduced capacity
            } else if total_loads > 500 {
                0.90 // Medium usage
            } else {
                0.95 // Low usage
            };
            
            let estimated_remaining = (theoretical_capacity as f64 * wear_factor) as u64;
            
            info!("Wear-based capacity estimation: {} bytes (loads: {}, threads: {}, factor: {})", 
                  estimated_remaining, total_loads, thread_count, wear_factor);
            
            return Ok(estimated_remaining);
        }
        
        Err(RustLtfsError::scsi("No wear indicators available"))
    }
    
    /// Calculate used capacity from MAM statistics (enhanced implementation)
    fn calculate_used_capacity_from_mam(&self) -> Result<u64> {
        debug!("Calculating used capacity from MAM statistics");
        
        // Try to get total MB written from MAM
        const TOTAL_MB_WRITTEN_ATTR_ID: u16 = 0x0204;
        
        match self.read_mam_capacity_attribute(TOTAL_MB_WRITTEN_ATTR_ID) {
            Ok(mb_written_bytes) => {
                // Add overhead for LTFS metadata (typically 2-5% as per LTFSCopyGUI)
                let overhead = mb_written_bytes / 20; // 5% overhead
                let total_used = mb_written_bytes + overhead;
                
                info!("Used capacity from MAM: {} bytes (data: {}, overhead: {})", 
                      total_used, mb_written_bytes, overhead);
                Ok(total_used)
            },
            Err(_) => {
                // Fallback: estimate from tape position and block usage
                self.estimate_used_capacity_from_position()
            }
        }
    }
    
    /// Estimate used capacity from tape position (new method)
    fn estimate_used_capacity_from_position(&self) -> Result<u64> {
        debug!("Estimating used capacity from tape position");
        
        // This would require SCSI interface to read position
        // For now, return a conservative estimate
        Err(RustLtfsError::scsi("Position-based estimation not implemented"))
    }
    
    /// Estimate remaining capacity from tape position and media type (enhanced)
    fn estimate_capacity_from_position(&self) -> Result<u64> {
        debug!("Estimating capacity from tape position");
        
        // This method exists in other modules, we need to implement it here
        // For now, return error to use other methods
        Err(RustLtfsError::scsi("Position-based capacity estimation not implemented in this context"))
    }

    /// Read capacity attribute from MAM (enhanced implementation based on LTFSCopyGUI)
    fn read_mam_capacity_attribute(&self, attribute_id: u16) -> Result<u64> {
        debug!("Reading MAM capacity attribute 0x{:04X}", attribute_id);
        
        match self.scsi.read_mam_attribute(attribute_id) {
            Ok(attribute) => {
                debug!("MAM attribute data length: {}", attribute.data.len());
                
                match attribute.attribute_format {
                    0x00 => { // BINARY format (对应LTFSCopyGUI的Binary属性)
                        if attribute.data.len() >= 8 {
                            // 64-bit big-endian value (LTO standard format)
                            let capacity = ((attribute.data[0] as u64) << 56) |
                                         ((attribute.data[1] as u64) << 48) |
                                         ((attribute.data[2] as u64) << 40) |
                                         ((attribute.data[3] as u64) << 32) |
                                         ((attribute.data[4] as u64) << 24) |
                                         ((attribute.data[5] as u64) << 16) |
                                         ((attribute.data[6] as u64) << 8) |
                                         (attribute.data[7] as u64);
                            
                            // Convert based on attribute type (following LTFSCopyGUI logic)
                            let capacity_bytes = match attribute_id {
                                0x0220 | 0x0221 => capacity * 1024, // Capacity attributes in KB
                                0x0204 => capacity * 1048576, // Total MB written to bytes
                                _ => capacity, // Others in bytes
                            };
                            
                            debug!("MAM capacity attribute 0x{:04X}: {} bytes (raw: {})", 
                                   attribute_id, capacity_bytes, capacity);
                            return Ok(capacity_bytes);
                        } else if attribute.data.len() >= 4 {
                            // 32-bit value (fallback for older drives)
                            let capacity = ((attribute.data[0] as u64) << 24) |
                                         ((attribute.data[1] as u64) << 16) |
                                         ((attribute.data[2] as u64) << 8) |
                                         (attribute.data[3] as u64);
                            
                            let capacity_bytes = capacity * 1024; // Assume KB
                            debug!("MAM capacity attribute 0x{:04X} (32-bit): {} bytes", 
                                   attribute_id, capacity_bytes);
                            return Ok(capacity_bytes);
                        }
                    },
                    0x01 => { // ASCII format (对应LTFSCopyGUI的Text属性)
                        if let Ok(ascii_str) = String::from_utf8(attribute.data.clone()) {
                            if let Ok(value) = ascii_str.trim().parse::<u64>() {
                                let capacity_bytes = value * 1024; // Assume KB
                                debug!("MAM capacity attribute 0x{:04X} (ASCII): {} bytes", 
                                       attribute_id, capacity_bytes);
                                return Ok(capacity_bytes);
                            }
                        }
                    },
                    _ => {
                        debug!("Unsupported MAM attribute format: 0x{:02X}", attribute.attribute_format);
                    }
                }
                
                Err(RustLtfsError::parse(
                    format!("Cannot parse MAM attribute 0x{:04X} data", attribute_id)
                ))
            },
            Err(e) => {
                debug!("Failed to read MAM attribute 0x{:04X}: {}", attribute_id, e);
                Err(e)
            }
        }
    }

    /// Estimate total capacity based on media type
    fn estimate_total_capacity(&self) -> Result<u64> {
        match self.scsi.check_media_status()? {
            crate::scsi::MediaType::Lto9Rw | crate::scsi::MediaType::Lto9Worm | crate::scsi::MediaType::Lto9Ro => {
                Ok(18_000_000_000_000) // 18TB for LTO-9
            }
            crate::scsi::MediaType::Lto8Rw | crate::scsi::MediaType::Lto8Worm | crate::scsi::MediaType::Lto8Ro => {
                Ok(12_000_000_000_000) // 12TB for LTO-8
            }
            crate::scsi::MediaType::Lto7Rw | crate::scsi::MediaType::Lto7Worm | crate::scsi::MediaType::Lto7Ro => {
                Ok(6_000_000_000_000) // 6TB for LTO-7
            }
            crate::scsi::MediaType::Lto6Rw | crate::scsi::MediaType::Lto6Worm | crate::scsi::MediaType::Lto6Ro => {
                Ok(2_500_000_000_000) // 2.5TB for LTO-6
            }
            crate::scsi::MediaType::Lto5Rw | crate::scsi::MediaType::Lto5Worm | crate::scsi::MediaType::Lto5Ro => {
                Ok(1_500_000_000_000) // 1.5TB for LTO-5
            }
            crate::scsi::MediaType::Lto4Rw | crate::scsi::MediaType::Lto4Worm | crate::scsi::MediaType::Lto4Ro => {
                Ok(800_000_000_000) // 800GB for LTO-4
            }
            crate::scsi::MediaType::Lto3Rw | crate::scsi::MediaType::Lto3Worm | crate::scsi::MediaType::Lto3Ro => {
                Ok(400_000_000_000) // 400GB for LTO-3
            }
            _ => {
                warn!("Unknown media type, using default capacity estimate");
                Ok(2_500_000_000_000) // Default to 2.5TB
            }
        }
    }

    /// Estimate capacity when MAM is not available
    fn estimate_capacity(&self) -> Result<TapeCapacity> {
        let total = self.estimate_total_capacity()?;
        
        // Without MAM, assume 50% usage as a rough estimate
        let used = total / 2;
        
        info!("Using estimated capacity: Total={}, Used={} (estimated)", 
              format_bytes(total), format_bytes(used));
        
        Ok(TapeCapacity::new(total, used))
    }
}

/// Format bytes in human-readable format
fn format_bytes(bytes: u64) -> String {
    const UNITS: &[&str] = &["B", "KB", "MB", "GB", "TB", "PB"];
    const THRESHOLD: u64 = 1024;

    if bytes == 0 {
        return "0 B".to_string();
    }

    let mut size = bytes as f64;
    let mut unit_index = 0;

    while size >= THRESHOLD as f64 && unit_index < UNITS.len() - 1 {
        size /= THRESHOLD as f64;
        unit_index += 1;
    }

    if unit_index == 0 {
        format!("{} {}", bytes, UNITS[unit_index])
    } else {
        format!("{:.2} {}", size, UNITS[unit_index])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_tape_capacity() {
        let capacity = TapeCapacity::new(1000, 300);
        assert_eq!(capacity.available_capacity, 700);
        assert_eq!(capacity.utilization_percentage(), 30.0);
        assert!(capacity.has_space_for(500));
        assert!(!capacity.has_space_for(800));
    }

    #[test]
    fn test_format_bytes() {
        assert_eq!(format_bytes(0), "0 B");
        assert_eq!(format_bytes(1024), "1.00 KB");
        assert_eq!(format_bytes(1048576), "1.00 MB");
        assert_eq!(format_bytes(1073741824), "1.00 GB");
    }
}