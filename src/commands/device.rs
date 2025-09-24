use crate::error::Result;
use crate::tape;
use tracing::info;

pub async fn handle_device_command(
    device: Option<String>,
    detailed: bool,
    status: bool,
    info_flag: bool
) -> Result<()> {
    match device {
        Some(device_path) => {
            // 处理特定设备的信息请求
            if status {
                info!("Getting device status: {}", device_path);
                tape::get_device_status(device_path).await
            } else if info_flag {
                info!("Getting device information: {}", device_path);
                tape::get_device_info(device_path).await
            } else {
                // 默认显示设备状态和信息
                info!("Getting comprehensive device info: {}", device_path);
                println!("📱 Device: {}", device_path);
                println!("\n🔧 Configuration Information:");
                if let Err(e) = tape::get_device_info(device_path.clone()).await {
                    println!("❌ Failed to get device info: {}", e);
                }
                println!("\n📊 Status Information:");
                tape::get_device_status(device_path).await
            }
        }
        None => {
            // 列出所有设备
            info!("Listing tape devices");
            tape::list_devices(detailed).await
        }
    }
}