#!/bin/bash

# RustLTFS 构建脚本
# 用于在 Linux/macOS 环境下交叉编译 Windows 版本

set -e

echo "=== RustLTFS Windows 交叉编译脚本 ==="

# 检查 Rust 环境
echo "检查 Rust 版本..."
rustc --version

# 添加 Windows 目标（如果尚未添加）
echo "添加 Windows 编译目标..."
rustup target add x86_64-pc-windows-gnu

# 检查 mingw-w64 是否已安装
if ! command -v x86_64-w64-mingw32-gcc &> /dev/null; then
    echo "警告: mingw-w64 未找到，尝试安装..."
    if command -v apt &> /dev/null; then
        sudo apt update && sudo apt install -y gcc-mingw-w64-x86-64
    elif command -v brew &> /dev/null; then
        brew install mingw-w64
    else
        echo "错误: 请手动安装 mingw-w64 交叉编译工具链"
        exit 1
    fi
fi

# 清理之前的构建
echo "清理之前的构建..."
cargo clean

# 构建 Windows 版本
echo "构建 Windows x86-64 版本..."
cargo build --release --target x86_64-pc-windows-gnu

# 检查构建结果
if [ -f "target/x86_64-pc-windows-gnu/release/rustltfs.exe" ]; then
    echo "✅ 构建成功!"
    echo "可执行文件位于: target/x86_64-pc-windows-gnu/release/rustltfs.exe"
    
    # 显示文件大小
    ls -lh target/x86_64-pc-windows-gnu/release/rustltfs.exe
    
    echo ""
    echo "使用方法:"
    echo "1. 将 rustltfs.exe 复制到 Windows 机器"
    echo "2. 以管理员权限运行命令提示符"
    echo "3. 执行 rustltfs.exe --help 查看使用说明"
else
    echo "❌ 构建失败!"
    exit 1
fi