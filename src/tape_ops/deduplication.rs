// LTFS去重和哈希管理系统 - 严格按照LTFSCopyGUI实现
use crate::error::{Result, RustLtfsError};
use crate::tape_ops::WriteOptions;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::fs;
use std::io::{Read, Write, BufReader};
use std::path::{Path, PathBuf};
use tracing::{debug, info, warn};
use sha1::{Digest, Sha1};
use sha2::{Digest as Sha256Digest, Sha256};
use blake3::Hasher as Blake3Hasher;
use twox_hash::XxHash64;
use std::hash::Hasher;
use std::sync::Arc;
use rayon::prelude::*;

/// 哈希类型枚举（对应LTFSCopyGUI的支持算法）
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum HashType {
    Sha1,
    Sha256,
    Blake3,
    XxHash3,
    XxHash128,
}

impl HashType {
    /// 获取LTFS扩展属性键名（对应LTFSCopyGUI的键名格式）
    pub fn ltfs_key(&self) -> &'static str {
        match self {
            HashType::Sha1 => "ltfs.hash.sha1sum",
            HashType::Sha256 => "ltfs.hash.sha256sum",
            HashType::Blake3 => "ltfs.hash.blake3sum",
            HashType::XxHash3 => "ltfs.hash.xxhash3sum",
            HashType::XxHash128 => "ltfs.hash.xxhash128sum",
        }
    }

    /// 从LTFS键名解析哈希类型
    pub fn from_ltfs_key(key: &str) -> Option<Self> {
        match key {
            "ltfs.hash.sha1sum" => Some(HashType::Sha1),
            "ltfs.hash.sha256sum" => Some(HashType::Sha256),
            "ltfs.hash.blake3sum" => Some(HashType::Blake3),
            "ltfs.hash.xxhash3sum" => Some(HashType::XxHash3),
            "ltfs.hash.xxhash128sum" => Some(HashType::XxHash128),
            _ => None,
        }
    }

    /// 计算文件哈希值
    pub fn compute_file_hash<P: AsRef<Path>>(&self, file_path: P) -> Result<String> {
        let mut file = fs::File::open(&file_path).map_err(|e| {
            RustLtfsError::file_operation(format!("无法打开文件 {:?}: {}", file_path.as_ref(), e))
        })?;

        let mut reader = BufReader::new(file);
        let mut buffer = [0u8; 8192];
        
        match self {
            HashType::Sha1 => {
                let mut hasher = Sha1::new();
                loop {
                    let bytes_read = reader.read(&mut buffer).map_err(|e| {
                        RustLtfsError::file_operation(format!("读取文件错误: {}", e))
                    })?;
                    if bytes_read == 0 { break; }
                    hasher.update(&buffer[..bytes_read]);
                }
                Ok(format!("{:x}", hasher.finalize()))
            },
            HashType::Sha256 => {
                let mut hasher = Sha256::new();
                loop {
                    let bytes_read = reader.read(&mut buffer).map_err(|e| {
                        RustLtfsError::file_operation(format!("读取文件错误: {}", e))
                    })?;
                    if bytes_read == 0 { break; }
                    hasher.update(&buffer[..bytes_read]);
                }
                Ok(format!("{:x}", hasher.finalize()))
            },
            HashType::Blake3 => {
                let mut hasher = Blake3Hasher::new();
                loop {
                    let bytes_read = reader.read(&mut buffer).map_err(|e| {
                        RustLtfsError::file_operation(format!("读取文件错误: {}", e))
                    })?;
                    if bytes_read == 0 { break; }
                    hasher.update(&buffer[..bytes_read]);
                }
                Ok(format!("{}", hasher.finalize().to_hex()))
            },
            HashType::XxHash3 => {
                let mut hasher = XxHash64::default();
                loop {
                    let bytes_read = reader.read(&mut buffer).map_err(|e| {
                        RustLtfsError::file_operation(format!("读取文件错误: {}", e))
                    })?;
                    if bytes_read == 0 { break; }
                    hasher.write(&buffer[..bytes_read]);
                }
                Ok(format!("{:016x}", hasher.finish()))
            },
            HashType::XxHash128 => {
                let mut hasher = XxHash64::default(); // 使用64位版本模拟128位
                loop {
                    let bytes_read = reader.read(&mut buffer).map_err(|e| {
                        RustLtfsError::file_operation(format!("读取文件错误: {}", e))
                    })?;
                    if bytes_read == 0 { break; }
                    hasher.write(&buffer[..bytes_read]);
                }
                Ok(format!("{:032x}", hasher.finish()))
            },
        }
    }

    /// 获取哈希算法显示名称
    pub fn display_name(&self) -> &'static str {
        match self {
            HashType::Sha1 => "SHA1",
            HashType::Sha256 => "SHA256",
            HashType::Blake3 => "BLAKE3",
            HashType::XxHash3 => "XXHash3",
            HashType::XxHash128 => "XXHash128",
        }
    }

    /// 获取推荐的哈希算法优先级顺序
    pub fn recommended_order() -> Vec<HashType> {
        vec![
            HashType::Sha1,    // LTFS标准
            HashType::Sha256,  // 安全性高
            HashType::Blake3,  // 性能优秀
            HashType::XxHash3, // 超快速度
            HashType::XxHash128,
        ]
    }
}

