#[cfg(test)]
mod xml_parse_tests {
    use crate::ltfs_index::*;
    use quick_xml::de::from_str;

    #[test]
    fn test_parse_simple_extent() {
        let extent_xml = r#"<extent>
<fileoffset>0</fileoffset>
<partition>b</partition>
<startblock>7</startblock>
<byteoffset>0</byteoffset>
<bytecount>15491</bytecount>
</extent>"#;
        
        match from_str::<FileExtent>(extent_xml) {
            Ok(extent) => {
                println!("✅ FileExtent 解析成功！");
                println!("partition: {}", extent.partition);
                println!("startblock: {}", extent.start_block);
                println!("bytecount: {}", extent.byte_count);
            },
            Err(e) => {
                println!("❌ FileExtent 解析失败: {}", e);
                panic!("FileExtent parsing failed: {}", e);
            }
        }
    }

    #[test]
    fn test_parse_simple_directory() {
        let dir_xml = r#"<directory>
<name>drivers</name>
<readonly>false</readonly>
<creationtime>2025-08-14T04:33:40.191740900Z</creationtime>
<changetime>2025-08-14T04:33:40.660477700Z</changetime>
<modifytime>2025-08-14T04:33:40.660477700Z</modifytime>
<accesstime>2025-08-14T04:33:40.660477700Z</accesstime>
<backuptime>2025-08-22T03:51:43.892812500Z</backuptime>
<fileuid>2</fileuid>
<contents>
</contents>
</directory>"#;
        
        match from_str::<Directory>(dir_xml) {
            Ok(dir) => {
                println!("✅ Directory 解析成功！");
                println!("name: {}", dir.name);
                println!("uid: {}", dir.uid);
                println!("files count: {}", dir.contents.files.len());
            },
            Err(e) => {
                println!("❌ Directory 解析失败: {}", e);
                panic!("Directory parsing failed: {}", e);
            }
        }
    }

    #[test]
    fn test_parse_file_without_extents() {
        let file_xml = r#"<file>
<name>test.txt</name>
<length>1024</length>
<readonly>false</readonly>
<openforwrite>false</openforwrite>
<creationtime>2017-11-07T03:48:12.000000000Z</creationtime>
<changetime>2017-11-07T03:48:12.000000000Z</changetime>
<modifytime>2017-11-07T03:48:12.000000000Z</modifytime>
<accesstime>2025-08-14T04:33:40.191740900Z</accesstime>
<backuptime>2025-08-22T03:52:06.585070800Z</backuptime>
<fileuid>5</fileuid>
</file>"#;
        
        println!("测试不含 extents 的文件结构:");
        match from_str::<File>(file_xml) {
            Ok(file) => {
                println!("✅ 不含 extents 的文件结构解析成功！");
                println!("name: {}", file.name);
                println!("length: {}", file.length);
                println!("uid: {}", file.uid);
                println!("openforwrite: {}", file.openforwrite);
            },
            Err(e) => {
                println!("❌ 不含 extents 的文件结构解析失败: {}", e);
                panic!("File without extents parsing failed: {}", e);
            }
        }
    }

