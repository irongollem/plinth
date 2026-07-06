pub(crate) mod events;

use serde::{Deserialize, Serialize};
use specta::Type;
use uuid::Uuid;

#[derive(Serialize, Deserialize, Clone, Debug, Type)]
pub struct Settings {
    pub scratch_dir: Option<String>,
    pub target_dir: Option<String>,
    pub compression_type: Option<CompressionType>,
    pub chunk_size: Option<u32>,
    pub max_compression_threads: Option<u32>,
    pub blender_path: Option<String>,
    pub catalog_root: Option<String>,
    /// Studios the scanner recognizes in folder names to infer a designer.
    /// Seeded from scanner::DEFAULT_DESIGNERS on first load; user-editable.
    pub known_designers: Option<Vec<String>>,
    /// What the catalog's print button does: "open-in-slicer" (default —
    /// hand the files to the OS-default slicer app) or "reveal-folder"
    /// (the drag-it-yourself flow for people juggling several slicers).
    pub print_action: Option<String>,
    /// Release-builder fields the user asked to keep across drafts (the
    /// "remember" checkboxes), keyed by field id — e.g. "designer" so
    /// creators don't retype their own name every release.
    pub release_field_defaults: Option<std::collections::HashMap<String, String>>,
}

#[derive(Serialize, Deserialize, Clone, Debug, Type)]
pub struct BlenderInfo {
    pub path: String,
    pub version: String,
}

#[derive(Serialize, Deserialize, Clone, Debug, Type)]
pub struct RenderOptions {
    /// Euler XYZ rotation in degrees, matching render_mini.py --rotate
    pub rotate: (f64, f64, f64),
    /// Linear RGB resin base color, matching --color (None = locked look default)
    pub color: Option<(f64, f64, f64)>,
    pub azimuth: Option<f64>,
    pub elevation: Option<f64>,
    pub zoom: Option<f64>,
    pub resolution: Option<u32>,
    pub samples: Option<u32>,
    /// Tonal look: "rich" (promo contrast) or "flat" (even lighting)
    pub look: Option<String>,
    /// Output PNG path (None = next to the first STL part)
    pub output_path: Option<String>,
    /// Allow replacing an existing file; when false an existing output gets
    /// a unique -N suffix instead of being clobbered
    #[serde(default)]
    pub overwrite: bool,
    /// Re-seat parts exported around different origins by stacking them on
    /// the part named *base* (render_mini.py --align-parts)
    #[serde(default)]
    pub align_parts: bool,
}

#[derive(Serialize, Deserialize, Clone, Debug, Type)]
pub enum CompressionType {
    SevenZip,
    Zip,
}

#[derive(Debug, Clone, Serialize, Deserialize, Type)]
pub struct Release {
    pub name: String,
    pub designer: String,
    pub description: String,
    pub date: String,
    pub version: String,
    pub model_references: Vec<ModelReference>,
    pub groups: Vec<String>,
    pub release_dir: String,
    pub images: Vec<String>,
    pub other_files: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Type)]
pub enum ModelLocation {
    Local(String),
    External(String),
}

#[derive(Debug, Clone, Serialize, Deserialize, Type)]
pub struct ModelReference {
    #[specta(type = String)]
    pub id: Uuid,
    pub location: ModelLocation,
}

/// A model as the release builder stages it and `model.json` records it.
/// The rich fields mirror the scanner's ModelJson reader — this is the WRITE
/// side of metadata portability (docs/3PK.md): whatever curation the catalog
/// holds rides into the sidecar, the manifest, and back out on another
/// user's scan. All optional with defaults so old sidecars still parse.
#[derive(Serialize, Deserialize, Clone, Debug, Type)]
pub struct StlModel {
    #[specta(type = Option<String>)]
    pub id: Option<Uuid>,
    pub name: String,
    pub description: Option<String>,
    pub tags: Vec<String>,
    pub images: Vec<String>, // the path of the temporary location of the image during archive creation
    pub model_files: Vec<String>, // the path of the temporary location of the model file during archive creation
    pub group: Option<String>,
    #[serde(default)]
    pub variant: Option<String>,
    #[serde(default)]
    pub pose: Option<String>,
    #[serde(default)]
    pub scale: Option<String>,
    #[serde(default)]
    pub support_status: Option<String>,
    #[serde(default)]
    pub release_date: Option<String>,
    #[serde(default)]
    pub designer: Option<String>,
    #[serde(default)]
    pub sculptor: Option<String>,
    #[serde(default)]
    pub release_name: Option<String>,
    /// Base sizes in mm, number only ("25", never "25mm"): round is the
    /// diameter, square the side. Both optional — plenty of models ship
    /// without a base at all. Additive to model.json/3pk.
    #[serde(default)]
    pub base_round_mm: Option<u32>,
    #[serde(default)]
    pub base_square_mm: Option<u32>,
    /// Per-file pose/variant assignments (a curated dump folder), restored
    /// into file_variants on scan. Names are file basenames.
    #[serde(default)]
    pub file_poses: Vec<FilePose>,
}

#[derive(Serialize, Deserialize, Clone, Debug, Type)]
pub struct FilePose {
    pub name: String,
    #[serde(default)]
    pub variant: Option<String>,
    #[serde(default)]
    pub pose: Option<String>,
    #[serde(default)]
    pub support_status: Option<String>,
}
