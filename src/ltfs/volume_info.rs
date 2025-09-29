/// LTFS volume information management

use crate::error::{Result, RustLtfsError};
use crate::ltfs_index::LtfsIndex;
use tracing::{debug, info};

/// LTFS volume information
#[derive(Debug, Clone)]
pub struct LtfsVolumeInfo {
    pub volume_name: String,
    pub format_time: String,
    pub generation: u32,
    pub block_size: u32,
}

impl LtfsVolumeInfo {
    /// Create new volume information
    pub fn new(volume_name: String, format_time: String, generation: u32, block_size: u32) -> Self {
        Self {
            volume_name,
            format_time,
            generation,
            block_size,
        }
    }

    /// Create volume info from LTFS index
    pub fn from_index(index: &LtfsIndex) -> Self {
        Self {
            volume_name: index.volumeuuid.clone(),
            format_time: index.updatetime.clone(),
            generation: index.generationnumber as u32,
            block_size: 65536, // Default LTO block size
        }
    }

    /// Get formatted volume information string
    pub fn format_info(&self) -> String {
        format!(
            "Volume: {}, Generation: {}, Format Time: {}, Block Size: {}",
            self.volume_name, self.generation, self.format_time, self.block_size
        )
    }

    /// Check if volume information is valid
    pub fn is_valid(&self) -> bool {
        !self.volume_name.is_empty() && 
        !self.format_time.is_empty() && 
        self.generation > 0 &&
        self.block_size > 0
    }

    /// Get volume age in days since format time
    pub fn age_in_days(&self) -> Result<i64> {
        use chrono::{DateTime, Utc};
        
        let format_time = DateTime::parse_from_rfc3339(&self.format_time)
            .map_err(|e| RustLtfsError::parse(format!("Invalid format time: {}", e)))?;
        
        let now = Utc::now();
        let duration = now.signed_duration_since(format_time.with_timezone(&Utc));
        
        Ok(duration.num_days())
    }

    /// Update generation number
    pub fn increment_generation(&mut self) {
        self.generation += 1;
        debug!("Volume generation incremented to {}", self.generation);
    }

    /// Update format time to current time
    pub fn update_format_time(&mut self) {
        self.format_time = super::utils::get_current_timestamp();
        info!("Volume format time updated to {}", self.format_time);
    }

    /// Validate volume name format
    pub fn validate_volume_name(&self) -> Result<()> {
        if self.volume_name.is_empty() {
            return Err(RustLtfsError::parameter_validation("Volume name cannot be empty".to_string()));
        }
        
        if self.volume_name.len() > 32 {
            return Err(RustLtfsError::parameter_validation("Volume name too long (max 32 characters)".to_string()));
        }
        
        // Check for invalid characters
        if self.volume_name.chars().any(|c| !c.is_ascii_alphanumeric() && c != '-' && c != '_') {
            return Err(RustLtfsError::parameter_validation("Volume name contains invalid characters".to_string()));
        }
        
        Ok(())
    }

    /// Export volume info as JSON string
    pub fn to_json(&self) -> Result<String> {
        let json = serde_json::json!({
            "volume_name": self.volume_name,
            "format_time": self.format_time,
            "generation": self.generation,
            "block_size": self.block_size
        });
        
        serde_json::to_string_pretty(&json)
            .map_err(|e| RustLtfsError::file_operation(format!("Failed to serialize volume info: {}", e)))
    }

    /// Import volume info from JSON string
    pub fn from_json(json_str: &str) -> Result<Self> {
        let json: serde_json::Value = serde_json::from_str(json_str)
            .map_err(|e| RustLtfsError::parse(format!("Failed to parse volume info JSON: {}", e)))?;
        
        let volume_name = json["volume_name"]
            .as_str()
            .ok_or_else(|| RustLtfsError::parse("Missing volume_name in JSON".to_string()))?
            .to_string();
        
        let format_time = json["format_time"]
            .as_str()
            .ok_or_else(|| RustLtfsError::parse("Missing format_time in JSON".to_string()))?
            .to_string();
        
        let generation = json["generation"]
            .as_u64()
            .ok_or_else(|| RustLtfsError::parse("Missing or invalid generation in JSON".to_string()))? as u32;
        
        let block_size = json["block_size"]
            .as_u64()
            .ok_or_else(|| RustLtfsError::parse("Missing or invalid block_size in JSON".to_string()))? as u32;
        
        let volume_info = Self::new(volume_name, format_time, generation, block_size);
        volume_info.validate_volume_name()?;
        
        Ok(volume_info)
    }
}