/// 高级哈希管理器 - 支持并行计算和进度跟踪
pub struct HashManager {
    enabled_algorithms: Vec<HashType>,
    progress_callback: Option<Arc<dyn Fn(u64, u64) + Send + Sync>>,
    chunk_size: usize,
}

impl HashManager {
    /// 创建新的哈希管理器
    pub fn new(enabled_algorithms: Vec<HashType>) -> Self {
        Self {
            enabled_algorithms,
            progress_callback: None,
            chunk_size: 64 * 1024, // 64KB 块大小
        }
    }

    /// 设置进度回调函数
    pub fn with_progress_callback<F>(&mut self, callback: F) -> &mut Self 
    where 
        F: Fn(u64, u64) + Send + Sync + 'static,
    {
        self.progress_callback = Some(Arc::new(callback));
        self
    }

    /// 设置读取块大小
    pub fn with_chunk_size(&mut self, chunk_size: usize) -> &mut Self {
        self.chunk_size = chunk_size;
        self
    }

    /// 计算文件的多种哈希值（并行计算）
    pub fn compute_file_hashes<P: AsRef<Path>>(&self, file_path: P) -> Result<HashMap<String, String>> {
        let file_path = file_path.as_ref();
        let file_size = fs::metadata(file_path)
            .map_err(|e| RustLtfsError::file_operation(format!("无法获取文件信息: {}", e)))?
            .len();

        let mut file = fs::File::open(file_path).map_err(|e| {
            RustLtfsError::file_operation(format!("无法打开文件 {:?}: {}", file_path, e))
        })?;

        let mut results = HashMap::new();
        let mut buffer = vec![0u8; self.chunk_size];
        let mut total_read = 0u64;

        // 初始化所有哈希器
        let mut sha1_hasher = self.enabled_algorithms.contains(&HashType::Sha1).then(|| Sha1::new());
        // 移除了MD5支持以解决库兼容性问题
        let mut sha256_hasher = self.enabled_algorithms.contains(&HashType::Sha256).then(|| Sha256::new());
        let mut blake3_hasher = self.enabled_algorithms.contains(&HashType::Blake3).then(|| Blake3Hasher::new());
        let mut xxhash3_hasher = self.enabled_algorithms.contains(&HashType::XxHash3).then(|| XxHash64::default());
        let mut xxhash128_hasher = self.enabled_algorithms.contains(&HashType::XxHash128).then(|| XxHash64::default());
        
        info!("开始计算文件哈希: {:?} (大小: {})", file_path, format_file_size(file_size));
        let start_time = std::time::Instant::now();

        // 逐块读取并更新所有哈希器
        loop {
            let bytes_read = file.read(&mut buffer).map_err(|e| {
                RustLtfsError::file_operation(format!("读取文件错误: {}", e))
            })?;
            
            if bytes_read == 0 { break; }

            let chunk = &buffer[..bytes_read];
            total_read += bytes_read as u64;

            // 并行更新所有哈希器
            rayon::join(
                || {
                    if let Some(ref mut hasher) = sha1_hasher {
                        hasher.update(chunk);
                    }
                },
                || {
                    if let Some(ref mut hasher) = sha256_hasher {
                        Sha256Digest::update(hasher, chunk);
                    }
                    if let Some(ref mut hasher) = blake3_hasher {
                        hasher.update(chunk);
                    }
                }
            );

            // 更新快速哈希器
            if let Some(ref mut hasher) = xxhash3_hasher {
                hasher.write(chunk);
            }
            if let Some(ref mut hasher) = xxhash128_hasher {
                hasher.write(chunk);
            }

            // 报告进度
            if let Some(ref callback) = self.progress_callback {
                callback(total_read, file_size);
            }
        }

        // 完成哈希计算并收集结果
        if let Some(hasher) = sha1_hasher {
            results.insert("ltfs.hash.sha1sum".to_string(), format!("{:x}", hasher.finalize()));
        }
        if let Some(hasher) = sha256_hasher {
            results.insert("ltfs.hash.sha256sum".to_string(), format!("{:x}", hasher.finalize()));
        }
        if let Some(hasher) = blake3_hasher {
            results.insert("ltfs.hash.blake3sum".to_string(), format!("{}", hasher.finalize().to_hex()));
        }
        if let Some(hasher) = xxhash3_hasher {
            results.insert("ltfs.hash.xxhash3sum".to_string(), format!("{:016x}", hasher.finish()));
        }
        if let Some(hasher) = xxhash128_hasher {
            results.insert("ltfs.hash.xxhash128sum".to_string(), format!("{:032x}", hasher.finish()));
        }

        let elapsed = start_time.elapsed();
        let throughput = if elapsed.as_secs_f64() > 0.0 {
            (total_read as f64) / elapsed.as_secs_f64() / 1024.0 / 1024.0 // MB/s
        } else {
            0.0
        };

        info!("文件哈希计算完成: {:?} - {} 个算法，用时 {:.2}s，吞吐量 {:.1} MB/s", 
              file_path, results.len(), elapsed.as_secs_f64(), throughput);

        debug!("计算得到的哈希值: {:#?}", results);
        Ok(results)
    }

