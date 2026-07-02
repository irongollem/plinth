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
    /// Output PNG path (None = next to the first STL part)
    pub output_path: Option<String>,
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
}
