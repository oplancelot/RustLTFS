use crate::error::Result;
use crate::{cli::ExportFormat, tape_ops};
use std::path::PathBuf;
use tracing::info;

pub async fn handle_view_index_command(
    index_file: PathBuf, 
    detailed: bool, 
    export_format: Option<ExportFormat>, 
    output: Option<PathBuf>
) -> Result<()> {
    info!("Viewing LTFS index file: {:?}", index_file);
    tape_ops::IndexViewer::handle_view_index_command(
        &index_file,
        detailed,
        export_format,
        output.as_deref(),
    ).await
}