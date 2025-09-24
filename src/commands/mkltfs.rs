use crate::error::Result;
use crate::tape_ops;
use tracing::{info, warn};

#[allow(clippy::too_many_arguments)]
pub async fn handle_mkltfs_command(
    device: String,
    barcode: Option<String>,
    volume_label: Option<String>,
    partition_count: u8,
    block_size: u32,
    capacity: u16,
    p0_size: u16,
    p1_size: u16,
    immediate: bool,
    force: bool,
    progress: bool
) -> Result<()> {
    info!("Starting MKLTFS operation on device: {}", device);
    
    // Create MKLTFS parameters
    let mut params = tape_ops::MkltfsParams::default();
    
    if let Some(ref bc) = barcode {
        params.set_barcode(bc);
    }
    
    if let Some(ref label) = volume_label {
        params.volume_label = label.clone();
    }
    
    params.extra_partition_count = partition_count;
    params.block_length = block_size;
    params.capacity = capacity;
    params.p0_size = p0_size;
    params.p1_size = p1_size;
    params.immediate_mode = immediate;
    
    // Display configuration information
    info!("MKLTFS Configuration:");
    info!("  Device: {}", device);
    if let Some(ref bc) = barcode {
        info!("  Barcode: {}", bc);
    }
    if let Some(ref label) = volume_label {
        info!("  Volume Label: {}", label);
    }
    info!("  Partition Count: {}", partition_count);
    info!("  Block Size: {} bytes", block_size);
    info!("  P0 Size: {}GB", p0_size);
    info!("  P1 Size: {}GB", p1_size);
    
    // Safety confirmation (unless using --force)
    if !force {
        println!("⚠️  WARNING: This operation will completely format the tape and ALL existing data will be lost!");
        println!("📋 MKLTFS Configuration:");
        println!("   Device: {}", device);
        if let Some(ref bc) = barcode {
            println!("   Barcode: {}", bc);
        }
        if let Some(ref label) = volume_label {
            println!("   Volume Label: {}", label);
        }
        println!("   Partition Config: {} ({})", 
            partition_count, 
            if partition_count > 0 { "Dual Partition" } else { "Single Partition" }
        );
        println!("   P0 Partition: {}GB", p0_size);
        println!("   P1 Partition: {}GB", p1_size);
        println!();
        println!("❓ Confirm to continue? (Type 'yes' to confirm)");
        
        let mut input = String::new();
        std::io::stdin().read_line(&mut input)?;
        
        if input.trim().to_lowercase() != "yes" {
            info!("User cancelled MKLTFS operation");
            println!("⛔ Operation cancelled");
            return Ok(());
        }
    }
    
    // Create tape operations instance
    let mut ops = tape_ops::TapeOperations::new(&device, false);
    
    // Set progress callback (if enabled)
    let progress_callback: Option<tape_ops::MkltfsProgressCallback> = if progress {
        Some(std::sync::Arc::new(|msg: &str| {
            println!("📈 {}", msg);
        }))
    } else {
        None
    };
    
    let finish_callback: Option<tape_ops::MkltfsFinishCallback> = Some(std::sync::Arc::new(|msg: &str| {
        println!("✅ {}", msg);
    }));
    
    let error_callback: Option<tape_ops::MkltfsErrorCallback> = Some(std::sync::Arc::new(|msg: &str| {
        eprintln!("❌ {}", msg);
    }));
    
    // Execute MKLTFS operation
    match ops.mkltfs(params, progress_callback, finish_callback, error_callback).await {
        Ok(true) => {
            println!("🎉 MKLTFS operation completed successfully! Tape has been formatted as LTFS");
            Ok(())
        }
        Ok(false) => {
            warn!("MKLTFS operation was not completed (possibly offline mode)");
            Ok(())
        }
        Err(e) => {
            tracing::error!("MKLTFS operation failed: {}", e);
            Err(e)
        }
    }
}