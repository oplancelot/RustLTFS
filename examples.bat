@echo off
REM Windows 批处理脚本示例
REM 演示 rustltfs 的基本用法

echo "=== RustLTFS CLI 工具使用示例 ==="
echo.

echo "1. 列出可用的磁带设备:"
rustltfs.exe list --detailed
echo.

echo "2. 检查第一个磁带设备的状态:"
rustltfs.exe status \\.\TAPE0
echo.

echo "3. 获取设备详细信息:"
rustltfs.exe info \\.\TAPE0
echo.

echo "4. 复制文件示例 (需要确认):"
echo "   rustltfs.exe copy \"C:\example\file.txt\" \\.\TAPE0 \"/backup/file.txt\" --progress --verify"
echo.

echo "5. 复制目录示例:"
echo "   rustltfs.exe copy \"C:\Documents\" \\.\TAPE0 \"/backup/documents\" --force --progress"
echo.

echo "6. 从磁带读取文件:"
echo "   rustltfs.exe read \\.\TAPE0 \"/backup/file.txt\" \"C:\restore\file.txt\" --verify"
echo.

echo "使用完整帮助信息:"
rustltfs.exe --help

pause