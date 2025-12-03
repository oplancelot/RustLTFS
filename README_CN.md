# RustLTFS

**🇨🇳 中文** | [🇺🇸 English](README.md)

## 概述

RustLTFS 是一个精简的 LTFS 磁带读写命令行工具，专注于 `write`、`read` 和 `space` 三个核心功能。

本项目受 [LTFSCopyGUI](https://github.com/zhaoyangwx/LTFSCopyGUI) 启发并与其兼容。

> **注意：** 除了命令行写入与读取索引功能外，其他功能（如格式化磁带、图形化浏览文件等）请使用 **LTFSCopyGUI**。

## 使用方法

### 1. 写入 (`write`)
将本地文件或目录写入磁带。

```powershell
# 写入文件夹
rustltfs write C:\local\folder --tape \\.\TAPE0 /tape/target_folder

# 写入单个文件
rustltfs write C:\local\file.txt --tape \\.\TAPE0 /tape/file.txt

# 从标准输入 (stdin) 写入
rustltfs write --tape \\.\TAPE0 /tape/stream.tar < C:\local\stream.tar
or
Get-Content -Path 'C:\local\stream.tar' -Encoding Byte -Raw | rustltfs write --tape \\.\TAPE0 /tape/stream.tar
```

### 2. 读取 (`read`)
解析索引并列出磁带上的目录和文件。

```powershell
# 列出根目录内容
rustltfs read --tape \\.\TAPE0
```

### 3. 空间 (`space`)
查看磁带容量与使用情况。

```powershell
rustltfs space --tape \\.\TAPE0
```

## 构建与安装

```powershell
git clone https://github.com/oplancelot/RustLTFS.git
cd RustLTFS
cargo build --release
```

## 许可证

[Apache-2.0](./LICENSE.md)
