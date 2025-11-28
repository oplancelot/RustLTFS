# RustLTFS

[🇨🇳 中文版本](README_CN.md) | **🇺🇸 English**

## Overview

RustLTFS is a streamlined CLI tool for direct LTFS tape access, focusing solely on `write`, `read`, and `space` operations.

This project is inspired by and compatible with [LTFSCopyGUI](https://github.com/zhaoyangwx/LTFSCopyGUI).

> **Note:** For features other than command-line writing/reading (such as formatting, mounting, or GUI browsing), please use **LTFSCopyGUI**.

## Usage

### 1. Write (`write`)
Write files or directories to tape.

```powershell
# Write a folder
rustltfs write C:\local\folder --tape \\.\TAPE0 /tape/target_folder

# Write a single file
rustltfs write C:\local\file.txt --tape \\.\TAPE0 /tape/file.txt

# Write from stdin
rustltfs write --tape \\.\TAPE0 /tape/stream.tar < C:\local\stream.tar
```

### 2. Read (`read`)
Parse the index to list directories and files on the tape.

```powershell
# List root directory contents
rustltfs read --tape \\.\TAPE0
```

### 3. Space (`space`)
Check tape capacity and usage.

```powershell
rustltfs space --tape \\.\TAPE0
```

## Building

```powershell
git clone https://github.com/oplancelot/RustLTFS.git
cd RustLTFS
cargo build --release
```

## License

Apache-2.0
