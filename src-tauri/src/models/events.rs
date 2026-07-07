use crate::models::BlenderInfo;
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
pub enum PackStatus {
    Started(PackStartedStatus),
    Progress(PackProgressStatus),
    Completed(PackCompletedStatus),
    Failed(PackFailedStatus),
    Cancelled(PackCancelledStatus),
}

#[derive(Serialize, Deserialize, Debug, Clone, Type)]
pub struct PackStartedStatus {
    pub job_id: String,
    /// "pack" or "unpack" — one event stream serves both directions.
    pub action: String,
    pub total_models: u32,
}

#[derive(Serialize, Deserialize, Debug, Clone, Type)]
pub struct PackProgressStatus {
    pub job_id: String,
    /// "compress" | "verify" | "extract" — what the current model is doing.
    pub phase: String,
    pub current_model: String,
    /// 1-based position of the current model in the batch.
    pub model_index: u32,
    pub total_models: u32,
    pub processed_size_kb: u32,
    pub total_size_kb: u32,
    pub percent: u32,
}

#[derive(Serialize, Deserialize, Debug, Clone, Type)]
pub struct PackCompletedStatus {
    pub job_id: String,
    pub action: String,
    pub succeeded: u32,
    pub total_models: u32,
    /// Loose files left in place because they changed since compression —
    /// surfaced so nothing silently stays behind.
    pub kept_files: Vec<String>,
    pub elapsed_seconds: f64,
}

#[derive(Serialize, Deserialize, Debug, Clone, Type)]
pub struct PackFailedStatus {
    pub job_id: String,
    pub error: String,
    /// Models finished before the failure — their state change is real and
    /// re-running the batch resumes after them.
    pub succeeded: u32,
}

#[derive(Serialize, Deserialize, Debug, Clone, Type)]
pub struct PackCancelledStatus {
    pub job_id: String,
    pub succeeded: u32,
}

#[derive(Serialize, Deserialize, Debug, Clone, Type, Event)]
pub enum BlenderProvisionStatus {
    Started(ProvisionStartedStatus),
    Progress(ProvisionProgressStatus),
    /// Post-download work; Progress stops once the bytes are all here.
    Extracting(ProvisionExtractingStatus),
    Completed(ProvisionCompletedStatus),
    Failed(ProvisionFailedStatus),
    Cancelled(ProvisionCancelledStatus),
}

#[derive(Serialize, Deserialize, Debug, Clone, Type)]
pub struct ProvisionStartedStatus {
    pub job_id: String,
    /// The pinned Blender being installed, e.g. "5.1.2".
    pub version: String,
}

#[derive(Serialize, Deserialize, Debug, Clone, Type)]
pub struct ProvisionProgressStatus {
    pub job_id: String,
    /// f64 like wasted_bytes above — u32 caps at 4 GB.
    pub downloaded_bytes: f64,
    pub total_bytes: f64,
    pub percent: u32,
}

#[derive(Serialize, Deserialize, Debug, Clone, Type)]
pub struct ProvisionExtractingStatus {
    pub job_id: String,
    /// "verify" | "extract" | "install" — mirrors PackProgressStatus.phase.
    pub phase: String,
}

#[derive(Serialize, Deserialize, Debug, Clone, Type)]
pub struct ProvisionCompletedStatus {
    pub job_id: String,
    pub info: BlenderInfo,
}

#[derive(Serialize, Deserialize, Debug, Clone, Type)]
pub struct ProvisionFailedStatus {
    pub job_id: String,
    pub error: String,
}

#[derive(Serialize, Deserialize, Debug, Clone, Type)]
pub struct ProvisionCancelledStatus {
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
