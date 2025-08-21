# RustLTFS - IBM 磁带直接读写工具

一个用 Rust 开发的 Windows CLI 工具，支持通过 SCSI 命令直接与 IBM 磁带设备进行文件读写操作，无需传统的 LTFS 挂载。

## 特性

- ✅ **直接读写**：无需挂载，直接与 IBM 磁带设备通信
- ✅ **高性能**：使用 Rust 异步 I/O 和 SCSI 直接命令
- ✅ **进度显示**：实时显示文件传输进度
- ✅ **数据验证**：可选的写入后数据完整性验证
- ✅ **批量操作**：支持单文件和整个目录的递归复制

## 系统要求

- Windows 10/11 或 Windows Server 2016+
- 管理员权限（用于设备访问）
- IBM LTO 磁带驱动器和相应驱动程序

## 安装

```bash
# 编译安装
cargo build --release

# 可执行文件位于 target/release/rustltfs.exe
```

## 使用示例

### 列出可用磁带设备

```bash
# 简单列表
rustltfs list

# 详细信息
rustltfs list --detailed
```

### 获取设备信息

```bash
rustltfs info \\.\TAPE0
```

### 复制文件到磁带

```bash
# 复制单个文件
rustltfs copy "C:\data\file.txt" \\.\TAPE0 "/backup/file.txt"

# 复制整个目录
rustltfs copy "C:\documents" \\.\TAPE0 "/backup/documents" --progress --verify

# 强制执行（跳过确认）
rustltfs copy "C:\data" \\.\TAPE0 "/backup" --force
```

### 从磁带读取文件

```bash
# 读取单个文件
rustltfs read \\.\TAPE0 "/backup/file.txt" "C:\restore\file.txt"

# 带验证的读取
rustltfs read \\.\TAPE0 "/backup/data" "C:\restore\data" --verify
```

### 检查磁带状态

```bash
rustltfs status \\.\TAPE0
```

## 命令参数

### 全局选项

- `-v, --verbose`：启用详细输出
- `-c, --config <文件>`：指定配置文件路径

### copy 命令

- `--force`：跳过确认提示
- `--verify`：验证写入数据
- `--progress`：显示进度条

### 其他命令

- `--detailed`：显示详细信息（list 命令）
- `--verify`：验证数据完整性（read 命令）

## 技术架构

本工具基于以下核心原则设计：

- **KISS**：简洁的 CLI 接口，直接明了的命令结构
- **SOLID**：模块化设计，每个模块单一职责
- **DRY**：复用 SCSI 操作和错误处理逻辑
- **YAGNI**：仅实现必要功能，避免过度设计

### 模块结构

```
src/
├── main.rs          # 程序入口
├── cli.rs           # CLI 接口定义
├── error.rs         # 错误处理系统
├── logger.rs        # 日志记录
├── scsi.rs          # SCSI 接口封装
├── tape.rs          # 磁带设备操作
├── ltfs.rs          # LTFS 直接读写
└── file_ops.rs      # 文件操作逻辑
```

## 注意事项

⚠️ **重要提醒**：
- 必须以管理员权限运行
- 确保磁带驱动器驱动程序正确安装
- 操作前请备份重要数据
- 磁带写入操作不可逆，请谨慎使用

## 许可证

本项目采用 Apache License 2.0 许可证，详见 [LICENSE](LICENSE) 文件。

## 参考项目

- [LTFSCopyGUI](https://github.com/zhaoyangwx/LTFSCopyGUI) - VB.NET 实现的 GUI 版本
- [ltfscmd](https://github.com/inaxeon/ltfscmd) - C 语言实现的命令行版本