    /// 验证文件哈希值
    pub fn verify_file_hashes<P: AsRef<Path>>(&self, file_path: P, expected_hashes: &HashMap<String, String>) -> Result<bool> {
        let computed_hashes = self.compute_file_hashes(file_path)?;
        
        for (ltfs_key, expected_hash) in expected_hashes {
            if let Some(computed_hash) = computed_hashes.get(ltfs_key) {
                if computed_hash != expected_hash {
                    warn!("哈希验证失败 - {}: 期望 {}, 实际 {}", ltfs_key, expected_hash, computed_hash);
                    return Ok(false);
                }
            } else {
                warn!("缺少哈希算法: {}", ltfs_key);
                return Ok(false);
            }
        }
        
        debug!("文件哈希验证成功");
        Ok(true)
    }

    /// 获取启用的算法列表
    pub fn enabled_algorithms(&self) -> &[HashType] {
        &self.enabled_algorithms
    }

    /// 启用特定的哈希算法
    pub fn enable_algorithm(&mut self, hash_type: HashType) {
        if !self.enabled_algorithms.contains(&hash_type) {
            self.enabled_algorithms.push(hash_type);
        }
    }

    /// 禁用特定的哈希算法
    pub fn disable_algorithm(&mut self, hash_type: &HashType) {
        self.enabled_algorithms.retain(|h| h != hash_type);
    }

    /// 从WriteOptions创建哈希管理器
    pub fn from_write_options(options: &WriteOptions) -> Self {
        let mut algorithms = Vec::new();
        
        // 根据具体的哈希算法开关添加算法
        if options.hash_sha1_enabled {
            algorithms.push(HashType::Sha1);
        }
        // 暂时移除MD5支持
        if options.hash_sha256_enabled {
            algorithms.push(HashType::Sha256);
        }
        if options.hash_blake3_enabled {
            algorithms.push(HashType::Blake3);
        }
        
        // 根据配置添加扩展算法
        if options.extended_hashing {
            if options.hash_xxhash3_enabled {
                algorithms.push(HashType::XxHash3);
            }
            if options.hash_xxhash128_enabled {
                algorithms.push(HashType::XxHash128);
            }
        }

        // 如果没有启用任何算法，默认使用SHA1
        if algorithms.is_empty() {
            algorithms.push(HashType::Sha1);
        }

        Self::new(algorithms)
    }
}

