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
}

// ---- frontend-facing types ----

#[derive(Serialize, Deserialize, Clone, Debug, Type)]
pub struct CatalogEntry {
    pub dir_path: String,
    pub name: String,
    pub description: Option<String>,
    pub designer: Option<String>,
    pub release_name: Option<String>,
    pub preview_path: Option<String>,
    pub tags: Vec<String>,
    pub file_count: u32,
    pub total_size_bytes: f64,
}

#[derive(Serialize, Deserialize, Clone, Debug, Type)]
pub struct CatalogSearchResult {
    pub entries: Vec<CatalogEntry>,
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
