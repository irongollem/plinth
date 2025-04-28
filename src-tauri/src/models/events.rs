use serde::{Deserialize, Serialize};
use specta::Type;
use tauri_specta::Event;

#[derive(Serialize, Deserialize, Debug, Clone, Type, Event)]
pub enum CompressionStatus {
    Started(StartedStatus),
    Progress(ProgressStatus),
    Completed(CompletedStatus),
    Failed(FailedStatus),
    Cancelled(CancelledStatus),
}

#[derive(Serialize, Deserialize, Debug, Clone, Type)]
pub struct StartedStatus {
    pub total_files: u32,
    pub total_size: u32,
}

#[derive(Serialize, Deserialize, Debug, Clone, Type)]
pub struct ProgressStatus {
    pub processed_files: u32,
    pub total_files: u32,
    pub processed_size: u32,
    pub total_size: u32,
    pub percent_size: u32,
    pub percent_files: u32,
    pub current_file: String,
}

#[derive(Serialize, Deserialize, Debug, Clone, Type)]
pub struct CompletedStatus {
    pub total_files: u32,
    pub total_size: u32,
    pub elapsed_seconds: f64,
}

#[derive(Serialize, Deserialize, Debug, Clone, Type)]
pub struct FailedStatus {
    pub error: String,
}

#[derive(Serialize, Deserialize, Debug, Clone, Type)]
pub struct CancelledStatus {}