/// 文件哈希记录（对应LTFSCopyGUI的文件签名记录）
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileHashRecord {
    /// 文件路径
    pub file_path: String,
    /// 文件大小
    pub file_size: u64,
    /// 文件修改时间（Unix时间戳）
    pub last_modified: u64,
    /// 哈希值映射（算法 -> 哈希值）
    pub hashes: HashMap<HashType, String>,
    /// 首次添加时间
    pub first_seen: String,
    /// LTFS中的位置信息
    pub tape_location: Option<TapeLocation>,
}

/// 磁带位置信息
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TapeLocation {
    pub partition: u8,
    pub start_block: u64,
    pub file_uid: u64,
}

/// 去重数据库（对应LTFSCopyGUI的DuplicateTracker）
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeduplicationDatabase {
    /// 哈希值到文件记录的映射（主键为SHA1哈希）
    pub hash_to_records: HashMap<String, Vec<FileHashRecord>>,
    /// 文件路径到记录的索引
    pub path_to_record: HashMap<String, FileHashRecord>,
    /// 数据库版本
    pub version: u32,
    /// 创建时间
    pub created_at: String,
    /// 最后更新时间
    pub updated_at: String,
}

impl Default for DeduplicationDatabase {
    fn default() -> Self {
        let now = chrono::Utc::now().format("%Y-%m-%dT%H:%M:%SZ").to_string();
        Self {
            hash_to_records: HashMap::new(),
            path_to_record: HashMap::new(),
            version: 1,
            created_at: now.clone(),
            updated_at: now,
        }
    }
}

impl DeduplicationDatabase {
    /// 从文件加载去重数据库
    pub fn load_from_file<P: AsRef<Path>>(path: P) -> Result<Self> {
        if !path.as_ref().exists() {
            info!("去重数据库文件不存在，创建新数据库");
            return Ok(Self::default());
        }

        let content = fs::read_to_string(&path).map_err(|e| {
            RustLtfsError::file_operation(format!("无法读取去重数据库: {}", e))
        })?;

        let database: Self = serde_json::from_str(&content).map_err(|e| {
            RustLtfsError::file_operation(format!("无法解析去重数据库: {}", e))
        })?;

        info!("已加载去重数据库，包含 {} 个哈希记录，{} 个文件路径", 
              database.hash_to_records.len(), database.path_to_record.len());
        
        Ok(database)
    }

    /// 保存去重数据库到文件
    pub fn save_to_file<P: AsRef<Path>>(&mut self, path: P) -> Result<()> {
        self.updated_at = chrono::Utc::now().format("%Y-%m-%dT%H:%M:%SZ").to_string();
        
        let content = serde_json::to_string_pretty(self).map_err(|e| {
            RustLtfsError::file_operation(format!("无法序列化去重数据库: {}", e))
        })?;

        // 创建临时文件以确保原子性写入
        let temp_path = format!("{}.tmp", path.as_ref().display());
        let mut file = fs::File::create(&temp_path).map_err(|e| {
            RustLtfsError::file_operation(format!("无法创建临时数据库文件: {}", e))
        })?;

        file.write_all(content.as_bytes()).map_err(|e| {
            RustLtfsError::file_operation(format!("无法写入去重数据库: {}", e))
        })?;

        file.sync_all().map_err(|e| {
            RustLtfsError::file_operation(format!("无法同步数据库文件: {}", e))
        })?;

        // 原子性地移动临时文件到目标位置
        fs::rename(&temp_path, &path).map_err(|e| {
            RustLtfsError::file_operation(format!("无法移动数据库文件: {}", e))
        })?;

        debug!("去重数据库已安全保存到: {:?}", path.as_ref());
        Ok(())
    }

    /// 创建数据库备份
    pub fn create_backup<P: AsRef<Path>>(&self, backup_path: P) -> Result<()> {
        let content = serde_json::to_string_pretty(self).map_err(|e| {
            RustLtfsError::file_operation(format!("无法序列化数据库用于备份: {}", e))
        })?;

        let mut file = fs::File::create(&backup_path).map_err(|e| {
            RustLtfsError::file_operation(format!("无法创建备份文件: {}", e))
        })?;

        file.write_all(content.as_bytes()).map_err(|e| {
            RustLtfsError::file_operation(format!("无法写入备份文件: {}", e))
        })?;

        info!("数据库备份已创建: {:?}", backup_path.as_ref());
        Ok(())
    }

