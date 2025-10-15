# RustLTFS - Rust 实现的 LTFS 磁带直接读写工具

**🇨🇳 中文** | [🇺🇸 English](README.md)

## 概述

RustLTFS 是一个用 Rust 编写的 IBM LTFS 磁带直接读写命令行工具，支持无需挂载磁带文件系统即可直接读写 LTO 磁带。

## 系统要求

- Windows 10/11 x64
- 兼容的 LTO 磁带驱动器（LTO-3 到 LTO-8）
- 管理员权限（用于 SCSI 命令）

## 开发环境配置

### 必需工具

- Rust 编译器 (nightly)
- mingw-w64 或 Visual Studio Build Tools
- Git

### 安装 Rust 开发环境

```cmd
# 安装 Rust
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh

# 安装 nightly 工具链
rustup install nightly
rustup default nightly

# 安装 Windows 目标平台
rustup target add x86_64-pc-windows-gnu
rustup target add x86_64-pc-windows-msvc
```

## 编译

### 从源码编译

```cmd
# 克隆项目
git clone https://github.com/oplancelot/RustLTFS.git
cd RustLTFS

# 开发构建
cargo build

# 发布构建（优化版本）
cargo build --release
cargo build --release --target x86_64-pc-windows-msvc

# 运行测试
cargo test

# 检查代码
cargo check
```

### 直接运行（开发模式）

```cmd
# 查看帮助
cargo run -- --help

# 查看具体命令帮助
cargo run -- read --help
cargo run -- write --help
cargo run -- view-index --help
```

## 安装

### 方式一：从源码编译安装

```cmd
# 编译并安装到 ~/.cargo/bin/
cargo install --path .

# 使用安装的版本
rustltfs --help
```

### 方式二：使用预编译版本

1. 下载 `rustltfs.exe` 文件
2. 将其放置在 PATH 环境变量包含的目录中，或直接使用完整路径

## 主要功能

### 智能读取命令

```cmd
# 列出磁带根目录内容
rustltfs read --tape TAPE0

# 显示文件内容（前50行）
rustltfs read --tape TAPE0 /backup/file.txt

# 复制文件到本地
rustltfs read --tape TAPE0 /backup/file.txt C:\restore\file.txt --verify

# 列出目录内容
rustltfs read --tape TAPE0 /backup/documents/
```

### 写入文件到磁带

```cmd
# 写入单个文件
rustltfs write C:\data\file.txt --tape TAPE0 /backup/file.txt --verify --progress

# 写入整个目录
rustltfs write C:\data\folder --tape TAPE0 /backup/folder --verify --progress
```

### 查看和解析 LTFS 索引文件

```cmd
# 基本查看索引信息
rustltfs view-index src/example/LTFSIndex_Load_71583245.schema

# 查看详细文件信息
rustltfs view-index src/example/LTFSIndex_Load_71583245.schema --detailed

# 导出为 TSV 格式（Excel 可打开）
rustltfs view-index src/example/LTFSIndex_Load_71583245.schema --export-format tsv --output filelist.tsv

# 导出为 JSON 格式
rustltfs view-index src/example/LTFSIndex_Load_71583245.schema --export-format json

# 导出为 XML 格式
rustltfs view-index src/example/LTFSIndex_Load_71583245.schema --export-format xml
```

### 离线模式磁带操作

```cmd
# 离线模式查看磁带根目录（使用本地索引文件）
rustltfs read --tape TAPE0 --skip-index --index-file src/example/LTFSIndex_Load_71583245.schema

# 离线模式模拟写入文件
rustltfs write src/example/README.md --tape TAPE0 /test/readme.md --skip-index

# 离线模式模拟写入目录
rustltfs write src/example/drivers/ --tape TAPE0 /test/drivers/ --skip-index
```

### 磁带设备管理（企业级）

