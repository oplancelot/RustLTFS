#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_xml_name_tag_fix() {
        // 创建一个简单的根目录
        let root_dir = Directory {
            name: "".to_string(), // 空字符串应该生成 <name></name> 而不是 <name/>
            fileuid: 1,
            creationtime: chrono::Utc::now(),
            changetime: chrono::Utc::now(),
            modifytime: chrono::Utc::now(),
            accesstime: chrono::Utc::now(),
            backuptime: chrono::Utc::now(),
            readonly: false,
            contents: DirectoryContents {
                files: vec![],
                directories: vec![],
            },
        };

        // 创建索引
        let index = LtfsIndex {
            version: "2.4.0".to_string(),
            creator: "RustLTFS Test".to_string(),
            volumeuuid: "test-uuid".to_string(),
            generationnumber: 1,
            updatetime: chrono::Utc::now(),
            location: Location {
                partition: "0".to_string(),
                startblock: 5,
            },
            previousgenerationlocation: None,
            allowpolicyupdate: false,
            volumelockstate: VolumeLockState::Unlocked,
            highestfileuid: 1,
            root_directory: root_dir,
        };

        // 生成XML
        let xml_result = index.to_xml();
        assert!(xml_result.is_ok(), "XML生成失败: {:?}", xml_result);
        
        let xml = xml_result.unwrap();
        
        // 检查修复效果
        assert!(!xml.contains("<name/>"), "❌ 修复失败：XML仍包含 <name/>");
        assert!(xml.contains("<name></name>"), "❌ 修复不完整：XML不包含 <name></name>");
        
        println!("✅ XML name标签修复测试通过");
        println!("XML preview:\n{}", &xml[..std::cmp::min(500, xml.len())]);
    }
}