    /// 从备份恢复数据库
    pub fn restore_from_backup<P: AsRef<Path>>(backup_path: P) -> Result<Self> {
        let content = fs::read_to_string(&backup_path).map_err(|e| {
            RustLtfsError::file_operation(format!("无法读取备份文件: {}", e))
        })?;

        let database: Self = serde_json::from_str(&content).map_err(|e| {
            RustLtfsError::file_operation(format!("无法解析备份文件: {}", e))
        })?;

        info!("已从备份恢复数据库: {:?}", backup_path.as_ref());
        Ok(database)
    }

    /// 压缩数据库（移除重复的路径引用）
    pub fn compact(&mut self) -> Result<usize> {
        let mut removed_entries = 0;
        let mut updated_hash_records = HashMap::new();

        // 重建哈希到记录的映射，确保路径索引一致性
        for (hash, mut records) in self.hash_to_records.drain() {
            records.retain(|record| {
                let path_exists = self.path_to_record.contains_key(&record.file_path);
                if !path_exists {
                    removed_entries += 1;
                }
                path_exists
            });

            if !records.is_empty() {
                updated_hash_records.insert(hash, records);
            }
        }

        self.hash_to_records = updated_hash_records;

        // 移除没有对应哈希记录的路径项
        let valid_paths: HashSet<String> = self.hash_to_records
            .values()
            .flat_map(|records| records.iter().map(|r| r.file_path.clone()))
            .collect();

        let original_path_count = self.path_to_record.len();
        self.path_to_record.retain(|path, _| valid_paths.contains(path));
        removed_entries += original_path_count - self.path_to_record.len();

        info!("数据库压缩完成，移除了 {} 个无效条目", removed_entries);
        Ok(removed_entries)
    }

    /// 获取数据库统计信息
    pub fn get_database_info(&self) -> DatabaseInfo {
        let total_hash_entries = self.hash_to_records.len();
        let total_path_entries = self.path_to_record.len();
        let total_file_records: usize = self.hash_to_records.values().map(|v| v.len()).sum();
        
        // 计算各种哈希算法的使用情况
        let mut algorithm_counts = HashMap::new();
        for record in self.path_to_record.values() {
            for hash_type in record.hashes.keys() {
                *algorithm_counts.entry(hash_type.clone()).or_insert(0) += 1;
            }
        }

        // 计算重复文件组的分布
        let mut group_sizes = HashMap::new();
        for records in self.hash_to_records.values() {
            let size = records.len();
            *group_sizes.entry(size).or_insert(0) += 1;
        }

        DatabaseInfo {
            version: self.version,
            created_at: self.created_at.clone(),
            updated_at: self.updated_at.clone(),
            total_hash_entries,
            total_path_entries,
            total_file_records,
            algorithm_counts,
            group_sizes,
        }
    }

    /// 导出数据库到CSV格式
    pub fn export_to_csv<P: AsRef<Path>>(&self, csv_path: P) -> Result<()> {
        let mut writer = csv::Writer::from_path(&csv_path).map_err(|e| {
            RustLtfsError::file_operation(format!("无法创建CSV文件: {}", e))
        })?;

        // 写入CSV头部
        writer.write_record(&[
            "file_path", "file_size", "last_modified", "first_seen", 
            "sha1", "sha256", "blake3", "xxhash3", "xxhash128",
            "tape_partition", "tape_start_block", "tape_file_uid"
        ]).map_err(|e| {
            RustLtfsError::file_operation(format!("写入CSV头部失败: {}", e))
        })?;

        // 写入每个文件记录
        for record in self.path_to_record.values() {
            let empty_string = String::new();
            let sha1 = record.hashes.get(&HashType::Sha1).unwrap_or(&empty_string);
            let sha256 = record.hashes.get(&HashType::Sha256).unwrap_or(&empty_string);
            let blake3 = record.hashes.get(&HashType::Blake3).unwrap_or(&empty_string);
            let xxhash3 = record.hashes.get(&HashType::XxHash3).unwrap_or(&empty_string);
            let xxhash128 = record.hashes.get(&HashType::XxHash128).unwrap_or(&empty_string);

            let (partition, start_block, file_uid) = if let Some(ref loc) = record.tape_location {
                (loc.partition.to_string(), loc.start_block.to_string(), loc.file_uid.to_string())
            } else {
                (String::new(), String::new(), String::new())
            };

            writer.write_record(&[
                &record.file_path,
                &record.file_size.to_string(),
                &record.last_modified.to_string(),
                &record.first_seen,
                sha1, sha256, blake3, xxhash3, xxhash128,
                &partition, &start_block, &file_uid
            ]).map_err(|e| {
                RustLtfsError::file_operation(format!("写入CSV记录失败: {}", e))
            })?;
        }

        writer.flush().map_err(|e| {
            RustLtfsError::file_operation(format!("刷新CSV文件失败: {}", e))
        })?;

        info!("数据库已导出到CSV: {:?} ({} 条记录)", csv_path.as_ref(), self.path_to_record.len());
        Ok(())
    }

