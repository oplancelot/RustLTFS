# RustLTFS 项目文档

## 文档概述

本套文档为RustLTFS项目提供完整的技术参考。RustLTFS是一个用Rust编写的IBM LTFS磁带直接读写命令行工具，支持无需挂载磁带文件系统即可直接读写LTO磁带。

## 项目当前状态

### 已实现的核心功能
- ✅ **CLI命令行界面** - 基于clap的现代化命令行工具
- ✅ **磁带操作模块** - TapeOperations核心功能实现
- ✅ **LTFS索引解析** - 完整的XML索引文件解析和处理
- ✅ **离线模式支持** - 可在无磁带设备时进行索引解析和模拟操作
- ✅ **多种导出格式** - 支持TSV、JSON、XML、批处理脚本导出
- ✅ **设备管理** - 磁带设备列举、状态查询和信息获取

### 核心模块架构
```
src/
├── main.rs           # CLI主程序入口，命令分发和处理
├── cli.rs            # 命令行参数定义和解析
├── tape_ops.rs       # 磁带操作核心模块 (重构自direct_tape_ops)
├── ltfs_index.rs     # LTFS索引XML解析和数据结构
├── scsi.rs           # Windows SCSI命令接口
├── error.rs          # 统一错误处理
├── logger.rs         # 日志系统初始化
└── file_ops.rs       # 文件操作辅助功能
```

## 文档结构

### 核心分析文档
**文件**: `LTFSCopyGUI_DirectTapeAccess_Analysis_Part1.md` 和 `Part2.md`

**内容**：
- LTFSCopyGUI原始实现的深度分析
- 为RustLTFS实现提供的设计参考
- 核心操作流程和技术实现要点

**注意**：这些文档主要作为历史参考，当前RustLTFS实现已经有所演进。

### CLI集成总结
**文件**: `CLI_Integration_Summary.md`

**内容**：
- 命令行架构设计总结
- 与LTFSCopyGUI功能映射关系
- 已完成功能的验证记录

## 当前CLI命令结构

### Write命令 - 文件/目录写入磁带
```bash
rustltfs write <SOURCE> --tape <DEVICE> <DESTINATION> [options]
```
- 支持单文件和目录写入
- 自动索引更新
- 离线模式支持
- 数据验证功能

### Read命令 - 磁带内容读取和提取
```bash
rustltfs read --tape <DEVICE> [SOURCE] [DESTINATION] [options]
```
- 磁带索引自动读取
- 路径浏览和文件预览
- 文件/目录提取
- 详细信息显示

### ViewIndex命令 - 本地索引文件解析
```bash
rustltfs view-index <INDEX_FILE> [options]
```
- 本地LTFS索引文件解析
- 多种导出格式（TSV、JSON、XML、批处理）
- 详细文件信息显示

### 设备管理命令
```bash
rustltfs list [--detailed]     # 列出磁带设备
rustltfs info <DEVICE>         # 设备信息查询
rustltfs status <DEVICE>       # 设备状态检查
```

## 重要实现特性

### 1. 模块重构成果
- **TapeOperations结构体** - 统一的磁带操作接口
- **IndexViewer结构体** - 专门的索引查看功能
- **去除冗余模块** - 删除了direct_tape_commands等重复代码
- **命名一致性** - 统一使用write而非copy进行文件操作

### 2. 离线模式支持
- 支持使用本地索引文件进行操作
- 可以在无磁带设备环境下解析和查看索引
- 模拟写入操作用于测试和开发

### 3. LTFS索引处理
- 完整的LTFS XML格式解析
- 支持LTFSIndex_Load_*.schema格式
- 自动时间戳生成（YYYYMMDD_HHMMSS格式）
- 索引验证和完整性检查

### 4. 错误处理和日志
- 统一的Result类型错误传播
- 基于tracing的结构化日志系统
- 详细的错误信息和用户友好提示

## 开发状态

### 已完成功能
- ✅ 完整的CLI架构实现
- ✅ LTFS索引文件解析和导出
- ✅ 离线模式完整支持
- ✅ 代码重构和模块整理
- ✅ 错误处理和日志系统
- ✅ 文档和README更新

### 待完成功能
- 🔄 真实SCSI设备操作（当前为模拟实现）
- 🔄 文件内容预览功能实现
- 🔄 进度显示和状态更新
- 🔄 批量操作优化

### 测试验证
- ✅ 编译测试通过（cargo build/check）
- ✅ CLI命令解析正常
- ✅ 离线模式功能验证
- ✅ ViewIndex命令完整测试
- ✅ 索引文件导出功能

## 使用示例

### 查看本地索引文件
```bash
# 基本查看
cargo run -- view-index src/example/LTFSIndex_Load_71583245.schema

# 详细信息
cargo run -- view-index src/example/LTFSIndex_Load_71583245.schema --detailed

# 导出为TSV（Excel兼容）
cargo run -- view-index src/example/LTFSIndex_Load_71583245.schema --export-format tsv --output filelist.tsv
```

### 离线模式磁带操作
```bash
# 查看磁带内容（使用本地索引）
cargo run -- read --tape TAPE0 --skip-index --index-file src/example/LTFSIndex_Load_71583245.schema

# 模拟写入操作
cargo run -- write README.md --tape TAPE0 /test/readme.md --skip-index
```

## 相关文件

### 核心代码模块
- `src/tape_ops.rs` - 磁带操作核心实现
- `src/ltfs_index.rs` - LTFS索引处理
- `src/main.rs` - CLI命令分发
- `src/cli.rs` - 命令行参数定义

### 测试和示例文件
- `src/example/LTFSIndex_Load_71583245.schema` - 测试用LTFS索引文件
- `README.md` 和 `README_CN.md` - 项目使用文档

### 构建和配置
- `Cargo.toml` - 项目依赖和配置
- `target/` - 编译输出目录

## 技术栈

- **语言**: Rust (2021 edition)
- **CLI框架**: clap 4.4
- **异步运行时**: tokio 1.0
- **XML解析**: quick-xml 0.31
- **日志系统**: tracing 0.1
- **错误处理**: anyhow 1.0, thiserror 1.0
- **时间处理**: chrono 0.4

## 更新记录

- **2024-08-25**: 完成模块重构，删除重复代码，统一命名规范
- **2024-08-25**: 实现完整的离线模式支持
- **2024-08-25**: 更新README文档，修正CLI命令格式
- **2024-08-25**: 更新项目文档，反映当前实现状态