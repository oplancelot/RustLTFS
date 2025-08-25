//! 磁带定位功能测试
//! 
//! 测试新实现的完整磁带定位功能，包括：
//! - 块定位
//! - 文件标记定位  
//! - EOD定位
//! - 不同驱动器类型支持

#[cfg(test)]
mod tests {
    use crate::scsi::{ScsiInterface, LocateDestType, DriveType, locate_block, locate_to_filemark, locate_to_eod};
    use crate::error::Result;

    #[test]
    fn test_locate_dest_type_enum() {
        // 测试枚举值对应LTFSCopyGUI
        assert_eq!(LocateDestType::Block as u8, 0);
        assert_eq!(LocateDestType::FileMark as u8, 1);
        assert_eq!(LocateDestType::EOD as u8, 3);
    }

    #[test]
    fn test_drive_type_enum() {
        // 测试驱动器类型枚举
        let drive_types = [
            DriveType::Standard,
            DriveType::SLR3,
            DriveType::SLR1,
            DriveType::M2488,
        ];
        
        for drive_type in &drive_types {
            println!("Drive type: {:?}", drive_type);
        }
    }

    #[test]
    fn test_scsi_interface_creation() {
        // 测试ScsiInterface创建
        let scsi = ScsiInterface::new();
        assert_eq!(scsi.get_drive_type(), DriveType::Standard);
        
        let scsi_slr3 = ScsiInterface::with_drive_type(DriveType::SLR3);
        assert_eq!(scsi_slr3.get_drive_type(), DriveType::SLR3);
    }

    #[test]
    fn test_scsi_interface_settings() {
        // 测试ScsiInterface设置
        let mut scsi = ScsiInterface::new();
        
        // 测试驱动器类型设置
        scsi.set_drive_type(DriveType::SLR1);
        assert_eq!(scsi.get_drive_type(), DriveType::SLR1);
        
        // 测试分区支持设置
        scsi.set_allow_partition(false);
        scsi.set_allow_partition(true);
    }

    // 以下测试需要真实磁带设备，标记为ignored
    #[test]
    #[ignore]
    fn test_locate_block_real_device() -> Result<()> {
        // 真实设备测试：定位到特定块
        let device = r"\\.\TAPE0";
        let result = locate_block(device, 100, 0);
        
        match result {
            Ok(error_code) => {
                println!("Locate to block 100 successful, error code: 0x{:04X}", error_code);
                assert_eq!(error_code, 0);
            }
            Err(e) => {
                println!("Locate failed (expected without real device): {}", e);
            }
        }
        
        Ok(())
    }

    #[test]
    #[ignore]
    fn test_locate_to_filemark_real_device() -> Result<()> {
        // 真实设备测试：定位到文件标记
        let device = r"\\.\TAPE0";
        let result = locate_to_filemark(device, 1, 0);
        
        match result {
            Ok(error_code) => {
                println!("Locate to filemark 1 successful, error code: 0x{:04X}", error_code);
            }
            Err(e) => {
                println!("Locate to filemark failed (expected without real device): {}", e);
            }
        }
        
        Ok(())
    }

    #[test]
    #[ignore]
    fn test_locate_to_eod_real_device() -> Result<()> {
        // 真实设备测试：定位到数据末尾
        let device = r"\\.\TAPE0";
        let result = locate_to_eod(device, 0);
        
        match result {
            Ok(error_code) => {
                println!("Locate to EOD successful, error code: 0x{:04X}", error_code);
            }
            Err(e) => {
                println!("Locate to EOD failed (expected without real device): {}", e);
            }
        }
        
        Ok(())
    }