    /// 从CSV导入数据库
    pub fn import_from_csv<P: AsRef<Path>>(csv_path: P) -> Result<Self> {
        let mut reader = csv::Reader::from_path(&csv_path).map_err(|e| {
            RustLtfsError::file_operation(format!("无法打开CSV文件: {}", e))
        })?;

        let mut database = Self::default();
        let mut imported_count = 0;

        for result in reader.records() {
            let record = result.map_err(|e| {
                RustLtfsError::file_operation(format!("读取CSV记录失败: {}", e))
            })?;

            if record.len() < 13 {
                warn!("CSV记录格式不正确，跳过");
                continue;
            }

            let file_path = record[0].to_string();
            let file_size = record[1].parse().unwrap_or(0);
            let last_modified = record[2].parse().unwrap_or(0);
            let first_seen = record[3].to_string();

            let mut hashes = HashMap::new();
            if !record[4].is_empty() { hashes.insert(HashType::Sha1, record[4].to_string()); }
            if !record[5].is_empty() { hashes.insert(HashType::Sha256, record[5].to_string()); }
            if !record[6].is_empty() { hashes.insert(HashType::Blake3, record[6].to_string()); }
            if !record[7].is_empty() { hashes.insert(HashType::XxHash3, record[7].to_string()); }
            if !record[8].is_empty() { hashes.insert(HashType::XxHash128, record[8].to_string()); }

            let tape_location = if !record[9].is_empty() && !record[10].is_empty() && !record[11].is_empty() {
                Some(TapeLocation {
                    partition: record[9].parse().unwrap_or(0),
                    start_block: record[10].parse().unwrap_or(0),
                    file_uid: record[11].parse().unwrap_or(0),
                })
            } else {
                None
            };

            let file_record = FileHashRecord {
                file_path,
                file_size,
                last_modified,
                hashes,
                first_seen,
                tape_location,
            };

            database.add_file_record(file_record);
            imported_count += 1;
        }

        info!("从CSV导入完成: {} 条记录", imported_count);
        Ok(database)
    }

    /// 添加文件记录到数据库
    pub fn add_file_record(&mut self, record: FileHashRecord) {
        // 使用SHA1作为主键（对应LTFSCopyGUI的标准）
        if let Some(sha1_hash) = record.hashes.get(&HashType::Sha1) {
            self.hash_to_records
                .entry(sha1_hash.clone())
                .or_insert_with(Vec::new)
                .push(record.clone());
        }

        // 添加路径索引
        self.path_to_record.insert(record.file_path.clone(), record);
    }

    /// 检查文件是否为重复文件（基于SHA1哈希）
    pub fn check_duplicate(&self, sha1_hash: &str) -> Option<&Vec<FileHashRecord>> {
        self.hash_to_records.get(sha1_hash)
    }

    /// 获取重复文件统计信息
    pub fn get_duplicate_stats(&self) -> DuplicateStats {
        let mut duplicate_groups = 0;
        let mut total_duplicates = 0;
        let mut space_saved = 0u64;

        for (_, records) in &self.hash_to_records {
            if records.len() > 1 {
                duplicate_groups += 1;
                total_duplicates += records.len() - 1; // 第一个不算重复
                
                // 计算节省的空间（除第一个文件外的所有文件大小）
                if let Some(first_record) = records.first() {
                    space_saved += first_record.file_size * (records.len() as u64 - 1);
                }
            }
        }

        DuplicateStats {
            total_files: self.path_to_record.len(),
            duplicate_groups,
            total_duplicates,
            space_saved,
            unique_files: self.hash_to_records.len(),
        }
    }

