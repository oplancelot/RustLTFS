pub mod write;
pub mod read;
pub mod view_index;
pub mod device;
pub mod space;
pub mod read_index;
pub mod read_data_index;
pub mod update_index;
pub mod mkltfs;

use crate::error::Result;
use crate::cli::Commands;

pub async fn handle_command(command: Commands) -> Result<()> {
    match command {
        Commands::Write { 
            source, device, destination, skip_index, index_file, force, verify, progress,
            skip_symlinks, parallel, speed_limit, index_interval, exclude_extensions,
            resume, dry_run, compression_level, encrypt, checkpoint_interval,
            max_file_size, quiet
        } => {
            write::handle_write_command(
                source, device, destination, skip_index, index_file, force, verify, progress,
                skip_symlinks, parallel, speed_limit, index_interval, exclude_extensions,
                resume, dry_run, compression_level, encrypt, checkpoint_interval,
                max_file_size, quiet
            ).await
        }
        
        Commands::Read { device, source, destination, skip_index, index_file, verify, lines, detailed } => {
            read::handle_read_command(device, source, destination, skip_index, index_file, verify, lines, detailed).await
        }
        
        Commands::ViewIndex { index_file, detailed, export_format, output } => {
            view_index::handle_view_index_command(index_file, detailed, export_format, output).await
        }
        
        Commands::Device { device, detailed, status, info } => {
            device::handle_device_command(device, detailed, status, info).await
        }
        
        Commands::Space { device, skip_index, detailed } => {
            space::handle_space_command(device, skip_index, detailed).await
        }
        
        Commands::ReadIndex { device, output, detailed } => {
            read_index::handle_read_index_command(device, output, detailed).await
        }
        
        Commands::ReadDataIndex { device, output, detailed } => {
            read_data_index::handle_read_data_index_command(device, output, detailed).await
        }
        
        Commands::UpdateIndex { device, force, detailed } => {
            update_index::handle_update_index_command(device, force, detailed).await
        }
        
        Commands::Mkltfs { 
            device, barcode, volume_label, partition_count, block_size, capacity,
            p0_size, p1_size, immediate, force, progress
        } => {
            mkltfs::handle_mkltfs_command(
                device, barcode, volume_label, partition_count, block_size, capacity,
                p0_size, p1_size, immediate, force, progress
            ).await
        }
    }
}