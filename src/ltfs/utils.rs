/// Utility functions for LTFS operations

/// Find the end of XML content in a byte array
pub fn find_xml_end(xml_content: &[u8]) -> Option<usize> {
    // Look for "</ltfsindex>" closing tag
    let end_tag = b"</ltfsindex>";
    
    // Search for the closing tag
    if let Some(pos) = xml_content.windows(end_tag.len()).position(|window| window == end_tag) {
        // Return position after the closing tag plus the ">" character
        Some(pos + end_tag.len())
    } else {
        None
    }
}

/// Get current timestamp in LTFS format
pub fn get_current_timestamp() -> String {
    use chrono::Utc;
    Utc::now().format("%Y-%m-%dT%H:%M:%S%.9fZ").to_string()
}

/// Index location information for LTFS operations
#[derive(Debug, Clone)]
pub struct IndexLocation {
    pub partition: u8,
    pub start_block: u64,
    pub xml_size: usize,
}

impl IndexLocation {
    pub fn new(partition: u8, start_block: u64, xml_size: usize) -> Self {
        Self {
            partition,
            start_block,
            xml_size,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_find_xml_end() {
        let xml_data = b"<?xml version=\"1.0\"?><ltfsindex>content</ltfsindex>";
        let end_pos = find_xml_end(xml_data);
        assert!(end_pos.is_some());
        assert_eq!(end_pos.unwrap(), xml_data.len());
    }

    #[test]
    fn test_find_xml_end_not_found() {
        let xml_data = b"<?xml version=\"1.0\"?><incomplete>";
        let end_pos = find_xml_end(xml_data);
        assert!(end_pos.is_none());
    }

    #[test]
    fn test_get_current_timestamp() {
        let timestamp = get_current_timestamp();
        assert!(timestamp.ends_with('Z'));
        assert!(timestamp.contains('T'));
    }
}