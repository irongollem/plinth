pub mod commands;
pub mod db;
pub mod dups;
pub mod scanner;

use serde::{Deserialize, Serialize};
use specta::Type;

/// Extensions treated as printable model files during scans.
pub const MODEL_EXTENSIONS: &[&str] = &["stl", "obj", "3mf", "lys", "chitubox", "blend", "gcode"];
/// Extensions usable as a model preview image.
pub const IMAGE_EXTENSIONS: &[&str] = &["jpg", "jpeg", "png", "webp", "gif"];

// ---- internal scan rows ----

#[derive(Debug, Clone)]
pub struct FileRow {
    pub path: String,
    pub dir_path: String,
    pub file_name: String,
    pub extension: String,
    pub size_bytes: i64,
    pub modified_at: i64,
}

#[derive(Debug, Clone)]
pub struct ModelRow {
    pub dir_path: String,
    pub name: String,
    pub description: Option<String>,
    pub designer: Option<String>,
    pub release_name: Option<String>,
    pub preview_path: Option<String>,
    pub source: String,
    pub uuid: Option<String>,
    pub file_count: u32,
    pub total_size_bytes: i64,
    pub pose: Option<String>,
    pub scale: Option<String>,
    pub support_status: Option<String>,
    pub release_date: Option<String>,
    /// The logical model this row is a variant of; rows sharing it collapse
    /// into one catalog group (see db::search_groups).
    pub group_name: Option<String>,
}

// ---- frontend-facing types ----

#[derive(Serialize, Deserialize, Clone, Debug, Type)]
pub struct CatalogEntry {
    pub dir_path: String,
    /// Effective display name: the user's custom_name when set, else the
    /// scanner's. custom_name below carries the raw override so the UI can
    /// distinguish "renamed" from "inferred" (and clear it to revert).
    pub name: String,
    pub description: Option<String>,
    pub designer: Option<String>,
    pub release_name: Option<String>,
    pub preview_path: Option<String>,
    pub tags: Vec<String>,
    pub file_count: u32,
    pub total_size_bytes: f64,
    pub pose: Option<String>,
    pub scale: Option<String>,
    pub support_status: Option<String>,
    pub release_date: Option<String>,
    pub custom_name: Option<String>,
}

#[derive(Serialize, Deserialize, Clone, Debug, Type)]
pub struct CatalogSearchResult {
    pub entries: Vec<CatalogEntry>,
    pub total: u32,
}

/// One logical model: every variant dir (supported/unsupported builds,
/// poses) sharing a group_name, aggregated for the card view. Drill in
/// with get_catalog_group_members.
#[derive(Serialize, Deserialize, Clone, Debug, Type)]
pub struct CatalogGroup {
    pub group_name: String,
    pub designer: Option<String>,
    pub variant_count: u32,
    pub pose_count: u32,
    pub support_statuses: Vec<String>,
    pub file_count: u32,
    pub total_size_bytes: f64,
    pub preview_path: Option<String>,
}

#[derive(Serialize, Deserialize, Clone, Debug, Type)]
pub struct CatalogGroupResult {
    pub groups: Vec<CatalogGroup>,
    pub total: u32,
}

#[derive(Serialize, Deserialize, Clone, Debug, Type)]
pub struct CatalogFile {
    pub path: String,
    pub file_name: String,
    pub extension: String,
    pub size_bytes: f64,
}

#[derive(Serialize, Deserialize, Clone, Debug, Type)]
pub struct ExtensionStat {
    pub extension: String,
    pub file_count: u32,
    pub total_size_bytes: f64,
}

#[derive(Serialize, Deserialize, Clone, Debug, Type)]
pub struct CatalogStats {
    pub total_models: u32,
    pub total_files: u32,
    pub total_size_bytes: f64,
    pub extensions: Vec<ExtensionStat>,
    pub last_scan_epoch: Option<f64>,
}

#[derive(Serialize, Deserialize, Clone, Debug, Type)]
pub struct DuplicateGroup {
    pub hash: String,
    pub size_bytes: f64,
    pub paths: Vec<String>,
}

#[derive(Serialize, Deserialize, Clone, Debug, Type)]
pub struct TagCount {
    pub tag: String,
    pub count: u32,
}

/// One requested directory move: rename `from` to `to` on disk and repoint
/// the catalog index to match.
#[derive(Serialize, Deserialize, Clone, Debug, Type)]
pub struct MoveOperation {
    pub from: String,
    pub to: String,
}

/// Outcome of a batch that may partially succeed — the counts and the
/// per-item errors travel together so the UI can report both.
#[derive(Serialize, Deserialize, Clone, Debug, Type)]
pub struct BatchOutcome {
    pub succeeded: u32,
    pub errors: Vec<String>,
}

/// One distinct release_name found across scanned models — a read-only
/// aggregation over already-indexed data, NOT a persisted publish log (the
/// catalog doesn't track a "finalized" event, so this just reflects what
/// scanning found on disk).
#[derive(Serialize, Deserialize, Clone, Debug, Type)]
pub struct ReleaseSummary {
    pub release_name: String,
    pub designer: Option<String>,
    pub model_count: u32,
    pub total_size_bytes: f64,
}