    #[test]
    fn test_complete_ltfs_index_with_extents() {
        let full_ltfs_xml = r#"<?xml version="1.0" encoding="UTF-8"?>
<ltfsindex version="2.4">
    <creator>IBM LTFS Windows 4.1.1.0</creator>
    <volumeuuid>c19a3c45-3244-4f9e-b00a-3b5e3f5e5f5a</volumeuuid>
    <generationnumber>1</generationnumber>
    <updatetime>2025-08-14T04:33:40.000000000Z</updatetime>
    <location>
        <partition>a</partition>
        <startblock>3</startblock>
    </location>
    <directory>
        <name></name>
        <readonly>false</readonly>
        <creationtime>2025-08-14T04:33:40.000000000Z</creationtime>
        <changetime>2025-08-14T04:33:40.000000000Z</changetime>
        <modifytime>2025-08-14T04:33:40.000000000Z</modifytime>
        <accesstime>2025-08-14T04:33:40.000000000Z</accesstime>
        <backuptime>2025-08-22T03:52:06.000000000Z</backuptime>
        <fileuid>1</fileuid>
        <contents>
            <file>
                <name>test_file.txt</name>
                <length>15491</length>
                <readonly>false</readonly>
                <openforwrite>false</openforwrite>
                <creationtime>2017-11-07T03:48:12.000000000Z</creationtime>
                <changetime>2017-11-07T03:48:12.000000000Z</changetime>
                <modifytime>2017-11-07T03:48:12.000000000Z</modifytime>
                <accesstime>2025-08-14T04:33:40.000000000Z</accesstime>
                <backuptime>2025-08-22T03:52:06.000000000Z</backuptime>
                <fileuid>5</fileuid>
                <extentinfo>
                    <extent>
                        <fileoffset>0</fileoffset>
                        <partition>b</partition>
                        <startblock>7</startblock>
                        <byteoffset>0</byteoffset>
                        <bytecount>15491</bytecount>
                    </extent>
                </extentinfo>
            </file>
            <file>
                <name>another_file.dat</name>
                <length>2048</length>
                <readonly>true</readonly>
                <openforwrite>false</openforwrite>
                <creationtime>2025-01-01T00:00:00.000000000Z</creationtime>
                <changetime>2025-01-01T00:00:00.000000000Z</changetime>
                <modifytime>2025-01-01T00:00:00.000000000Z</modifytime>
                <accesstime>2025-08-14T04:33:40.000000000Z</accesstime>
                <backuptime>2025-08-22T03:52:06.000000000Z</backuptime>
                <fileuid>6</fileuid>
                <extentinfo>
                    <extent>
                        <fileoffset>0</fileoffset>
                        <partition>b</partition>
                        <startblock>10</startblock>
                        <byteoffset>0</byteoffset>
                        <bytecount>2048</bytecount>
                    </extent>
                </extentinfo>
            </file>
        </contents>
    </directory>
</ltfsindex>"#;

        println!("测试完整的LTFS索引XML解析...");
        
        match LtfsIndex::from_xml(full_ltfs_xml) {
            Ok(index) => {
                println!("✅ 完整LTFS索引XML解析成功！");
                
                // 验证基本信息
                assert_eq!(index.version, "2.4");
                assert_eq!(index.creator, "IBM LTFS Windows 4.1.1.0");
                assert_eq!(index.generationnumber, 1);
                
                // 验证根目录
                assert_eq!(index.root_directory.name, "");
                assert_eq!(index.root_directory.uid, 1);
                assert!(!index.root_directory.read_only);
                
                // 验证文件数量
                assert_eq!(index.root_directory.contents.files.len(), 2);
                
                // 验证第一个文件
                let file1 = &index.root_directory.contents.files[0];
                assert_eq!(file1.name, "test_file.txt");
                assert_eq!(file1.length, 15491);
                assert_eq!(file1.uid, 5);
                assert!(!file1.read_only);
                assert!(!file1.openforwrite);
                assert_eq!(file1.extent_info.extents.len(), 1);
                
                let extent1 = &file1.extent_info.extents[0];
                assert_eq!(extent1.partition, "b");
                assert_eq!(extent1.start_block, 7);
                assert_eq!(extent1.byte_count, 15491);
                assert_eq!(extent1.file_offset, 0);
                assert_eq!(extent1.byte_offset, 0);
                
                // 验证第二个文件
                let file2 = &index.root_directory.contents.files[1];
                assert_eq!(file2.name, "another_file.dat");
                assert_eq!(file2.length, 2048);
                assert_eq!(file2.uid, 6);
                assert!(file2.read_only);  // 这个文件是只读的
                assert!(!file2.openforwrite);
                assert_eq!(file2.extent_info.extents.len(), 1);
                
                let extent2 = &file2.extent_info.extents[0];
                assert_eq!(extent2.partition, "b");
                assert_eq!(extent2.start_block, 10);
                assert_eq!(extent2.byte_count, 2048);
                
                println!("🎉 所有验证都通过！XML解析修复成功！");
                
                // 测试方法是否工作正常
                assert!(file1.has_extents());
                assert!(file2.has_extents());
                
                let sorted_extents1 = file1.get_sorted_extents();
                assert_eq!(sorted_extents1.len(), 1);
                
                println!("✅ 所有方法也正常工作！");
            }
            Err(e) => {
                panic!("❌ 完整LTFS索引XML解析失败: {}", e);
            }
        }
    }
}