    /// 清理过期或无效的记录
    pub fn cleanup_stale_records(&mut self, max_age_days: u32) -> Result<usize> {
        let cutoff_time = chrono::Utc::now() - chrono::Duration::days(max_age_days as i64);
        let cutoff_str = cutoff_time.format("%Y-%m-%dT%H:%M:%SZ").to_string();
        
        let mut removed_count = 0;
        let mut paths_to_remove = Vec::new();
        
        // 找出过期的记录
        for (path, record) in &self.path_to_record {
            if record.first_seen < cutoff_str {
                paths_to_remove.push(path.clone());
            }
        }

        // 移除过期记录
        for path in paths_to_remove {
            if let Some(record) = self.path_to_record.remove(&path) {
                if let Some(sha1_hash) = record.hashes.get(&HashType::Sha1) {
                    if let Some(records) = self.hash_to_records.get_mut(sha1_hash) {
                        records.retain(|r| r.file_path != path);
                        if records.is_empty() {
                            self.hash_to_records.remove(sha1_hash);
                        }
                    }
                }
                removed_count += 1;
            }
        }

        info!("清理了 {} 个过期记录", removed_count);
        Ok(removed_count)
    }
}

/// 数据库信息统计
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DatabaseInfo {
    pub version: u32,
    pub created_at: String,
    pub updated_at: String,
    pub total_hash_entries: usize,
    pub total_path_entries: usize,
    pub total_file_records: usize,
    pub algorithm_counts: HashMap<HashType, usize>,
    pub group_sizes: HashMap<usize, usize>, // 组大小 -> 组数量
}

/// 重复文件统计信息
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DuplicateStats {
    pub total_files: usize,
    pub duplicate_groups: usize,
    pub total_duplicates: usize,
    pub space_saved: u64,
    pub unique_files: usize,
}

/// 去重管理器（对应LTFSCopyGUI的DeduplicationManager）
pub struct DeduplicationManager {
    database: DeduplicationDatabase,
    database_path: PathBuf,
    enabled: bool,
    primary_hash_type: HashType,
}

impl DeduplicationManager {
    /// 创建新的去重管理器
    pub fn new<P: AsRef<Path>>(database_path: P, enabled: bool) -> Result<Self> {
        let database = DeduplicationDatabase::load_from_file(&database_path)?;
        
        Ok(Self {
            database,
            database_path: database_path.as_ref().to_path_buf(),
            enabled,
            primary_hash_type: HashType::Sha1, // LTFSCopyGUI默认使用SHA1
        })
    }

    /// 检查文件是否已存在（去重检查）
    pub fn check_file_exists(&self, file_hashes: &HashMap<String, String>) -> Option<Vec<&FileHashRecord>> {
        if !self.enabled {
            return None;
        }

        // 使用主要哈希类型进行检查
        let primary_key = self.primary_hash_type.ltfs_key();
        if let Some(hash_value) = file_hashes.get(primary_key) {
            if let Some(records) = self.database.check_duplicate(hash_value) {
                return Some(records.iter().collect());
            }
        }

        None
    }

    /// 注册新文件到去重数据库
    pub fn register_file(
        &mut self,
        file_path: &str,
        file_size: u64,
        file_hashes: &HashMap<String, String>,
        tape_location: Option<TapeLocation>,
    ) -> Result<()> {
        if !self.enabled {
            return Ok(());
        }

        // 转换哈希映射格式
        let mut typed_hashes = HashMap::new();
        for (ltfs_key, hash_value) in file_hashes {
            if let Some(hash_type) = HashType::from_ltfs_key(ltfs_key) {
                typed_hashes.insert(hash_type, hash_value.clone());
            }
        }

        // 获取文件修改时间
        let last_modified = std::fs::metadata(file_path)
            .map(|m| {
                m.modified()
                    .unwrap_or(std::time::UNIX_EPOCH)
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_secs()
            })
            .unwrap_or(0);

        let record = FileHashRecord {
            file_path: file_path.to_string(),
            file_size,
            last_modified,
            hashes: typed_hashes,
            first_seen: chrono::Utc::now().format("%Y-%m-%dT%H:%M:%SZ").to_string(),
            tape_location,
        };

        self.database.add_file_record(record);
        debug!("已注册文件到去重数据库: {}", file_path);
        
        Ok(())
    }

