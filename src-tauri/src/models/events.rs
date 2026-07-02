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
    pub job_id: String,
    pub total_files: u32,
    pub total_size_kb: u32,
}

#[derive(Serialize, Deserialize, Debug, Clone, Type)]
pub struct ProgressStatus {
    pub job_id: String,
    pub processed_files: u32,
    pub total_files: u32,
    pub processed_size_kb: u32,
    pub total_size_kb: u32,
    pub percent_size: u32,
    pub percent_files: u32,
    pub current_file: String,
}

#[derive(Serialize, Deserialize, Debug, Clone, Type)]
pub struct CompletedStatus {
    pub job_id: String,
    pub total_files: u32,
    pub total_size_kb: u32,
    pub elapsed_seconds: f64,
    pub folder_path: String,
}

#[derive(Serialize, Deserialize, Debug, Clone, Type)]
pub struct FailedStatus {
    pub job_id: String,
    pub error: String,
}

#[derive(Serialize, Deserialize, Debug, Clone, Type)]
pub struct CancelledStatus {
    pub job_id: String,
}

#[derive(Serialize, Deserialize, Debug, Clone, Type, Event)]
pub enum ScanStatus {
    Started(ScanStartedStatus),
    Progress(ScanProgressStatus),
    Completed(ScanCompletedStatus),
    Failed(ScanFailedStatus),
    Cancelled(ScanCancelledStatus),
}

#[derive(Serialize, Deserialize, Debug, Clone, Type)]
pub struct ScanStartedStatus {
    pub job_id: String,
    pub root: String,
}

#[derive(Serialize, Deserialize, Debug, Clone, Type)]
pub struct ScanProgressStatus {
    pub job_id: String,
    pub files_indexed: u32,
    pub current_dir: String,
}

#[derive(Serialize, Deserialize, Debug, Clone, Type)]
pub struct ScanCompletedStatus {
    pub job_id: String,
    pub total_files: u32,
    pub total_models: u32,
    pub elapsed_seconds: f64,
}

#[derive(Serialize, Deserialize, Debug, Clone, Type)]
pub struct ScanFailedStatus {
    pub job_id: String,
    pub error: String,
}

#[derive(Serialize, Deserialize, Debug, Clone, Type)]
pub struct ScanCancelledStatus {
    pub job_id: String,
}

#[derive(Serialize, Deserialize, Debug, Clone, Type, Event)]
pub enum DuplicateStatus {
    Started(DuplicateStartedStatus),
    Progress(DuplicateProgressStatus),
    Completed(DuplicateCompletedStatus),
    Failed(DuplicateFailedStatus),
    Cancelled(DuplicateCancelledStatus),
}

#[derive(Serialize, Deserialize, Debug, Clone, Type)]
pub struct DuplicateStartedStatus {
    pub job_id: String,
}

#[derive(Serialize, Deserialize, Debug, Clone, Type)]
pub struct DuplicateProgressStatus {
    pub job_id: String,
    pub processed: u32,
    pub total: u32,
}

#[derive(Serialize, Deserialize, Debug, Clone, Type)]
pub struct DuplicateCompletedStatus {
    pub job_id: String,
    pub group_count: u32,
    pub wasted_bytes: f64,
}

#[derive(Serialize, Deserialize, Debug, Clone, Type)]
pub struct DuplicateFailedStatus {
    pub job_id: String,
    pub error: String,
}

#[derive(Serialize, Deserialize, Debug, Clone, Type)]
pub struct DuplicateCancelledStatus {
    pub job_id: String,
}

#[derive(Serialize, Deserialize, Debug, Clone, Type, Event)]
pub enum RenderStatus {
    Started(RenderStartedStatus),
    Progress(RenderProgressStatus),
    Completed(RenderCompletedStatus),
    Failed(RenderFailedStatus),
    Cancelled(RenderCancelledStatus),
}

#[derive(Serialize, Deserialize, Debug, Clone, Type)]
pub struct RenderStartedStatus {
    pub job_id: String,
    pub output_path: String,
}

#[derive(Serialize, Deserialize, Debug, Clone, Type)]
pub struct RenderProgressStatus {
    pub job_id: String,
    pub current_sample: u32,
    pub total_samples: u32,
    pub percent: u32,
}

#[derive(Serialize, Deserialize, Debug, Clone, Type)]
pub struct RenderCompletedStatus {
    pub job_id: String,
    pub output_path: String,
    pub elapsed_seconds: f64,
}

#[derive(Serialize, Deserialize, Debug, Clone, Type)]
pub struct RenderFailedStatus {
    pub job_id: String,
    pub error: String,
}

#[derive(Serialize, Deserialize, Debug, Clone, Type)]
pub struct RenderCancelledStatus {
    pub job_id: String,
}
