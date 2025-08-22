# RustLTFS - Windows x86-64 发布版

## 概述

RustLTFS 是一个用 Rust 编写的 IBM LTFS 磁带直接读写命令行工具，支持无需挂载磁带文件系统即可直接读写 LTO 磁带。

## 系统要求

- Windows 10/11 x64
- 兼容的 LTO 磁带驱动器（LTO-3 到 LTO-8）
- 管理员权限（用于 SCSI 命令）

## 安装

1. 下载 `rustltfs.exe` 文件
2. 将其放置在 PATH 环境变量包含的目录中，或直接使用完整路径

## 主要功能

### 智能读取命令

```cmd
# 列出磁带根目录内容
rustltfs read TAPE0 /

# 显示文件内容（前50行）
rustltfs read TAPE0 /backup/file.txt

# 复制文件到本地
rustltfs read TAPE0 /backup/file.txt C:\restore\file.txt --verify

# 列出目录内容
rustltfs read TAPE0 /backup/documents/
```

### 写入文件到磁带

```cmd
# 写入单个文件
rustltfs copy C:\data\file.txt TAPE0 /backup/file.txt --verify --progress

# 写入整个目录
rustltfs copy C:\data\folder TAPE0 /backup/folder --verify --progress
```

### 磁带设备管理

```cmd
# 列出可用磁带设备
rustltfs list --detailed

# 查看设备状态
rustltfs status TAPE0

# 查看设备详细信息
rustltfs info TAPE0
```

## 命令参数说明

### read 命令

- `device`: 磁带设备名（如 TAPE0）
- `source`: 磁带上的文件/目录路径
- `destination`: （可选）本地目标路径
- `--verify`: 读取后验证数据完整性
- `--lines <N>`: 文本文件显示行数（默认 50）

### copy 命令

- `source`: 本地源文件/目录路径
- `device`: 磁带设备名
- `destination`: 磁带上的目标路径
- `--verify`: 写入后验证数据完整性
- `--progress`: 显示进度条
- `--force`: 跳过确认提示

## 使用示例

### 备份重要文件

```cmd
# 备份文档文件夹
rustltfs copy "C:\Users\%USERNAME%\Documents" TAPE0 /backup/documents --verify --progress

# 备份单个大文件
rustltfs copy "C:\data\database.bak" TAPE0 /backup/database.bak --verify
```

### 恢复文件

```cmd
# 查看磁带上有什么
rustltfs read TAPE0 /backup/

# 恢复整个文档文件夹
rustltfs read TAPE0 /backup/documents "C:\restore\documents"

# 预览文件内容
rustltfs read TAPE0 /backup/config.txt --lines 20
```

### 磁带管理

```cmd
# 检查磁带状态
rustltfs status TAPE0

# 查看磁带容量信息
rustltfs info TAPE0
```

## 技术特性

- **直接读写**: 无需挂载，直接通过 SCSI 命令访问磁带
- **LTFS 兼容**: 完全兼容 IBM LTFS 格式
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

   - 使用 `rustltfs info TAPE0` 查看剩余空间
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