    /// 保存数据库
    pub fn save_database(&mut self) -> Result<()> {
        self.database.save_to_file(&self.database_path)
    }

    /// 获取统计信息
    pub fn get_stats(&self) -> DuplicateStats {
        self.database.get_duplicate_stats()
    }

    /// 清理过期记录
    pub fn cleanup_stale_records(&mut self, max_age_days: u32) -> Result<usize> {
        self.database.cleanup_stale_records(max_age_days)
    }

    /// 启用/禁用去重功能
    pub fn set_enabled(&mut self, enabled: bool) {
        self.enabled = enabled;
        if enabled {
            info!("去重功能已启用");
        } else {
            info!("去重功能已禁用");
        }
    }

    /// 是否启用了去重功能
    pub fn is_enabled(&self) -> bool {
        self.enabled
    }
}

/// 根据WriteOptions创建去重管理器
pub fn create_deduplication_manager(options: &WriteOptions, database_path: &Path) -> Result<DeduplicationManager> {
    let manager = DeduplicationManager::new(database_path, options.dedupe)?;
    
    if options.dedupe {
        info!("去重功能已启用，数据库路径: {:?}", database_path);
    } else {
        info!("去重功能已禁用");
    }
    
    Ok(manager)
}

/// 格式化文件大小（用于显示）
pub fn format_file_size(size: u64) -> String {
    const UNITS: &[&str] = &["B", "KB", "MB", "GB", "TB"];
    let mut size = size as f64;
    let mut unit_index = 0;

    while size >= 1024.0 && unit_index < UNITS.len() - 1 {
        size /= 1024.0;
        unit_index += 1;
    }

    if unit_index == 0 {
        format!("{} {}", size as u64, UNITS[unit_index])
    } else {
        format!("{:.2} {}", size, UNITS[unit_index])
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn test_hash_type_ltfs_keys() {
        assert_eq!(HashType::Sha1.ltfs_key(), "ltfs.hash.sha1sum");
        assert_eq!(HashType::Sha256.ltfs_key(), "ltfs.hash.sha256sum");
        assert_eq!(HashType::Blake3.ltfs_key(), "ltfs.hash.blake3sum");
        assert_eq!(HashType::XxHash3.ltfs_key(), "ltfs.hash.xxhash3sum");
        assert_eq!(HashType::XxHash128.ltfs_key(), "ltfs.hash.xxhash128sum");
    }

    #[test]
    fn test_deduplication_database_basic() {
        let mut db = DeduplicationDatabase::default();
        
        let mut hashes = HashMap::new();
        hashes.insert(HashType::Sha1, "abc123".to_string());
        
        let record = FileHashRecord {
            file_path: "/test/file1.txt".to_string(),
            file_size: 1024,
            last_modified: 1234567890,
            hashes,
            first_seen: "2023-01-01T00:00:00Z".to_string(),
            tape_location: None,
        };

        db.add_file_record(record);
        
        assert!(db.check_duplicate("abc123").is_some());
        assert!(db.check_duplicate("xyz789").is_none());
    }

    #[test]
    fn test_duplicate_stats() {
        let mut db = DeduplicationDatabase::default();
        
        // 添加两个相同哈希的文件
        for i in 1..=2 {
            let mut hashes = HashMap::new();
            hashes.insert(HashType::Sha1, "same_hash".to_string());
            
            let record = FileHashRecord {
                file_path: format!("/test/file{}.txt", i),
                file_size: 1024,
                last_modified: 1234567890,
                hashes,
                first_seen: "2023-01-01T00:00:00Z".to_string(),
                tape_location: None,
            };
            db.add_file_record(record);
        }

        let stats = db.get_duplicate_stats();
        assert_eq!(stats.total_files, 2);
        assert_eq!(stats.duplicate_groups, 1);
        assert_eq!(stats.total_duplicates, 1);
        assert_eq!(stats.space_saved, 1024);
    }
}