```cmd
# 设备发现和枚举
rustltfs device discover                    # 发现所有可用磁带设备
rustltfs device discover --detailed        # 发现设备并显示详细信息

# 设备状态监控
rustltfs device status TAPE0               # 查看设备状态
rustltfs device status TAPE0 --monitor     # 持续监控设备状态
rustltfs device status TAPE0 --monitor --interval 30  # 每30秒监控一次

# 设备报告生成
rustltfs device report --type summary      # 生成设备摘要报告
rustltfs device report --type detailed --device TAPE0  # 生成详细设备报告
rustltfs device report --type inventory    # 生成设备清单（CSV格式）
rustltfs device report --type performance  # 生成性能报告
rustltfs device report --type health       # 生成健康状态报告

# 设备健康检查
rustltfs device health-check TAPE0         # 检查单个设备健康状态
rustltfs device health-check all           # 检查所有设备健康状态
rustltfs device health-check TAPE0 --comprehensive  # 全面健康检查

# 导出报告到文件
rustltfs device report --type summary --output report.txt
rustltfs device report --type inventory --output devices.csv
```

### 高级LTFS操作

```cmd
# LTFS磁带格式化（MKLTFS）
rustltfs mkltfs --tape TAPE0                           # 基本格式化
rustltfs mkltfs --tape TAPE0 --barcode ABC123L8        # 设置条形码
rustltfs mkltfs --tape TAPE0 --label "MyTape" --partition 1  # 双分区格式化
rustltfs mkltfs --tape TAPE0 --block-size 524288 --progress  # 自定义块大小

# 索引操作
rustltfs read-index --tape TAPE0                       # 从磁带读取索引
rustltfs read-index --tape TAPE0 --output index.schema # 保存索引到文件
rustltfs read-data-index --tape TAPE0                  # 读取数据分区索引
rustltfs update-index --tape TAPE0                     # 手动更新索引

# 磁带空间管理
rustltfs space --tape TAPE0                            # 查看磁带空间信息
rustltfs space --tape TAPE0 --detailed                 # 详细空间分析
```

### 去重和性能优化

```cmd
# 启用文件去重（基于哈希）
rustltfs write C:\data --tape TAPE0 /backup/data --dedupe --verify

# 高级写入选项
rustltfs write C:\data --tape TAPE0 /backup/data \
    --parallel \                    # 并行处理
    --speed-limit 100 \            # 限制速度为100MB/s
    --index-interval 36 \          # 每36GB更新一次索引
    --exclude .tmp,.log \          # 排除临时文件
    --compression-level 2 \        # 启用压缩
    --max-file-size 10 \          # 限制单文件大小（GB）
    --checkpoint 1000 \           # 每1000个文件创建检查点
    --progress                    # 显示进度

# 数据验证和完整性检查
rustltfs write C:\data --tape TAPE0 /backup/data --verify --hash-on-write
rustltfs read --tape TAPE0 /backup/data C:\restore --verify
```
## 命令参数说明

### read 命令

- `--tape <DEVICE>`: 磁带设备名（如 TAPE0）
- `[SOURCE]`: 磁带上的文件/目录路径（可选）
- `[DESTINATION]`: 本地目标路径（可选）
- `--skip-index`: 跳过自动索引读取（离线模式）
- `--index-file <FILE>`: 从本地文件加载索引
- `--verify`: 读取后验证数据完整性
- `--lines <N>`: 文本文件显示行数（默认 50）
- `--detailed`: 显示详细文件信息

### write 命令

- `<SOURCE>`: 本地源文件/目录路径
- `--tape <DEVICE>`: 磁带设备名
- `<DESTINATION>`: 磁带上的目标路径
- `--skip-index`: 跳过自动索引读取（离线模式）
- `--index-file <FILE>`: 从本地文件加载索引
- `--verify`: 写入后验证数据完整性
- `--progress`: 显示进度条
- `--force`: 跳过确认提示

### view-index 命令

- `<INDEX_FILE>`: LTFS 索引文件路径（.schema 文件）
- `--detailed`: 显示详细文件信息
- `--export-format <FORMAT>`: 导出文件列表格式（tsv, json, xml, batch）
- `--output <FILE>`: 导出输出文件

### 其他命令

- `device [DEVICE] [OPTIONS]`: 统一的设备管理命令
  - 不指定设备路径：列出所有设备
  - `--detailed`: 显示详细信息
  - `--status`: 显示设备状态
  - `--info`: 显示设备配置信息

## 使用示例

### 备份重要文件

```cmd
# 备份文档文件夹
rustltfs write "C:\Users\%USERNAME%\Documents" --tape TAPE0 /backup/documents --verify --progress

# 备份单个大文件
rustltfs write "C:\data\database.bak" --tape TAPE0 /backup/database.bak --verify
```

### 恢复文件