/// Volume information manager
pub struct VolumeInfoManager {
    current_info: Option<LtfsVolumeInfo>,
}

impl VolumeInfoManager {
    /// Create new volume info manager
    pub fn new() -> Self {
        Self {
            current_info: None,
        }
    }

    /// Set current volume information
    pub fn set_volume_info(&mut self, info: LtfsVolumeInfo) -> Result<()> {
        info.validate_volume_name()?;
        self.current_info = Some(info);
        Ok(())
    }

    /// Get current volume information
    pub fn get_volume_info(&self) -> Option<&LtfsVolumeInfo> {
        self.current_info.as_ref()
    }

    /// Update volume information from LTFS index
    pub fn update_from_index(&mut self, index: &LtfsIndex) {
        let info = LtfsVolumeInfo::from_index(index);
        self.current_info = Some(info);
        debug!("Volume info updated from LTFS index");
    }

    /// Clear current volume information
    pub fn clear(&mut self) {
        self.current_info = None;
        debug!("Volume info cleared");
    }

    /// Check if volume info is loaded
    pub fn is_loaded(&self) -> bool {
        self.current_info.is_some()
    }

    /// Get formatted volume information
    pub fn format_current_info(&self) -> Option<String> {
        self.current_info.as_ref().map(|info| info.format_info())
    }

    /// Increment generation of current volume
    pub fn increment_generation(&mut self) -> Result<()> {
        match self.current_info.as_mut() {
            Some(info) => {
                info.increment_generation();
                Ok(())
            }
            None => Err(RustLtfsError::ltfs_index("No volume info loaded".to_string()))
        }
    }

    /// Update format time of current volume
    pub fn update_format_time(&mut self) -> Result<()> {
        match self.current_info.as_mut() {
            Some(info) => {
                info.update_format_time();
                Ok(())
            }
            None => Err(RustLtfsError::ltfs_index("No volume info loaded".to_string()))
        }
    }
}

impl Default for VolumeInfoManager {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_volume_info_creation() {
        let info = LtfsVolumeInfo::new(
            "TestVolume".to_string(),
            "2023-01-01T00:00:00Z".to_string(),
            1,
            65536
        );
        
        assert_eq!(info.volume_name, "TestVolume");
        assert_eq!(info.generation, 1);
        assert!(info.is_valid());
    }

    #[test]
    fn test_volume_name_validation() {
        let mut info = LtfsVolumeInfo::new(
            "ValidName123".to_string(),
            "2023-01-01T00:00:00Z".to_string(),
            1,
            65536
        );
        
        assert!(info.validate_volume_name().is_ok());
        
        info.volume_name = "Invalid Name!@#".to_string();
        assert!(info.validate_volume_name().is_err());
        
        info.volume_name = "".to_string();
        assert!(info.validate_volume_name().is_err());
    }

    #[test]
    fn test_generation_increment() {
        let mut info = LtfsVolumeInfo::new(
            "TestVolume".to_string(),
            "2023-01-01T00:00:00Z".to_string(),
            1,
            65536
        );
        
        info.increment_generation();
        assert_eq!(info.generation, 2);
    }

    #[test]
    fn test_volume_info_manager() {
        let mut manager = VolumeInfoManager::new();
        assert!(!manager.is_loaded());
        
        let info = LtfsVolumeInfo::new(
            "TestVolume".to_string(),
            "2023-01-01T00:00:00Z".to_string(),
            1,
            65536
        );
        
        manager.set_volume_info(info).unwrap();
        assert!(manager.is_loaded());
        
        manager.increment_generation().unwrap();
        assert_eq!(manager.get_volume_info().unwrap().generation, 2);
        
        manager.clear();
        assert!(!manager.is_loaded());
    }
}