# RustLTFS - LTFS Direct Access Tool for Tape Drives

## Overview

RustLTFS is an IBM LTFS tape direct read/write command-line tool written in Rust, supporting direct read/write access to LTO tapes without mounting the tape file system.

## System Requirements

- Windows 10/11 x64
- Compatible LTO tape drives (LTO-3 to LTO-8)
- Administrator privileges (for SCSI commands)

## Development Environment Setup

### Required Tools

- Rust compiler (nightly)
- mingw-w64 or Visual Studio Build Tools
- Git

### Installing Rust Development Environment

```cmd
# Install Rust
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh

# Install nightly toolchain
rustup install nightly
rustup default nightly

# Install Windows target platforms
rustup target add x86_64-pc-windows-gnu
rustup target add x86_64-pc-windows-msvc
```

## Building

### Building from Source Code

```cmd
# Clone the project
git clone https://github.com/oplancelot/RustLTFS.git
cd RustLTFS

# Development build
cargo build

# Release build (optimized version)
cargo build --release

# Run tests
cargo test

# Check code
cargo check
```

### Direct Execution (Development Mode)

```cmd
# View help
cargo run -- --help

# View specific command help
cargo run -- read --help
cargo run -- write --help
cargo run -- view-index --help
```

## Installation

### Method 1: Build and Install from Source

```cmd
# Build and install to ~/.cargo/bin/
cargo install --path .

# Use the installed version
rustltfs --help
```

### Method 2: Use Pre-built Binary

1. Download the `rustltfs.exe` file
2. Place it in a directory included in the PATH environment variable, or use the full path directly

## Main Features

### Smart Read Commands

```cmd
# List tape root directory contents
rustltfs read --tape TAPE0

# Display file content (first 50 lines)
rustltfs read --tape TAPE0 /backup/file.txt

# Copy file to local
rustltfs read --tape TAPE0 /backup/file.txt C:\restore\file.txt --verify

# List directory contents
rustltfs read --tape TAPE0 /backup/documents/
```

### Write Files to Tape

```cmd
# Write single file
rustltfs write C:\data\file.txt --tape TAPE0 /backup/file.txt --verify --progress

# Write entire directory
rustltfs write C:\data\folder --tape TAPE0 /backup/folder --verify --progress
```

### View and Parse LTFS Index Files

```cmd
# Basic view of index information
rustltfs view-index src/example/LTFSIndex_Load_71583245.schema

# View detailed file information
rustltfs view-index src/example/LTFSIndex_Load_71583245.schema --detailed

# Export as TSV format (Excel compatible)
rustltfs view-index src/example/LTFSIndex_Load_71583245.schema --export-format tsv --output filelist.tsv

# Export as JSON format
rustltfs view-index src/example/LTFSIndex_Load_71583245.schema --export-format json

# Export as XML format
rustltfs view-index src/example/LTFSIndex_Load_71583245.schema --export-format xml
```

### Offline Mode Tape Operations

```cmd
# View tape root directory in offline mode (using local index file)
rustltfs read --tape TAPE0 --skip-index --index-file src/example/LTFSIndex_Load_71583245.schema

# Simulate file write in offline mode
rustltfs write src/example/README.md --tape TAPE0 /test/readme.md --skip-index

# Simulate directory write in offline mode
rustltfs write src/example/drivers/ --tape TAPE0 /test/drivers/ --skip-index
```

### Tape Device Management

```cmd
# List all available tape devices
rustltfs device

# List devices with detailed information
rustltfs device --detailed

# Check specific device status
rustltfs device TAPE0 --status

# View device configuration information
rustltfs device TAPE0 --info

# Show comprehensive device information
rustltfs device TAPE0 --detailed
```

## Command Parameters

### read Command

