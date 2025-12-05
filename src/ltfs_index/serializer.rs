//! LTFS Index XML Serializer
//!
//! This module handles serializing LTFS index to XML format.

use crate::error::Result;
use super::types::LtfsIndex;
use tracing::debug;

impl LtfsIndex {
    /// Serialize LTFS index to XML string
    pub fn to_xml(&self) -> Result<String> {
        let xml_string = quick_xml::se::to_string(self).map_err(|e| {
            crate::error::RustLtfsError::file_operation(format!(
                "Failed to serialize LTFS index to XML: {}",
                e
            ))
        })?;

        // Add XML declaration
        let complete_xml = format!("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n{}", xml_string);

        debug!(
            "Serialized LTFS index to XML ({} bytes)",
            complete_xml.len()
        );
        Ok(complete_xml)
    }
}
