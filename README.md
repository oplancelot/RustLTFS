# RustLTFS - LTFS Direct Access Tool for Tape Drives

[🇨🇳 中文版本](README_CN.md) | **🇺🇸 English**

## Overview

RustLTFS is a CLI tool for direct LTFS tape access (read/write/space) written in Rust. It provides low-level tape operations without mounting the LTFS filesystem and focuses on the core workflows for writing data to tape, reading data from tape, and showing tape space information.

This repository has been streamlined: only the `write`, `read`, and `space` commands remain. Other utility commands (index viewing, mkltfs/format, device management, diagnostics) were removed from the main CLI to reduce surface area. If you rely on removed features, use a prior release or keep a local copy of the removed modules.

## System Requirements

- Windows 10/11 x64 (primary tested platform)
- Compatible LTO tape drives (LTO-3 and newer)
- Administrator privileges when performing low-level SCSI operations

## Development Environment Setup

### Required Tools

- Rust (compatible stable or nightly depending on your toolchain)
- Build tools for your target (MSVC or GNU toolchain on Windows)
- Git

### Installing Rust (example)

```powershell
# Install rustup and Rust toolchain
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
rustup install stable
rustup default stable
```

## Building

```powershell
# Clone the project
git clone https://github.com/oplancelot/RustLTFS.git
cd RustLTFS

# Development build
cargo build

# Release build (optimized)
cargo build --release

# Run tests
cargo test

# Quick check
cargo check
```

## Usage

RustLTFS now exposes three primary subcommands:

- `write` — write files or directories to tape
- `read` — read files or list directory contents from tape
- `space` — show tape space information (total / used / available)

Run `rustltfs --help` for the top-level help and `rustltfs <command> --help` for command-specific options.

### Common option names

- `--tape <DEVICE>` or `-t <DEVICE>` — tape device path (e.g. `\\.\TAPE0`)
- `--skip-index` or `-s` — skip automatic index reading (offline mode)
- `--index-file <FILE>` or `-f <FILE>` — load LTFS index from a local file instead of reading from tape
- `--verify` — verify data after read/write (hash/compare)
- `--progress` — show progress for long operations
- `--detailed` or `-d` — show additional details in output

### write — Write files or folders to tape

Description:
Write local files or directories to the tape. The command supports verification, progress reporting, exclusions and advanced write options.

Basic examples:

```powershell
# Write a single file to tape (verify after write, show progress)
rustltfs write C:\data\file.bin --tape \\.\TAPE0 /backup/file.bin --verify --progress

# Write a directory to tape
rustltfs write C:\data\backup_folder --tape \\.\TAPE0 /backup/folder --progress

# Dry-run (show what would be done without writing)
rustltfs write C:\data\project --tape \\.\TAPE0 /backup/project --dry-run
```

Key options:

- `<SOURCE>` — local file or directory to write
- `--tape <DEVICE>` — target tape device
- `<DESTINATION>` — target path on the tape
- `--force` — overwrite existing files without confirmation
- `--verify` — verify data integrity after write
- `--skip-symlinks` — skip symbolic links
- `--parallel` — enable parallel file processing (uses more memory)
- `--speed-limit <MBPS>` — limit write speed (MiB/s)
- `--exclude <EXTENSIONS>` — comma separated list of extensions to exclude
- `--resume` — attempt to resume from previous interrupted operation
- `--dry-run` — simulate the write without performing tape operations
- `--compress/--encrypt` — compression/encryption toggles for future extensibility (implementation status may vary)

Notes:

- The tool attempts to read the LTFS index automatically unless `--skip-index` is used or `--index-file` is provided.
- After successful writes, the tool may attempt to update the LTFS index on tape (if not in offline mode).

### read — Read from tape

Description:
Read/list files and directories stored on the tape. Supports extracting files to local filesystem and displaying file content.

Basic examples:

```powershell
# List root directory
rustltfs read --tape \\.\TAPE0

# Show a file content (first N lines)
rustltfs read --tape \\.\TAPE0 /backup/file.txt --lines 50

# Extract a file to the current directory
rustltfs read --tape \\.\TAPE0 /backup/file.txt

# Extract to a specific local destination
rustltfs read --tape \\.\TAPE0 /backup/file.txt C:\restore\file.txt --verify
```

Key options:

- `--tape <DEVICE>` — tape device path
- `[SOURCE]` — tape path to file or directory (omit to list root)
- `[DESTINATION]` — local destination path for extracted files
- `--skip-index` — operate in offline mode (won't attempt to read index from tape)
- `--index-file <FILE>` — load index from local file for offline operations
- `--verify` — verify extracted data
- `--lines <N>` — limit output when printing text files
- `--detailed` — show extended file/directory metadata

Notes:

- If reading fails due to index issues, you may use `--index-file` to supply a previously saved LTFS index for offline extraction.

### space — Show tape space information

Description:
Show basic tape space metrics: total capacity, used space and available space.

Example:

```powershell
rustltfs space --tape \\.\TAPE0
```

Key options:

- `--tape <DEVICE>` — tape device path
- `--detailed` / `-d` — show more detailed breakdown (if available)
- `--skip-index` — skip index reading if you only need raw capacity values

## Offline workflows

You can operate in offline mode by providing `--index-file <file>` or using `--skip-index`. This is useful when the tape drive is not available and you have a saved LTFS index file.

```powershell
# Use a saved index to extract files without connecting to the tape
rustltfs read --tape \\.\TAPE0 --skip-index --index-file saved_index.schema /backup/myfile.txt C:\out\myfile.txt
```

## Logging

The tool uses structured logging. Enable verbose logs with `-v` / `--verbose` and consult output for troubleshooting.

## Notes about removed commands

To keep the CLI focused, the following previously-available commands have been removed from the main CLI:

- `view-index` (index viewing utilities)
- `mkltfs` (format / mkltfs)
- `read-index`, `read-data-index` (specialized index-reading helpers)
- `device` (device discovery and management)
- `diagnose-block38`, `update-index` (diagnostics and manual index update utilities)

If you require any of these capabilities, check out an earlier release or maintain a local branch that preserves the removed modules. You can also reimplement the specific behaviors you need on top of the remaining `write`/`read`/`space` primitives.

## Contributing

- Fork the repository and open a pull request for proposed changes.
- Include unit tests for new functionality where appropriate.
- Keep changes focused and provide rationale in PR descriptions.

## License

Apache-2.0

---

If you want, I can also update `README_CN.md` and other documentation files to match these changes.