- `--tape <DEVICE>`: Tape device name (e.g., TAPE0)
- `[SOURCE]`: File/directory path on tape (optional)
- `[DESTINATION]`: Local destination path (optional)
- `--skip-index`: Skip automatic index reading (offline mode)
- `--index-file <FILE>`: Load index from local file
- `--verify`: Verify data integrity after reading
- `--lines <N>`: Number of lines to display for text files (default 50)
- `--detailed`: Show detailed file information

### write Command

- `<SOURCE>`: Local source file/directory path
- `--tape <DEVICE>`: Tape device name
- `<DESTINATION>`: Target path on tape
- `--skip-index`: Skip automatic index reading (offline mode)
- `--index-file <FILE>`: Load index from local file
- `--verify`: Verify data integrity after writing
- `--progress`: Show progress bar
- `--force`: Skip confirmation prompt

### view-index Command

- `<INDEX_FILE>`: LTFS index file path (.schema file)
- `--detailed`: Show detailed file information
- `--export-format <FORMAT>`: Export file list format (tsv, json, xml, batch)
- `--output <FILE>`: Export output file

### Other Commands

- `device [DEVICE] [OPTIONS]`: Unified device management command
  - Without device path: List all devices
  - `--detailed`: Show detailed information
  - `--status`: Show device status
  - `--info`: Show device configuration

## Usage Examples

### Backup Important Files

```cmd
# Backup Documents folder
rustltfs write "C:\Users\%USERNAME%\Documents" --tape TAPE0 /backup/documents --verify --progress

# Backup single large file
rustltfs write "C:\data\database.bak" --tape TAPE0 /backup/database.bak --verify
```

### Restore Files

```cmd
# View what's on the tape
rustltfs read --tape TAPE0 /backup/

# Restore entire Documents folder
rustltfs read --tape TAPE0 /backup/documents "C:\restore\documents"

# Preview file content
rustltfs read --tape TAPE0 /backup/config.txt --lines 20
```

### Tape Management

```cmd
# Check all available tape devices
rustltfs device

# Check specific tape status and capacity
rustltfs device TAPE0 --status --detailed

# View device configuration
rustltfs device TAPE0 --info
```

## Technical Features

- **Direct Access**: No mounting required, direct SCSI command access to tape
- **LTFS Compatible**: Fully compatible with IBM LTFS format
- **Offline Mode**: Support simulation operations and index parsing without tape devices
- **Index Parsing**: Parse and export LTFS index files to multiple formats
- **Smart Operations**: Automatic file/directory recognition with appropriate operations
- **Capacity Management**: Automatic tape space checking before writing
- **Data Verification**: Support data integrity verification after read/write
- **Progress Display**: Progress bar for large file operations
- **Error Handling**: Detailed error messages and recovery suggestions

## Performance Optimization

- Use 64KB block size to match LTO standards
- Asynchronous I/O for improved transfer efficiency
- Smart caching to reduce tape seeking
- Batch operations to reduce overhead

## Important Notes

1. **Permission Requirements**: Administrator privileges required for SCSI commands
2. **Device Compatibility**: Supports LTO-3 to LTO-8 drives
3. **Data Safety**: Always recommend using `--verify` parameter
4. **Capacity Limits**: Automatic tape space checking
5. **Format Compatibility**: Generated tapes are interoperable with other LTFS tools

## Troubleshooting

### Common Issues

1. **"Access denied" error**

   - Run command prompt as administrator
   - Ensure user has tape device access permissions

2. **"No tape detected" error**

   - Check if tape is properly inserted
   - Confirm tape drive is working correctly

3. **"Insufficient space" error**

   - Use `rustltfs device TAPE0 --info` to check remaining space
   - Consider using new tape or cleaning old data

4. **Slow read/write speeds**
   - Ensure using high-quality LTO tapes
   - Avoid frequent small file operations
   - Consider batch packaging before writing

## Technical Support

This tool references the IBM LTFSCopyGUI implementation to ensure full compatibility with standard LTFS format.

## Version Information

- Version: 0.1.0
- Build Target: x86_64-pc-windows-gnu
- Build Time: $(date)
- Rust Version: $(rustc --version)
