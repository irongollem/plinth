use serde::{Deserialize, Serialize};
use specta::Type;
use tauri_specta::Event;

#[derive(Serialize, Deserialize, Debug, Clone, Type, Event)]
pub struct CompressionProgessEvent {
    pub processed_files: u32,
    pub total_files: u32,
    pub processed_size: u32,
    pub total_size: u32,
    pub percent_size: u32,
    pub percent_files: u32,
}
