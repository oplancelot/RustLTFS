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

### 磁带设备管理

```cmd
# 列出所有可用磁带设备
rustltfs device

# 列出设备详细信息
rustltfs device --detailed

# 检查特定设备状态
rustltfs device TAPE0 --status

# 查看设备配置信息
rustltfs device TAPE0 --info

# 显示设备综合信息
rustltfs device TAPE0 --detailed
```

### test
```cmd
# 先检查设备列表
rustltfs.exe device

# 然后检查特定设备状态
rustltfs.exe device \\.\TAPE1 --status --detailed

# 最后尝试读取索引
rustltfs.exe read --tape \\.\TAPE1
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
- **LTFS 兼容**: 完全兼容 IBM LTFS 格式
- **离线模式**: 支持在无磁带设备时模拟操作和索引解析
- **索引解析**: 可解析和导出 LTFS 索引文件为多种格式
- **智能操作**: 自动识别文件/目录，提供相应操作
- **容量管理**: 写入前自动检查磁带剩余空间
- **数据验证**: 支持读写后的数据完整性验证
- **进度显示**: 大文件操作时显示进度条
- **错误处理**: 详细的错误信息和恢复建议

## 性能优化

- 使用 64KB 块大小匹配 LTO 标准
- 异步 I/O 提高传输效率
- 智能缓存减少磁带寻址
- 批量操作减少开销

## 注意事项

1. **权限要求**: 需要管理员权限才能发送 SCSI 命令
2. **设备兼容性**: 支持 LTO-3 到 LTO-8 驱动器
3. **数据安全**: 建议总是使用 `--verify` 参数
4. **容量限制**: 会自动检查磁带剩余空间
5. **格式兼容**: 生成的磁带可与其他 LTFS 工具互操作

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

## 技术支持

本工具参考了 IBM LTFSCopyGUI 的实现，确保与标准 LTFS 格式的完全兼容性。

## 版本信息

- 版本: 0.1.0
- 编译目标: x86_64-pc-windows-gnu
- 编译时间: $(date)
- Rust 版本: $(rustc --version)
