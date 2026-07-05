pub mod commands;
pub mod db;
pub mod dups;
pub mod scanner;

use serde::{Deserialize, Serialize};
use specta::Type;

/// Extensions treated as printable model files during scans.
pub const MODEL_EXTENSIONS: &[&str] = &[
    "stl", "obj", "3mf", "lys", "chitu", "chitubox", "blend", "gcode",
];
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
    pub variant: Option<String>,
    pub pose: Option<String>,
    pub scale: Option<String>,
    pub support_status: Option<String>,
    pub release_date: Option<String>,
    pub sculptor: Option<String>,
    /// The logical model this row is a variant of; rows sharing it collapse
    /// into one catalog group (see db::search_groups).
    pub group_name: Option<String>,
}

/// A per-file pose/variant assignment imported from a model.json's
/// `file_poses` — the scanner resolves each entry to a scanned file path.
#[derive(Debug, Clone)]
pub struct FileVariantRow {
    pub path: String,
    pub variant: Option<String>,
    pub pose: Option<String>,
    pub support_status: Option<String>,
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
    /// The facet between support and pose — a weapon/sculpt option
    /// (sword, mounted, …). Free-text, user-supplied (no scanner inference).
    pub variant: Option<String>,
    pub pose: Option<String>,
    pub scale: Option<String>,
    pub support_status: Option<String>,
    pub release_date: Option<String>,
    pub custom_name: Option<String>,
    /// The studio/brand. Scanned from the release for release models, and
    /// user-overridable per model. sculptor (the individual artist) has no
    /// folder signal, so it comes only from the user or an imported manifest.
    pub sculptor: Option<String>,
    /// Set only on members synthesized from file→pose assignments: a stable
    /// `{dir_path}\u{1f}{pose}` handle ("...\u{1f}" for the residual
    /// unassigned member). None means a whole-folder member — its dir_path
    /// alone identifies it. The UI keys members on `variant_key ?? dir_path`
    /// and passes it back to fetch only that pose's files.
    pub variant_key: Option<String>,
    /// The scanner-level group this member belongs to — the unit a combine
    /// maps and a detach removes. Differs from the card's group_name when
    /// the member is only in the card via a rename/combine.
    pub source_group: String,
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

/// The user-editable metadata for one model, saved together from the drawer.
/// A struct rather than positional args because specta caps command arity.
#[derive(Serialize, Deserialize, Clone, Debug, Type)]
pub struct ModelMetaUpdate {
    pub custom_name: Option<String>,
    pub variant: Option<String>,
    pub pose: Option<String>,
    pub scale: Option<String>,
    pub support_status: Option<String>,
    pub release_date: Option<String>,
    pub designer: Option<String>,
    pub sculptor: Option<String>,
    pub release_name: Option<String>,
}

/// A user's per-file pose assignment for a "dump everything in one folder"
/// model. Purely metadata (keyed by path, rescan-safe): the file stays put
/// on disk, but the catalog fans the folder out into one member per pose.
#[derive(Serialize, Deserialize, Clone, Debug, Type)]
pub struct FileVariant {
    pub path: String,
    pub dir_path: String,
    pub variant: Option<String>,
    pub pose: Option<String>,
    pub support_status: Option<String>,
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
    /// How many physical copies the paths actually occupy on disk. Hardlinked
    /// paths count once, so a fully merged group reports 1 ("shared", nothing
    /// to reclaim) and reclaimable space is size_bytes × (distinct_copies − 1).
    pub distinct_copies: u32,
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
