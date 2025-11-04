# RustLTFS - Rust 实现的 LTFS 磁带直接读写工具（精简版）

**🇨🇳 中文** | [🇺🇸 English](README.md)

## 概述

RustLTFS 是一个用 Rust 编写的 LTFS（Linear Tape File System）磁带直接读写命令行工具，支持无需挂载文件系统即可直接访问 LTO 磁带。本仓库已简化 CLI，仅保留三个核心命令：`write`、`read` 和 `space`。这些命令覆盖常用的写入、读取与容量查询场景，保留了与磁带读写直接相关的核心实现。

> 注意：之前仓库中存在的其它子命令（例如 `device`、`mkltfs`、`view-index`、`read-index`、`diagnose-block38` 等）已从主 CLI 中移除。如果你需要这些功能，请查看项目历史分支或归档副本（如有）。

## 系统要求

- Windows 10/11 x64（其他平台也可编译，需自行测试）
- 兼容的 LTO 磁带驱动器（LTO-3 到 LTO-9 等）
- 管理员权限（发送 SCSI 命令时通常需要）
- Rust 工具链（建议 nightly，但 stable 在多数情况下也能工作）

## 开发与编译

- 安装 Rust（rustup）
- 常用命令：
  - 开发构建：`cargo build`
  - 发布构建：`cargo build --release`
  - 运行帮助：`cargo run -- --help`
  - 运行命令示例：`cargo run -- write --help`

建议在你的开发环境或 CI 中运行 `cargo check` / `cargo test` 来验证更改。

## 安装

- 从源码安装：
  - `cargo install --path .`
  - 安装后使用 `rustltfs --help` 查看帮助

- 也可使用预编译的可执行文件（若提供）

## 精简后的主要命令

当前 CLI 仅保留以下三个子命令。命令设计尽量保持直观与兼容常见场景。

- `write`：将本地文件或目录写入磁带（支持校验、进度显示、去重选项等）
- `read`：从磁带列出或提取文件/目录（支持验证与离线索引模式）
- `space`：查询磁带空间与容量信息

下面分别说明用法与常见选项。

### write 命令（写入到磁带）

用途：将本地文件或目录写入到磁带上的目标路径。

示例用法（命令行形式）：

- 写入单个文件：
  `rustltfs write C:\data\file.txt --tape TAPE0 /backup/file.txt --verify --progress`
- 写入目录：
  `rustltfs write C:\data\folder --tape TAPE0 /backup/folder --verify --progress`

常用选项（示例）：

- `--tape <DEVICE>`: 指定磁带设备，如 `TAPE0`
- `--verify`: 写入后进行数据完整性校验
- `--progress`: 显示写入进度
- `--skip-index`: 跳过自动从磁带读取索引（用于离线模拟或索引不可用时）
- `--index-file <FILE>`: 使用本地 LTFS 索引文件（离线模式）
- `--force`: 跳过确认提示
- （高级写入选项如去重、并行、压缩等在保留的实现中可能有支持，请参见命令帮助）

注意事项：

- 写入前会对磁带剩余空间进行基本检查以避免写入失败。
- 对大规模写入，建议使用 `--progress` 并在必要时启用验证。

### read 命令（从磁带读取或列出）

用途：列出磁带根目录或读取指定文件/目录到本地。

示例用法：

- 列出磁带根目录：`rustltfs read --tape TAPE0`
- 下载文件到当前目录：`rustltfs read --tape TAPE0 /backup/config.txt`
- 下载文件到指定路径并校验：`rustltfs read --tape TAPE0 /backup/file.txt C:\restore\file.txt --verify`

常用选项：

- `--tape <DEVICE>`: 指定磁带设备
- `[SOURCE]`: 磁带上的路径（可选，省略时列出根目录）
- `[DESTINATION]`: 本地保存路径（可选）
- `--skip-index`: 跳过读取磁带索引，使用离线模式
- `--index-file <FILE>`: 指定本地索引文件用于离线操作
- `--verify`: 读取后进行数据完整性验证
- `--lines <N>`: 若读取文本，显示前 N 行（默认 50）
- `--detailed`: 显示详细文件信息

注意：

- `read` 在无法读取磁带索引时支持离线索引文件以加速操作或进行模拟恢复。
- 对于二进制大文件，建议提供目标路径并使用 `--verify` 做完整性检查。

### space 命令（查询磁带空间）

用途：显示磁带剩余空间、容量与分区信息（支持基本详细模式）。

示例：

- `rustltfs space --tape TAPE0`
- 详细分析（若支持）：`rustltfs space --tape TAPE0 --detailed`

常用选项：

- `--tape <DEVICE>`: 指定磁带设备
- `--detailed`: 输出更详细的空间与分区统计

备注：

- 该命令用于在写入前评估可用容量，避免写入中断或失败。

## 离线模式与索引

由于磁带可能在不同环境下无法直接读取索引，工具保留了使用本地索引文件的能力（通过 `--index-file` 与 `--skip-index` 选项）。离线模式可用于：

- 在没有实际磁带设备时模拟读写
- 使用从磁带导出的索引文件进行恢复或审计

## 安全与权限

- 发送 SCSI 命令通常需要管理员权限或相应的系统权限。请在具备权限的环境下运行对磁带设备的操作。
- 强烈建议在关键数据写入/读取时使用 `--verify` 选项。

## 常见问题与故障排除

1. 权限不足（Access denied）
   - 以管理员身份运行命令提示符或 PowerShell
   - 检查用户对设备的访问权限

2. 未检测到磁带（No tape detected）
   - 确认磁带已正确插入并就绪
   - 检查磁带驱动器连接与驱动程序状态

3. 空间不足（Insufficient space）
   - 使用 `rustltfs space --tape <DEVICE>` 检查剩余容量
   - 考虑使用新磁带或清理旧数据

4. 读写速度慢
   - 避免大量小文件直接写入，建议先打包或归档
   - 确认磁带硬件与连接性能

## 项目状态说明

- 本仓库的 CLI 已精简为仅保留 `write`、`read` 与 `space` 三个高频命令，以便聚焦核心读写与空间管理能力。
- 如果你依赖被移除的其它子命令（例如设备枚举、mkltfs 格式化、索引导出/查看或诊断工具），请在源码历史或归档分支中查找原始实现，或与维护者协商恢复策略。
- 保留的代码路径包含写入/读取、索引解析、容量管理、数据完整性校验与 SCSI 交互层的核心功能。

## 更新日志（摘要）

- 精简 CLI：移除非核心子命令，保留 `write` / `read` / `space`
- 保留核心读写逻辑、索引解析与容量检查等关键实现
- 文档同步以反映当前可用命令与使用方式

## 支持与贡献

- 提交 issue 或 pull request 以报告 bug 或请求功能
- 若需恢复被移除的功能，请在 issue 中说明使用场景与优先级
- 欢迎社区贡献测试用例、平台支持和设备兼容性补丁

## 版本信息

- 版本：0.1.0（文档为精简 CLI 版本）
- 构建目标和时间请参见实际构建输出：`rustc --version` 与 `cargo build` 的运行结果

---

若需进一步将 README 或其它本地化文档与英文版保持同步，我可以继续帮助整理或生成对应变更说明。
