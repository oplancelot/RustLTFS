//! LTFS Index XML Parser
//!
//! This module handles parsing LTFS index from XML format.

use crate::error::Result;
use super::types::*;
use tracing::{debug, info, warn};

impl LtfsIndex {
    /// Parse LTFS index from XML content with enhanced error handling
    pub fn from_xml(xml_content: &str) -> Result<Self> {
        debug!("Parsing LTFS index XML, length: {}", xml_content.len());

        // 首先提取纯LTFS索引部分（跳过可能的ltfslabel部分）
        let index_xml = Self::extract_ltfs_index_section(xml_content)?;
        
        Self::validate_xml_structure(&index_xml)?;

        // 添加XML结构调试信息
        if tracing::enabled!(tracing::Level::DEBUG) {
            Self::debug_xml_structure(&index_xml);
        }

        let index: LtfsIndex = quick_xml::de::from_str(&index_xml).map_err(|e| {
            // 添加详细的解析错误信息
            let error_msg = format!(
                "Failed to parse LTFS index XML: {} (XML size: {} bytes)",
                e,
                index_xml.len()
            );
            
            // 输出XML的前1000个字符用于调试
            let preview = &index_xml[..std::cmp::min(1000, index_xml.len())];
            warn!("XML parsing failed. Content preview:\n{}", preview);
            
            crate::error::RustLtfsError::parse(error_msg)
        })?;

        // Post-validation of parsed index
        Self::validate_parsed_index(&index)?;

        info!(
            "Successfully parsed LTFS index, version: {}, generation: {}, files: {}",
            index.version,
            index.generationnumber,
            Self::count_files_in_index(&index)
        );

        Ok(index)
    }



    /// Extract LTFS index section from combined XML content 
    /// (separates ltfsindex from ltfslabel if both are present)
    fn extract_ltfs_index_section(xml_content: &str) -> Result<String> {
        debug!("Extracting LTFS index section from XML content");
        
        // 查找ltfsindex标签的开始和结束位置
        if let Some(index_start) = xml_content.find("<ltfsindex") {
            if let Some(index_end) = xml_content.find("</ltfsindex>") {
                let start_pos = index_start;
                let end_pos = index_end + "</ltfsindex>".len();
                
                // 提取ltfsindex部分
                let mut index_section = xml_content[start_pos..end_pos].to_string();
                
                // 确保有XML声明
                if !index_section.trim_start().starts_with("<?xml") {
                    index_section = format!("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n{}", index_section);
                }
                
                debug!(
                    "Successfully extracted LTFS index section: {} bytes (from {} bytes total)",
                    index_section.len(),
                    xml_content.len()
                );
                
                return Ok(index_section);
            }
        }
        
        // 如果没有找到ltfsindex标签，可能整个内容就是索引
        if xml_content.contains("<ltfsindex") {
            debug!("XML content appears to be pure LTFS index format");
            return Ok(xml_content.to_string());
        }
        
        Err(crate::error::RustLtfsError::parse(
            "No LTFS index section found in XML content".to_string()
        ))
    }

    /// Debug XML structure to understand format issues
    fn debug_xml_structure(xml_content: &str) {
        debug!("Analyzing LTFS XML structure for compatibility debugging");
        
        // 查找location标签
        if let Some(location_start) = xml_content.find("<location") {
            let location_end = xml_content[location_start..]
                .find("</location>")
                .unwrap_or(200);
            let location_section = &xml_content[location_start..location_start + location_end + 11];
            debug!("Found location section: {}", location_section);
        }
        
        // 查找extent标签
        if let Some(extent_start) = xml_content.find("<extent") {
            let extent_end = xml_content[extent_start..]
                .find("/>")
                .or_else(|| xml_content[extent_start..].find("</extent>"))
                .unwrap_or(200);
            let extent_section = &xml_content[extent_start..extent_start + extent_end + 2];
            debug!("Found extent section: {}", extent_section);
        }
        
        // 查找所有可能的startblock字段变体
        let startblock_variants = ["startblock", "startBlock", "start_block", "block"];
        for variant in &startblock_variants {
            if xml_content.contains(variant) {
                debug!("Found field variant '{}' in XML", variant);
            }
        }
    }
}
