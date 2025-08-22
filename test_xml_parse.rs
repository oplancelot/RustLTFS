use quick_xml::de::from_str;
use serde::{Deserialize, Serialize};

// 简化的测试结构
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename = "ltfsindex")]
pub struct TestLtfsIndex {
    #[serde(rename = "@version")]
    pub version: String,
    pub creator: String,
    pub volumeuuid: String,
    pub generationnumber: u64,
    pub updatetime: String,
    pub location: TestLocation,
    #[serde(default)]
    pub previousgenerationlocation: Option<TestLocation>,
    #[serde(default)]
    pub allowpolicyupdate: Option<bool>,
    #[serde(default)]
    pub volumelockstate: Option<String>,
    #[serde(default)]
    pub highestfileuid: Option<u64>,
    #[serde(rename = "directory")]
    pub root_directory: TestDirectory,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TestLocation {
    pub partition: String,
    pub startblock: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TestDirectory {
    pub name: String,
    #[serde(rename = "fileuid")]
    pub uid: u64,
    #[serde(rename = "readonly")]
    pub read_only: bool,
    #[serde(rename = "creationtime")]
    pub creation_time: String,
    #[serde(rename = "changetime")]
    pub change_time: String,
    #[serde(rename = "modifytime")]
    pub modify_time: String,
    #[serde(rename = "accesstime")]
    pub access_time: String,
    #[serde(rename = "backuptime")]
    pub backup_time: String,
    #[serde(default)]
    pub contents: TestDirectoryContents,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct TestDirectoryContents {
    #[serde(rename = "directory", default)]
    pub directories: Vec<TestDirectory>,
    #[serde(rename = "file", default)]
    pub files: Vec<TestFile>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TestFile {
    pub name: String,
    #[serde(rename = "fileuid")]
    pub uid: u64,
    pub length: u64,
    #[serde(rename = "readonly")]
    pub read_only: bool,
    #[serde(default)]
    pub openforwrite: Option<bool>,
    #[serde(rename = "creationtime")]
    pub creation_time: String,
    #[serde(rename = "changetime")]
    pub change_time: String,
    #[serde(rename = "modifytime")]
    pub modify_time: String,
    #[serde(rename = "accesstime")]
    pub access_time: String,
    #[serde(rename = "backuptime")]
    pub backup_time: String,
    #[serde(rename = "extentinfo", default)]
    pub extents: Vec<TestFileExtent>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TestFileExtent {
    #[serde(rename = "extent")]
    pub extent: TestExtent,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TestExtent {
    pub partition: String,
    #[serde(rename = "startblock")]
    pub start_block: u64,
    #[serde(rename = "bytecount")]
    pub byte_count: u64,
    #[serde(rename = "fileoffset")]
    pub file_offset: u64,
    #[serde(rename = "byteoffset")]
    pub byte_offset: u64,
}

fn main() {
    // 读取实际的 XML 文件
    let xml_content = std::fs::read_to_string("/home/ubuntu/RustLTFS/index/LTFSIndex_5.schema")
        .expect("Failed to read XML file");
    
    // 尝试解析
    match from_str::<TestLtfsIndex>(&xml_content) {
        Ok(index) => {
            println!("✅ XML 解析成功！");
            println!("版本: {}", index.version);
            println!("创建者: {}", index.creator);
            println!("卷UUID: {}", index.volumeuuid);
            println!("生成编号: {}", index.generationnumber);
            println!("根目录名称: '{}'", index.root_directory.name);
            println!("根目录UID: {}", index.root_directory.uid);
            println!("子目录数量: {}", index.root_directory.contents.directories.len());
            println!("文件数量: {}", index.root_directory.contents.files.len());
            
            if !index.root_directory.contents.directories.is_empty() {
                let first_dir = &index.root_directory.contents.directories[0];
                println!("第一个子目录: {}", first_dir.name);
                println!("  子文件数量: {}", first_dir.contents.files.len());
            }
        },
        Err(e) => {
            println!("❌ XML 解析失败: {}", e);
            println!("错误详情: {:?}", e);
        }
    }
}