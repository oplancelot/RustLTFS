//! LTFS Index Type Definitions
//!
//! This module contains all data structure definitions for LTFS indexes.

use serde::{Deserialize, Serialize};

/// LTFS Index structure based on LTFS specification
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename = "ltfsindex")]
pub struct LtfsIndex {
    #[serde(rename = "@version")]
    pub version: String,
    pub creator: String,
    pub volumeuuid: String,
    pub generationnumber: u64,
    pub updatetime: String,
    pub location: Location,
    #[serde(default)]
    pub previousgenerationlocation: Option<Location>,
    #[serde(default)]
    pub allowpolicyupdate: Option<bool>,
    #[serde(default)]
    pub volumelockstate: Option<String>,
    #[serde(default)]
    pub highestfileuid: Option<u64>,
    #[serde(rename = "directory")]
    pub root_directory: Directory,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Location {
    pub partition: String,
    #[serde(alias = "startBlock", alias = "start_block", alias = "block")]
    pub startblock: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Directory {
    pub name: String,
    #[serde(rename = "fileuid")]
    pub uid: u64,
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
    #[serde(rename = "readonly")]
    pub read_only: bool,
    #[serde(rename = "contents")]
    pub contents: DirectoryContents,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct DirectoryContents {
    #[serde(rename = "directory", default)]
    pub directories: Vec<Directory>,
    #[serde(rename = "file", default)]
    pub files: Vec<File>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ExtentInfo {
    #[serde(rename = "extent", default)]
    pub extents: Vec<FileExtent>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct File {
    pub name: String,
    #[serde(rename = "fileuid")]
    pub uid: u64,
    pub length: u64,
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
    #[serde(rename = "readonly")]
    pub read_only: bool,
    pub openforwrite: bool,
    #[serde(rename = "symlink", default)]
    pub symlink: Option<String>,
    #[serde(rename = "extentinfo", default)]
    pub extent_info: ExtentInfo,
    #[serde(rename = "extendedattributes", default)]
    pub extended_attributes: Option<ExtendedAttributes>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileExtent {
    pub partition: String,
    #[serde(rename = "startblock", alias = "startBlock", alias = "start_block", alias = "block")]
    pub start_block: u64,
    #[serde(rename = "bytecount")]
    pub byte_count: u64,
    #[serde(rename = "fileoffset")]
    pub file_offset: u64,
    #[serde(rename = "byteoffset")]
    pub byte_offset: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExtendedAttributes {
    #[serde(rename = "xattr", default)]
    pub attributes: Vec<ExtendedAttribute>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExtendedAttribute {
    pub key: String,
    #[serde(rename = "value")]
    pub value: String,
}