```cmd
# 查看磁带上有什么
rustltfs read --tape TAPE0

# 下载整个文档文件夹到当前目录
rustltfs read --tape TAPE0 /backup/documents

# 下载到指定位置
rustltfs read --tape TAPE0 /backup/documents "C:\restore\documents"

# 下载单个文件到当前目录
rustltfs read --tape TAPE0 /backup/config.txt
```

### 磁带管理

```cmd
# 检查所有可用磁带设备
rustltfs device

# 检查特定磁带状态和容量
rustltfs device TAPE0 --status --detailed

# 查看设备配置
rustltfs device TAPE0 --info
```

## 技术特性

- **直接读写**: 无需挂载，直接通过 SCSI 命令访问磁带
- **LTFS 兼容**: 完全兼容 IBM LTFS 格式，与LTFSCopyGUI互操作
- **企业级设备管理**: 设备发现、状态监控、健康评估、性能报告
- **文件去重**: 基于SHA1/MD5/SHA256/Blake3/XxHash的智能去重系统
- **双分区支持**: 正确处理LTFS双分区映射，数据写入数据分区
- **离线模式**: 支持在无磁带设备时模拟操作和索引解析
- **索引解析**: 可解析和导出 LTFS 索引文件为多种格式
- **智能操作**: 自动识别文件/目录，提供相应操作
- **容量管理**: 写入前自动检查磁带剩余空间
- **数据验证**: 支持读写后的数据完整性验证
- **进度显示**: 大文件操作时显示进度条
- **错误处理**: 详细的错误信息和恢复建议
- **跨平台编译**: 支持Linux、Windows GNU、Windows MSVC编译

## 性能优化

- 使用 64KB 块大小匹配 LTO 标准
- 异步 I/O 提高传输效率  
- 智能缓存减少磁带寻址
- 批量操作减少开销
- 基于哈希的文件去重，节省磁带空间
- 并行文件处理，提升写入速度
- 可配置的索引更新间隔，减少磁带操作
- 数据库持久化去重记录，避免重复计算

## 注意事项

1. **权限要求**: 需要管理员权限才能发送 SCSI 命令
2. **设备兼容性**: 支持 LTO-3 到 LTO-9 驱动器
3. **数据安全**: 建议总是使用 `--verify` 参数
4. **容量限制**: 会自动检查磁带剩余空间
5. **格式兼容**: 生成的磁带可与其他 LTFS 工具互操作
6. **双分区磁带**: 自动检测并正确处理双分区LTFS格式
7. **去重功能**: 启用去重时会创建本地数据库文件
8. **设备监控**: 支持实时监控多个磁带设备的健康状态

## 故障排除

### 常见问题

1. **"Access denied"错误**

   - 以管理员身份运行命令提示符
   - 确保用户有访问磁带设备的权限

2. **"No tape detected"错误**

   - 检查磁带是否正确插入
   - 确认磁带驱动器工作正常

3. **"Insufficient space"错误**

   - 使用 `rustltfs device TAPE0 --info` 查看剩余空间
   - 考虑使用新磁带或清理旧数据

4. **读写速度慢**
   - 确保使用高质量 LTO 磁带
   - 避免频繁的小文件操作
   - 考虑批量打包后再写入
   - 启用并行处理和去重功能

5. **设备发现失败**
   - 确认磁带驱动器已正确安装
   - 检查设备驱动程序是否最新
   - 使用 `rustltfs device discover --detailed` 获取详细信息

6. **去重数据库错误**
   - 删除损坏的去重数据库文件
   - 重新运行写入操作以重建数据库

## 更新日志

### v0.1.0 (最新版本)
- ✅ 修复关键分区映射问题，数据正确写入数据分区
- ✅ 实现完整的文件去重系统（SHA1/MD5/SHA256/Blake3/XxHash）
- ✅ 添加企业级设备管理和监控功能
- ✅ 完整的CLI集成，支持设备发现、状态监控、报告生成
- ✅ 跨平台编译支持（Linux/Windows GNU/MSVC）
- ✅ 与LTFSCopyGUI达到同等功能水平

## 技术支持

本工具参考了 IBM LTFSCopyGUI 的实现，确保与标准 LTFS 格式的完全兼容性。

## 版本信息

- 版本: 0.1.0
- 编译目标: x86_64-pc-windows-gnu
- 编译时间: $(date)
- Rust 版本: $(rustc --version)