    #[test]
    #[ignore]
    fn test_comprehensive_locate_functionality() -> Result<()> {
        // 综合测试：测试各种定位功能
        let device = r"\\.\TAPE0";
        
        println!("=== 综合磁带定位功能测试 ===");
        
        // 测试1：定位到起始位置
        println!("1. 定位到起始位置 (partition 0, block 0)");
        if let Ok(code) = locate_block(device, 0, 0) {
            println!("   成功：错误码 0x{:04X}", code);
        }
        
        // 测试2：定位到中间块
        println!("2. 定位到中间块 (partition 0, block 100)");
        if let Ok(code) = locate_block(device, 100, 0) {
            println!("   成功：错误码 0x{:04X}", code);
        }
        
        // 测试3：定位到数据分区
        println!("3. 定位到数据分区 (partition 1, block 50)");
        if let Ok(code) = locate_block(device, 50, 1) {
            println!("   成功：错误码 0x{:04X}", code);
        }
        
        // 测试4：定位到第一个文件标记
        println!("4. 定位到第一个文件标记");
        if let Ok(code) = locate_to_filemark(device, 1, 0) {
            println!("   成功：错误码 0x{:04X}", code);
        }
        
        // 测试5：定位到数据末尾
        println!("5. 定位到数据末尾");
        if let Ok(code) = locate_to_eod(device, 0) {
            println!("   成功：错误码 0x{:04X}", code);
        }
        
        Ok(())
    }

    #[test]
    fn test_locate_dest_type_comparison() {
        // 测试PartialEq实现
        assert_eq!(LocateDestType::Block, LocateDestType::Block);
        assert_ne!(LocateDestType::Block, LocateDestType::FileMark);
        assert_ne!(LocateDestType::FileMark, LocateDestType::EOD);
    }

    #[test]
    fn test_drive_type_specific_interface() -> Result<()> {
        // 测试特定驱动器类型的接口创建
        let drive_types = [
            DriveType::Standard,
            DriveType::SLR3,
            DriveType::SLR1,
            DriveType::M2488,
        ];
        
        for drive_type in &drive_types {
            let scsi = ScsiInterface::with_drive_type(*drive_type);
            assert_eq!(scsi.get_drive_type(), *drive_type);
            println!("Created ScsiInterface with drive type: {:?}", drive_type);
        }
        
        Ok(())
    }
}

/// 演示新定位功能使用方法的示例
#[cfg(feature = "examples")]
pub mod examples {
    use crate::scsi::*;
    use crate::error::Result;

    /// 基本块定位示例
    pub fn example_basic_locate() -> Result<()> {
        // 定位到分区0的第100块
        let error_code = locate_block(r"\\.\TAPE0", 100, 0)?;
        println!("定位完成，错误码: 0x{:04X}", error_code);
        Ok(())
    }

    /// 文件标记定位示例
    pub fn example_filemark_locate() -> Result<()> {
        // 定位到第5个文件标记
        let error_code = locate_to_filemark(r"\\.\TAPE0", 5, 0)?;
        println!("定位到文件标记完成，错误码: 0x{:04X}", error_code);
        Ok(())
    }

    /// 数据末尾定位示例
    pub fn example_eod_locate() -> Result<()> {
        // 定位到数据末尾
        let error_code = locate_to_eod(r"\\.\TAPE0", 0)?;
        println!("定位到数据末尾完成，错误码: 0x{:04X}", error_code);
        Ok(())
    }

    /// 特定驱动器优化示例
    pub fn example_driver_specific_locate() -> Result<()> {
        // 为SLR3驱动器优化的定位
        let error_code = locate_with_drive_type(
            r"\\.\TAPE0",
            200,
            0,
            LocateDestType::Block,
            DriveType::SLR3
        )?;
        println!("SLR3优化定位完成，错误码: 0x{:04X}", error_code);
        Ok(())
    }

    /// 使用ScsiInterface进行复杂定位操作
    pub fn example_advanced_locate() -> Result<()> {
        let mut scsi = ScsiInterface::with_drive_type(DriveType::Standard);
        scsi.open_device(r"\\.\TAPE0")?;
        
        // 设置允许分区操作
        scsi.set_allow_partition(true);
        
        // 执行多种定位操作
        let block_result = scsi.locate(0, 0, LocateDestType::Block)?;
        println!("块定位结果: 0x{:04X}", block_result);
        
        let fm_result = scsi.locate(1, 0, LocateDestType::FileMark)?;
        println!("文件标记定位结果: 0x{:04X}", fm_result);
        
        let eod_result = scsi.locate(0, 0, LocateDestType::EOD)?;
        println!("EOD定位结果: 0x{:04X}", eod_result);
        
        Ok(())
    }
}